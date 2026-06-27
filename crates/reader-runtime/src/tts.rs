//! TTS vertical command handlers (V1 minimal).
//!
//! Core owns text slicing, the playback queue state machine, and chapter
//! boundary transitions. System vocalization (actual audio output) remains a
//! host responsibility — see `docs/host-app-contracts/05-tts.md`.
//!
//! # V1 boundaries (honest, no over-claiming)
//!
//! - `tts.slice`: real pure-logic slicer (`LineBreak` / `Paragraph` /
//!   `Sentence` / `ParagraphThenSentence`). `LineBreak` mirrors Legado
//!   `TTS.kt:88` (`splitNotBlank("\n")` + `QUEUE_ADD` + `utteranceId = tag +
//!   index`): split on `\n`, drop blank segments.
//! - `tts.queue.status`: returns an `Idle` snapshot. V1 has no queue control
//!   commands (`tts.queue.play/pause/resume/stop/next/prev` — Gap F in
//!   `docs/host-app-contracts/05-tts.md`), so Core's state machine has no
//!   transition inputs from the host.
//! - `tts.chapter.plan`: returns a transition with `next: None`. V1 params
//!   carry no TOC, so Core cannot resolve the next chapter (Gap G).

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use reader_contract::{
    self as contract,
    remote::parse_params,
    tts::{TtsChapterPlanParams, TtsQueueStatusParams, TtsSliceParams},
    CoreError, Event, TtsChapterPlanData, TtsChapterRef, TtsChapterTransition,
    TtsQueueDrainBehavior, TtsQueueSnapshot, TtsQueueState, TtsQueueStatusData, TtsSlice,
    TtsSliceData, TtsSlicePlan, TtsSlicingStrategy,
};
use serde_json::Value;

use crate::sink::EventSink;

/// Outcome of a TTS dispatch attempt.
///
/// Mirrors `remote::RemoteDispatch` but without a `Pending` variant: V1 TTS
/// is pure logic and never blocks on a host capability.
pub enum TtsDispatch {
    /// Method is not a `tts.*` command; caller should try other dispatchers.
    NotHandled,
    /// Method was handled and a result/error event has been emitted.
    Finished,
}

/// Dispatch entry point for `tts.*` methods.
///
/// Returns `NotHandled` for non-TTS methods so the runtime can fall through
/// to other vertical dispatchers (`dispatch_remote`) and ultimately
/// `unknown_method`.
pub fn dispatch_tts(
    method: &str,
    cmd: &contract::Command,
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
) -> TtsDispatch {
    let request_id = cmd.request_id;
    let result: Result<Value, CoreError> = match method {
        contract::methods::TTS_SLICE => tts_slice(cmd).and_then(serde_value),
        contract::methods::TTS_QUEUE_STATUS => tts_queue_status(cmd).and_then(serde_value),
        contract::methods::TTS_CHAPTER_PLAN => tts_chapter_plan(cmd).and_then(serde_value),
        _ => return TtsDispatch::NotHandled,
    };
    match result {
        Ok(data) => {
            finish(
                sink,
                active_requests,
                request_id,
                Event::result(request_id, data),
            );
            TtsDispatch::Finished
        }
        Err(err) => {
            finish(
                sink,
                active_requests,
                request_id,
                Event::error(request_id, err),
            );
            TtsDispatch::Finished
        }
    }
}

fn serde_value<T: serde::Serialize>(value: T) -> Result<Value, CoreError> {
    serde_json::to_value(value).map_err(|err| {
        CoreError::internal("tts serialization failed")
            .with_details(serde_json::json!({ "source": err.to_string() }))
    })
}

fn finish(
    sink: &Arc<dyn EventSink>,
    active_requests: &Mutex<HashSet<u64>>,
    request_id: u64,
    event: Event,
) {
    if let Ok(mut active) = active_requests.lock() {
        active.remove(&request_id);
    }
    sink.emit(&event);
}

