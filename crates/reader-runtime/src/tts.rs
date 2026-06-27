//! TTS vertical command handlers.
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
//! - `tts.queue.play/pause/resume/stop/next/prev`: Core-owned queue state
//!   machine (Gap F closure). State transitions mirror Legado
//!   `BaseReadAloudService` control actions (`play` / `pauseReadAloud` /
//!   `resumeReadAloud` / `stopSelf` / `nextP` / `prevP`). Core holds the queue
//!   state per chapter; the host drives transitions and vocalizes the slice at
//!   `currentSliceIndex`. Core never emits audio.
//! - `tts.queue.status`: returns the real queue snapshot (state +
//!   `currentSliceIndex` + `sliceStatuses`) from the in-memory state machine.
//!   Returns `Idle` for chapters with no loaded queue.
//! - `tts.chapter.plan`: returns a transition with `next: None`. V1 params
//!   carry no TOC, so Core cannot resolve the next chapter (Gap G).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use reader_contract::{
    self as contract,
    remote::parse_params,
    tts::{
        TtsChapterPlanParams, TtsQueueNextParams, TtsQueuePauseParams, TtsQueuePlayParams,
        TtsQueuePrevParams, TtsQueueResumeParams, TtsQueueStatusParams, TtsQueueStopParams,
        TtsSliceParams,
    },
    CoreError, Event, TtsChapterPlanData, TtsChapterRef, TtsChapterTransition,
    TtsQueueDrainBehavior, TtsQueueNextData, TtsQueuePauseData, TtsQueuePlayData, TtsQueuePrevData,
    TtsQueueResumeData, TtsQueueSnapshot, TtsQueueState, TtsQueueStatusData, TtsQueueStopData,
    TtsSlice, TtsSliceData, TtsSlicePlan, TtsSliceStatus, TtsSlicingStrategy,
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

/// In-memory TTS playback queue state, keyed by chapter.
///
/// Core holds this so `tts.queue.status` can return a real snapshot and the
/// queue control commands (`play/pause/resume/stop/next/prev`) can drive
/// state transitions. The host never reads this directly — it observes
/// snapshots via the protocol. Pure logic: no audio, no platform APIs.
#[derive(Debug, Default)]
pub struct TtsState {
    queues: Mutex<HashMap<ChapterKey, TtsQueueEntry>>,
}

impl TtsState {
    /// Create an empty TTS state (no loaded queues).
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot for a chapter. Returns an `Idle` snapshot if no queue is
    /// loaded for the chapter.
    pub fn snapshot_for(&self, chapter: &TtsChapterRef) -> TtsQueueSnapshot {
        let key = ChapterKey::from(chapter);
        let queues = match self.queues.lock() {
            Ok(g) => g,
            Err(_) => {
                return TtsQueueSnapshot {
                    state: TtsQueueState::Idle,
                    current_slice_index: None,
                    total_slices: 0,
                    completed_slices: 0,
                    chapter: chapter.clone(),
                    slice_statuses: vec![],
                }
            }
        };
        match queues.get(&key) {
            Some(entry) => entry.snapshot(),
            None => TtsQueueSnapshot {
                state: TtsQueueState::Idle,
                current_slice_index: None,
                total_slices: 0,
                completed_slices: 0,
                chapter: chapter.clone(),
                slice_statuses: vec![],
            },
        }
    }

    /// Apply a `play` transition. Loads `plan` and starts at `start_slice_index`.
    pub fn play(&self, params: TtsQueuePlayParams) -> Result<TtsQueueSnapshot, CoreError> {
        let plan = params.plan;
        let total = plan.slices.len() as u32;
        if total == 0 {
            return Err(CoreError::invalid_params(
                "tts.queue.play requires a non-empty slice plan",
            )
            .with_details(serde_json::json!({ "action": "play" })));
        }
        if params.start_slice_index >= total {
            return Err(
                CoreError::invalid_params("tts.queue.play startSliceIndex out of range")
                    .with_details(serde_json::json!({
                        "action": "play",
                        "startSliceIndex": params.start_slice_index,
                        "totalSlices": total,
                    })),
            );
        }
        let key = ChapterKey::from(&plan.chapter);
        let mut queues = self.queues.lock().map_err(|_| {
            CoreError::internal("tts queue state poisoned").with_details(serde_json::json!({
                "action": "play"
            }))
        })?;
        if let Some(existing) = queues.get(&key) {
            if matches!(
                existing.state,
                TtsQueueState::Playing | TtsQueueState::Paused
            ) {
                return Err(CoreError::invalid_params(
                    "tts.queue.play rejected: queue already active, use pause/resume",
                )
                .with_details(serde_json::json!({
                    "action": "play",
                    "currentState": state_as_str(existing.state),
                })));
            }
        }
        let mut slice_statuses = vec![TtsSliceStatus::Pending; plan.slices.len()];
        slice_statuses[params.start_slice_index as usize] = TtsSliceStatus::Speaking;
        let entry = TtsQueueEntry {
            state: TtsQueueState::Playing,
            current_slice_index: params.start_slice_index,
            slice_statuses,
            plan,
        };
        let snapshot = entry.snapshot();
        queues.insert(key, entry);
        Ok(snapshot)
    }

    /// Apply a `pause` transition. Requires `Playing`.
    pub fn pause(&self, params: TtsQueuePauseParams) -> Result<TtsQueueSnapshot, CoreError> {
        self.transition(params.chapter, "pause", |entry| {
            if !matches!(entry.state, TtsQueueState::Playing) {
                return Err(state_error("pause", entry.state));
            }
            entry.state = TtsQueueState::Paused;
            Ok(())
        })
    }

