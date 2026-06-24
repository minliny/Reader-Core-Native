use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use reader_contract::{
    core_info, methods, Command, CoreError, Event, HostCompleteParams, HostErrorParams,
    HostSmokeParams, RuntimeCancelParams, RuntimeConfig,
};

use crate::remote::{
    complete_remote_host, dispatch_remote, PendingHostRequest, RemoteDispatch,
    RemoteHostContinuation, RemoteState,
};
use crate::sink::EventSink;

/// C ABI version this runtime advertises via `core.info`. The authoritative
/// value lives with the FFI; mirrored here so the pure-Rust runtime can answer
/// `core.info` without depending on `reader-ffi`.
pub const ABI_VERSION: u32 = 1;

/// Build version string embedded in `core.info`.
const BUILD_VERSION: &str = concat!("reader-core-native ", env!("CARGO_PKG_VERSION"));

enum WorkItem {
    Command(Command),
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HostOperationState {
    Pending,
}

#[derive(Debug, Clone, PartialEq)]
enum HostOperationContinuation {
    Echo,
    Remote(RemoteHostContinuation),
}

#[derive(Debug, Clone, PartialEq)]
struct HostOperation {
    request_id: u64,
    state: HostOperationState,
    continuation: HostOperationContinuation,
}

/// A handle to a running Core runtime.
///
/// Owns the worker thread. Dropping [`Runtime`] is equivalent to
/// `rc_runtime_destroy`: it signals shutdown, drains the queue, and joins the
/// worker. Once dropped, the event sink is never invoked again.
pub struct Runtime {
    tx: std::sync::mpsc::Sender<WorkItem>,
    worker: Option<JoinHandle<()>>,
    sink: Arc<dyn EventSink>,
    config: RuntimeConfig,
    /// Request IDs accepted by [`Runtime::send`] and not yet completed.
    active_requests: Arc<Mutex<HashSet<u64>>>,
    /// Request IDs cancelled via [`Runtime::cancel`].
    cancelled: Arc<Mutex<HashSet<u64>>>,
    /// Pending host operations keyed by operationId.
    host_operations: Arc<Mutex<HashMap<u64, HostOperation>>>,
    /// Shared remote-reading state (content pipeline + in-memory storage).
    remote_state: Arc<RemoteState>,
    /// Shutdown latch so the worker can stop even mid-processing.
    shutdown: Arc<AtomicBool>,
}

impl Runtime {
    /// Spawn a new runtime with default config.
    pub fn new(sink: Arc<dyn EventSink>) -> Self {
        Self::new_with_config(sink, RuntimeConfig::default())
            .expect("default runtime config must validate")
    }

    /// Spawn a new runtime with a typed, already parsed config.
    pub fn new_with_config(
        sink: Arc<dyn EventSink>,
        config: RuntimeConfig,
    ) -> Result<Self, CoreError> {
        config.validate()?;

        let (tx, rx) = std::sync::mpsc::channel::<WorkItem>();
        let active_requests = Arc::new(Mutex::new(HashSet::new()));
        let cancelled = Arc::new(Mutex::new(HashSet::new()));
        let host_operations = Arc::new(Mutex::new(HashMap::new()));
        let next_operation_id = Arc::new(AtomicU64::new(1));
        let shutdown = Arc::new(AtomicBool::new(false));
        let remote_state = Arc::new(RemoteState::new());

        let worker_active = active_requests.clone();
        let worker_cancelled = cancelled.clone();
        let worker_host_operations = host_operations.clone();
        let worker_next_operation_id = next_operation_id.clone();
        let worker_shutdown = shutdown.clone();
        let worker_sink = sink.clone();
        let worker_remote_state = remote_state.clone();

        let worker = thread::Builder::new()
            .name("reader-core-worker".into())
            .spawn(move || {
                Self::worker_loop(
                    rx,
                    worker_sink,
                    worker_active,
                    worker_cancelled,
                    worker_host_operations,
                    worker_next_operation_id,
                    worker_shutdown,
                    worker_remote_state,
                );
            })
            .expect("reader-core worker thread spawn failed");

        Ok(Self {
            tx,
            worker: Some(worker),
            sink,
            config,
            active_requests,
            cancelled,
            host_operations,
            remote_state,
            shutdown,
        })
    }