fn tts_slice(cmd: &contract::Command) -> Result<TtsSliceData, CoreError> {
    let params: TtsSliceParams = parse_params(contract::methods::TTS_SLICE, &cmd.params)?;
    let plan = slice_chapter(params.chapter, &params.content, params.strategy);
    Ok(TtsSliceData { plan })
}

fn tts_queue_status(cmd: &contract::Command) -> Result<TtsQueueStatusData, CoreError> {
    let params: TtsQueueStatusParams =
        parse_params(contract::methods::TTS_QUEUE_STATUS, &cmd.params)?;
    let snapshot = queue_snapshot_for(&params.chapter);
    Ok(TtsQueueStatusData { snapshot })
}

fn tts_chapter_plan(cmd: &contract::Command) -> Result<TtsChapterPlanData, CoreError> {
    let params: TtsChapterPlanParams =
        parse_params(contract::methods::TTS_CHAPTER_PLAN, &cmd.params)?;
    let transition = chapter_transition_for(&params.chapter, params.drain_behavior);
    Ok(TtsChapterPlanData { transition })
}

/// Slice chapter content into speakable utterances using the requested
/// strategy. Pure logic — no platform dependencies.
///
/// Char offsets (`char_start` / `char_end`) are Unicode scalar offsets into
/// the original `content`, not byte offsets. This lets the host map slice
/// boundaries back to source positions for UI highlight sync regardless of
/// UTF-8 encoding.
pub fn slice_chapter(
    chapter: TtsChapterRef,
    content: &str,
    strategy: TtsSlicingStrategy,
) -> TtsSlicePlan {
    let chars: Vec<char> = content.chars().collect();
    let source_char_count = chars.len() as u32;
    let segments: Vec<(String, u32, u32, u32)> = match strategy {
        TtsSlicingStrategy::LineBreak => slice_line_break(&chars),
        TtsSlicingStrategy::Paragraph => slice_paragraph(&chars),
        TtsSlicingStrategy::Sentence => slice_sentence(&chars),
        TtsSlicingStrategy::ParagraphThenSentence => slice_paragraph_then_sentence(&chars),
    };
    let slices = segments
        .into_iter()
        .enumerate()
        .map(
            |(i, (text, char_start, char_end, paragraph_index))| TtsSlice {
                index: i as u32,
                text,
                char_start,
                char_end,
                paragraph_index,
            },
        )
        .collect();
    TtsSlicePlan {
        chapter,
        strategy,
        slices,
        source_char_count,
    }
}

/// Return the current queue snapshot for the given chapter.
///
/// V1 returns an `Idle` snapshot — there are no queue control commands to
/// drive state transitions (Gap F in `docs/host-app-contracts/05-tts.md`).
pub fn queue_snapshot_for(chapter: &TtsChapterRef) -> TtsQueueSnapshot {
    TtsQueueSnapshot {
        state: TtsQueueState::Idle,
        current_slice_index: None,
        total_slices: 0,
        completed_slices: 0,
        chapter: chapter.clone(),
        slice_statuses: vec![],
    }
}

/// Compute the chapter boundary transition plan.
///
/// V1 returns `next: None` because params carry no TOC, so Core cannot
/// resolve the next chapter (Gap G in `docs/host-app-contracts/05-tts.md`).
pub fn chapter_transition_for(
    chapter: &TtsChapterRef,
    drain_behavior: TtsQueueDrainBehavior,
) -> TtsChapterTransition {
    TtsChapterTransition {
        current: chapter.clone(),
        next: None,
        drain_behavior,
    }
}

// --- Slicer helpers --------------------------------------------------------

/// A line of content with its starting char offset in the original text.
struct Line {
    chars: Vec<char>,
    start: u32,
}

