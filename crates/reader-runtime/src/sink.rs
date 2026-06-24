use reader_contract::Event;

/// Sink for Core → platform events.
///
/// Implementations MUST be `Send + Sync`: [`Runtime`] invokes `emit` from a
/// Core-owned background worker thread. The FFI implementation serializes the
/// event to JSON and forwards it through the C callback documented in
/// `include/reader_core.h` (buffer valid only for the duration of the call).
pub trait EventSink: Send + Sync {
    fn emit(&self, event: &Event);
}
