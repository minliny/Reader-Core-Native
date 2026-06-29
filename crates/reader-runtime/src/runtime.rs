use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use reader_contract::{
    core_info, methods, Command, CoreError, EmptyParams, Event, HostCacheGetResponse,
    HostCachePutResponse, HostCapability, HostCompleteParams, HostCookieGetResponse,
    HostCookieSetResponse, HostErrorDiagnostics, HostErrorParams, HostFileReadResponse,
    HostFileWriteResponse, HostLogEmitResponse, HostPersistenceGetResponse,
    HostPersistencePutResponse, HostSmokeParams, HostSystemInfoResponse, HostTimeNowResponse,
    HostWebViewEvaluateJavaScriptResponse, PendingHostOperationStatus, RuntimeCancelParams,
    RuntimeConfig, RuntimeShutdownParams, RuntimeStatus, RuntimeStatusParams,
};

use crate::remote::{
    complete_remote_host, dispatch_remote, PendingHostRequest, RemoteCommandResult, RemoteDispatch,
    RemoteHostContinuation, RemoteState,
};
use crate::sink::EventSink;
use crate::tts::TtsState;

/// C ABI version this runtime advertises via `core.info`. The authoritative
/// value lives with the FFI; mirrored here so the pure-Rust runtime can answer
/// `core.info` without depending on `reader-ffi`.
pub const ABI_VERSION: u32 = 1;

/// Build version string embedded in `core.info`.
const BUILD_VERSION: &str = concat!("reader-core-native ", env!("CARGO_PKG_VERSION"));

