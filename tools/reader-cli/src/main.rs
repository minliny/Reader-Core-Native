//! `reader-cli` — minimal Core driver.
//!
//! v0: prints `core.info` from a transient runtime. Becomes the sample-runner
//! harness in ARCHITECTURE.md phase 3.

use std::sync::Arc;

use reader_contract::{methods, Command, Event};
use reader_runtime::{EventSink, Runtime};

struct StdoutSink;
impl EventSink for StdoutSink {
    fn emit(&self, event: &Event) {
        println!("{}", serde_json::to_string(event).unwrap_or_default());
    }
}

fn main() {
    let sink = Arc::new(StdoutSink);
    let rt = Runtime::new(sink);
    rt.send(Command::new(1, methods::CORE_INFO, serde_json::json!({})))
        .expect("send core.info");
    // The worker runs on its own thread; give it a beat before teardown.
    std::thread::sleep(std::time::Duration::from_millis(50));
}