    /// Apply a `resume` transition. Requires `Paused`.
    pub fn resume(&self, params: TtsQueueResumeParams) -> Result<TtsQueueSnapshot, CoreError> {
        self.transition(params.chapter, "resume", |entry| {
            if !matches!(entry.state, TtsQueueState::Paused) {
                return Err(state_error("resume", entry.state));
            }
            entry.state = TtsQueueState::Playing;
            Ok(())
        })
    }

    /// Apply a `stop` transition. Requires a non-`Idle`/non-`Stopped` queue.
    pub fn stop(&self, params: TtsQueueStopParams) -> Result<TtsQueueSnapshot, CoreError> {
        self.transition(params.chapter, "stop", |entry| {
            if matches!(
                entry.state,
                TtsQueueState::Stopped | TtsQueueState::Completed
            ) {
                return Err(state_error("stop", entry.state));
            }
            entry.state = TtsQueueState::Stopped;
            Ok(())
        })
    }

    /// Apply a `next` transition. Marks the current slice `Done` and advances
    /// the cursor. At the last slice, enters `Completed`. Cross-chapter
    /// advance is Gap G (out of V1 scope).
    pub fn next(&self, params: TtsQueueNextParams) -> Result<TtsQueueSnapshot, CoreError> {
        self.transition(params.chapter, "next", |entry| {
            if !matches!(entry.state, TtsQueueState::Playing | TtsQueueState::Paused) {
                return Err(state_error("next", entry.state));
            }
            let last = entry.plan.slices.len().saturating_sub(1) as u32;
            if entry.current_slice_index >= last {
                // Mark current Done and enter Completed.
                let i = entry.current_slice_index as usize;
                entry.slice_statuses[i] = TtsSliceStatus::Done;
                entry.state = TtsQueueState::Completed;
                return Ok(());
            }
            let i = entry.current_slice_index as usize;
            entry.slice_statuses[i] = TtsSliceStatus::Done;
            entry.current_slice_index += 1;
            entry.slice_statuses[entry.current_slice_index as usize] = TtsSliceStatus::Speaking;
            Ok(())
        })
    }

    /// Apply a `prev` transition. Moves the cursor backward. Errors at the
    /// first slice. Cross-chapter retreat is Gap G (out of V1 scope).
    pub fn prev(&self, params: TtsQueuePrevParams) -> Result<TtsQueueSnapshot, CoreError> {
        self.transition(params.chapter, "prev", |entry| {
            if !matches!(entry.state, TtsQueueState::Playing | TtsQueueState::Paused) {
                return Err(state_error("prev", entry.state));
            }
            if entry.current_slice_index == 0 {
                return Err(CoreError::invalid_params(
                    "tts.queue.prev rejected: already at first slice",
                )
                .with_details(serde_json::json!({
                    "action": "prev",
                    "currentState": state_as_str(entry.state),
                    "currentSliceIndex": entry.current_slice_index,
                })));
            }
            // Current slice returns to Pending; new current becomes Speaking.
            let i = entry.current_slice_index as usize;
            entry.slice_statuses[i] = TtsSliceStatus::Pending;
            entry.current_slice_index -= 1;
            entry.slice_statuses[entry.current_slice_index as usize] = TtsSliceStatus::Speaking;
            Ok(())
        })
    }