enum WorkItem {
    Command(Command),
    ProtocolShutdown(Command),
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HostOperationState {
    Pending,
}

impl HostOperationState {
    fn as_protocol_str(&self) -> &'static str {
        match self {
            HostOperationState::Pending => "pending",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum HostOperationContinuation {
    Echo,
    Remote(RemoteHostContinuation),
}

#[derive(Debug, Clone, PartialEq)]
struct HostOperation {
    request_id: u64,
    capability: HostCapability,
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
    /// Shared TTS queue state (per-chapter playback state machine).
    tts_state: Arc<TtsState>,
    /// Shutdown latch so the worker can stop even mid-processing.
    shutdown: Arc<AtomicBool>,
    /// JS host-callback bridge (mirrored from [`RemoteState::bridge`]) used to
    /// intercept `host.complete`/`host.error` for operationIds emitted from
    /// inside `java.get`/`java.post`/`java.ajax` JS callbacks. `None` when the
    /// runtime was constructed with the legacy [`RemoteState::new`] path.
    host_callback_bridge: Option<crate::host_callback_bridge::HostCallbackBridge>,
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
        // Wire the JS host-callback bridge: the pipeline's QuickJsSandbox now
        // has `java.get`/`java.post`/`java.ajax`/`java.connect`/`java.ajaxAll`
        // callbacks that emit `host.request` events through `sink` and block
        // the worker thread until `host.complete` is routed back here in
        // [`Runtime::send`]. This unblocks the 28% of P0 corpus sources whose
        // `@js:` searchUrl calls `java.get`/`java.ajax`.
        let remote_state = Arc::new(RemoteState::with_sink(sink.clone()));
        let host_callback_bridge = remote_state.bridge().cloned();
        let tts_state = Arc::new(TtsState::new());

        let worker_active = active_requests.clone();
        let worker_cancelled = cancelled.clone();
        let worker_host_operations = host_operations.clone();
        let worker_next_operation_id = next_operation_id.clone();
        let worker_shutdown = shutdown.clone();
        let worker_sink = sink.clone();
        let worker_remote_state = remote_state.clone();
        let worker_tts_state = tts_state.clone();

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
                    worker_tts_state,
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
            tts_state,
            shutdown,
            host_callback_bridge,
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

    /// Shared TTS queue state. Exposed so tests (and hosts that embed the
    /// pure-Rust runtime) can inspect queue snapshots. Not part of the C ABI.
    pub fn tts_state(&self) -> &TtsState {
        &self.tts_state
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

        // JS host-callback fast path: when a `host.complete` / `host.error`
        // arrives for an operationId that belongs to a JS callback (emitted
        // from inside `java.get`/`java.ajax` on the worker thread), deliver the
        // result directly to the blocked callback WITHOUT enqueueing on the
        // worker mpsc queue. The worker thread is blocked inside JS waiting for
        // this very signal — routing through the queue would deadlock (the
        // worker would never dequeue the completion because it is blocked
        // waiting for it). Non-JS host operations fall through to the normal
        // enqueue path below (zero behavioral change for existing flows).
        if command.method == methods::HOST_COMPLETE || command.method == methods::HOST_ERROR {
            if let Some(bridge) = &self.host_callback_bridge {
                if bridge.try_complete(&command.method, &command.params) {
                    return Ok(());
                }
            }
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

        let work_item = if command.method == methods::RUNTIME_SHUTDOWN {
            WorkItem::ProtocolShutdown(command.clone())
        } else {
            WorkItem::Command(command.clone())
        };

        match self.tx.send(work_item) {
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
        remote_state: Arc<RemoteState>,
        tts_state: Arc<TtsState>,
    ) {
        for item in &rx {
            match item {
                WorkItem::Shutdown => break,
                WorkItem::Command(cmd) => {
                    if shutdown.load(Ordering::Acquire) {
                        break;
                    }
                    dispatch_command(
                        &cmd,
                        &sink,
                        &active_requests,
                        &cancelled,
                        &host_operations,
                        &next_operation_id,
                        &shutdown,
                        &remote_state,
                        &tts_state,
                    );
                }
                WorkItem::ProtocolShutdown(cmd) => {
                    if shutdown.load(Ordering::Acquire) {
                        break;
                    }
                    dispatch_command(
                        &cmd,
                        &sink,
                        &active_requests,
                        &cancelled,
                        &host_operations,
                        &next_operation_id,
                        &shutdown,
                        &remote_state,
                        &tts_state,
                    );
                    if shutdown.load(Ordering::Acquire) {
                        drain_queued_after_protocol_shutdown(
                            &rx,
                            &sink,
                            &active_requests,
                            &cancelled,
                        );
                        break;
                    }
                }
            }
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // 1. Signal shutdown first so the worker loop breaks at the next
        //    iteration boundary (between WorkItems).
        self.shutdown.store(true, Ordering::Release);

        // 2. Abort any JS host callbacks still waiting for `host.complete` /
        //    `host.error` inside `java.get`/`java.ajax`/etc. Without this,
        //    the worker thread is stuck inside `PendingCallback::wait` for
        //    up to DEFAULT_CALLBACK_TIMEOUT (30s) per pending call, and the
        //    `join()` below would block until every one of them times out.
        //    This was the root cause of batch corpus tests stalling at
        //    270/459 sources: each source that issued a `java.get` and never
        //    got a host.complete kept the worker blocked for 30s during Drop.
        if let Some(bridge) = &self.host_callback_bridge {
            bridge.abort_all_pending("runtime shutting down");
        }

        // 3. Enqueue the protocol Shutdown work item so the worker breaks
        //    out of its recv loop even if it isn't currently dispatching.
        let _ = self.tx.send(WorkItem::Shutdown);

        // 4. Join with a timeout. The abort above should wake the worker
        //    promptly, but if it is stuck in some other uninterruptible
        //    blocking call (e.g. inside JS execution that catches the abort
        //    error and retries), we detach the worker instead of blocking
        //    Drop indefinitely. The worker will exit on its own when its
        //    current operation completes or its callback timeout fires.
        if let Some(handle) = self.worker.take() {
            join_with_timeout(handle, Duration::from_secs(5));
        }
    }
}

/// Join a worker thread with a timeout. If the worker does not exit within
/// `timeout`, the joiner thread is detached (the worker keeps running until
/// it exits on its own). This prevents `Drop` from blocking indefinitely
/// when the worker is stuck in an uninterruptible blocking call.
///
/// Implementation: spawn a helper thread that calls `handle.join()` and
/// signals via a oneshot channel when done. `recv_timeout` on the channel
/// gives us the timeout semantics; if it fires, we simply return and let
/// the helper thread (and the worker it's joining) finish in the background.
fn join_with_timeout(handle: JoinHandle<()>, timeout: Duration) {
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let spawn = thread::Builder::new()
        .name("reader-core-worker-join".into())
        .spawn(move || {
            let _ = handle.join();
            let _ = done_tx.send(());
        });
    if spawn.is_err() {
        // Could not spawn a join helper — fall back to blocking join. We
        // cannot recover the handle (it was moved into the closure), so
        // this branch is effectively unreachable; the worker thread will
        // still terminate when the runtime's other resources drop.
        return;
    }
    let _ = done_rx.recv_timeout(timeout);
}

fn dispatch_command(
    cmd: &Command,
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
    cancelled: &Mutex<HashSet<u64>>,
    host_operations: &Mutex<HashMap<u64, HostOperation>>,
    next_operation_id: &AtomicU64,
    shutdown: &AtomicBool,
    remote_state: &RemoteState,
    tts_state: &TtsState,
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
        methods::CORE_INFO => dispatch_core_info(cmd, sink, active_requests, cancelled),
        methods::RUNTIME_PING | methods::LEGACY_CORE_PING => {
            dispatch_runtime_ping(cmd, sink, active_requests, cancelled)
        }
        methods::RUNTIME_HOST_SMOKE => dispatch_host_smoke(
            cmd,
            sink,
            active_requests,
            cancelled,
            host_operations,
            next_operation_id,
        ),
        methods::RUNTIME_CANCEL => {
            dispatch_runtime_cancel(cmd, sink, active_requests, cancelled, host_operations)
        }
        methods::RUNTIME_STATUS => dispatch_runtime_status(
            cmd,
            sink,
            active_requests,
            cancelled,
            host_operations,
            shutdown,
        ),
        methods::RUNTIME_SHUTDOWN => dispatch_runtime_shutdown(
            cmd,
            sink,
            active_requests,
            cancelled,
            host_operations,
            shutdown,
        ),
        methods::HOST_COMPLETE => dispatch_host_complete(
            cmd,
            sink,
            active_requests,
            cancelled,
            host_operations,
            next_operation_id,
            remote_state,
        ),
        methods::HOST_ERROR => {
            dispatch_host_error(cmd, sink, active_requests, cancelled, host_operations)
        }
        other => {
            // TTS vertical (pure logic, no host I/O) is tried first so that
            // tts.* commands don't fall through to unknown_method.
            match crate::tts::dispatch_tts(other, cmd, sink, active_requests, tts_state) {
                crate::tts::TtsDispatch::Finished => return,
                crate::tts::TtsDispatch::NotHandled => {}
            }
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

fn dispatch_core_info(
    cmd: &Command,
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
    cancelled: &Mutex<HashSet<u64>>,
) {
    if let Err(error) = parse_empty_params(cmd) {
        finish_request(
            sink,
            active_requests,
            cancelled,
            cmd.request_id,
            Event::error(cmd.request_id, error),
        );
        return;
    }

    finish_request(
        sink,
        active_requests,
        cancelled,
        cmd.request_id,
        Event::result(cmd.request_id, core_info(ABI_VERSION, BUILD_VERSION)),
    );
}

fn dispatch_runtime_ping(
    cmd: &Command,
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
    cancelled: &Mutex<HashSet<u64>>,
) {
    if let Err(error) = parse_empty_params(cmd) {
        finish_request(
            sink,
            active_requests,
            cancelled,
            cmd.request_id,
            Event::error(cmd.request_id, error),
        );
        return;
    }

    finish_request(
        sink,
        active_requests,
        cancelled,
        cmd.request_id,
        Event::result(
            cmd.request_id,
            serde_json::json!({ "pong": true, "method": methods::RUNTIME_PING }),
        ),
    );
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
                capability: params.capability.clone(),
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
    let params = match parse_runtime_cancel_params(cmd) {
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

    if params.request_id == cmd.request_id {
        finish_request(
            sink,
            active_requests,
            cancelled,
            cmd.request_id,
            Event::error(
                cmd.request_id,
                CoreError::invalid_params(
                    "runtime.cancel requestId must differ from the cancel command requestId",
                )
                .with_details(serde_json::json!({ "requestId": params.request_id })),
            ),
        );
        return;
    }

    let did_cancel = cancel_request_by_id(
        sink,
        active_requests,
        cancelled,
        host_operations,
        params.request_id,
    );
    finish_request(
        sink,
        active_requests,
        cancelled,
        cmd.request_id,
        Event::result(
            cmd.request_id,
            serde_json::json!({ "cancelled": did_cancel }),
        ),
    );
}

fn dispatch_runtime_status(
    cmd: &Command,
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
    cancelled: &Mutex<HashSet<u64>>,
    host_operations: &Mutex<HashMap<u64, HostOperation>>,
    shutdown: &AtomicBool,
) {
    let _params: RuntimeStatusParams = match serde_json::from_value(cmd.params.clone()) {
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

    let status = match runtime_status_snapshot(
        active_requests,
        host_operations,
        cmd.request_id,
        shutdown.load(Ordering::Acquire),
    ) {
        Ok(status) => status,
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

    let data = serde_json::to_value(status)
        .unwrap_or_else(|_| serde_json::json!({ "internalSerializationError": true }));
    finish_request(
        sink,
        active_requests,
        cancelled,
        cmd.request_id,
        Event::result(cmd.request_id, data),
    );
}

fn dispatch_runtime_shutdown(
    cmd: &Command,
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
    cancelled: &Mutex<HashSet<u64>>,
    host_operations: &Mutex<HashMap<u64, HostOperation>>,
    shutdown: &AtomicBool,
) {
    let _params = match parse_runtime_shutdown_params(cmd) {
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

    shutdown.store(true, Ordering::Release);
    let cancelled_request_ids = match cancel_active_requests_for_shutdown(
        sink,
        active_requests,
        cancelled,
        host_operations,
        cmd.request_id,
    ) {
        Ok(cancelled_request_ids) => cancelled_request_ids,
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

    finish_request(
        sink,
        active_requests,
        cancelled,
        cmd.request_id,
        Event::result(
            cmd.request_id,
            serde_json::json!({
                "shuttingDown": true,
                "cancelledRequestIds": cancelled_request_ids,
            }),
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
                capability: pending.capability.clone(),
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
    next_operation_id: &AtomicU64,
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

    // Stamp the originating requestId so any JS host callbacks invoked while
    // resuming the continuation (e.g. AnalyzeUrl JS that calls java.get after
    // the initial search response arrives) attribute host.request to the
    // original request, not the host.complete command.
    if let Some(bridge) = remote_state.bridge() {
        bridge.set_current_request_id(operation.request_id);
    }

    let request_id = operation.request_id;
    match operation.continuation {
        HostOperationContinuation::Echo => {
            let event = match complete_echo_host_operation(&operation, params.result) {
                Ok(data) => Event::result(request_id, data),
                Err(error) => Event::error(request_id, error),
            };
            finish_request(sink, active_requests, cancelled, request_id, event);
        }
        HostOperationContinuation::Remote(continuation) => {
            match complete_remote_host(continuation, params.result, remote_state) {
                Ok(RemoteCommandResult::Complete(data)) => {
                    finish_request(
                        sink,
                        active_requests,
                        cancelled,
                        request_id,
                        Event::result(request_id, data),
                    );
                }
                Ok(RemoteCommandResult::Pending(pending)) => {
                    register_pending_host_request(
                        request_id,
                        pending,
                        sink,
                        active_requests,
                        cancelled,
                        host_operations,
                        next_operation_id,
                    );
                }
                Err(error) => {
                    finish_request(
                        sink,
                        active_requests,
                        cancelled,
                        request_id,
                        Event::error(request_id, error),
                    );
                }
            }
        }
    }
}

/// Register a new pending host operation (emitted by a `complete_remote_host`
/// continuation that needs another HTTP round-trip, e.g. pagination) and emit
/// the corresponding `host.request` event. Mirrors `dispatch_remote_host_request`
/// but reuses an existing `request_id` instead of deriving it from a `Command`.
fn register_pending_host_request(
    request_id: u64,
    pending: PendingHostRequest,
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
    cancelled: &Mutex<HashSet<u64>>,
    host_operations: &Mutex<HashMap<u64, HostOperation>>,
    next_operation_id: &AtomicU64,
) {
    let operation_id = next_operation_id.fetch_add(1, Ordering::AcqRel);
    if let Ok(mut operations) = host_operations.lock() {
        operations.insert(
            operation_id,
            HostOperation {
                request_id,
                capability: pending.capability.clone(),
                state: HostOperationState::Pending,
                continuation: HostOperationContinuation::Remote(pending.continuation),
            },
        );
    } else {
        finish_request(
            sink,
            active_requests,
            cancelled,
            request_id,
            Event::error(
                request_id,
                CoreError::internal("host operation registry poisoned"),
            ),
        );
        return;
    }

    if take_cancelled(cancelled, request_id) {
        remove_operation(host_operations, operation_id);
        finish_request(
            sink,
            active_requests,
            cancelled,
            request_id,
            Event::error(request_id, CoreError::cancelled()),
        );
        return;
    }

    sink.emit(&Event::host_request(
        request_id,
        operation_id,
        pending.capability,
        pending.params,
    ));
}

fn complete_echo_host_operation(
    operation: &HostOperation,
    result: serde_json::Value,
) -> Result<serde_json::Value, CoreError> {
    match operation.capability {
        HostCapability::WebViewEvaluateJavaScript => {
            let response =
                serde_json::from_value::<HostWebViewEvaluateJavaScriptResponse>(result.clone())
                    .map_err(|err| {
                        CoreError::invalid_params("invalid result for webview.evaluateJavaScript")
                            .with_details(serde_json::json!({
                                "source": err.to_string(),
                                "capability": operation.capability,
                            }))
                    })?;
            response.validate()?;
            Ok(result)
        }
        HostCapability::FileRead => validate_echo_host_result::<HostFileReadResponse>(
            operation.capability,
            result,
            "file.read",
        ),
        HostCapability::FileWrite => validate_echo_host_result::<HostFileWriteResponse>(
            operation.capability,
            result,
            "file.write",
        ),
        HostCapability::CacheGet => validate_echo_host_result::<HostCacheGetResponse>(
            operation.capability,
            result,
            "cache.get",
        ),
        HostCapability::CachePut => validate_echo_host_result::<HostCachePutResponse>(
            operation.capability,
            result,
            "cache.put",
        ),
        HostCapability::CookieGet => validate_echo_host_result::<HostCookieGetResponse>(
            operation.capability,
            result,
            "cookie.get",
        ),
        HostCapability::CookieSet => validate_echo_host_result::<HostCookieSetResponse>(
            operation.capability,
            result,
            "cookie.set",
        ),
        HostCapability::LogEmit => validate_echo_host_result::<HostLogEmitResponse>(
            operation.capability,
            result,
            "log.emit",
        ),
        HostCapability::TimeNow => validate_echo_host_result::<HostTimeNowResponse>(
            operation.capability,
            result,
            "time.now",
        ),
        HostCapability::SystemInfo => validate_echo_host_result::<HostSystemInfoResponse>(
            operation.capability,
            result,
            "system.info",
        ),
        HostCapability::PersistenceGet => validate_echo_host_result::<HostPersistenceGetResponse>(
            operation.capability,
            result,
            "persistence.get",
        ),
        HostCapability::PersistencePut => validate_echo_host_result::<HostPersistencePutResponse>(
            operation.capability,
            result,
            "persistence.put",
        ),
        _ => Ok(result),
    }
}

fn validate_echo_host_result<T>(
    capability: HostCapability,
    result: serde_json::Value,
    label: &'static str,
) -> Result<serde_json::Value, CoreError>
where
    T: for<'de> serde::Deserialize<'de> + HostResultValidation,
{
    let response = serde_json::from_value::<T>(result.clone()).map_err(|err| {
        CoreError::invalid_params(format!("invalid result for {label}")).with_details(
            serde_json::json!({
                "source": err.to_string(),
                "capability": capability,
            }),
        )
    })?;
    response.validate()?;
    Ok(result)
}

trait HostResultValidation {
    fn validate(&self) -> Result<(), CoreError>;
}

impl HostResultValidation for HostFileReadResponse {
    fn validate(&self) -> Result<(), CoreError> {
        HostFileReadResponse::validate(self)
    }
}

impl HostResultValidation for HostFileWriteResponse {
    fn validate(&self) -> Result<(), CoreError> {
        HostFileWriteResponse::validate(self)
    }
}

impl HostResultValidation for HostCacheGetResponse {
    fn validate(&self) -> Result<(), CoreError> {
        HostCacheGetResponse::validate(self)
    }
}

impl HostResultValidation for HostCachePutResponse {
    fn validate(&self) -> Result<(), CoreError> {
        HostCachePutResponse::validate(self)
    }
}

impl HostResultValidation for HostCookieGetResponse {
    fn validate(&self) -> Result<(), CoreError> {
        HostCookieGetResponse::validate(self)
    }
}

impl HostResultValidation for HostCookieSetResponse {
    fn validate(&self) -> Result<(), CoreError> {
        HostCookieSetResponse::validate(self)
    }
}

impl HostResultValidation for HostLogEmitResponse {
    fn validate(&self) -> Result<(), CoreError> {
        HostLogEmitResponse::validate(self)
    }
}

impl HostResultValidation for HostTimeNowResponse {
    fn validate(&self) -> Result<(), CoreError> {
        HostTimeNowResponse::validate(self)
    }
}

impl HostResultValidation for HostSystemInfoResponse {
    fn validate(&self) -> Result<(), CoreError> {
        HostSystemInfoResponse::validate(self)
    }
}

impl HostResultValidation for HostPersistenceGetResponse {
    fn validate(&self) -> Result<(), CoreError> {
        HostPersistenceGetResponse::validate(self)
    }
}

impl HostResultValidation for HostPersistencePutResponse {
    fn validate(&self) -> Result<(), CoreError> {
        HostPersistencePutResponse::validate(self)
    }
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
        Event::error(
            operation.request_id,
            host_error_with_details(
                params.error,
                params.operation_id,
                &operation,
                params.diagnostics,
            ),
        ),
    );
}

fn host_error_with_details(
    mut error: CoreError,
    operation_id: u64,
    operation: &HostOperation,
    diagnostics: Option<HostErrorDiagnostics>,
) -> CoreError {
    let mut details = serde_json::Map::new();
    if let Some(existing) = error.details.as_object() {
        if !existing.is_empty() {
            details.insert(
                "cause".to_string(),
                serde_json::Value::Object(existing.clone()),
            );
        }
    }

    let mut host = serde_json::Map::new();
    host.insert("operationId".to_string(), serde_json::json!(operation_id));
    host.insert(
        "requestId".to_string(),
        serde_json::json!(operation.request_id),
    );
    host.insert(
        "capability".to_string(),
        serde_json::json!(operation.capability),
    );
    if let Some(diagnostics) = diagnostics {
        host.insert(
            "diagnostics".to_string(),
            serde_json::to_value(diagnostics).unwrap_or_else(|err| {
                serde_json::json!({
                    "code": "INTERNAL",
                    "phase": "runtime",
                    "message": format!("failed to encode host diagnostics: {err}")
                })
            }),
        );
    }
    details.insert("host".to_string(), serde_json::Value::Object(host));
    error.details = serde_json::Value::Object(details);
    error
}

fn parse_empty_params(cmd: &Command) -> Result<EmptyParams, CoreError> {
    serde_json::from_value::<EmptyParams>(cmd.params.clone()).map_err(|err| {
        CoreError::invalid_params(format!("invalid params for {}", cmd.method)).with_details(
            serde_json::json!({
                "source": err.to_string(),
                "method": cmd.method,
            }),
        )
    })
}

fn parse_host_smoke_params(cmd: &Command) -> Result<HostSmokeParams, CoreError> {
    let params = serde_json::from_value::<HostSmokeParams>(cmd.params.clone()).map_err(|err| {
        let message = if err.to_string().contains("host capability") {
            format!("invalid capability for {}", cmd.method)
        } else {
            format!("invalid params for {}", cmd.method)
        };
        CoreError::invalid_params(message).with_details(serde_json::json!({
            "source": err.to_string(),
            "method": cmd.method,
            "capability": cmd.params.get("capability").cloned().unwrap_or(serde_json::Value::Null),
        }))
    })?;
    params.validate()?;
    Ok(params)
}

fn parse_runtime_cancel_params(cmd: &Command) -> Result<RuntimeCancelParams, CoreError> {
    let params =
        serde_json::from_value::<RuntimeCancelParams>(cmd.params.clone()).map_err(|err| {
            CoreError::invalid_params(format!("invalid params for {}", cmd.method)).with_details(
                serde_json::json!({
                    "source": err.to_string(),
                    "method": cmd.method,
                }),
            )
        })?;
    params.validate()?;
    Ok(params)
}

fn parse_runtime_shutdown_params(cmd: &Command) -> Result<RuntimeShutdownParams, CoreError> {
    serde_json::from_value::<RuntimeShutdownParams>(cmd.params.clone()).map_err(|err| {
        CoreError::invalid_params(format!("invalid params for {}", cmd.method)).with_details(
            serde_json::json!({
                "source": err.to_string(),
                "method": cmd.method,
            }),
        )
    })
}

fn parse_host_complete_params(cmd: &Command) -> Result<HostCompleteParams, CoreError> {
    let params =
        serde_json::from_value::<HostCompleteParams>(cmd.params.clone()).map_err(|err| {
            CoreError::invalid_params(format!("invalid params for {}", cmd.method)).with_details(
                serde_json::json!({
                    "source": err.to_string(),
                    "method": cmd.method,
                }),
            )
        })?;
    params.validate()?;
    Ok(params)
}

fn parse_host_error_params(cmd: &Command) -> Result<HostErrorParams, CoreError> {
    let params = serde_json::from_value::<HostErrorParams>(cmd.params.clone()).map_err(|err| {
        CoreError::invalid_params(format!("invalid params for {}", cmd.method)).with_details(
            serde_json::json!({
                "source": err.to_string(),
                "method": cmd.method,
            }),
        )
    })?;
    params.validate()?;
    Ok(params)
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

fn runtime_status_snapshot(
    active_requests: &Mutex<HashSet<u64>>,
    host_operations: &Mutex<HashMap<u64, HostOperation>>,
    current_request_id: u64,
    shutting_down: bool,
) -> Result<RuntimeStatus, CoreError> {
    let mut active_request_ids = active_requests
        .lock()
        .map_err(|_| CoreError::internal("active request registry poisoned"))?
        .iter()
        .copied()
        .filter(|request_id| *request_id != current_request_id)
        .collect::<Vec<_>>();
    active_request_ids.sort_unstable();

    let mut pending_host_operations = host_operations
        .lock()
        .map_err(|_| CoreError::internal("host operation registry poisoned"))?
        .iter()
        .map(|(operation_id, operation)| PendingHostOperationStatus {
            operation_id: *operation_id,
            request_id: operation.request_id,
            capability: operation.capability,
            state: operation.state.as_protocol_str().to_string(),
        })
        .collect::<Vec<_>>();
    pending_host_operations.sort_by_key(|operation| operation.operation_id);

    Ok(RuntimeStatus {
        active_request_count: active_request_ids.len() as u64,
        active_request_ids,
        pending_host_operation_count: pending_host_operations.len() as u64,
        pending_host_operations,
        shutting_down,
    })
}

fn cancel_active_requests_for_shutdown(
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
    cancelled: &Mutex<HashSet<u64>>,
    host_operations: &Mutex<HashMap<u64, HostOperation>>,
    current_request_id: u64,
) -> Result<Vec<u64>, CoreError> {
    let mut cancelled_request_ids = {
        let mut active = active_requests
            .lock()
            .map_err(|_| CoreError::internal("active request registry poisoned"))?;
        let mut ids = active
            .iter()
            .copied()
            .filter(|request_id| *request_id != current_request_id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        for request_id in &ids {
            active.remove(request_id);
        }
        ids
    };

    {
        let mut operations = host_operations
            .lock()
            .map_err(|_| CoreError::internal("host operation registry poisoned"))?;
        operations.retain(|_, operation| {
            if operation.request_id == current_request_id {
                true
            } else {
                cancelled_request_ids.push(operation.request_id);
                false
            }
        });
    }

    cancelled_request_ids.sort_unstable();
    cancelled_request_ids.dedup();

    {
        let mut cancelled_set = cancelled
            .lock()
            .map_err(|_| CoreError::internal("cancel registry poisoned"))?;
        for request_id in &cancelled_request_ids {
            cancelled_set.remove(request_id);
        }
    }

    for request_id in &cancelled_request_ids {
        sink.emit(&Event::error(*request_id, CoreError::cancelled()));
    }

    Ok(cancelled_request_ids)
}

fn drain_queued_after_protocol_shutdown(
    rx: &std::sync::mpsc::Receiver<WorkItem>,
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
    cancelled: &Mutex<HashSet<u64>>,
) {
    for item in rx.try_iter() {
        match item {
            WorkItem::Command(cmd) | WorkItem::ProtocolShutdown(cmd) => {
                finish_request(
                    sink,
                    active_requests,
                    cancelled,
                    cmd.request_id,
                    Event::error(cmd.request_id, CoreError::cancelled()),
                );
            }
            WorkItem::Shutdown => break,
        }
    }
}

fn contains_active(active_requests: &Mutex<HashSet<u64>>, request_id: u64) -> bool {
    active_requests
        .lock()
        .map(|active| active.contains(&request_id))
        .unwrap_or(false)
}

fn cancel_request_by_id(
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
    cancelled: &Mutex<HashSet<u64>>,
    host_operations: &Mutex<HashMap<u64, HostOperation>>,
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
    fn no_param_control_methods_reject_unknown_params() {
        for (name, json, expected_request_id) in [
            (
                "runtime.ping",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/invalid-runtime-ping-unknown-field.json"
                ),
                202,
            ),
            (
                "core.info",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/invalid-core-info-unknown-field.json"
                ),
                212,
            ),
        ] {
            let sink = Arc::new(CollectSink::new());
            let rt = Runtime::new(sink.clone());
            rt.send_json(json.as_bytes()).unwrap();

            let events = sink.wait_len(1);
            match &events[0] {
                Event::Error {
                    request_id, error, ..
                } => {
                    assert_eq!(*request_id, expected_request_id, "{name}");
                    assert_eq!(error.code, ErrorCode::InvalidParams, "{name}");
                    assert!(error.message.contains(name), "{name}: {error:?}");
                }
                other => panic!("{name} expected params error, got {other:?}"),
            }
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
                    methods::RUNTIME_CANCEL,
                    methods::RUNTIME_STATUS,
                    methods::RUNTIME_SHUTDOWN,
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
    fn runtime_status_reports_empty_runtime_without_counting_itself() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(
            31,
            methods::RUNTIME_STATUS,
            serde_json::json!({}),
        ))
        .unwrap();

        let events = sink.wait_len(1);
        match &events[0] {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(*request_id, 31);
                assert_eq!(data["activeRequestCount"], 0);
                assert!(data["activeRequestIds"].as_array().unwrap().is_empty());
                assert_eq!(data["pendingHostOperationCount"], 0);
                assert!(data["pendingHostOperations"].as_array().unwrap().is_empty());
                assert_eq!(data["shuttingDown"], false);
            }
            other => panic!("expected runtime.status result, got {other:?}"),
        }
    }

    #[test]
    fn runtime_status_reports_pending_host_operation_without_payload() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(
            32,
            methods::RUNTIME_HOST_SMOKE,
            serde_json::json!({
                "capability": "host.smoke.echo",
                "params": { "message": "not exposed in status" }
            }),
        ))
        .unwrap();
        match &sink.wait_len(1)[0] {
            Event::HostRequest { .. } => {}
            other => panic!("expected host request, got {other:?}"),
        }

        rt.send(Command::new(
            33,
            methods::RUNTIME_STATUS,
            serde_json::json!({}),
        ))
        .unwrap();
        let events = sink.wait_len(2);
        match &events[1] {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(*request_id, 33);
                assert_eq!(data["activeRequestCount"], 1);
                assert_eq!(data["activeRequestIds"], serde_json::json!([32]));
                assert_eq!(data["pendingHostOperationCount"], 1);
                let operations = data["pendingHostOperations"].as_array().unwrap();
                assert_eq!(operations.len(), 1);
                assert_eq!(operations[0]["operationId"], 1);
                assert_eq!(operations[0]["requestId"], 32);
                assert_eq!(operations[0]["capability"], "host.smoke.echo");
                assert_eq!(operations[0]["state"], "pending");
                assert!(operations[0].get("params").is_none());
            }
            other => panic!("expected runtime.status result, got {other:?}"),
        }
    }

    #[test]
    fn runtime_status_rejects_unknown_params() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(
            34,
            methods::RUNTIME_STATUS,
            serde_json::json!({ "includePayloads": true }),
        ))
        .unwrap();

        let events = sink.wait_len(1);
        match &events[0] {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(*request_id, 34);
                assert_eq!(error.code, ErrorCode::InvalidParams);
                assert!(error.message.contains("runtime.status"));
            }
            other => panic!("expected runtime.status params error, got {other:?}"),
        }
    }

    #[test]
    fn runtime_shutdown_stops_future_commands() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(
            35,
            methods::RUNTIME_SHUTDOWN,
            serde_json::json!({}),
        ))
        .unwrap();

        let events = sink.wait_len(1);
        match &events[0] {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(*request_id, 35);
                assert_eq!(data["shuttingDown"], true);
                assert!(data["cancelledRequestIds"].as_array().unwrap().is_empty());
            }
            other => panic!("expected runtime.shutdown result, got {other:?}"),
        }

        let err = rt
            .send(Command::new(
                36,
                methods::RUNTIME_PING,
                serde_json::json!({}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Internal);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(sink.snapshot().len(), 1);
    }

    #[test]
    fn runtime_shutdown_cancels_pending_host_request() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(
            37,
            methods::RUNTIME_HOST_SMOKE,
            serde_json::json!({
                "capability": "host.smoke.echo",
                "params": { "message": "cancel on shutdown" }
            }),
        ))
        .unwrap();
        assert!(matches!(sink.wait_len(1)[0], Event::HostRequest { .. }));

        rt.send(Command::new(
            38,
            methods::RUNTIME_SHUTDOWN,
            serde_json::json!({}),
        ))
        .unwrap();

        let events = sink.wait_len(3);
        match &events[1] {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(*request_id, 37);
                assert_eq!(error.code, ErrorCode::Cancelled);
            }
            other => panic!("expected pending request cancellation, got {other:?}"),
        }
        match &events[2] {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(*request_id, 38);
                assert_eq!(data["shuttingDown"], true);
                assert_eq!(data["cancelledRequestIds"], serde_json::json!([37]));
            }
            other => panic!("expected runtime.shutdown result, got {other:?}"),
        }
    }

    #[test]
    fn runtime_shutdown_rejects_unknown_params_without_shutting_down() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(
            39,
            methods::RUNTIME_SHUTDOWN,
            serde_json::json!({ "force": true }),
        ))
        .unwrap();