fn split_lines(chars: &[char]) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut current: Vec<char> = Vec::new();
    let mut start = 0u32;
    for (i, &c) in chars.iter().enumerate() {
        if c == '\n' {
            lines.push(Line {
                chars: std::mem::take(&mut current),
                start,
            });
            start = (i + 1) as u32;
        } else {
            current.push(c);
        }
    }
    // Final line (no trailing `\n`).
    lines.push(Line {
        chars: current,
        start,
    });
    lines
}

fn is_blank(chars: &[char]) -> bool {
    chars.iter().all(|c| c.is_whitespace())
}

/// Trim leading/trailing whitespace from a char slice and return
/// `(trimmed_text, char_start, char_end)` where the offsets are absolute
/// (i.e. `line_start`-relative).
fn trim_segment(chars: &[char], line_start: u32) -> (String, u32, u32) {
    let leading = chars.iter().take_while(|&&c| c.is_whitespace()).count();
    let trailing = chars
        .iter()
        .rev()
        .take_while(|&&c| c.is_whitespace())
        .count();
    let len = chars.len().saturating_sub(leading + trailing);
    let text: String = chars[leading..leading + len].iter().collect();
    let char_start = line_start + leading as u32;
    let char_end = char_start + len as u32;
    (text, char_start, char_end)
}

fn is_sentence_terminal(c: char) -> bool {
    matches!(c, '。' | '！' | '？' | '.' | '!' | '?')
}

/// LineBreak strategy: split on `\n`, drop blank segments.
/// Mirrors Legado `TTS.kt:88` (`splitNotBlank("\n")` + `QUEUE_ADD`).
fn slice_line_break(chars: &[char]) -> Vec<(String, u32, u32, u32)> {
    let lines = split_lines(chars);
    let mut result = Vec::new();
    let mut paragraph_index = 0u32;
    for line in &lines {
        if is_blank(&line.chars) {
            continue;
        }
        let (text, char_start, char_end) = trim_segment(&line.chars, line.start);
        if !text.is_empty() {
            result.push((text, char_start, char_end, paragraph_index));
            paragraph_index += 1;
        }
    }
    result
}

