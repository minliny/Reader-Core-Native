use std::ffi::c_void;

/// The C event callback signature, matching `rc_event_callback` in
/// `reader_core.h`.
pub type CEventCallback =
    Option<unsafe extern "C" fn(context: *mut c_void, json: *const u8, json_length: usize)>;

/// Adapter implementing [`reader_runtime::EventSink`] by serializing each event
/// to JSON and forwarding it through the C callback.
///
/// `Send + Sync`: the callback + context are raw pointers, but the C side
/// guarantees the callback and context remain valid until `rc_runtime_destroy`
/// (which joins the worker before returning). The C side also guarantees
/// destroy is not called reentrantly from this callback. The context is opaque
/// to Rust and only handed back to C, which is the one place it's dereferenced.
pub struct CEventSink {
    callback: CEventCallback,
    context: *mut c_void,
}

// SAFETY: the C contract guarantees the callback/context are valid for the
// lifetime of the runtime, does not destroy reentrantly from the callback, and
// we only pass the context back to C.
unsafe impl Send for CEventSink {}
unsafe impl Sync for CEventSink {}

impl CEventSink {
    pub fn new(callback: CEventCallback, context: *mut c_void) -> Self {
        Self { callback, context }
    }
}

impl reader_runtime::EventSink for CEventSink {
    fn emit(&self, event: &reader_contract::Event) {
        let Some(callback) = self.callback else {
            return;
        };
        // Serialize; on failure there's nowhere to report, so skip silently.
        let Ok(json) = serde_json::to_vec(event) else {
            return;
        };
        // SAFETY: the C contract guarantees the callback and context are valid
        // until destroy joins this worker; the header forbids reentrant destroy
        // from this callback. The buffer is borrowed for the call only and
        // never freed by the platform.
        unsafe { callback(self.context, json.as_ptr(), json.len()) };
    }
}