    /// Parse JSON runtime config and spawn a runtime.
    pub fn new_with_config_json(
        sink: Arc<dyn EventSink>,
        config_json: &[u8],
    ) -> Result<Self, CoreError> {
        Self::new_with_config(sink, RuntimeConfig::from_json_bytes(config_json)?)
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    /// Shared remote-reading state. Exposed so tests (and hosts that embed the
    /// pure-Rust runtime) can inspect storage/cache contents after a vertical
    /// pipeline run. The FFI layer does not surface this; it is not part of the
    /// C ABI.
    pub fn remote_state(&self) -> &RemoteState {
        &self.remote_state
    }

    /// Parse and enqueue a JSON command payload.
    pub fn send_json(&self, command_json: &[u8]) -> Result<(), CoreError> {
        self.send(Command::from_json_bytes(command_json)?)
    }

    /// Enqueue a command. Returns `Err` for invalid protocol/message shape or
    /// if the runtime is shutting down.
    pub fn send(&self, command: Command) -> Result<(), CoreError> {
        command.validate()?;
        if self.shutdown.load(Ordering::Acquire) {
            return Err(CoreError::internal("runtime shutting down"));
        }

        {
            let mut active = self
                .active_requests
                .lock()
                .map_err(|_| CoreError::internal("active request registry poisoned"))?;
            if active.contains(&command.request_id) {
                return Err(CoreError::invalid_message("duplicate active requestId")
                    .with_details(serde_json::json!({ "requestId": command.request_id })));
            }
            active.insert(command.request_id);
        }

        match self.tx.send(WorkItem::Command(command.clone())) {
            Ok(()) => Ok(()),
            Err(_) => {
                remove_active(&self.active_requests, command.request_id);
                Err(CoreError::internal("runtime shutting down"))
            }
        }
    }

    /// Mark a request as cancelled. Unknown IDs are ignored per ABI contract.
    ///
    /// If the request is blocked on a host operation, cancellation completes it
    /// immediately with a `CANCELLED` error and removes the pending operation.
    /// If the command is still queued or currently dispatching, the worker
    /// emits the same structured error when it observes the cancellation.
    pub fn cancel(&self, request_id: u64) {
        cancel_request(
            &self.active_requests,
            &self.cancelled,
            &self.host_operations,
            &self.sink,
            request_id,
        );
    }

    fn worker_loop(
        rx: std::sync::mpsc::Receiver<WorkItem>,
        sink: Arc<dyn EventSink>,
        active_requests: Arc<Mutex<HashSet<u64>>>,
        cancelled: Arc<Mutex<HashSet<u64>>>,
        host_operations: Arc<Mutex<HashMap<u64, HostOperation>>>,
        next_operation_id: Arc<AtomicU64>,
        shutdown: Arc<AtomicBool>,
        remote_state: Arc<RemoteState>,
    ) {
        for item in &rx {
            if shutdown.load(Ordering::Acquire) {
                break;
            }
            match item {
                WorkItem::Shutdown => break,
                WorkItem::Command(cmd) => {
                    dispatch_command(
                        &cmd,
                        &sink,
                        &active_requests,
                        &cancelled,
                        &host_operations,
                        &next_operation_id,
                        &remote_state,
                    );
                }
            }
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = self.tx.send(WorkItem::Shutdown);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

fn dispatch_command(
    cmd: &Command,
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
    cancelled: &Mutex<HashSet<u64>>,
    host_operations: &Mutex<HashMap<u64, HostOperation>>,
    next_operation_id: &AtomicU64,
    remote_state: &RemoteState,
) {
    if take_cancelled(cancelled, cmd.request_id) {
        finish_request(
            sink,
            active_requests,
            cancelled,
            cmd.request_id,
            Event::error(cmd.request_id, CoreError::cancelled()),
        );
        return;
    }

    // Built-in host-bus / runtime commands first. Remote-reading vertical
    // commands are dispatched afterwards; if `dispatch_remote` returns false
    // (method not recognized by either), we fall through to unknown_method.
    match cmd.method.as_str() {
        methods::CORE_INFO => finish_request(
            sink,
            active_requests,
            cancelled,
            cmd.request_id,
            Event::result(cmd.request_id, core_info(ABI_VERSION, BUILD_VERSION)),
        ),
        methods::RUNTIME_PING | methods::LEGACY_CORE_PING => finish_request(
            sink,
            active_requests,
            cancelled,
            cmd.request_id,
            Event::result(
                cmd.request_id,
                serde_json::json!({ "pong": true, "method": methods::RUNTIME_PING }),
            ),
        ),
        methods::RUNTIME_HOST_SMOKE => dispatch_host_smoke(
            cmd,
            sink,
            active_requests,
            cancelled,
            host_operations,
            next_operation_id,
        ),
        methods::RUNTIME_CANCEL => dispatch_runtime_cancel(
            cmd,
            sink,
            active_requests,
            cancelled,
            host_operations,
        ),
        methods::HOST_COMPLETE => dispatch_host_complete(
            cmd,
            sink,
            active_requests,
            cancelled,
            host_operations,
            remote_state,
        ),
        methods::HOST_ERROR => {
            dispatch_host_error(cmd, sink, active_requests, cancelled, host_operations)
        }
        other => {
            match dispatch_remote(other, cmd, sink, active_requests, remote_state) {
                RemoteDispatch::Finished => return,
                RemoteDispatch::Pending(pending) => {
                    dispatch_remote_host_request(
                        cmd,
                        sink,
                        active_requests,
                        cancelled,
                        host_operations,
                        next_operation_id,
                        pending,
                    );
                    return;
                }
                RemoteDispatch::NotHandled => {}
            }
            finish_request(
                sink,
                active_requests,
                cancelled,
                cmd.request_id,
                Event::error(cmd.request_id, CoreError::unknown_method(other)),
            )
        }
    }
}

fn dispatch_host_smoke(
    cmd: &Command,
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
    cancelled: &Mutex<HashSet<u64>>,
    host_operations: &Mutex<HashMap<u64, HostOperation>>,
    next_operation_id: &AtomicU64,
) {
    let params = match parse_host_smoke_params(cmd) {
        Ok(params) => params,
        Err(error) => {
            finish_request(
                sink,
                active_requests,
                cancelled,
                cmd.request_id,
                Event::error(cmd.request_id, error),
            );
            return;
        }
    };
    if !params.params.is_object() {
        finish_request(
            sink,
            active_requests,
            cancelled,
            cmd.request_id,
            Event::error(
                cmd.request_id,
                CoreError::invalid_params("runtime.hostSmoke params.params must be a JSON object"),
            ),
        );
        return;
    }

    let operation_id = next_operation_id.fetch_add(1, Ordering::AcqRel);
    if let Ok(mut operations) = host_operations.lock() {
        operations.insert(
            operation_id,
            HostOperation {
                request_id: cmd.request_id,
                state: HostOperationState::Pending,
                continuation: HostOperationContinuation::Echo,
            },
        );
    } else {
        finish_request(
            sink,
            active_requests,
            cancelled,
            cmd.request_id,
            Event::error(
                cmd.request_id,
                CoreError::internal("host operation registry poisoned"),
            ),
        );
        return;
    }

    if take_cancelled(cancelled, cmd.request_id) {
        remove_operation(host_operations, operation_id);
        finish_request(
            sink,
            active_requests,
            cancelled,
            cmd.request_id,
            Event::error(cmd.request_id, CoreError::cancelled()),
        );
        return;
    }

    sink.emit(&Event::host_request(
        cmd.request_id,
        operation_id,
        params.capability,
        params.params,
    ));
}

fn dispatch_runtime_cancel(
    cmd: &Command,
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
    cancelled: &Mutex<HashSet<u64>>,
    host_operations: &Mutex<HashMap<u64, HostOperation>>,
) {
    let params: RuntimeCancelParams = match serde_json::from_value(cmd.params.clone()) {
        Ok(params) => params,
        Err(err) => {
            finish_request(
                sink,
                active_requests,
                cancelled,
                cmd.request_id,
                Event::error(
                    cmd.request_id,
                    CoreError::invalid_params(format!("invalid params for {}", cmd.method))
                        .with_details(serde_json::json!({ "source": err.to_string() })),
                ),
            );
            return;
        }
    };

    // Self-cancellation would race the cancel command's own completion and
    // produce a CANCELLED error instead of a result; reject it explicitly so
    // hosts get a predictable outcome.
    if params.request_id == cmd.request_id {
        finish_request(
            sink,
            active_requests,
            cancelled,
            cmd.request_id,
            Event::error(
                cmd.request_id,
                CoreError::invalid_params(
                    "runtime.cancel target requestId must differ from the command requestId",
                )
                .with_details(serde_json::json!({ "requestId": params.request_id })),
            ),
        );
        return;
    }

    let was_cancelled = cancel_request(
        active_requests,
        cancelled,
        host_operations,
        sink,
        params.request_id,
    );

    finish_request(
        sink,
        active_requests,
        cancelled,
        cmd.request_id,
        Event::result(
            cmd.request_id,
            serde_json::json!({ "cancelled": was_cancelled }),
        ),
    );
}

fn dispatch_remote_host_request(
    cmd: &Command,
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
    cancelled: &Mutex<HashSet<u64>>,
    host_operations: &Mutex<HashMap<u64, HostOperation>>,
    next_operation_id: &AtomicU64,
    pending: PendingHostRequest,
) {
    let operation_id = next_operation_id.fetch_add(1, Ordering::AcqRel);
    if let Ok(mut operations) = host_operations.lock() {
        operations.insert(
            operation_id,
            HostOperation {
                request_id: cmd.request_id,
                state: HostOperationState::Pending,
                continuation: HostOperationContinuation::Remote(pending.continuation),
            },
        );
    } else {
        finish_request(
            sink,
            active_requests,
            cancelled,
            cmd.request_id,
            Event::error(
                cmd.request_id,
                CoreError::internal("host operation registry poisoned"),
            ),
        );
        return;
    }

    if take_cancelled(cancelled, cmd.request_id) {
        remove_operation(host_operations, operation_id);
        finish_request(
            sink,
            active_requests,
            cancelled,
            cmd.request_id,
            Event::error(cmd.request_id, CoreError::cancelled()),
        );
        return;
    }

    sink.emit(&Event::host_request(
        cmd.request_id,
        operation_id,
        pending.capability,
        pending.params,
    ));
}

fn dispatch_host_complete(
    cmd: &Command,
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
    cancelled: &Mutex<HashSet<u64>>,
    host_operations: &Mutex<HashMap<u64, HostOperation>>,
    remote_state: &RemoteState,
) {
    let params = match parse_host_complete_params(cmd) {
        Ok(params) => params,
        Err(error) => {
            finish_request(
                sink,
                active_requests,
                cancelled,
                cmd.request_id,
                Event::error(cmd.request_id, error),
            );
            return;
        }
    };
    if !params.result.is_object() {
        finish_request(
            sink,
            active_requests,
            cancelled,
            cmd.request_id,
            Event::error(
                cmd.request_id,
                CoreError::invalid_params("host.complete result must be a JSON object"),
            ),
        );
        return;
    }

    remove_active(active_requests, cmd.request_id);
    let Some(operation) = take_host_operation(host_operations, params.operation_id) else {
        sink.emit(&Event::error(
            cmd.request_id,
            CoreError::host_operation_not_found(params.operation_id),
        ));
        return;
    };

    let event = match operation.continuation {
        HostOperationContinuation::Echo => Event::result(operation.request_id, params.result),
        HostOperationContinuation::Remote(continuation) => {
            match complete_remote_host(continuation, params.result, remote_state) {
                Ok(data) => Event::result(operation.request_id, data),
                Err(error) => Event::error(operation.request_id, error),
            }
        }
    };

    finish_request(
        sink,
        active_requests,
        cancelled,
        operation.request_id,
        event,
    );
}

fn dispatch_host_error(
    cmd: &Command,
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
    cancelled: &Mutex<HashSet<u64>>,
    host_operations: &Mutex<HashMap<u64, HostOperation>>,
) {
    let params = match parse_host_error_params(cmd) {
        Ok(params) => params,
        Err(error) => {
            finish_request(
                sink,
                active_requests,
                cancelled,
                cmd.request_id,
                Event::error(cmd.request_id, error),
            );
            return;
        }
    };

    remove_active(active_requests, cmd.request_id);
    let Some(operation) = take_host_operation(host_operations, params.operation_id) else {
        sink.emit(&Event::error(
            cmd.request_id,
            CoreError::host_operation_not_found(params.operation_id),
        ));
        return;
    };

    finish_request(
        sink,
        active_requests,
        cancelled,
        operation.request_id,
        Event::error(operation.request_id, params.error),
    );
}

fn parse_host_smoke_params(cmd: &Command) -> Result<HostSmokeParams, CoreError> {
    serde_json::from_value::<HostSmokeParams>(cmd.params.clone()).map_err(|err| {
        CoreError::invalid_params(format!("invalid params for {}", cmd.method)).with_details(
            serde_json::json!({
                "source": err.to_string(),
                "method": cmd.method,
            }),
        )
    })
}

fn parse_host_complete_params(cmd: &Command) -> Result<HostCompleteParams, CoreError> {
    serde_json::from_value::<HostCompleteParams>(cmd.params.clone()).map_err(|err| {
        CoreError::invalid_params(format!("invalid params for {}", cmd.method)).with_details(
            serde_json::json!({
                "source": err.to_string(),
                "method": cmd.method,
            }),
        )
    })
}

fn parse_host_error_params(cmd: &Command) -> Result<HostErrorParams, CoreError> {
    serde_json::from_value::<HostErrorParams>(cmd.params.clone()).map_err(|err| {
        CoreError::invalid_params(format!("invalid params for {}", cmd.method)).with_details(
            serde_json::json!({
                "source": err.to_string(),
                "method": cmd.method,
            }),
        )
    })
}

fn finish_request(
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
    cancelled: &Mutex<HashSet<u64>>,
    request_id: u64,
    event: Event,
) {
    let event = if take_cancelled(cancelled, request_id) {
        Event::error(request_id, CoreError::cancelled())
    } else {
        event
    };
    remove_active(active_requests, request_id);
    sink.emit(&event);
}

fn contains_active(active_requests: &Mutex<HashSet<u64>>, request_id: u64) -> bool {
    active_requests
        .lock()
        .map(|active| active.contains(&request_id))
        .unwrap_or(false)
}

fn remove_active(active_requests: &Mutex<HashSet<u64>>, request_id: u64) {
    if let Ok(mut active) = active_requests.lock() {
        active.remove(&request_id);
    }
}

fn take_cancelled(cancelled: &Mutex<HashSet<u64>>, request_id: u64) -> bool {
    cancelled
        .lock()
        .map(|mut set| set.remove(&request_id))
        .unwrap_or(false)
}

fn take_host_operation(
    host_operations: &Mutex<HashMap<u64, HostOperation>>,
    operation_id: u64,
) -> Option<HostOperation> {
    let operation = host_operations.lock().ok()?.remove(&operation_id)?;
    match operation.state {
        HostOperationState::Pending => Some(operation),
    }
}

fn remove_operation(host_operations: &Mutex<HashMap<u64, HostOperation>>, operation_id: u64) {
    if let Ok(mut operations) = host_operations.lock() {
        operations.remove(&operation_id);
    }
}

fn remove_host_operations_for_request(
    host_operations: &Mutex<HashMap<u64, HostOperation>>,
    request_id: u64,
) -> bool {
    let Ok(mut operations) = host_operations.lock() else {
        return false;
    };
    let before = operations.len();
    operations.retain(|_, operation| operation.request_id != request_id);
    operations.len() != before
}

/// Core cancellation logic shared by the C ABI path ([`Runtime::cancel`]) and
/// the JSON `runtime.cancel` command. Returns `true` if `request_id` was active
/// and got cancelled.
///
/// - If the request is blocked on a host operation, the operation is removed
///   and a `CANCELLED` error is emitted immediately for `request_id`.
/// - If the request is still queued or currently dispatching, it is marked in
///   the cancelled set; the worker emits the `CANCELLED` error when it
///   observes the mark.
/// - Unknown / already-completed IDs are no-ops and return `false`.
fn cancel_request(
    active_requests: &Mutex<HashSet<u64>>,
    cancelled: &Mutex<HashSet<u64>>,
    host_operations: &Mutex<HashMap<u64, HostOperation>>,
    sink: &Arc<dyn EventSink>,
    request_id: u64,
) -> bool {
    if !contains_active(active_requests, request_id) {
        return false;
    }

    if let Ok(mut set) = cancelled.lock() {
        set.insert(request_id);
    }

    let removed_host_operation = remove_host_operations_for_request(host_operations, request_id);
    if removed_host_operation {
        if let Ok(mut set) = cancelled.lock() {
            set.remove(&request_id);
        }
        remove_active(active_requests, request_id);
        sink.emit(&Event::error(request_id, CoreError::cancelled()));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use reader_contract::{ErrorCode, PROTOCOL_VERSION, V1_CAPABILITIES};
    use std::sync::Condvar;
    use std::sync::Mutex as StdMutex;
    use std::time::{Duration, Instant};

    struct CollectSink {
        events: StdMutex<Vec<Event>>,
        ready: Condvar,
    }

    impl CollectSink {
        fn new() -> Self {
            Self {
                events: StdMutex::new(Vec::new()),
                ready: Condvar::new(),
            }
        }

        fn wait_len(&self, len: usize) -> Vec<Event> {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut events = self.events.lock().unwrap();
            while events.len() < len {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                let timeout = deadline.saturating_duration_since(now);
                let (guard, _) = self.ready.wait_timeout(events, timeout).unwrap();
                events = guard;
            }
            events.clone()
        }

        fn snapshot(&self) -> Vec<Event> {
            self.events.lock().unwrap().clone()
        }
    }

    impl EventSink for CollectSink {
        fn emit(&self, event: &Event) {
            self.events.lock().unwrap().push(event.clone());
            self.ready.notify_all();
        }
    }

    #[test]
    fn protocol_version_rejected_before_enqueue() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let mut command = Command::new(1, methods::RUNTIME_PING, serde_json::json!({}));
        command.protocol_version = PROTOCOL_VERSION + 1;

        let err = rt.send(command).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidProtocolVersion);
        assert!(sink.wait_len(1).is_empty());
    }

    #[test]
    fn ping_round_trips() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(
            1,
            methods::RUNTIME_PING,
            serde_json::json!({}),
        ))
        .unwrap();

        let events = sink.wait_len(1);
        match &events[0] {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(*request_id, 1);
                assert_eq!(data["pong"], true);
                assert_eq!(data["method"], methods::RUNTIME_PING);
            }
            other => panic!("expected result, got {other:?}"),
        }
    }

