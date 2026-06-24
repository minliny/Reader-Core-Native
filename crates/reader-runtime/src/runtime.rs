use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use reader_contract::{
    core_info, methods, Command, CoreError, Event, HostCompleteParams, HostErrorParams,
    HostSmokeParams, RuntimeConfig,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostOperation {
    request_id: u64,
    state: HostOperationState,
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

        let worker_active = active_requests.clone();
        let worker_cancelled = cancelled.clone();
        let worker_host_operations = host_operations.clone();
        let worker_next_operation_id = next_operation_id.clone();
        let worker_shutdown = shutdown.clone();
        let worker_sink = sink.clone();

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
        if !contains_active(&self.active_requests, request_id) {
            return;
        }

        if let Ok(mut set) = self.cancelled.lock() {
            set.insert(request_id);
        }

        let removed_host_operation =
            remove_host_operations_for_request(&self.host_operations, request_id);
        if removed_host_operation {
            if let Ok(mut set) = self.cancelled.lock() {
                set.remove(&request_id);
            }
            remove_active(&self.active_requests, request_id);
            self.sink
                .emit(&Event::error(request_id, CoreError::cancelled()));
        }
    }

    fn worker_loop(
        rx: std::sync::mpsc::Receiver<WorkItem>,
        sink: Arc<dyn EventSink>,
        active_requests: Arc<Mutex<HashSet<u64>>>,
        cancelled: Arc<Mutex<HashSet<u64>>>,
        host_operations: Arc<Mutex<HashMap<u64, HostOperation>>>,
        next_operation_id: Arc<AtomicU64>,
        shutdown: Arc<AtomicBool>,
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
        methods::HOST_COMPLETE => {
            dispatch_host_complete(cmd, sink, active_requests, cancelled, host_operations)
        }
        methods::HOST_ERROR => {
            dispatch_host_error(cmd, sink, active_requests, cancelled, host_operations)
        }
        other => finish_request(
            sink,
            active_requests,
            cancelled,
            cmd.request_id,
            Event::error(cmd.request_id, CoreError::unknown_method(other)),
        ),
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

fn dispatch_host_complete(
    cmd: &Command,
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
    cancelled: &Mutex<HashSet<u64>>,
    host_operations: &Mutex<HashMap<u64, HostOperation>>,
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

    finish_request(
        sink,
        active_requests,
        cancelled,
        operation.request_id,
        Event::result(operation.request_id, params.result),
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