/// Paragraph strategy: split on blank lines. Each maximal run of non-blank
/// lines is one paragraph (and one slice).
fn slice_paragraph(chars: &[char]) -> Vec<(String, u32, u32, u32)> {
    let lines = split_lines(chars);
    let mut result = Vec::new();
    let mut paragraph_index = 0u32;
    let mut i = 0;
    while i < lines.len() {
        // Skip blank lines.
        while i < lines.len() && is_blank(&lines[i].chars) {
            i += 1;
        }
        if i >= lines.len() {
            break;
        }
        // Collect non-blank lines as one paragraph.
        let para_start = lines[i].start;
        let mut para_lines: Vec<&[char]> = Vec::new();
        while i < lines.len() && !is_blank(&lines[i].chars) {
            para_lines.push(&lines[i].chars[..]);
            i += 1;
        }
        // Join trimmed lines with `\n` and trim the whole paragraph.
        let joined: String = para_lines
            .iter()
            .map(|l| {
                let s: String = l.iter().collect();
                s.trim().to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        let text = joined.trim().to_string();
        if text.is_empty() {
            continue;
        }
        // char_start/char_end: trimmed range of the first line. Correct for
        // single-line paragraphs; approximate for multi-line (covers only
        // the first line, not the joined text).
        let (_, char_start, char_end) = trim_segment(para_lines[0], para_start);
        result.push((text, char_start, char_end, paragraph_index));
        paragraph_index += 1;
    }
    result
}

/// Sentence strategy: split on terminal punctuation (。！？.!?).
/// Each slice includes the terminal punctuation. Trailing text without
/// terminal punctuation becomes a final slice if non-blank.
fn slice_sentence(chars: &[char]) -> Vec<(String, u32, u32, u32)> {
    let mut result = Vec::new();
    let mut current: Vec<char> = Vec::new();
    let mut current_start = 0u32;
    for (i, &c) in chars.iter().enumerate() {
        current.push(c);
        if is_sentence_terminal(c) {
            let (text, char_start, char_end) = trim_segment(&current, current_start);
            if !text.is_empty() {
                result.push((text, char_start, char_end, 0));
            }
            current.clear();
            current_start = (i + 1) as u32;
        }
    }
    // Trailing text without terminal punctuation.
    let (text, char_start, char_end) = trim_segment(&current, current_start);
    if !text.is_empty() {
        result.push((text, char_start, char_end, 0));
    }
    result
}

/// ParagraphThenSentence strategy: split into paragraphs on blank lines,
/// then split each paragraph by sentence. `paragraph_index` reflects the
/// source paragraph; slice `index` is global across all sentences.
fn slice_paragraph_then_sentence(chars: &[char]) -> Vec<(String, u32, u32, u32)> {
    let lines = split_lines(chars);
    let mut result = Vec::new();
    let mut paragraph_index = 0u32;
    let mut i = 0;
    while i < lines.len() {
        while i < lines.len() && is_blank(&lines[i].chars) {
            i += 1;
        }
        if i >= lines.len() {
            break;
        }
        let para_start = lines[i].start;
        let mut para_chars: Vec<char> = Vec::new();
        while i < lines.len() && !is_blank(&lines[i].chars) {
            if !para_chars.is_empty() {
                para_chars.push('\n');
            }
            para_chars.extend(&lines[i].chars);
            i += 1;
        }
        let sentences = slice_sentence(&para_chars);
        for (text, rel_start, rel_end, _) in sentences {
            result.push((
                text,
                para_start + rel_start,
                para_start + rel_end,
                paragraph_index,
            ));
        }
        paragraph_index += 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use reader_contract::{Command, ErrorCode, TtsChapterRef};

    struct ChannelSink {
        tx: std::sync::mpsc::Sender<Event>,
    }
    impl EventSink for ChannelSink {
        fn emit(&self, event: &Event) {
            let _ = self.tx.send(event.clone());
        }
    }

    fn make_chapter() -> TtsChapterRef {
        TtsChapterRef {
            source_id: "src-1".into(),
            book_id: "book-1".into(),
            chapter_index: 0,
            chapter_title: "第一章".into(),
            chapter_url: String::new(),
        }
    }

    fn make_sink_and_active() -> (
        Arc<dyn EventSink>,
        std::sync::mpsc::Receiver<Event>,
        Mutex<HashSet<u64>>,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        let sink: Arc<dyn EventSink> = Arc::new(ChannelSink { tx });
        let active = Mutex::new(HashSet::new());
        (sink, rx, active)
    }

    fn recv(rx: &std::sync::mpsc::Receiver<Event>) -> Event {
        rx.recv_timeout(std::time::Duration::from_secs(2))
            .expect("tts dispatch should emit an event")
    }

    // --- Slicer tests ------------------------------------------------------

    #[test]
    fn slice_chapter_line_break_splits_on_newline_dropping_blanks() {
        // Mirrors Legado TTS.kt:88: splitNotBlank("\n") + QUEUE_ADD.
        // Blank lines (consecutive \n) are dropped, non-blank segments kept.
        let chapter = make_chapter();
        let content = "第一段。\n\n第二段。\n";
        let plan = slice_chapter(chapter.clone(), content, TtsSlicingStrategy::LineBreak);
        assert_eq!(plan.strategy, TtsSlicingStrategy::LineBreak);
        assert_eq!(plan.slices.len(), 2, "LineBreak drops blank lines");
        assert_eq!(plan.slices[0].text, "第一段。");
        assert_eq!(plan.slices[1].text, "第二段。");
        assert_eq!(plan.slices[0].index, 0);
        assert_eq!(plan.slices[1].index, 1);
        assert_eq!(plan.chapter, chapter);
        assert_eq!(
            plan.source_char_count,
            content.chars().count() as u32,
            "source_char_count is the char count of the original content"
        );
    }

    #[test]
    fn slice_chapter_paragraph_splits_on_blank_lines() {
        let chapter = make_chapter();
        let content = "第一段。\n\n第二段。";
        let plan = slice_chapter(chapter, content, TtsSlicingStrategy::Paragraph);
        assert_eq!(plan.slices.len(), 2);
        assert_eq!(plan.slices[0].text, "第一段。");
        assert_eq!(plan.slices[1].text, "第二段。");
    }

    #[test]
    fn slice_chapter_sentence_splits_on_terminal_punctuation() {
        let chapter = make_chapter();
        let content = "第一句。第二句！第三句？";
        let plan = slice_chapter(chapter, content, TtsSlicingStrategy::Sentence);
        assert_eq!(plan.slices.len(), 3);
        assert_eq!(plan.slices[0].text, "第一句。");
        assert_eq!(plan.slices[1].text, "第二句！");
        assert_eq!(plan.slices[2].text, "第三句？");
    }

    #[test]
    fn slice_chapter_paragraph_then_sentence_hybrid() {
        // Short paragraph stays whole; long paragraph splits by sentence.
        let chapter = make_chapter();
        let content = "短段。\n\n这是第一句。这是第二句。";
        let plan = slice_chapter(chapter, content, TtsSlicingStrategy::ParagraphThenSentence);
        assert_eq!(
            plan.slices.len(),
            3,
            "second paragraph splits into 2 sentences"
        );
        assert_eq!(plan.slices[0].text, "短段。");
        assert_eq!(plan.slices[1].text, "这是第一句。");
        assert_eq!(plan.slices[2].text, "这是第二句。");
    }

    #[test]
    fn slice_chapter_sets_char_ranges_as_unicode_scalar_offsets() {
        let chapter = make_chapter();
        let content = "第一段。第二段。";
        let plan = slice_chapter(chapter, content, TtsSlicingStrategy::Sentence);
        assert_eq!(plan.slices.len(), 2);
        // char_start/char_end are char offsets, not byte offsets.
        assert_eq!(plan.slices[0].char_start, 0);
        assert_eq!(plan.slices[0].char_end, 4, "\"第一段。\" is 4 chars");
        assert_eq!(plan.slices[1].char_start, 4);
        assert_eq!(plan.slices[1].char_end, 8);
    }

    #[test]
    fn slice_chapter_empty_content_yields_empty_slices() {
        // Slicer handles empty gracefully; dispatch layer rejects empty via
        // the contract's non-empty text validator (see dispatch tests).
        let chapter = make_chapter();
        let plan = slice_chapter(chapter, "", TtsSlicingStrategy::LineBreak);
        assert!(plan.slices.is_empty());
        assert_eq!(plan.source_char_count, 0);
    }

    #[test]
    fn slice_chapter_assigns_paragraph_index_for_line_break_strategy() {
        let chapter = make_chapter();
        let content = "第一段。\n第二段。\n第三段。";
        let plan = slice_chapter(chapter, content, TtsSlicingStrategy::LineBreak);
        assert_eq!(plan.slices.len(), 3);
        assert_eq!(plan.slices[0].paragraph_index, 0);
        assert_eq!(plan.slices[1].paragraph_index, 1);
        assert_eq!(plan.slices[2].paragraph_index, 2);
    }

    // --- Queue status tests ------------------------------------------------

    #[test]
    fn queue_snapshot_for_returns_idle_for_any_chapter() {
        let chapter = make_chapter();
        let snap = queue_snapshot_for(&chapter);
        assert_eq!(
            snap.state,
            TtsQueueState::Idle,
            "V1 queue has no transitions"
        );
        assert_eq!(snap.current_slice_index, None);
        assert_eq!(snap.total_slices, 0);
        assert_eq!(snap.completed_slices, 0);
        assert!(snap.slice_statuses.is_empty());
        assert_eq!(snap.chapter, chapter);
    }

    // --- Chapter transition tests ------------------------------------------

    #[test]
    fn chapter_transition_for_returns_no_next_when_no_toc() {
        let chapter = make_chapter();
        let transition = chapter_transition_for(&chapter, TtsQueueDrainBehavior::StopOnBoundary);
        assert_eq!(transition.current, chapter);
        assert!(transition.next.is_none(), "V1 params carry no TOC");
        assert_eq!(
            transition.drain_behavior,
            TtsQueueDrainBehavior::StopOnBoundary
        );
    }

    #[test]
    fn chapter_transition_for_echoes_drain_behavior() {
        let chapter = make_chapter();
        let transition = chapter_transition_for(&chapter, TtsQueueDrainBehavior::AdvanceToNext);
        assert_eq!(
            transition.drain_behavior,
            TtsQueueDrainBehavior::AdvanceToNext
        );
        assert!(
            transition.next.is_none(),
            "V1 still has no TOC even with advance"
        );
    }

    // --- Dispatch tests (direct, via dispatch_tts) -------------------------

    #[test]
    fn dispatch_tts_slice_returns_result_event_with_plan() {
        let (sink, rx, active) = make_sink_and_active();
        let cmd = Command::new(
            420,
            contract::methods::TTS_SLICE,
            serde_json::json!({
                "chapter": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 0 },
                "content": "第一段。\n第二段。",
                "strategy": "line-break"
            }),
        );
        let outcome = dispatch_tts(contract::methods::TTS_SLICE, &cmd, &sink, &active);
        assert!(matches!(outcome, TtsDispatch::Finished));
        match recv(&rx) {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(request_id, 420);
                let data: TtsSliceData = serde_json::from_value(data).unwrap();
                assert_eq!(data.plan.slices.len(), 2);
                assert_eq!(data.plan.strategy, TtsSlicingStrategy::LineBreak);
            }
            other => panic!("unexpected event {other:?}"),
        }
    }

    #[test]
    fn dispatch_tts_queue_status_returns_result_event_with_idle_snapshot() {
        let (sink, rx, active) = make_sink_and_active();
        let cmd = Command::new(
            421,
            contract::methods::TTS_QUEUE_STATUS,
            serde_json::json!({
                "chapter": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 2 }
            }),
        );
        let outcome = dispatch_tts(contract::methods::TTS_QUEUE_STATUS, &cmd, &sink, &active);
        assert!(matches!(outcome, TtsDispatch::Finished));
        match recv(&rx) {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(request_id, 421);
                let data: TtsQueueStatusData = serde_json::from_value(data).unwrap();
                assert_eq!(data.snapshot.state, TtsQueueState::Idle);
                assert_eq!(data.snapshot.total_slices, 0);
            }
            other => panic!("unexpected event {other:?}"),
        }
    }

    #[test]
    fn dispatch_tts_chapter_plan_returns_result_event_with_no_next() {
        let (sink, rx, active) = make_sink_and_active();
        let cmd = Command::new(
            422,
            contract::methods::TTS_CHAPTER_PLAN,
            serde_json::json!({
                "chapter": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 2 },
                "drainBehavior": "advance-to-next"
            }),
        );
        let outcome = dispatch_tts(contract::methods::TTS_CHAPTER_PLAN, &cmd, &sink, &active);
        assert!(matches!(outcome, TtsDispatch::Finished));
        match recv(&rx) {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(request_id, 422);
                let data: TtsChapterPlanData = serde_json::from_value(data).unwrap();
                assert!(data.transition.next.is_none());
                assert_eq!(
                    data.transition.drain_behavior,
                    TtsQueueDrainBehavior::AdvanceToNext
                );
            }
            other => panic!("unexpected event {other:?}"),
        }
    }

    #[test]
    fn dispatch_tts_slice_rejects_empty_content_with_invalid_params() {
        let (sink, rx, active) = make_sink_and_active();
        let cmd = Command::new(
            423,
            contract::methods::TTS_SLICE,
            serde_json::json!({
                "chapter": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 0 },
                "content": ""
            }),
        );
        let outcome = dispatch_tts(contract::methods::TTS_SLICE, &cmd, &sink, &active);
        assert!(matches!(outcome, TtsDispatch::Finished));
        match recv(&rx) {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(request_id, 423);
                assert_eq!(error.code, ErrorCode::InvalidParams);
            }
            other => panic!("unexpected event {other:?}"),
        }
    }

    #[test]
    fn dispatch_tts_slice_rejects_unknown_field_with_invalid_params() {
        let (sink, rx, active) = make_sink_and_active();
        let cmd = Command::new(
            424,
            contract::methods::TTS_SLICE,
            serde_json::json!({
                "chapter": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 0 },
                "content": "text",
                "unexpectedField": true
            }),
        );
        let outcome = dispatch_tts(contract::methods::TTS_SLICE, &cmd, &sink, &active);
        assert!(matches!(outcome, TtsDispatch::Finished));
        match recv(&rx) {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(request_id, 424);
                assert_eq!(error.code, ErrorCode::InvalidParams);
            }
            other => panic!("unexpected event {other:?}"),
        }
    }

    #[test]
    fn dispatch_tts_queue_status_rejects_unknown_field() {
        let (sink, rx, active) = make_sink_and_active();
        let cmd = Command::new(
            425,
            contract::methods::TTS_QUEUE_STATUS,
            serde_json::json!({
                "chapter": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 0 },
                "unexpectedField": true
            }),
        );
        let outcome = dispatch_tts(contract::methods::TTS_QUEUE_STATUS, &cmd, &sink, &active);
        assert!(matches!(outcome, TtsDispatch::Finished));
        match recv(&rx) {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(request_id, 425);
                assert_eq!(error.code, ErrorCode::InvalidParams);
            }
            other => panic!("unexpected event {other:?}"),
        }
    }

    #[test]
    fn dispatch_tts_chapter_plan_rejects_unknown_field() {
        let (sink, rx, active) = make_sink_and_active();
        let cmd = Command::new(
            426,
            contract::methods::TTS_CHAPTER_PLAN,
            serde_json::json!({
                "chapter": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 0 },
                "unexpectedField": true
            }),
        );
        let outcome = dispatch_tts(contract::methods::TTS_CHAPTER_PLAN, &cmd, &sink, &active);
        assert!(matches!(outcome, TtsDispatch::Finished));
        match recv(&rx) {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(request_id, 426);
                assert_eq!(error.code, ErrorCode::InvalidParams);
            }
            other => panic!("unexpected event {other:?}"),
        }
    }

    #[test]
    fn dispatch_tts_unknown_method_returns_not_handled() {
        let (sink, _rx, active) = make_sink_and_active();
        let cmd = Command::new(999, "tts.unknown", serde_json::json!({}));
        let outcome = dispatch_tts("tts.unknown", &cmd, &sink, &active);
        assert!(matches!(outcome, TtsDispatch::NotHandled));
    }

    #[test]
    fn dispatch_tts_slice_default_strategy_is_paragraph() {
        // Omitting strategy should default to Paragraph per the contract's
        // `#[default]` on TtsSlicingStrategy.
        let (sink, rx, active) = make_sink_and_active();
        let cmd = Command::new(
            430,
            contract::methods::TTS_SLICE,
            serde_json::json!({
                "chapter": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 0 },
                "content": "第一段。\n\n第二段。"
            }),
        );
        let outcome = dispatch_tts(contract::methods::TTS_SLICE, &cmd, &sink, &active);
        assert!(matches!(outcome, TtsDispatch::Finished));
        match recv(&rx) {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(request_id, 430);
                let data: TtsSliceData = serde_json::from_value(data).unwrap();
                assert_eq!(data.plan.strategy, TtsSlicingStrategy::Paragraph);
                assert_eq!(data.plan.slices.len(), 2);
            }
            other => panic!("unexpected event {other:?}"),
        }
    }
}
