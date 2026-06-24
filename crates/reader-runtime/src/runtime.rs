use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::collections::HashSet;

use reader_contract::{
    core_info, methods, Command, CoreError, Event, PROTOCOL_VERSION,
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

/// A handle to a running Core runtime.
///
/// Owns the worker thread. Dropping [`Runtime`] is equivalent to
/// `rc_runtime_destroy`: it signals shutdown, drains the queue, and joins the
/// worker. Once dropped, the event sink is never invoked again.
pub struct Runtime {
    tx: std::sync::mpsc::Sender<WorkItem>,
    worker: Option<JoinHandle<()>>,
    /// Request IDs cancelled via [`Runtime::cancel`].
    cancelled: Arc<Mutex<HashSet<u64>>>,
    /// Shutdown latch so the worker can stop even mid-processing.
    shutdown: Arc<AtomicBool>,
}

impl Runtime {
    /// Spawn a new runtime with the given event sink.
    pub fn new(sink: Arc<dyn EventSink>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<WorkItem>();
        let cancelled = Arc::new(Mutex::new(HashSet::new()));
        let shutdown = Arc::new(AtomicBool::new(false));

        let worker_cancelled = cancelled.clone();
        let worker_shutdown = shutdown.clone();
        let worker_sink = sink.clone();

        let worker = thread::Builder::new()
            .name("reader-core-worker".into())
            .spawn(move || {
                Self::worker_loop(rx, worker_sink, worker_cancelled, worker_shutdown);
            })
            .expect("reader-core worker thread spawn failed");

        Self { tx, worker: Some(worker), cancelled, shutdown }
    }

    /// Enqueue a command. Returns `Err` for protocol-version mismatch or if
    /// the runtime is shutting down.
    pub fn send(&self, command: Command) -> Result<(), CoreError> {
        if command.protocol_version != PROTOCOL_VERSION {
            return Err(CoreError::invalid_protocol_version(command.protocol_version));
        }
        match self.tx.send(WorkItem::Command(command)) {
            Ok(()) => Ok(()),
            Err(_) => Err(CoreError::internal("runtime shutting down")),
        }
    }

    /// Mark a request as cancelled. The worker skips emission for an in-flight
    /// or pending command with this ID. Returns silently if the ID is unknown
    /// (per the `rc_runtime_cancel` contract).
    pub fn cancel(&self, request_id: u64) {
        if let Ok(mut set) = self.cancelled.lock() {
            set.insert(request_id);
        }
    }

    fn worker_loop(
        rx: std::sync::mpsc::Receiver<WorkItem>,
        sink: Arc<dyn EventSink>,
        cancelled: Arc<Mutex<HashSet<u64>>>,
        shutdown: Arc<AtomicBool>,
    ) {
        for item in &rx {
            if shutdown.load(Ordering::Acquire) {
                break;
            }
            match item {
                WorkItem::Shutdown => break,
                WorkItem::Command(cmd) => {
                    if is_cancelled(&cancelled, cmd.request_id) {
                        continue;
                    }
                    let event = dispatch(&cmd);
                    // Re-check before emitting: a cancel may have landed
                    // while processing.
                    if !is_cancelled(&cancelled, cmd.request_id) {
                        sink.emit(&event);
                    }
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

fn is_cancelled(set: &Mutex<HashSet<u64>>, id: u64) -> bool {
    set.lock().map(|s| s.contains(&id)).unwrap_or(false)
}

/// Dispatch a single command to an event. Host capabilities (http.execute etc.)
/// are not wired in v1; unknown methods return a structured error.
fn dispatch(cmd: &Command) -> Event {
    match cmd.method.as_str() {
        methods::CORE_INFO => {
            Event::result(cmd.request_id, core_info(ABI_VERSION, BUILD_VERSION))
        }
        methods::CORE_PING => {
            Event::result(cmd.request_id, serde_json::json!({ "pong": true }))
        }
        other => Event::error(cmd.request_id, CoreError::unknown_method(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    struct CollectSink(StdMutex<Vec<Event>>);
    impl EventSink for CollectSink {
        fn emit(&self, event: &Event) {
            self.0.lock().unwrap().push(event.clone());
        }
    }

    #[test]
    fn ping_round_trips() {
        let sink = Arc::new(CollectSink(StdMutex::new(Vec::new())));
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(1, methods::CORE_PING, serde_json::json!({})))
            .unwrap();
        // Give the worker a moment to process.
        std::thread::sleep(std::time::Duration::from_millis(50));
        drop(rt);
        let events = sink.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::Result { request_id, data, .. } => {
                assert_eq!(*request_id, 1);
                assert_eq!(data["pong"], true);
            }
            other => panic!("expected result, got {other:?}"),
        }
    }

    #[test]
    fn unknown_method_yields_error() {
        let sink = Arc::new(CollectSink(StdMutex::new(Vec::new())));
        let rt = Runtime::new(sink.clone());
        rt.send(Command::new(7, "bogus.method", serde_json::json!({})))
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        drop(rt);
        let events = sink.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::Error { request_id, error, .. } => {
                assert_eq!(*request_id, 7);
                assert_eq!(error.code, reader_contract::ErrorCode::UnknownMethod);
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn create_destroy_1000_times_no_leak_or_crash() {
        for i in 0..1000 {
            let sink = Arc::new(CollectSink(StdMutex::new(Vec::new())));
            let rt = Runtime::new(sink);
            rt.send(Command::new(i, methods::CORE_PING, serde_json::json!({})))
                .unwrap();
            drop(rt); // destroy
        }
    }
}