    #[test]
    fn core_info_advertises_v1_capabilities() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(2, methods::CORE_INFO, serde_json::json!({})))
            .unwrap();

        let events = sink.wait_len(1);
        match &events[0] {
            Event::Result { data, .. } => {
                assert_eq!(data["protocolVersion"], PROTOCOL_VERSION);
                for capability in [
                    methods::CORE_INFO,
                    methods::RUNTIME_PING,
                    methods::RUNTIME_HOST_SMOKE,
                    methods::HOST_COMPLETE,
                    methods::HOST_ERROR,
                ] {
                    assert!(
                        data["capabilities"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|value| value == capability),
                        "missing capability {capability}; got {}",
                        data["capabilities"]
                    );
                }
                assert!(!V1_CAPABILITIES.contains(&methods::LEGACY_CORE_PING));
            }
            other => panic!("expected result, got {other:?}"),
        }
    }

    #[test]
    fn unknown_method_yields_error() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(7, "bogus.method", serde_json::json!({})))
            .unwrap();

        let events = sink.wait_len(1);
        match &events[0] {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(*request_id, 7);
                assert_eq!(error.code, ErrorCode::UnknownMethod);
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn send_json_malformed_command_returns_structured_error() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());

        let err = rt
            .send_json(
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/invalid-malformed-json.json"
                )
                .as_bytes(),
            )
            .unwrap_err();

        assert_eq!(err.code, ErrorCode::InvalidMessage);
        assert!(sink.wait_len(1).is_empty());
    }

    #[test]
    fn send_json_missing_request_id_returns_structured_error() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());

        let err = rt
            .send_json(
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/invalid-missing-request-id.json"
                )
                .as_bytes(),
            )
            .unwrap_err();

        assert_eq!(err.code, ErrorCode::InvalidMessage);
        assert!(sink.wait_len(1).is_empty());
    }

    #[test]
    fn duplicate_active_request_id_is_rejected_before_enqueue() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(
            90,
            methods::RUNTIME_HOST_SMOKE,
            serde_json::json!({}),
        ))
        .unwrap();

        let events = sink.wait_len(1);
        assert!(matches!(events[0], Event::HostRequest { .. }));

        let err = rt
            .send(Command::new(
                90,
                methods::RUNTIME_PING,
                serde_json::json!({}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidMessage);
        assert_eq!(sink.snapshot().len(), 1);
    }

    #[test]
    fn host_request_event_shape_and_completion_route_to_original_request() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(
            10,
            methods::RUNTIME_HOST_SMOKE,
            serde_json::json!({
                "capability": "host.smoke.echo",
                "params": { "url": "https://example.invalid" }
            }),
        ))
        .unwrap();

        let events = sink.wait_len(1);
        let operation_id = match &events[0] {
            Event::HostRequest {
                request_id,
                operation_id,
                capability,
                params,
                ..
            } => {
                assert_eq!(*request_id, 10);
                assert_eq!(*operation_id, 1);
                assert_eq!(capability, "host.smoke.echo");
                assert_eq!(params["url"], "https://example.invalid");
                *operation_id
            }
            other => panic!("expected host.request, got {other:?}"),
        };

        rt.send(Command::new(
            11,
            methods::HOST_COMPLETE,
            serde_json::json!({
                "operationId": operation_id,
                "result": { "status": "ok" }
            }),
        ))
        .unwrap();

        let events = sink.wait_len(2);
        match &events[1] {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(*request_id, 10);
                assert_eq!(data["status"], "ok");
            }
            other => panic!("expected original request result, got {other:?}"),
        }
    }

    #[test]
    fn host_error_routes_error_to_original_request() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send_json(
            include_str!("../../../protocol/fixtures/conformance/host/request.json").as_bytes(),
        )
        .unwrap();

        let events = sink.wait_len(1);
        assert!(matches!(events[0], Event::HostRequest { .. }));

        rt.send_json(
            include_str!("../../../protocol/fixtures/conformance/host/error.json").as_bytes(),
        )
        .unwrap();

        let events = sink.wait_len(2);
        match &events[1] {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(*request_id, 301);
                assert_eq!(error.code, ErrorCode::Internal);
                assert!(error.retryable);
            }
            other => panic!("expected original request error, got {other:?}"),
        }
    }

    #[test]
    fn unknown_host_completion_returns_error_for_completion_request() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send_json(
            include_str!("../../../protocol/fixtures/conformance/host/unknown-complete.json")
                .as_bytes(),
        )
        .unwrap();

        let events = sink.wait_len(1);
        match &events[0] {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(*request_id, 304);
                assert_eq!(error.code, ErrorCode::InvalidParams);
                assert_eq!(error.details["operationId"], 404);
            }
            other => panic!("expected completion request error, got {other:?}"),
        }
    }

    #[test]
    fn repeated_host_completion_after_operation_completed_is_rejected() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send_json(
            include_str!("../../../protocol/fixtures/conformance/host/request.json").as_bytes(),
        )
        .unwrap();
        assert!(matches!(sink.wait_len(1)[0], Event::HostRequest { .. }));

        rt.send_json(
            include_str!("../../../protocol/fixtures/conformance/host/complete.json").as_bytes(),
        )
        .unwrap();
        let events = sink.wait_len(2);
        assert!(matches!(events[1], Event::Result { .. }));

        rt.send_json(
            include_str!("../../../protocol/fixtures/conformance/host/complete.json").as_bytes(),
        )
        .unwrap();
        let events = sink.wait_len(3);
        match &events[2] {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(*request_id, 302);
                assert_eq!(error.code, ErrorCode::InvalidParams);
                assert_eq!(error.details["operationId"], 1);
            }
            other => panic!("expected duplicate completion error, got {other:?}"),
        }
    }

    #[test]
    fn cancelling_pending_host_request_emits_cancelled() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(
            20,
            methods::RUNTIME_HOST_SMOKE,
            serde_json::json!({}),
        ))
        .unwrap();

        let events = sink.wait_len(1);
        assert!(matches!(events[0], Event::HostRequest { .. }));

        rt.cancel(20);
        let events = sink.wait_len(2);
        match &events[1] {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(*request_id, 20);
                assert_eq!(error.code, ErrorCode::Cancelled);
            }
            other => panic!("expected cancellation error, got {other:?}"),
        }
    }

    #[test]
    fn cancel_unknown_and_completed_requests_is_idempotent() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());

        rt.cancel(404);
        rt.cancel(404);
        std::thread::sleep(Duration::from_millis(20));
        assert!(sink.snapshot().is_empty());

        rt.send(Command::new(
            401,
            methods::RUNTIME_PING,
            serde_json::json!({}),
        ))
        .unwrap();
        let events = sink.wait_len(1);
        assert!(matches!(events[0], Event::Result { .. }));

        rt.cancel(401);
        rt.cancel(401);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(sink.snapshot().len(), 1);
    }

    // --- runtime.cancel JSON command ---------------------------------------

    #[test]
    fn runtime_cancel_command_cancels_pending_host_request() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        // Block request 50 on a host operation.
        rt.send(Command::new(
            50,
            methods::RUNTIME_HOST_SMOKE,
            serde_json::json!({
                "capability": "host.smoke.echo",
                "params": { "url": "https://example.invalid" }
            }),
        ))
        .unwrap();
        assert!(matches!(sink.wait_len(1)[0], Event::HostRequest { .. }));

        // Cancel request 50 via the JSON protocol command.
        rt.send(Command::new(
            51,
            methods::RUNTIME_CANCEL,
            serde_json::json!({ "requestId": 50 }),
        ))
        .unwrap();

        let events = sink.wait_len(3);
        // events[1]: CANCELLED error for the original request 50.
        match &events[1] {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(*request_id, 50);
                assert_eq!(error.code, ErrorCode::Cancelled);
            }
            other => panic!("expected cancelled error for 50, got {other:?}"),
        }
        // events[2]: result for the cancel command itself.
        match &events[2] {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(*request_id, 51);
                assert_eq!(data["cancelled"], true);
            }
            other => panic!("expected cancel result, got {other:?}"),
        }

        // The host operation is gone: a late host.complete for it is rejected.
        rt.send(Command::new(
            52,
            methods::HOST_COMPLETE,
            serde_json::json!({
                "operationId": 1,
                "result": { "status": "ok" }
            }),
        ))
        .unwrap();
        match &sink.wait_len(4)[3] {
            Event::Error { error, .. } => assert_eq!(error.code, ErrorCode::InvalidParams),
            other => panic!("expected unknown operation error, got {other:?}"),
        }
    }

    #[test]
    fn runtime_cancel_command_for_unknown_id_returns_false() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(
            60,
            methods::RUNTIME_CANCEL,
            serde_json::json!({ "requestId": 9999 }),
        ))
        .unwrap();

        let events = sink.wait_len(1);
        match &events[0] {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(*request_id, 60);
                assert_eq!(data["cancelled"], false);
            }
            other => panic!("expected cancel result false, got {other:?}"),
        }
    }

    #[test]
    fn runtime_cancel_command_for_completed_request_returns_false() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        // 70 completes immediately.
        rt.send(Command::new(
            70,
            methods::RUNTIME_PING,
            serde_json::json!({}),
        ))
        .unwrap();
        assert!(matches!(sink.wait_len(1)[0], Event::Result { .. }));

        // Cancelling the now-completed 70 is a no-op.
        rt.send(Command::new(
            71,
            methods::RUNTIME_CANCEL,
            serde_json::json!({ "requestId": 70 }),
        ))
        .unwrap();
        match &sink.wait_len(2)[1] {
            Event::Result { data, .. } => assert_eq!(data["cancelled"], false),
            other => panic!("expected cancel result false, got {other:?}"),
        }
    }

    #[test]
    fn runtime_cancel_command_rejects_self_cancel() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(
            80,
            methods::RUNTIME_CANCEL,
            serde_json::json!({ "requestId": 80 }),
        ))
        .unwrap();

        match &sink.wait_len(1)[0] {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(*request_id, 80);
                assert_eq!(error.code, ErrorCode::InvalidParams);
                assert_eq!(error.details["requestId"], 80);
            }
            other => panic!("expected self-cancel invalid params, got {other:?}"),
        }
    }

    #[test]
    fn runtime_cancel_command_rejects_invalid_params() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(
            81,
            methods::RUNTIME_CANCEL,
            serde_json::json!({ "notRequestId": 1 }),
        ))
        .unwrap();

        match &sink.wait_len(1)[0] {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(*request_id, 81);
                assert_eq!(error.code, ErrorCode::InvalidParams);
            }
            other => panic!("expected invalid params, got {other:?}"),
        }
    }

    #[test]
    fn runtime_cancel_command_via_send_json_fixture() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send_json(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/valid-runtime-cancel.json"
            )
            .as_bytes(),
        )
        .unwrap();

        // Fixture cancels requestId 301, which is unknown here → cancelled:false.
        match &sink.wait_len(1)[0] {
            Event::Result { data, .. } => assert_eq!(data["cancelled"], false),
            other => panic!("expected cancel result, got {other:?}"),
        }
    }

    #[test]
    fn parses_runtime_config_boundary() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new_with_config_json(
            sink,
            br#"{"dataDirectory":"/tmp/reader-data","cacheDirectory":"/tmp/reader-cache"}"#,
        )
        .unwrap();

        assert_eq!(
            rt.config().data_directory.as_deref(),
            Some("/tmp/reader-data")
        );
        assert_eq!(
            rt.config().cache_directory.as_deref(),
            Some("/tmp/reader-cache")
        );
    }

    // --- Remote-reading vertical (V1 minimal) -------------------------------

    /// A source definition with JSONPath/CSS rules matching the test fixtures.
    fn vertical_source() -> serde_json::Value {
        serde_json::json!({
            "sourceId": "vtest-src",
            "name": "Vertical Test Source",
            "baseUrl": "https://books.example.test",
            "rules": {
                "search": [ { "kind": "jsonPath", "path": "$.books[*]" } ],
                "detail": [ { "kind": "jsonPath", "path": "$.detail" } ],
                "toc":   [ { "kind": "jsonPath", "path": "$.toc" } ],
                "chapter": [ { "kind": "cssText", "selector": "p" } ]
            }
        })
    }

    fn send_and_wait(rt: &Runtime, sink: &CollectSink, command: Command) -> Event {
        rt.send(command).unwrap();
        let events = sink.wait_len(1);
        // wait_len is cumulative for this sink instance; we only use fresh
        // sinks per test, so the single emitted event is the last one.
        events.into_iter().last().expect("at least one event")
    }

    #[test]
    fn source_import_succeeds_and_stores() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                1,
                methods::SOURCE_IMPORT,
                serde_json::json!({
                    "sourceId": "vtest-src",
                    "name": "Vertical Test Source",
                    "baseUrl": "https://books.example.test",
                    "rules": vertical_source()["rules"].clone(),
                }),
            ),
        );
        match event {
            Event::Result { data, .. } => {
                assert_eq!(data["imported"], true);
                assert_eq!(data["sourceId"], "vtest-src");
            }
            other => panic!("expected result, got {other:?}"),
        }
        // Stored in remote_state storage.
        let stored = rt
            .remote_state()
            .storage()
            .get_source("vtest-src")
            .unwrap()
            .expect("source stored");
        assert_eq!(stored.name, "Vertical Test Source");
    }

    #[test]
    fn source_import_rejects_empty_name() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                1,
                methods::SOURCE_IMPORT,
                serde_json::json!({
                    "sourceId": "bad",
                    "name": "  ",
                    "baseUrl": "",
                    "rules": serde_json::Value::Null,
                }),
            ),
        );
        match event {
            Event::Error { error, .. } => assert_eq!(error.code, ErrorCode::InvalidParams),
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn book_search_returns_books() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                2,
                methods::BOOK_SEARCH,
                serde_json::json!({
                    "sourceId": "vtest-src",
                    "searchResponse": "{\"books\":[{\"bookId\":\"1\",\"title\":\"Dune\",\"author\":\"Herbert\"}]}",
                    "source": vertical_source(),
                }),
            ),
        );
        match event {
            Event::Result { data, .. } => {
                let books = data["books"].as_array().expect("books array");
                assert_eq!(books.len(), 1);
                assert_eq!(books[0]["title"], "Dune");
            }
            other => panic!("expected result, got {other:?}"),
        }
    }

    #[test]
    fn book_search_fetches_response_through_host_http() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(
            21,
            methods::BOOK_SEARCH,
            serde_json::json!({
                "sourceId": "vtest-src",
                "searchRequest": {
                    "url": "https://books.example.test/search?q=dune",
                    "headers": { "Accept": "application/json" }
                },
                "source": vertical_source(),
            }),
        ))
        .unwrap();

        let events = sink.wait_len(1);
        let operation_id = match &events[0] {
            Event::HostRequest {
                request_id,
                operation_id,
                capability,
                params,
                ..
            } => {
                assert_eq!(*request_id, 21);
                assert_eq!(capability, "http.execute");
                assert_eq!(params["method"], "GET");
                assert_eq!(params["url"], "https://books.example.test/search?q=dune");
                assert_eq!(params["headers"]["Accept"], "application/json");
                *operation_id
            }
            other => panic!("expected http host request, got {other:?}"),
        };

        rt.send(Command::new(
            22,
            methods::HOST_COMPLETE,
            serde_json::json!({
                "operationId": operation_id,
                "result": {
                    "status": 200,
                    "body": "{\"books\":[{\"bookId\":\"1\",\"title\":\"Dune\",\"author\":\"Herbert\"}]}"
                }
            }),
        ))
        .unwrap();

        let events = sink.wait_len(2);
        match &events[1] {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(*request_id, 21);
                let books = data["books"].as_array().expect("books array");
                assert_eq!(books[0]["title"], "Dune");
            }
            other => panic!("expected remote result after host completion, got {other:?}"),
        }
    }

    #[test]
    fn book_search_requires_response_or_host_request() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                23,
                methods::BOOK_SEARCH,
                serde_json::json!({
                    "sourceId": "vtest-src",
                    "source": vertical_source(),
                }),
            ),
        );
        match event {
            Event::Error { error, .. } => {
                assert_eq!(error.code, ErrorCode::InvalidParams);
                assert!(error.message.contains("searchResponse"));
            }
            other => panic!("expected missing response error, got {other:?}"),
        }
    }

    #[test]
    fn remote_host_http_completion_requires_body_string() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(
            24,
            methods::BOOK_SEARCH,
            serde_json::json!({
                "sourceId": "vtest-src",
                "searchRequest": { "url": "https://books.example.test/search" },
                "source": vertical_source(),
            }),
        ))
        .unwrap();

        let operation_id = match &sink.wait_len(1)[0] {
            Event::HostRequest { operation_id, .. } => *operation_id,
            other => panic!("expected host request, got {other:?}"),
        };

        rt.send(Command::new(
            25,
            methods::HOST_COMPLETE,
            serde_json::json!({
                "operationId": operation_id,
                "result": { "status": 200 }
            }),
        ))
        .unwrap();

        let events = sink.wait_len(2);
        match &events[1] {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(*request_id, 24);
                assert_eq!(error.code, ErrorCode::InvalidParams);
                assert!(error.message.contains("result.body"));
            }
            other => panic!("expected original request error, got {other:?}"),
        }
    }

    #[test]
    fn book_detail_merges_metadata() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                3,
                methods::BOOK_DETAIL,
                serde_json::json!({
                    "sourceId": "vtest-src",
                    "book": { "bookId": "1" },
                    "detailResponse": "{\"detail\":{\"bookId\":\"1\",\"title\":\"Dune\",\"author\":\"Frank Herbert\",\"intro\":\"desert\"}}",
                    "source": vertical_source(),
                }),
            ),
        );
        match event {
            Event::Result { data, .. } => {
                assert_eq!(data["book"]["title"], "Dune");
                assert_eq!(data["book"]["author"], "Frank Herbert");
                assert_eq!(data["book"]["intro"], "desert");
            }
            other => panic!("expected result, got {other:?}"),
        }
    }

    #[test]
    fn book_toc_returns_entries_and_caches() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                4,
                methods::BOOK_TOC,
                serde_json::json!({
                    "sourceId": "vtest-src",
                    "bookId": "1",
                    "tocResponse": "{\"toc\":[{\"title\":\"C1\",\"url\":\"u1\"},{\"title\":\"C2\",\"url\":\"u2\"}]}",
                    "source": vertical_source(),
                }),
            ),
        );
        match event {
            Event::Result { data, .. } => {
                let toc = data["toc"].as_array().expect("toc array");
                assert_eq!(toc.len(), 2);
                assert_eq!(toc[0]["title"], "C1");
            }
            other => panic!("expected result, got {other:?}"),
        }
        // Cache write verified via storage.
        let cached = rt
            .remote_state()
            .storage()
            .get_cache("toc:1")
            .unwrap()
            .expect("toc cached");
        assert!(cached.payload.contains("C1"));
    }

    #[test]
    fn chapter_content_extracts_body() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                5,
                methods::CHAPTER_CONTENT,
                serde_json::json!({
                    "sourceId": "vtest-src",
                    "bookId": "1",
                    "chapterTitle": "C1",
                    "chapterResponse": "<html><body><p>One.</p><p>Two.</p></body></html>",
                    "source": vertical_source(),
                }),
            ),
        );
        match event {
            Event::Result { data, .. } => {
                assert_eq!(data["via"], "rule");
                assert_eq!(data["content"], "One.\nTwo.");
            }
            other => panic!("expected result, got {other:?}"),
        }
        let cached = rt
            .remote_state()
            .storage()
            .get_cache("chapter:1:C1")
            .unwrap()
            .expect("chapter cached");
        assert_eq!(cached.payload, "One.\nTwo.");
    }

    #[test]
    fn js_rule_unsupported_is_structured_not_fake_network() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                6,
                methods::CHAPTER_CONTENT,
                serde_json::json!({
                    "sourceId": "vtest-src",
                    "bookId": "1",
                    "chapterTitle": "C1",
                    "chapterResponse": "<p>x</p>",
                    "jsRule": "java.get(\"https://books.example.test/protected\")",
                    "source": vertical_source(),
                }),
            ),
        );
        match event {
            Event::Error { error, .. } => {
                assert_eq!(error.code, ErrorCode::Internal);
                assert_eq!(error.details["unsupported"], true);
                // Must never claim a network result happened.
                assert!(error.message.contains("unsupported"));
            }
            other => panic!("expected structured unsupported error, got {other:?}"),
        }
    }

    #[test]
    fn js_rule_success_path_returns_value() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        // A pure-computation JS rule with no host calls succeeds.
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                7,
                methods::CHAPTER_CONTENT,
                serde_json::json!({
                    "sourceId": "vtest-src",
                    "bookId": "1",
                    "chapterTitle": "C1",
                    "chapterResponse": "<p>x</p>",
                    "jsRule": "({ status: 'ok', words: 42 })",
                    "source": vertical_source(),
                }),
            ),
        );
        match event {
            Event::Result { data, .. } => {
                assert_eq!(data["via"], "js");
                assert_eq!(data["content"]["status"], "ok");
                assert_eq!(data["content"]["words"], 42);
            }
            other => panic!("expected js result, got {other:?}"),
        }
    }

    #[test]
    fn reading_progress_writes_and_reads_back() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                8,
                methods::READING_PROGRESS_UPDATE,
                serde_json::json!({
                    "bookId": "1",
                    "chapterIndex": 2,
                    "chapterOffset": 128,
                    "chapterProgress": 0.5,
                }),
            ),
        );
        match event {
            Event::Result { data, .. } => {
                assert_eq!(data["stored"], true);
                assert_eq!(data["chapterIndex"], 2);
            }
            other => panic!("expected result, got {other:?}"),
        }
        let progress = rt
            .remote_state()
            .storage()
            .get_progress("1")
            .unwrap()
            .expect("progress stored");
        assert_eq!(progress.chapter_index, 2);
        assert_eq!(progress.chapter_offset, 128);
    }

    #[test]
    fn unknown_method_still_errors_after_remote_dispatch() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(9, "totally.bogus.method", serde_json::json!({})),
        );
        match event {
            Event::Error { error, .. } => assert_eq!(error.code, ErrorCode::UnknownMethod),
            other => panic!("expected unknown method error, got {other:?}"),
        }
    }

    #[test]
    fn create_destroy_1000_times_no_leak_or_crash() {
        for i in 0..1000 {
            let sink = Arc::new(CollectSink::new());
            let rt = Runtime::new(sink);
            rt.send(Command::new(
                i,
                methods::RUNTIME_PING,
                serde_json::json!({}),
            ))
            .unwrap();
            drop(rt);
        }
    }
}