        let events = sink.wait_len(1);
        match &events[0] {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(*request_id, 39);
                assert_eq!(error.code, ErrorCode::InvalidParams);
                assert!(error.message.contains("runtime.shutdown"));
            }
            other => panic!("expected runtime.shutdown params error, got {other:?}"),
        }

        rt.send(Command::new(
            40,
            methods::RUNTIME_PING,
            serde_json::json!({}),
        ))
        .unwrap();
        let events = sink.wait_len(2);
        match &events[1] {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(*request_id, 40);
                assert_eq!(data["pong"], true);
            }
            other => panic!("expected ping after invalid shutdown, got {other:?}"),
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
    fn send_json_rejects_invalid_command_envelope_before_enqueue() {
        for (name, json) in [
            (
                "request-id-zero",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/invalid-request-id-zero.json"
                ),
            ),
            (
                "empty-method",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/invalid-empty-method.json"
                ),
            ),
            (
                "method-whitespace",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/invalid-method-whitespace.json"
                ),
            ),
            (
                "method-empty-segment",
                include_str!(
                    "../../../protocol/fixtures/conformance/commands/invalid-method-empty-segment.json"
                ),
            ),
        ] {
            let sink = Arc::new(CollectSink::new());
            let rt = Runtime::new(sink.clone());
            let err = match rt.send_json(json.as_bytes()) {
                Ok(()) => panic!("{name} should be rejected before enqueue"),
                Err(err) => err,
            };

            assert_eq!(err.code, ErrorCode::InvalidMessage, "{name}: {err:?}");
            assert!(
                sink.wait_len(1).is_empty(),
                "{name} should not emit runtime events"
            );
        }
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
                assert_eq!(*capability, HostCapability::HostSmokeEcho);
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
    fn webview_host_request_shape_and_completion_are_typed() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send_json(
            include_str!("../../../protocol/fixtures/conformance/host/webview-request.json")
                .as_bytes(),
        )
        .unwrap();

        let events = sink.wait_len(1);
        match &events[0] {
            Event::HostRequest {
                request_id,
                operation_id,
                capability,
                params,
                ..
            } => {
                assert_eq!(*request_id, 431);
                assert_eq!(*operation_id, 1);
                assert_eq!(*capability, HostCapability::WebViewEvaluateJavaScript);
                assert_eq!(params["document"]["kind"], "html");
                assert_eq!(
                    params["javaScript"],
                    "document.querySelector('#book')?.textContent"
                );
            }
            other => panic!("expected webview host.request, got {other:?}"),
        }

        rt.send_json(
            include_str!("../../../protocol/fixtures/conformance/host/webview-complete.json")
                .as_bytes(),
        )
        .unwrap();

        let events = sink.wait_len(2);
        match &events[1] {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(*request_id, 431);
                assert_eq!(data["value"], "Dune");
                assert_eq!(data["finalUrl"], "https://books.example.test/detail");
            }
            other => panic!("expected webview completion result, got {other:?}"),
        }
    }

    #[test]
    fn webview_host_completion_rejects_invalid_result_shape() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send_json(
            include_str!("../../../protocol/fixtures/conformance/host/webview-request.json")
                .as_bytes(),
        )
        .unwrap();
        let events = sink.wait_len(1);
        assert!(matches!(events[0], Event::HostRequest { .. }));

        rt.send_json(
            include_str!(
                "../../../protocol/fixtures/conformance/host/webview-complete-blank-final-url.json"
            )
            .as_bytes(),
        )
        .unwrap();

        let events = sink.wait_len(2);
        match &events[1] {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(*request_id, 431);
                assert_eq!(error.code, ErrorCode::InvalidParams);
                assert!(error.message.contains("finalUrl"));
            }
            other => panic!("expected webview completion error, got {other:?}"),
        }
    }

    #[test]
    fn host_smoke_rejects_malformed_capability_names() {
        for (request_id, capability) in [
            (37, "host. smoke.echo"),
            (38, "host..echo"),
            (39, "custom.valid"),
        ] {
            let sink = Arc::new(CollectSink::new());
            let rt = Runtime::new(sink.clone());
            rt.send(Command::new(
                request_id,
                methods::RUNTIME_HOST_SMOKE,
                serde_json::json!({
                    "capability": capability,
                    "params": { "message": "invalid capability" }
                }),
            ))
            .unwrap();

            let events = sink.wait_len(1);
            match &events[0] {
                Event::Error {
                    request_id: actual_request_id,
                    error,
                    ..
                } => {
                    assert_eq!(*actual_request_id, request_id);
                    assert_eq!(error.code, ErrorCode::InvalidParams);
                    assert!(error.message.contains("capability"));
                    assert_eq!(error.details["capability"], capability);
                }
                other => panic!("expected capability validation error, got {other:?}"),
            }
        }
    }

    #[test]
    fn runtime_cancel_command_cancels_pending_host_request() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send_json(
            include_str!("../../../protocol/fixtures/conformance/host/request.json").as_bytes(),
        )
        .unwrap();

        let events = sink.wait_len(1);
        assert!(matches!(events[0], Event::HostRequest { .. }));

        rt.send_json(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/valid-runtime-cancel.json"
            )
            .as_bytes(),
        )
        .unwrap();

        let events = sink.wait_len(3);
        match &events[1] {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(*request_id, 301);
                assert_eq!(error.code, ErrorCode::Cancelled);
            }
            other => panic!("expected cancelled original request, got {other:?}"),
        }
        match &events[2] {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(*request_id, 310);
                assert_eq!(data["cancelled"], true);
            }
            other => panic!("expected runtime.cancel result, got {other:?}"),
        }
    }

    #[test]
    fn runtime_cancel_command_for_unknown_id_returns_false() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send_json(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/valid-runtime-cancel.json"
            )
            .as_bytes(),
        )
        .unwrap();

        let events = sink.wait_len(1);
        match &events[0] {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(*request_id, 310);
                assert_eq!(data["cancelled"], false);
            }
            other => panic!("expected runtime.cancel result, got {other:?}"),
        }
    }

    #[test]
    fn runtime_cancel_command_rejects_invalid_params() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send_json(
            include_str!(
                "../../../protocol/fixtures/conformance/commands/invalid-runtime-cancel-target-zero.json"
            )
            .as_bytes(),
        )
        .unwrap();

        let events = sink.wait_len(1);
        match &events[0] {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(*request_id, 311);
                assert_eq!(error.code, ErrorCode::InvalidParams);
                assert_eq!(error.details["requestId"], 0);
            }
            other => panic!("expected runtime.cancel params error, got {other:?}"),
        }
    }

    #[test]
    fn runtime_cancel_command_rejects_self_cancel() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(
            312,
            methods::RUNTIME_CANCEL,
            serde_json::json!({ "requestId": 312 }),
        ))
        .unwrap();

        let events = sink.wait_len(1);
        match &events[0] {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(*request_id, 312);
                assert_eq!(error.code, ErrorCode::InvalidParams);
                assert!(error.message.contains("differ"));
            }
            other => panic!("expected runtime.cancel self-cancel error, got {other:?}"),
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
                assert_eq!(error.details["host"]["operationId"], 1);
                assert_eq!(error.details["host"]["requestId"], 301);
                assert_eq!(error.details["host"]["capability"], "host.smoke.echo");
            }
            other => panic!("expected original request error, got {other:?}"),
        }
    }

    #[test]
    fn host_error_attaches_typed_diagnostics_to_original_request() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send_json(
            include_str!("../../../protocol/fixtures/conformance/host/request.json").as_bytes(),
        )
        .unwrap();

        let events = sink.wait_len(1);
        assert!(matches!(events[0], Event::HostRequest { .. }));

        rt.send_json(
            include_str!("../../../protocol/fixtures/conformance/host/error-diagnostics.json")
                .as_bytes(),
        )
        .unwrap();

        let events = sink.wait_len(2);
        match &events[1] {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(*request_id, 301);
                assert_eq!(error.code, ErrorCode::Internal);
                assert_eq!(error.details["host"]["operationId"], 1);
                assert_eq!(error.details["host"]["capability"], "host.smoke.echo");
                assert_eq!(error.details["host"]["diagnostics"]["code"], "TIMEOUT");
                assert_eq!(error.details["host"]["diagnostics"]["phase"], "transport");
                assert_eq!(
                    error.details["host"]["diagnostics"]["details"]["timeoutMillis"],
                    30000
                );
            }
            other => panic!("expected original request diagnostic error, got {other:?}"),
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
    fn host_completion_rejects_zero_operation_id() {
        for (name, json, expected_request_id) in [
            (
                "host.complete",
                include_str!(
                    "../../../protocol/fixtures/conformance/host/complete-operation-zero.json"
                ),
                305,
            ),
            (
                "host.error",
                include_str!(
                    "../../../protocol/fixtures/conformance/host/error-operation-zero.json"
                ),
                306,
            ),
        ] {
            let sink = Arc::new(CollectSink::new());
            let rt = Runtime::new(sink.clone());
            rt.send_json(json.as_bytes()).unwrap();

            let events = sink.wait_len(1);
            match &events[0] {
                Event::Error {
                    request_id, error, ..
                } => {
                    assert_eq!(*request_id, expected_request_id, "{name}");
                    assert_eq!(error.code, ErrorCode::InvalidParams, "{name}");
                    assert_eq!(error.details["operationId"], 0, "{name}");
                }
                other => panic!("expected {name} operationId error, got {other:?}"),
            }
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
                    "bookSource": {
                        "bookSourceName": "Vertical Test Source",
                        "bookSourceUrl": "https://books.example.test",
                        "ruleSearch": "div.list&&div.item;div.name&&a@text",
                        "futureLegadoField": {
                            "nested": true,
                            "rawRule": "span.future@text"
                        }
                    },
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
        assert_eq!(
            stored.book_source["ruleSearch"],
            "div.list&&div.item;div.name&&a@text"
        );
        assert_eq!(
            stored.book_source["futureLegadoField"],
            serde_json::json!({
                "nested": true,
                "rawRule": "span.future@text"
            })
        );
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

    /// Legado native BookSource JSON carries `bookSourceName` (not a top-level
    /// `name`). `source.import` must accept this form verbatim and derive the
    /// source name from `bookSource.bookSourceName`. Mirrors Legado
    /// `BookSource.kt` (red line 3: migrate against Legado, no skipping).
    #[test]
    fn source_import_derives_name_from_book_source_book_source_name() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                7,
                methods::SOURCE_IMPORT,
                serde_json::json!({
                    "sourceId": "legado-native-src",
                    "baseUrl": "https://books.example.test",
                    "bookSource": {
                        "bookSourceName": "Legado Native Source",
                        "bookSourceUrl": "https://books.example.test",
                        "searchUrl": "/search?q={{key}}",
                        "ruleSearch": "div.list&&div.item"
                    }
                }),
            ),
        );
        match event {
            Event::Result { data, .. } => {
                assert_eq!(data["imported"], true);
                assert_eq!(data["sourceId"], "legado-native-src");
                assert_eq!(data["name"], "Legado Native Source");
            }
            other => panic!("expected result, got {other:?}"),
        }
        let stored = rt
            .remote_state()
            .storage()
            .get_source("legado-native-src")
            .unwrap()
            .expect("legado native source stored");
        assert_eq!(stored.name, "Legado Native Source");
        assert_eq!(
            stored.book_source["bookSourceName"],
            serde_json::json!("Legado Native Source")
        );
        assert_eq!(
            stored.book_source["ruleSearch"],
            serde_json::json!("div.list&&div.item")
        );
    }

    #[test]
    fn source_import_rejects_missing_name_and_book_source_name() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        // No `name` and `bookSource` without `bookSourceName`: neither path
        // can derive a name → InvalidParams.
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                8,
                methods::SOURCE_IMPORT,
                serde_json::json!({
                    "sourceId": "no-name-src",
                    "baseUrl": "https://books.example.test",
                    "bookSource": {
                        "bookSourceUrl": "https://books.example.test"
                    }
                }),
            ),
        );
        match event {
            Event::Error { error, .. } => {
                assert_eq!(error.code, ErrorCode::InvalidParams);
                assert!(
                    error.message.contains("name"),
                    "error should mention name fallback: {:?}",
                    error
                );
            }
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
                    "headers": {
                        "Accept": "application/json",
                        "Cookie": "sid=old"
                    },
                    "charset": "gbk",
                    "followRedirects": false,
                    "maxRedirects": 0,
                    "retry": {
                        "maxAttempts": 2,
                        "backoffMillis": 50
                    },
                    "usePlatformCookieJar": false,
                    "session": {
                        "id": "core-session-main"
                    }
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
                assert_eq!(*capability, HostCapability::HttpExecute);
                assert_eq!(params["method"], "GET");
                assert_eq!(params["url"], "https://books.example.test/search?q=dune");
                assert_eq!(params["headers"]["Accept"], "application/json");
                assert_eq!(params["headers"]["Cookie"], "sid=old");
                assert_eq!(params["charset"], "gbk");
                assert_eq!(params["followRedirects"], false);
                assert_eq!(params["maxRedirects"], 0);
                assert_eq!(params["retry"]["maxAttempts"], 2);
                assert_eq!(params["retry"]["backoffMillis"], 50);
                assert_eq!(params["usePlatformCookieJar"], false);
                assert_eq!(params["session"]["id"], "core-session-main");
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
                    "headers": {
                        "content-type": "application/json; charset=gbk",
                        "set-cookie": ["sid=new; Path=/; HttpOnly"]
                    },
                    "finalUrl": "https://books.example.test/search?q=dune",
                    "charsetHint": "gbk",
                    "bodyBase64": "eyJib29rcyI6W119",
                    "session": {
                        "id": "core-session-main"
                    },
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
                assert_eq!(data["http"]["status"], 200);
                assert_eq!(
                    data["http"]["headers"]["content-type"],
                    "application/json; charset=gbk"
                );
                assert_eq!(
                    data["http"]["headers"]["set-cookie"][0],
                    "sid=new; Path=/; HttpOnly"
                );
                assert_eq!(
                    data["http"]["finalUrl"],
                    "https://books.example.test/search?q=dune"
                );
                assert_eq!(data["http"]["charsetHint"], "gbk");
                assert_eq!(data["http"]["session"]["id"], "core-session-main");
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
    fn remote_host_http_completion_rejects_invalid_status() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(
            26,
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
            27,
            methods::HOST_COMPLETE,
            serde_json::json!({
                "operationId": operation_id,
                "result": {
                    "status": 99,
                    "body": "{\"books\":[]}"
                }
            }),
        ))
        .unwrap();

        let events = sink.wait_len(2);
        match &events[1] {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(*request_id, 26);
                assert_eq!(error.code, ErrorCode::InvalidParams);
                assert!(error.message.contains("status"));
                assert_eq!(error.details["status"], 99);
            }
            other => panic!("expected invalid status error, got {other:?}"),
        }
    }

    #[test]
    fn remote_host_http_completion_rejects_invalid_headers_shape() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(
            28,
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
            29,
            methods::HOST_COMPLETE,
            serde_json::json!({
                "operationId": operation_id,
                "result": {
                    "headers": ["content-type", "application/json"],
                    "body": "{\"books\":[]}"
                }
            }),
        ))
        .unwrap();

        let events = sink.wait_len(2);
        match &events[1] {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(*request_id, 28);
                assert_eq!(error.code, ErrorCode::InvalidParams);
                assert!(error.message.contains("headers"));
            }
            other => panic!("expected invalid headers error, got {other:?}"),
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
        // jsRule evaluation uses an isolated (no-bridge) sandbox: a `java.get`
        // call inside a chapter-content jsRule returns `JsOutcome::Unsupported`
        // because no host callback is registered on the jsRule sandbox. The
        // bridge is reserved for URL-building (`@js:`/`<js>` in searchUrl/
        // bookUrl/tocUrl/chapterUrl), not chapter-content jsRules. The error
        // is structured (`details.unsupported = true`), never a fake network
        // result.
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
                assert_eq!(
                    error.details["unsupported"],
                    serde_json::Value::Bool(true),
                    "error should carry structured unsupported flag, got details: {:?}",
                    error.details
                );
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
                i + 1,
                methods::RUNTIME_PING,
                serde_json::json!({}),
            ))
            .unwrap();
            drop(rt);
        }
    }

    // ===================================================================
    // RSS / sync / local-book vertical dispatch (V1 minimal)
    // ===================================================================

    #[test]
    fn rss_parse_decodes_feed() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
                   <rss version=\"2.0\"><channel>\
                   <title>RT Feed</title>\
                   <link>https://books.example.test/feed</link>\
                   <description>Runtime test feed</description>\
                   <item><title>Item A</title><link>https://books.example.test/a</link>\
                   <guid>rt-entry-1</guid><description>First</description></item>\
                   </channel></rss>";
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                101,
                methods::RSS_PARSE,
                serde_json::json!({
                    "feedUrl": "https://books.example.test/feed",
                    "xml": xml,
                }),
            ),
        );
        match event {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(request_id, 101);
                assert_eq!(data["title"], "RT Feed");
                let entries = data["entries"].as_array().expect("entries array");
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0]["id"], "rt-entry-1");
                assert_eq!(entries[0]["title"], "Item A");
            }
            other => panic!("expected rss.parse result, got {other:?}"),
        }
    }

    #[test]
    fn rss_parse_rejects_blank_xml() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                102,
                methods::RSS_PARSE,
                serde_json::json!({
                    "feedUrl": "https://books.example.test/feed",
                    "xml": "   ",
                }),
            ),
        );
        match event {
            Event::Error { error, .. } => assert_eq!(error.code, ErrorCode::InvalidParams),
            other => panic!("expected invalid params, got {other:?}"),
        }
    }

    #[test]
    fn rss_refresh_returns_forced_decision() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                103,
                methods::RSS_REFRESH,
                serde_json::json!({
                    "subscriptionId": "rt-sub",
                    "enabled": true,
                    "forceRefresh": true,
                    "evaluatedAt": 1700000000000_i64,
                }),
            ),
        );
        match event {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(request_id, 103);
                assert_eq!(data["subscriptionId"], "rt-sub");
                assert_eq!(data["shouldFetch"], true);
                assert_eq!(data["reason"], "forced");
            }
            other => panic!("expected rss.refresh result, got {other:?}"),
        }
    }

    #[test]
    fn rss_refresh_rejects_blank_subscription_id() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                104,
                methods::RSS_REFRESH,
                serde_json::json!({
                    "subscriptionId": "  ",
                    "evaluatedAt": 1700000000000_i64,
                }),
            ),
        );
        match event {
            Event::Error { error, .. } => assert_eq!(error.code, ErrorCode::InvalidParams),
            other => panic!("expected invalid params, got {other:?}"),
        }
    }

    #[test]
    fn sync_merge_returns_merged_snapshot() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                105,
                methods::SYNC_MERGE,
                serde_json::json!({
                    "local": {
                        "snapshotId": "local-1",
                        "deviceId": "device-a",
                        "createdAt": 1000,
                        "records": []
                    },
                    "remote": {
                        "snapshotId": "remote-1",
                        "deviceId": "device-b",
                        "createdAt": 2000,
                        "records": []
                    },
                    "mergedSnapshotId": "merged-rt-1",
                    "mergedDeviceId": "device-merged",
                    "mergedCreatedAt": 3000
                }),
            ),
        );
        match event {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(request_id, 105);
                assert_eq!(data["snapshot"]["snapshotId"], "merged-rt-1");
                assert_eq!(data["snapshot"]["deviceId"], "device-merged");
                assert_eq!(data["conflicts"].as_array().unwrap().len(), 0);
            }
            other => panic!("expected sync.merge result, got {other:?}"),
        }
    }

    #[test]
    fn sync_merge_rejects_local_not_object() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                106,
                methods::SYNC_MERGE,
                serde_json::json!({
                    "local": [],
                    "remote": {
                        "snapshotId": "remote-1",
                        "deviceId": "device-b",
                        "createdAt": 2000,
                        "records": []
                    }
                }),
            ),
        );
        match event {
            Event::Error { error, .. } => assert_eq!(error.code, ErrorCode::InvalidParams),
            other => panic!("expected invalid params, got {other:?}"),
        }
    }

    #[test]
    fn sync_backup_returns_plan() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                107,
                methods::SYNC_BACKUP,
                serde_json::json!({
                    "package": {
                        "manifest": {
                            "backupID": "rt-backup-1",
                            "createdAt": 1000,
                            "entries": [],
                            "totalBytes": 0,
                            "bookCount": 0
                        }
                    },
                    "policy": {
                        "mode": "full",
                        "overwriteExisting": false
                    }
                }),
            ),
        );
        match event {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(request_id, 107);
                assert!(data["plan"].is_object());
            }
            other => panic!("expected sync.backup result, got {other:?}"),
        }
    }

    #[test]
    fn sync_backup_rejects_package_not_object() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                108,
                methods::SYNC_BACKUP,
                serde_json::json!({
                    "package": "not-an-object",
                    "policy": { "mode": "full", "overwriteExisting": false }
                }),
            ),
        );
        match event {
            Event::Error { error, .. } => assert_eq!(error.code, ErrorCode::InvalidParams),
            other => panic!("expected invalid params, got {other:?}"),
        }
    }

    #[test]
    fn local_book_parse_returns_book() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                109,
                methods::LOCAL_BOOK_PARSE,
                serde_json::json!({
                    "bookId": "rt-book-1",
                    "title": "RT TXT Book",
                    "author": "Tester",
                    "fileName": "rt.txt",
                    "text": "第一章 Intro\nHello world content."
                }),
            ),
        );
        match event {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(request_id, 109);
                assert_eq!(data["format"], "txt");
                assert_eq!(data["encoding"], "utf8");
                assert!(data["charLen"].as_u64().unwrap() > 0);
                assert!(data["chapterCount"].as_u64().unwrap() >= 1);
                assert!(data["book"].is_object());
            }
            other => panic!("expected local_book.parse result, got {other:?}"),
        }
    }

    #[test]
    fn local_book_parse_rejects_blank_text() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                110,
                methods::LOCAL_BOOK_PARSE,
                serde_json::json!({
                    "bookId": "rt-book-1",
                    "text": "   "
                }),
            ),
        );
        match event {
            Event::Error { error, .. } => assert_eq!(error.code, ErrorCode::InvalidParams),
            other => panic!("expected invalid params, got {other:?}"),
        }
    }

    /// S5: local_book.parse binary path — EPUB via bytesBase64 + format hint.
    /// Mirrors Legado's LocalBook dispatch by mimetype/extension, except the
    /// wire payload is base64 over JSON (no raw bytes ever cross the boundary).
    #[test]
    fn local_book_parse_dispatches_epub_bytes_base64() {
        let epub_bytes = build_minimal_epub_zip_for_test(
            "Runtime EPUB Title",
            "Runtime Tester",
            &[
                ("ch1", "Chapter One", "Hello from chapter one."),
                ("ch2", "Chapter Two", "Hello from chapter two."),
            ],
        );
        let bytes_b64 = base64_encode_for_test(&epub_bytes);
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                120,
                methods::LOCAL_BOOK_PARSE,
                serde_json::json!({
                    "bookId": "rt-epub-1",
                    "title": "Runtime EPUB Title",
                    "author": "Runtime Tester",
                    "format": "epub",
                    "bytesBase64": bytes_b64,
                }),
            ),
        );
        match event {
            Event::Result { data, .. } => {
                assert_eq!(data["format"], "epub");
                assert_eq!(data["byteLen"].as_u64().unwrap(), epub_bytes.len() as u64);
                assert!(
                    data["chapterCount"].as_u64().unwrap() >= 2,
                    "expected at least 2 chapters, got {data}"
                );
                assert!(data["book"].is_object());
            }
            other => panic!("expected local_book.parse epub result, got {other:?}"),
        }
    }

    /// S5: local_book.parse binary path — PDF via bytesBase64. Core detects
    /// PDF from `%PDF-` magic and extracts text from content stream Tj/TJ
    /// operators. No host OCR for text-bearing PDFs.
    #[test]
    fn local_book_parse_dispatches_pdf_bytes_base64() {
        let pdf = b"%PDF-1.4\n\
1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n\
3 0 obj\n<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>\nendobj\n\
4 0 obj\n<< /Length 44 >>\nstream\nBT /F1 12 Tf 72 720 Td (Runtime PDF Page One) Tj ET\nendstream\nendobj\n\
xref\n0 5\ntrailer\n<< /Root 1 0 R >>\nstartxref\n0\n%%EOF";
        let bytes_b64 = base64_encode_for_test(pdf);
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                121,
                methods::LOCAL_BOOK_PARSE,
                serde_json::json!({
                    "bookId": "rt-pdf-1",
                    "fileName": "rt.pdf",
                    "bytesBase64": bytes_b64,
                }),
            ),
        );
        match event {
            Event::Result { data, .. } => {
                assert_eq!(data["format"], "pdf");
                assert!(data["chapterCount"].as_u64().unwrap() >= 1);
            }
            other => panic!("expected local_book.parse pdf result, got {other:?}"),
        }
    }

    /// S5: contract enforces at-least-one-of (text, bytesBase64). Sending
    /// neither fails closed with INVALID_PARAMS before reaching the parser.
    #[test]
    fn local_book_parse_rejects_missing_text_and_bytes() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                122,
                methods::LOCAL_BOOK_PARSE,
                serde_json::json!({
                    "bookId": "rt-empty-1",
                    "fileName": "empty.epub",
                }),
            ),
        );
        match event {
            Event::Error { error, .. } => assert_eq!(error.code, ErrorCode::InvalidParams),
            other => panic!("expected invalid params, got {other:?}"),
        }
    }

    /// S5: malformed base64 surfaces as INVALID_PARAMS, never reaches parser.
    #[test]
    fn local_book_parse_rejects_malformed_bytes_base64() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                123,
                methods::LOCAL_BOOK_PARSE,
                serde_json::json!({
                    "bookId": "rt-bad-b64",
                    "format": "epub",
                    "bytesBase64": "!!!not-base64!!!",
                }),
            ),
        );
        match event {
            Event::Error { error, .. } => assert_eq!(error.code, ErrorCode::InvalidParams),
            other => panic!("expected invalid params, got {other:?}"),
        }
    }

    /// Build a minimal EPUB ZIP in memory for runtime tests. Mirrors the
    /// shape of reader-local-book's `epub3_nav_spine_resource_cover.epub`
    /// fixture: container.xml → OPF (with nav + spine) → chapter XHTMLs.
    fn build_minimal_epub_zip_for_test(
        title: &str,
        author: &str,
        chapters: &[(&str, &str, &str)],
    ) -> Vec<u8> {
        use std::io::Write;
        use zip::write::SimpleFileOptions;

        let mut buf = std::io::Cursor::new(Vec::<u8>::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts = SimpleFileOptions::default();

            // mimetype must be first and stored (uncompressed) per EPUB spec.
            zip.start_file("mimetype", opts).unwrap();
            zip.write_all(b"application/epub+zip").unwrap();

            zip.start_file("META-INF/container.xml", opts).unwrap();
            zip.write_all(
                br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#,
            )
            .unwrap();

            // OPF with metadata + manifest (nav + chapters) + spine.
            let mut opf = String::new();
            opf.push_str("<?xml version=\"1.0\"?>\n");
            opf.push_str("<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"bookid\">\n");
            opf.push_str("  <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n");
            opf.push_str(&format!("    <dc:title>{}</dc:title>\n", title));
            opf.push_str(&format!("    <dc:creator>{}</dc:creator>\n", author));
            opf.push_str("    <dc:language>en</dc:language>\n");
            opf.push_str("    <dc:identifier id=\"bookid\">rt-epub-001</dc:identifier>\n");
            opf.push_str("  </metadata>\n  <manifest>\n");
            opf.push_str("    <item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n");
            for (id, _title, _content) in chapters {
                opf.push_str(&format!(
                    "    <item id=\"{id}\" href=\"{id}.xhtml\" media-type=\"application/xhtml+xml\"/>\n"
                ));
            }
            opf.push_str("  </manifest>\n  <spine>\n");
            for (id, _title, _content) in chapters {
                opf.push_str(&format!("    <itemref idref=\"{id}\"/>\n"));
            }
            opf.push_str("  </spine>\n</package>");

            zip.start_file("OEBPS/content.opf", opts).unwrap();
            zip.write_all(opf.as_bytes()).unwrap();

            // nav.xhtml (EPUB3 TOC)
            let mut nav = String::from("<?xml version=\"1.0\"?>\n");
            nav.push_str("<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\">\n");
            nav.push_str("  <body><nav epub:type=\"toc\"><ol>\n");
            for (id, title, _content) in chapters {
                nav.push_str(&format!(
                    "    <li><a href=\"{id}.xhtml\">{title}</a></li>\n"
                ));
            }
            nav.push_str("  </ol></nav></body></html>");
            zip.start_file("OEBPS/nav.xhtml", opts).unwrap();
            zip.write_all(nav.as_bytes()).unwrap();

            // Chapter XHTMLs
            for (id, title, content) in chapters {
                let xhtml = format!(
                    "<?xml version=\"1.0\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\">\n  <head><title>{title}</title></head>\n  <body><h1>{title}</h1><p>{content}</p></body></html>"
                );
                zip.start_file(format!("OEBPS/{id}.xhtml"), opts).unwrap();
                zip.write_all(xhtml.as_bytes()).unwrap();
            }

            zip.finish().unwrap();
        }
        buf.into_inner()
    }

    fn base64_encode_for_test(bytes: &[u8]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn local_book_catalog_upserts_entry() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                111,
                methods::LOCAL_BOOK_CATALOG,
                serde_json::json!({
                    "catalog": {
                        "schemaVersion": 1,
                        "books": [],
                        "chapters": [],
                        "resources": []
                    },
                    "entry": {
                        "stableBookId": "rt-book-1",
                        "sourceFingerprint": {
                            "byteCount": 42,
                            "prefixChecksum": "sha256:abc",
                            "suffixChecksum": "sha256:def",
                            "detectedFormat": "txt"
                        },
                        "contentFingerprint": {
                            "fullInputChecksum": "sha256:full",
                            "parserConfigChecksum": "sha256:cfg",
                            "normalizedMetadataChecksum": "sha256:meta",
                            "chapterLocatorSequenceChecksum": "sha256:loc"
                        },
                        "semanticFingerprint": {
                            "normalizedTitle": "intro",
                            "chapterTitleSequenceChecksum": "sha256:titleseq",
                            "chapterCount": 1,
                            "format": "txt"
                        }
                    },
                    "chapters": [],
                    "resources": []
                }),
            ),
        );
        match event {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(request_id, 111);
                let books = data["catalog"]["books"]
                    .as_array()
                    .expect("books array in catalog");
                assert!(books.iter().any(|entry| {
                    entry.get("stableBookId").and_then(|v| v.as_str()) == Some("rt-book-1")
                }));
            }
            other => panic!("expected local_book.catalog result, got {other:?}"),
        }
    }

    #[test]
    fn local_book_catalog_rejects_entry_not_object() {
        let sink = Arc::new(CollectSink::new());
        let rt = Runtime::new(sink.clone());
        let event = send_and_wait(
            &rt,
            &sink,
            Command::new(
                112,
                methods::LOCAL_BOOK_CATALOG,
                serde_json::json!({
                    "catalog": {
                        "schemaVersion": 1,
                        "books": [],
                        "chapters": [],
                        "resources": []
                    },
                    "entry": "not-an-object"
                }),
            ),
        );
        match event {
            Event::Error { error, .. } => assert_eq!(error.code, ErrorCode::InvalidParams),
            other => panic!("expected invalid params, got {other:?}"),
        }
    }
}