    fn transition(
        &self,
        chapter: TtsChapterRef,
        action: &'static str,
        f: impl FnOnce(&mut TtsQueueEntry) -> Result<(), CoreError>,
    ) -> Result<TtsQueueSnapshot, CoreError> {
        let key = ChapterKey::from(&chapter);
        let mut queues = self.queues.lock().map_err(|_| {
            CoreError::internal("tts queue state poisoned")
                .with_details(serde_json::json!({ "action": action }))
        })?;
        let entry = queues.get_mut(&key).ok_or_else(|| {
            CoreError::invalid_params("tts queue control rejected: no queue loaded for chapter")
                .with_details(serde_json::json!({
                    "action": action,
                    "chapter": serde_json::to_value(&chapter).unwrap_or_default(),
                }))
        })?;
        f(entry)?;
        Ok(entry.snapshot())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ChapterKey {
    source_id: String,
    book_id: String,
    chapter_index: u32,
}

impl From<&TtsChapterRef> for ChapterKey {
    fn from(ch: &TtsChapterRef) -> Self {
        Self {
            source_id: ch.source_id.clone(),
            book_id: ch.book_id.clone(),
            chapter_index: ch.chapter_index,
        }
    }
}

/// One chapter's queue state. Held inside [`TtsState`].
#[derive(Debug)]
struct TtsQueueEntry {
    state: TtsQueueState,
    current_slice_index: u32,
    slice_statuses: Vec<TtsSliceStatus>,
    plan: TtsSlicePlan,
}

impl TtsQueueEntry {
    fn snapshot(&self) -> TtsQueueSnapshot {
        let completed_slices = self
            .slice_statuses
            .iter()
            .filter(|s| {
                matches!(
                    s,
                    TtsSliceStatus::Done | TtsSliceStatus::Failed | TtsSliceStatus::Skipped
                )
            })
            .count() as u32;
        TtsQueueSnapshot {
            state: self.state,
            current_slice_index: Some(self.current_slice_index),
            total_slices: self.plan.slices.len() as u32,
            completed_slices,
            chapter: self.plan.chapter.clone(),
            slice_statuses: self.slice_statuses.clone(),
        }
    }
}

fn state_error(action: &str, current: TtsQueueState) -> CoreError {
    CoreError::invalid_params(format!(
        "tts.queue.{action} rejected: invalid state transition from {current:?}"
    ))
    .with_details(serde_json::json!({
        "action": action,
        "currentState": state_as_str(current),
    }))
}

/// Map a `TtsQueueState` to its protocol wire string (matches the
/// `#[serde(rename_all = "lowercase")]` serialization in the contract).
fn state_as_str(state: TtsQueueState) -> &'static str {
    match state {
        TtsQueueState::Idle => "idle",
        TtsQueueState::Playing => "playing",
        TtsQueueState::Paused => "paused",
        TtsQueueState::Completed => "completed",
        TtsQueueState::Stopped => "stopped",
    }
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
    tts_state: &TtsState,
) -> TtsDispatch {
    let request_id = cmd.request_id;
    let result: Result<Value, CoreError> = match method {
        contract::methods::TTS_SLICE => tts_slice(cmd).and_then(serde_value),
        contract::methods::TTS_QUEUE_STATUS => {
            tts_queue_status(cmd, tts_state).and_then(serde_value)
        }
        contract::methods::TTS_CHAPTER_PLAN => tts_chapter_plan(cmd).and_then(serde_value),
        contract::methods::TTS_QUEUE_PLAY => tts_queue_play(cmd, tts_state).and_then(serde_value),
        contract::methods::TTS_QUEUE_PAUSE => tts_queue_pause(cmd, tts_state).and_then(serde_value),
        contract::methods::TTS_QUEUE_RESUME => {
            tts_queue_resume(cmd, tts_state).and_then(serde_value)
        }
        contract::methods::TTS_QUEUE_STOP => tts_queue_stop(cmd, tts_state).and_then(serde_value),
        contract::methods::TTS_QUEUE_NEXT => tts_queue_next(cmd, tts_state).and_then(serde_value),
        contract::methods::TTS_QUEUE_PREV => tts_queue_prev(cmd, tts_state).and_then(serde_value),
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

fn tts_queue_status(
    cmd: &contract::Command,
    tts_state: &TtsState,
) -> Result<TtsQueueStatusData, CoreError> {
    let params: TtsQueueStatusParams =
        parse_params(contract::methods::TTS_QUEUE_STATUS, &cmd.params)?;
    let snapshot = tts_state.snapshot_for(&params.chapter);
    Ok(TtsQueueStatusData { snapshot })
}

fn tts_chapter_plan(cmd: &contract::Command) -> Result<TtsChapterPlanData, CoreError> {
    let params: TtsChapterPlanParams =
        parse_params(contract::methods::TTS_CHAPTER_PLAN, &cmd.params)?;
    let transition = chapter_transition_for(&params.chapter, params.drain_behavior);
    Ok(TtsChapterPlanData { transition })
}

fn tts_queue_play(
    cmd: &contract::Command,
    tts_state: &TtsState,
) -> Result<TtsQueuePlayData, CoreError> {
    let params: TtsQueuePlayParams = parse_params(contract::methods::TTS_QUEUE_PLAY, &cmd.params)?;
    let snapshot = tts_state.play(params)?;
    Ok(TtsQueuePlayData { snapshot })
}

fn tts_queue_pause(
    cmd: &contract::Command,
    tts_state: &TtsState,
) -> Result<TtsQueuePauseData, CoreError> {
    let params: TtsQueuePauseParams =
        parse_params(contract::methods::TTS_QUEUE_PAUSE, &cmd.params)?;
    let snapshot = tts_state.pause(params)?;
    Ok(TtsQueuePauseData { snapshot })
}

fn tts_queue_resume(
    cmd: &contract::Command,
    tts_state: &TtsState,
) -> Result<TtsQueueResumeData, CoreError> {
    let params: TtsQueueResumeParams =
        parse_params(contract::methods::TTS_QUEUE_RESUME, &cmd.params)?;
    let snapshot = tts_state.resume(params)?;
    Ok(TtsQueueResumeData { snapshot })
}

fn tts_queue_stop(
    cmd: &contract::Command,
    tts_state: &TtsState,
) -> Result<TtsQueueStopData, CoreError> {
    let params: TtsQueueStopParams = parse_params(contract::methods::TTS_QUEUE_STOP, &cmd.params)?;
    let snapshot = tts_state.stop(params)?;
    Ok(TtsQueueStopData { snapshot })
}

fn tts_queue_next(
    cmd: &contract::Command,
    tts_state: &TtsState,
) -> Result<TtsQueueNextData, CoreError> {
    let params: TtsQueueNextParams = parse_params(contract::methods::TTS_QUEUE_NEXT, &cmd.params)?;
    let snapshot = tts_state.next(params)?;
    Ok(TtsQueueNextData { snapshot })
}

fn tts_queue_prev(
    cmd: &contract::Command,
    tts_state: &TtsState,
) -> Result<TtsQueuePrevData, CoreError> {
    let params: TtsQueuePrevParams = parse_params(contract::methods::TTS_QUEUE_PREV, &cmd.params)?;
    let snapshot = tts_state.prev(params)?;
    Ok(TtsQueuePrevData { snapshot })
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

/// Return the current queue snapshot for the given chapter from the shared
/// TTS state. Returns `Idle` if no queue is loaded for the chapter.
pub fn queue_snapshot_for(chapter: &TtsChapterRef, tts_state: &TtsState) -> TtsQueueSnapshot {
    tts_state.snapshot_for(chapter)
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
        TtsState,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        let sink: Arc<dyn EventSink> = Arc::new(ChannelSink { tx });
        let active = Mutex::new(HashSet::new());
        let state = TtsState::new();
        (sink, rx, active, state)
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
        let state = TtsState::new();
        let snap = queue_snapshot_for(&chapter, &state);
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
        let (sink, rx, active, state) = make_sink_and_active();
        let cmd = Command::new(
            420,
            contract::methods::TTS_SLICE,
            serde_json::json!({
                "chapter": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 0 },
                "content": "第一段。\n第二段。",
                "strategy": "line-break"
            }),
        );
        let outcome = dispatch_tts(contract::methods::TTS_SLICE, &cmd, &sink, &active, &state);
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
        let (sink, rx, active, state) = make_sink_and_active();
        let cmd = Command::new(
            421,
            contract::methods::TTS_QUEUE_STATUS,
            serde_json::json!({
                "chapter": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 2 }
            }),
        );
        let outcome = dispatch_tts(
            contract::methods::TTS_QUEUE_STATUS,
            &cmd,
            &sink,
            &active,
            &state,
        );
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
        let (sink, rx, active, state) = make_sink_and_active();
        let cmd = Command::new(
            422,
            contract::methods::TTS_CHAPTER_PLAN,
            serde_json::json!({
                "chapter": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 2 },
                "drainBehavior": "advance-to-next"
            }),
        );
        let outcome = dispatch_tts(
            contract::methods::TTS_CHAPTER_PLAN,
            &cmd,
            &sink,
            &active,
            &state,
        );
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
        let (sink, rx, active, state) = make_sink_and_active();
        let cmd = Command::new(
            423,
            contract::methods::TTS_SLICE,
            serde_json::json!({
                "chapter": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 0 },
                "content": ""
            }),
        );
        let outcome = dispatch_tts(contract::methods::TTS_SLICE, &cmd, &sink, &active, &state);
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
        let (sink, rx, active, state) = make_sink_and_active();
        let cmd = Command::new(
            424,
            contract::methods::TTS_SLICE,
            serde_json::json!({
                "chapter": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 0 },
                "content": "text",
                "unexpectedField": true
            }),
        );
        let outcome = dispatch_tts(contract::methods::TTS_SLICE, &cmd, &sink, &active, &state);
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
        let (sink, rx, active, state) = make_sink_and_active();
        let cmd = Command::new(
            425,
            contract::methods::TTS_QUEUE_STATUS,
            serde_json::json!({
                "chapter": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 0 },
                "unexpectedField": true
            }),
        );
        let outcome = dispatch_tts(
            contract::methods::TTS_QUEUE_STATUS,
            &cmd,
            &sink,
            &active,
            &state,
        );
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
        let (sink, rx, active, state) = make_sink_and_active();
        let cmd = Command::new(
            426,
            contract::methods::TTS_CHAPTER_PLAN,
            serde_json::json!({
                "chapter": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 0 },
                "unexpectedField": true
            }),
        );
        let outcome = dispatch_tts(
            contract::methods::TTS_CHAPTER_PLAN,
            &cmd,
            &sink,
            &active,
            &state,
        );
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
        let (sink, _rx, active, state) = make_sink_and_active();
        let cmd = Command::new(999, "tts.unknown", serde_json::json!({}));
        let outcome = dispatch_tts("tts.unknown", &cmd, &sink, &active, &state);
        assert!(matches!(outcome, TtsDispatch::NotHandled));
    }

    #[test]
    fn dispatch_tts_slice_default_strategy_is_paragraph() {
        // Omitting strategy should default to Paragraph per the contract's
        // `#[default]` on TtsSlicingStrategy.
        let (sink, rx, active, state) = make_sink_and_active();
        let cmd = Command::new(
            430,
            contract::methods::TTS_SLICE,
            serde_json::json!({
                "chapter": { "sourceId": "src-1", "bookId": "book-1", "chapterIndex": 0 },
                "content": "第一段。\n\n第二段。"
            }),
        );
        let outcome = dispatch_tts(contract::methods::TTS_SLICE, &cmd, &sink, &active, &state);
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

    // --- Queue state machine tests (Gap F closure) ------------------------
    //
    // Mirrors Legado BaseReadAloudService control actions (play / pause /
    // resume / stop / nextP / prevP). Pure logic; no audio. Verifies:
    //   - Valid transitions update state + sliceStatuses correctly
    //   - Invalid transitions return InvalidParams with details
    //   - tts.queue.status returns the real snapshot (not just Idle)

    fn sample_plan_for_runtime() -> TtsSlicePlan {
        let chapter = make_chapter();
        TtsSlicePlan {
            chapter,
            strategy: TtsSlicingStrategy::Paragraph,
            slices: vec![
                TtsSlice {
                    index: 0,
                    text: "第一段。".into(),
                    char_start: 0,
                    char_end: 4,
                    paragraph_index: 0,
                },
                TtsSlice {
                    index: 1,
                    text: "第二段。".into(),
                    char_start: 4,
                    char_end: 8,
                    paragraph_index: 1,
                },
                TtsSlice {
                    index: 2,
                    text: "第三段。".into(),
                    char_start: 8,
                    char_end: 12,
                    paragraph_index: 2,
                },
            ],
            source_char_count: 12,
        }
    }

    #[test]
    fn play_loads_plan_and_starts_at_slice_zero() {
        let state = TtsState::new();
        let plan = sample_plan_for_runtime();
        let chapter = plan.chapter.clone();
        let snap = state
            .play(TtsQueuePlayParams {
                plan,
                start_slice_index: 0,
            })
            .expect("play from Idle must succeed");
        assert_eq!(snap.state, TtsQueueState::Playing);
        assert_eq!(snap.current_slice_index, Some(0));
        assert_eq!(snap.total_slices, 3);
        assert_eq!(snap.completed_slices, 0);
        assert_eq!(
            snap.slice_statuses,
            vec![
                TtsSliceStatus::Speaking,
                TtsSliceStatus::Pending,
                TtsSliceStatus::Pending,
            ]
        );
        assert_eq!(snap.chapter, chapter);
    }

    #[test]
    fn play_with_start_slice_index_marks_given_slice_speaking() {
        let state = TtsState::new();
        let plan = sample_plan_for_runtime();
        let snap = state
            .play(TtsQueuePlayParams {
                plan,
                start_slice_index: 2,
            })
            .expect("play with startSliceIndex must succeed");
        assert_eq!(snap.state, TtsQueueState::Playing);
        assert_eq!(snap.current_slice_index, Some(2));
        assert_eq!(snap.slice_statuses[2], TtsSliceStatus::Speaking);
        assert_eq!(snap.slice_statuses[0], TtsSliceStatus::Pending);
    }

    #[test]
    fn play_rejects_empty_plan() {
        let state = TtsState::new();
        let chapter = make_chapter();
        let empty_plan = TtsSlicePlan {
            chapter,
            strategy: TtsSlicingStrategy::Paragraph,
            slices: vec![],
            source_char_count: 0,
        };
        let err = state
            .play(TtsQueuePlayParams {
                plan: empty_plan,
                start_slice_index: 0,
            })
            .expect_err("empty plan must error");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[test]
    fn play_rejects_out_of_range_start_slice_index() {
        let state = TtsState::new();
        let plan = sample_plan_for_runtime();
        let err = state
            .play(TtsQueuePlayParams {
                plan,
                start_slice_index: 99,
            })
            .expect_err("startSliceIndex out of range must error");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[test]
    fn play_rejects_when_queue_already_active() {
        let state = TtsState::new();
        let plan = sample_plan_for_runtime();
        let chapter = plan.chapter.clone();
        state
            .play(TtsQueuePlayParams {
                plan,
                start_slice_index: 0,
            })
            .unwrap();
        // Second play with a fresh plan should be rejected while the queue is
        // still active (Playing). User must pause/resume or stop first.
        let plan2 = sample_plan_for_runtime();
        let err = state
            .play(TtsQueuePlayParams {
                plan: plan2,
                start_slice_index: 0,
            })
            .expect_err("play on Playing queue must error");
        assert_eq!(err.code, ErrorCode::InvalidParams);
        // Pause, then try play again — still rejected (Paused is also active).
        state
            .pause(TtsQueuePauseParams {
                chapter: chapter.clone(),
            })
            .unwrap();
        let plan3 = sample_plan_for_runtime();
        let err = state
            .play(TtsQueuePlayParams {
                plan: plan3,
                start_slice_index: 0,
            })
            .expect_err("play on Paused queue must error");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[test]
    fn pause_resume_stop_happy_path_transitions() {
        let state = TtsState::new();
        let plan = sample_plan_for_runtime();
        let chapter = plan.chapter.clone();

        // Idle -> Playing
        let snap = state
            .play(TtsQueuePlayParams {
                plan,
                start_slice_index: 0,
            })
            .unwrap();
        assert_eq!(snap.state, TtsQueueState::Playing);

        // Playing -> Paused
        let snap = state
            .pause(TtsQueuePauseParams {
                chapter: chapter.clone(),
            })
            .unwrap();
        assert_eq!(snap.state, TtsQueueState::Paused);
        // Cursor preserved on pause.
        assert_eq!(snap.current_slice_index, Some(0));

        // Paused -> Playing
        let snap = state
            .resume(TtsQueueResumeParams {
                chapter: chapter.clone(),
            })
            .unwrap();
        assert_eq!(snap.state, TtsQueueState::Playing);

        // Playing -> Stopped (terminal)
        let snap = state
            .stop(TtsQueueStopParams {
                chapter: chapter.clone(),
            })
            .unwrap();
        assert_eq!(snap.state, TtsQueueState::Stopped);
    }

    #[test]
    fn pause_rejects_when_not_playing() {
        let state = TtsState::new();
        let chapter = make_chapter();
        // No queue loaded -> reject.
        let err = state
            .pause(TtsQueuePauseParams {
                chapter: chapter.clone(),
            })
            .expect_err("pause with no queue must error");
        assert_eq!(err.code, ErrorCode::InvalidParams);

        // Load queue, then pause twice — second should reject (already Paused).
        let plan = sample_plan_for_runtime();
        state
            .play(TtsQueuePlayParams {
                plan,
                start_slice_index: 0,
            })
            .unwrap();
        state
            .pause(TtsQueuePauseParams {
                chapter: chapter.clone(),
            })
            .unwrap();
        let err = state
            .pause(TtsQueuePauseParams {
                chapter: chapter.clone(),
            })
            .expect_err("pause on Paused queue must error");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[test]
    fn resume_rejects_when_not_paused() {
        let state = TtsState::new();
        let chapter = make_chapter();
        let plan = sample_plan_for_runtime();
        state
            .play(TtsQueuePlayParams {
                plan,
                start_slice_index: 0,
            })
            .unwrap();
        // Playing -> resume rejected.
        let err = state
            .resume(TtsQueueResumeParams {
                chapter: chapter.clone(),
            })
            .expect_err("resume on Playing queue must error");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[test]
    fn stop_rejects_from_terminal_states() {
        let state = TtsState::new();
        let chapter = make_chapter();
        let plan = sample_plan_for_runtime();
        state
            .play(TtsQueuePlayParams {
                plan,
                start_slice_index: 0,
            })
            .unwrap();
        state
            .stop(TtsQueueStopParams {
                chapter: chapter.clone(),
            })
            .unwrap();
        // Stop on Stopped -> reject.
        let err = state
            .stop(TtsQueueStopParams {
                chapter: chapter.clone(),
            })
            .expect_err("stop on Stopped queue must error");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[test]
    fn next_advances_cursor_and_marks_previous_slice_done() {
        let state = TtsState::new();
        let plan = sample_plan_for_runtime();
        let chapter = plan.chapter.clone();
        state
            .play(TtsQueuePlayParams {
                plan,
                start_slice_index: 0,
            })
            .unwrap();

        let snap = state
            .next(TtsQueueNextParams {
                chapter: chapter.clone(),
            })
            .unwrap();
        assert_eq!(snap.state, TtsQueueState::Playing);
        assert_eq!(snap.current_slice_index, Some(1));
        assert_eq!(snap.slice_statuses[0], TtsSliceStatus::Done);
        assert_eq!(snap.slice_statuses[1], TtsSliceStatus::Speaking);
        assert_eq!(snap.slice_statuses[2], TtsSliceStatus::Pending);
        assert_eq!(snap.completed_slices, 1);
    }

    #[test]
    fn next_at_last_slice_enters_completed() {
        let state = TtsState::new();
        let plan = sample_plan_for_runtime();
        let chapter = plan.chapter.clone();
        state
            .play(TtsQueuePlayParams {
                plan,
                start_slice_index: 2, // last slice
            })
            .unwrap();
        let snap = state
            .next(TtsQueueNextParams {
                chapter: chapter.clone(),
            })
            .unwrap();
        assert_eq!(snap.state, TtsQueueState::Completed);
        assert_eq!(snap.current_slice_index, Some(2));
        assert_eq!(snap.slice_statuses[2], TtsSliceStatus::Done);
        assert_eq!(snap.completed_slices, 1);
    }

    #[test]
    fn next_rejects_when_not_playing_or_paused() {
        let state = TtsState::new();
        let chapter = make_chapter();
        let err = state
            .next(TtsQueueNextParams {
                chapter: chapter.clone(),
            })
            .expect_err("next with no queue must error");
        assert_eq!(err.code, ErrorCode::InvalidParams);

        let plan = sample_plan_for_runtime();
        state
            .play(TtsQueuePlayParams {
                plan,
                start_slice_index: 0,
            })
            .unwrap();
        state
            .stop(TtsQueueStopParams {
                chapter: chapter.clone(),
            })
            .unwrap();
        let err = state
            .next(TtsQueueNextParams {
                chapter: chapter.clone(),
            })
            .expect_err("next on Stopped queue must error");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[test]
    fn prev_retreats_cursor_and_marks_current_pending() {
        let state = TtsState::new();
        let plan = sample_plan_for_runtime();
        let chapter = plan.chapter.clone();
        state
            .play(TtsQueuePlayParams {
                plan,
                start_slice_index: 0,
            })
            .unwrap();
        // Advance to slice 1 first.
        state
            .next(TtsQueueNextParams {
                chapter: chapter.clone(),
            })
            .unwrap();
        assert_eq!(state.snapshot_for(&chapter).current_slice_index, Some(1));
        // prev -> back to slice 0.
        let snap = state
            .prev(TtsQueuePrevParams {
                chapter: chapter.clone(),
            })
            .unwrap();
        assert_eq!(snap.state, TtsQueueState::Playing);
        assert_eq!(snap.current_slice_index, Some(0));
        assert_eq!(snap.slice_statuses[0], TtsSliceStatus::Speaking);
        // Slice 1 returns to Pending.
        assert_eq!(snap.slice_statuses[1], TtsSliceStatus::Pending);
    }

    #[test]
    fn prev_at_first_slice_errors() {
        let state = TtsState::new();
        let plan = sample_plan_for_runtime();
        let chapter = plan.chapter.clone();
        state
            .play(TtsQueuePlayParams {
                plan,
                start_slice_index: 0,
            })
            .unwrap();
        let err = state
            .prev(TtsQueuePrevParams {
                chapter: chapter.clone(),
            })
            .expect_err("prev at first slice must error");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[test]
    fn prev_rejects_when_not_playing_or_paused() {
        let state = TtsState::new();
        let chapter = make_chapter();
        let err = state
            .prev(TtsQueuePrevParams {
                chapter: chapter.clone(),
            })
            .expect_err("prev with no queue must error");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[test]
    fn play_can_restart_from_stopped_or_completed() {
        let state = TtsState::new();
        let plan = sample_plan_for_runtime();
        let chapter = plan.chapter.clone();
        state
            .play(TtsQueuePlayParams {
                plan,
                start_slice_index: 0,
            })
            .unwrap();
        state
            .stop(TtsQueueStopParams {
                chapter: chapter.clone(),
            })
            .unwrap();
        // Restart from Stopped with a fresh plan.
        let plan2 = sample_plan_for_runtime();
        let snap = state
            .play(TtsQueuePlayParams {
                plan: plan2,
                start_slice_index: 0,
            })
            .expect("play on Stopped queue must restart");
        assert_eq!(snap.state, TtsQueueState::Playing);
        assert_eq!(snap.current_slice_index, Some(0));

        // Stop the restarted queue, then drive a fresh queue to Completed via
        // next-at-last-slice (calling play on a Playing queue is rejected, so
        // we must stop first).
        state
            .stop(TtsQueueStopParams {
                chapter: chapter.clone(),
            })
            .unwrap();
        state
            .play(TtsQueuePlayParams {
                plan: sample_plan_for_runtime(),
                start_slice_index: 2, // last slice
            })
            .unwrap();
        state
            .next(TtsQueueNextParams {
                chapter: chapter.clone(),
            })
            .unwrap();
        assert_eq!(state.snapshot_for(&chapter).state, TtsQueueState::Completed);
        // Restart from Completed.
        let snap = state
            .play(TtsQueuePlayParams {
                plan: sample_plan_for_runtime(),
                start_slice_index: 0,
            })
            .expect("play on Completed queue must restart");
        assert_eq!(snap.state, TtsQueueState::Playing);
    }

    // --- Dispatch-level tests for queue control commands ------------------

    #[test]
    fn dispatch_tts_queue_play_returns_result_event_with_snapshot() {
        let (sink, rx, active, state) = make_sink_and_active();
        let plan = sample_plan_for_runtime();
        let cmd = Command::new(
            500,
            contract::methods::TTS_QUEUE_PLAY,
            serde_json::json!({
                "plan": serde_json::to_value(&plan).unwrap(),
                "startSliceIndex": 0,
            }),
        );
        let outcome = dispatch_tts(
            contract::methods::TTS_QUEUE_PLAY,
            &cmd,
            &sink,
            &active,
            &state,
        );
        assert!(matches!(outcome, TtsDispatch::Finished));
        match recv(&rx) {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(request_id, 500);
                let data: TtsQueuePlayData = serde_json::from_value(data).unwrap();
                assert_eq!(data.snapshot.state, TtsQueueState::Playing);
                assert_eq!(data.snapshot.current_slice_index, Some(0));
                assert_eq!(data.snapshot.total_slices, 3);
            }
            other => panic!("unexpected event {other:?}"),
        }
    }

    #[test]
    fn dispatch_tts_queue_control_round_trip_play_pause_resume_stop() {
        // Wire-level exercise of the full lifecycle.
        let (sink, rx, active, state) = make_sink_and_active();
        let plan = sample_plan_for_runtime();
        let chapter = plan.chapter.clone();

        // play
        let cmd = Command::new(
            501,
            contract::methods::TTS_QUEUE_PLAY,
            serde_json::json!({
                "plan": serde_json::to_value(&plan).unwrap(),
                "startSliceIndex": 0,
            }),
        );
        dispatch_tts(
            contract::methods::TTS_QUEUE_PLAY,
            &cmd,
            &sink,
            &active,
            &state,
        );
        match recv(&rx) {
            Event::Result { data, .. } => {
                let d: TtsQueuePlayData = serde_json::from_value(data).unwrap();
                assert_eq!(d.snapshot.state, TtsQueueState::Playing);
            }
            other => panic!("unexpected {other:?}"),
        }

        // pause
        let cmd = Command::new(
            502,
            contract::methods::TTS_QUEUE_PAUSE,
            serde_json::json!({ "chapter": serde_json::to_value(&chapter).unwrap() }),
        );
        dispatch_tts(
            contract::methods::TTS_QUEUE_PAUSE,
            &cmd,
            &sink,
            &active,
            &state,
        );
        match recv(&rx) {
            Event::Result { data, .. } => {
                let d: TtsQueuePauseData = serde_json::from_value(data).unwrap();
                assert_eq!(d.snapshot.state, TtsQueueState::Paused);
            }
            other => panic!("unexpected {other:?}"),
        }

        // resume
        let cmd = Command::new(
            503,
            contract::methods::TTS_QUEUE_RESUME,
            serde_json::json!({ "chapter": serde_json::to_value(&chapter).unwrap() }),
        );
        dispatch_tts(
            contract::methods::TTS_QUEUE_RESUME,
            &cmd,
            &sink,
            &active,
            &state,
        );
        match recv(&rx) {
            Event::Result { data, .. } => {
                let d: TtsQueueResumeData = serde_json::from_value(data).unwrap();
                assert_eq!(d.snapshot.state, TtsQueueState::Playing);
            }
            other => panic!("unexpected {other:?}"),
        }

        // stop
        let cmd = Command::new(
            504,
            contract::methods::TTS_QUEUE_STOP,
            serde_json::json!({ "chapter": serde_json::to_value(&chapter).unwrap() }),
        );
        dispatch_tts(
            contract::methods::TTS_QUEUE_STOP,
            &cmd,
            &sink,
            &active,
            &state,
        );
        match recv(&rx) {
            Event::Result { data, .. } => {
                let d: TtsQueueStopData = serde_json::from_value(data).unwrap();
                assert_eq!(d.snapshot.state, TtsQueueState::Stopped);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn dispatch_tts_queue_next_and_prev_advance_and_retreat_cursor() {
        let (sink, rx, active, state) = make_sink_and_active();
        let plan = sample_plan_for_runtime();
        let chapter = plan.chapter.clone();

        // play
        let cmd = Command::new(
            510,
            contract::methods::TTS_QUEUE_PLAY,
            serde_json::json!({
                "plan": serde_json::to_value(&plan).unwrap(),
                "startSliceIndex": 0,
            }),
        );
        dispatch_tts(
            contract::methods::TTS_QUEUE_PLAY,
            &cmd,
            &sink,
            &active,
            &state,
        );
        let _ = recv(&rx);

        // next -> slice 1
        let cmd = Command::new(
            511,
            contract::methods::TTS_QUEUE_NEXT,
            serde_json::json!({ "chapter": serde_json::to_value(&chapter).unwrap() }),
        );
        dispatch_tts(
            contract::methods::TTS_QUEUE_NEXT,
            &cmd,
            &sink,
            &active,
            &state,
        );
        match recv(&rx) {
            Event::Result { data, .. } => {
                let d: TtsQueueNextData = serde_json::from_value(data).unwrap();
                assert_eq!(d.snapshot.current_slice_index, Some(1));
                assert_eq!(d.snapshot.slice_statuses[0], TtsSliceStatus::Done);
            }
            other => panic!("unexpected {other:?}"),
        }

        // prev -> slice 0
        let cmd = Command::new(
            512,
            contract::methods::TTS_QUEUE_PREV,
            serde_json::json!({ "chapter": serde_json::to_value(&chapter).unwrap() }),
        );
        dispatch_tts(
            contract::methods::TTS_QUEUE_PREV,
            &cmd,
            &sink,
            &active,
            &state,
        );
        match recv(&rx) {
            Event::Result { data, .. } => {
                let d: TtsQueuePrevData = serde_json::from_value(data).unwrap();
                assert_eq!(d.snapshot.current_slice_index, Some(0));
                assert_eq!(d.snapshot.slice_statuses[0], TtsSliceStatus::Speaking);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn dispatch_tts_queue_invalid_transition_returns_error_event() {
        let (sink, rx, active, state) = make_sink_and_active();
        let chapter = make_chapter();
        // pause with no queue loaded -> error
        let cmd = Command::new(
            520,
            contract::methods::TTS_QUEUE_PAUSE,
            serde_json::json!({ "chapter": serde_json::to_value(&chapter).unwrap() }),
        );
        dispatch_tts(
            contract::methods::TTS_QUEUE_PAUSE,
            &cmd,
            &sink,
            &active,
            &state,
        );
        match recv(&rx) {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(request_id, 520);
                assert_eq!(error.code, ErrorCode::InvalidParams);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn dispatch_tts_queue_play_rejects_unknown_field() {
        let (sink, rx, active, state) = make_sink_and_active();
        let plan = sample_plan_for_runtime();
        let cmd = Command::new(
            521,
            contract::methods::TTS_QUEUE_PLAY,
            serde_json::json!({
                "plan": serde_json::to_value(&plan).unwrap(),
                "unexpectedField": true,
            }),
        );
        dispatch_tts(
            contract::methods::TTS_QUEUE_PLAY,
            &cmd,
            &sink,
            &active,
            &state,
        );
        match recv(&rx) {
            Event::Error {
                request_id, error, ..
            } => {
                assert_eq!(request_id, 521);
                assert_eq!(error.code, ErrorCode::InvalidParams);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn tts_queue_status_returns_real_snapshot_after_play() {
        // Verifies Gap F closure requirement #4: tts.queue.status returns the
        // real snapshot (state + currentSliceIndex + sliceStatuses) instead of
        // the V1 Idle-only stub.
        let (sink, rx, active, state) = make_sink_and_active();
        let plan = sample_plan_for_runtime();
        let chapter = plan.chapter.clone();

        // play first
        let cmd = Command::new(
            530,
            contract::methods::TTS_QUEUE_PLAY,
            serde_json::json!({
                "plan": serde_json::to_value(&plan).unwrap(),
                "startSliceIndex": 0,
            }),
        );
        dispatch_tts(
            contract::methods::TTS_QUEUE_PLAY,
            &cmd,
            &sink,
            &active,
            &state,
        );
        let _ = recv(&rx);

        // now query status — must reflect Playing state, not Idle
        let cmd = Command::new(
            531,
            contract::methods::TTS_QUEUE_STATUS,
            serde_json::json!({ "chapter": serde_json::to_value(&chapter).unwrap() }),
        );
        dispatch_tts(
            contract::methods::TTS_QUEUE_STATUS,
            &cmd,
            &sink,
            &active,
            &state,
        );
        match recv(&rx) {
            Event::Result {
                request_id, data, ..
            } => {
                assert_eq!(request_id, 531);
                let data: TtsQueueStatusData = serde_json::from_value(data).unwrap();
                assert_eq!(data.snapshot.state, TtsQueueState::Playing);
                assert_eq!(data.snapshot.current_slice_index, Some(0));
                assert_eq!(data.snapshot.total_slices, 3);
                assert_eq!(data.snapshot.completed_slices, 0);
                assert_eq!(
                    data.snapshot.slice_statuses,
                    vec![
                        TtsSliceStatus::Speaking,
                        TtsSliceStatus::Pending,
                        TtsSliceStatus::Pending,
                    ]
                );
            }
            other => panic!("unexpected event {other:?}"),
        }
    }

    #[test]
    fn tts_queue_status_reflects_state_after_next_advance() {
        let (sink, rx, active, state) = make_sink_and_active();
        let plan = sample_plan_for_runtime();
        let chapter = plan.chapter.clone();

        // play + next
        let cmd = Command::new(
            540,
            contract::methods::TTS_QUEUE_PLAY,
            serde_json::json!({
                "plan": serde_json::to_value(&plan).unwrap(),
                "startSliceIndex": 0,
            }),
        );
        dispatch_tts(
            contract::methods::TTS_QUEUE_PLAY,
            &cmd,
            &sink,
            &active,
            &state,
        );
        let _ = recv(&rx);
        let cmd = Command::new(
            541,
            contract::methods::TTS_QUEUE_NEXT,
            serde_json::json!({ "chapter": serde_json::to_value(&chapter).unwrap() }),
        );
        dispatch_tts(
            contract::methods::TTS_QUEUE_NEXT,
            &cmd,
            &sink,
            &active,
            &state,
        );
        let _ = recv(&rx);

        // status reflects cursor at slice 1
        let cmd = Command::new(
            542,
            contract::methods::TTS_QUEUE_STATUS,
            serde_json::json!({ "chapter": serde_json::to_value(&chapter).unwrap() }),
        );
        dispatch_tts(
            contract::methods::TTS_QUEUE_STATUS,
            &cmd,
            &sink,
            &active,
            &state,
        );
        match recv(&rx) {
            Event::Result { data, .. } => {
                let data: TtsQueueStatusData = serde_json::from_value(data).unwrap();
                assert_eq!(data.snapshot.state, TtsQueueState::Playing);
                assert_eq!(data.snapshot.current_slice_index, Some(1));
                assert_eq!(data.snapshot.completed_slices, 1);
                assert_eq!(data.snapshot.slice_statuses[0], TtsSliceStatus::Done);
                assert_eq!(data.snapshot.slice_statuses[1], TtsSliceStatus::Speaking);
            }
            other => panic!("unexpected event {other:?}"),
        }
    }
}
