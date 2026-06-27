//! TTS contract model.
//!
//! Core owns text slicing, the playback queue state machine, and chapter
//! boundary transitions. System vocalization (actual audio output) remains a
//! host responsibility — see `docs/host-app-contracts/05-tts.md`.
//!
//! This module deliberately mirrors the V1 remote-reading DTO style
//! (`crates/reader-contract/src/remote.rs`): camelCase JSON, `deny_unknown_fields`
//! via schema, non-blank validation for identifiers.

use serde::{de, Deserialize, Deserializer, Serialize};

fn deserialize_non_blank_tts_field<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.trim().is_empty() {
        Err(de::Error::custom("tts field must be non-empty"))
    } else {
        Ok(value)
    }
}

fn deserialize_non_empty_tts_text<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.trim().is_empty() {
        Err(de::Error::custom("tts slice text must be non-empty"))
    } else {
        Ok(value)
    }
}

/// Reference to a chapter being spoken. Mirrors the remote-reading chapter
/// identity so TTS can advance across the TOC without re-fetching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsChapterRef {
    #[serde(deserialize_with = "deserialize_non_blank_tts_field")]
    pub source_id: String,
    #[serde(deserialize_with = "deserialize_non_blank_tts_field")]
    pub book_id: String,
    pub chapter_index: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub chapter_title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub chapter_url: String,
}

/// Strategy for slicing chapter text into speakable utterances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TtsSlicingStrategy {
    /// Split on paragraph boundaries (blank lines). Preserves sentence structure.
    #[default]
    Paragraph,
    /// Split on sentence boundaries (。！？.!?).
    Sentence,
    /// Split on paragraphs first, then long paragraphs by sentence.
    ParagraphThenSentence,
    /// Legacy legado-style: split on "\n" only.
    LineBreak,
}

/// One speakable slice of chapter text produced by Core's slicer.
/// Core owns slicing; host owns vocalization of each slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsSlice {
    /// Zero-based index within the slice plan.
    pub index: u32,
    /// Speakable text. Non-empty after normalization.
    #[serde(deserialize_with = "deserialize_non_empty_tts_text")]
    pub text: String,
    /// Half-open char range [start, end) in the original chapter content.
    pub char_start: u32,
    pub char_end: u32,
    /// Paragraph index in source chapter (for UI highlight sync).
    pub paragraph_index: u32,
}

/// A slice plan: the result of slicing one chapter's content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsSlicePlan {
    pub chapter: TtsChapterRef,
    #[serde(default)]
    pub strategy: TtsSlicingStrategy,
    pub slices: Vec<TtsSlice>,
    /// Total char count of the source chapter content.
    pub source_char_count: u32,
}

/// High-level state of the TTS playback queue.
///
/// Core tracks this; the host drives transitions by reporting playback
/// events (slice started / finished / failed) back to Core. The actual
/// audio output is host-owned — this enum only describes queue progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TtsQueueState {
    /// No slice plan loaded; queue is empty.
    Idle,
    /// A plan is loaded and playback is active.
    Playing,
    /// A plan is loaded but playback is paused.
    Paused,
    /// All slices in the plan have been spoken.
    Completed,
    /// Playback stopped by user action (distinct from `Completed`).
    Stopped,
}

/// Per-slice lifecycle within the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TtsSliceStatus {
    /// Not yet reached by the playback cursor.
    Pending,
    /// Currently being vocalized by the host.
    Speaking,
    /// Vocalization finished successfully.
    Done,
    /// Skipped (user jumped past it).
    Skipped,
    /// Host vocalization failed for this slice.
    Failed,
}

/// Immutable snapshot of the queue at a point in time.
///
/// Core emits this; the host reads it to render UI and drive playback.
/// `slice_statuses` is parallel to the originating `TtsSlicePlan::slices`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsQueueSnapshot {
    pub state: TtsQueueState,
    /// Index of the current slice (the one `Speaking`, or next `Pending` if
    /// `Paused`/`Idle`). `None` if no plan is loaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_slice_index: Option<u32>,
    /// Total slices in the loaded plan. 0 if `Idle`.
    pub total_slices: u32,
    /// Number of slices in a terminal state (`Done` + `Failed` + `Skipped`).
    pub completed_slices: u32,
    pub chapter: TtsChapterRef,
    /// Status of each slice, parallel to the plan's `slices` vector.
    /// Empty when the host has not yet reported per-slice progress.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slice_statuses: Vec<TtsSliceStatus>,
}

/// Behavior when the current chapter's queue drains.
///
/// Core decides this; the host honors it so that auto-advance and
/// stop-at-boundary stay Core-controlled semantics rather than per-platform
/// policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TtsQueueDrainBehavior {
    /// Stop at the chapter boundary; do not auto-advance.
    #[default]
    StopOnBoundary,
    /// Auto-advance to the next chapter and continue playback.
    AdvanceToNext,
}

/// Chapter boundary transition plan.
///
/// Core emits this so the host knows what to preload and what to do when the
/// current chapter's queue drains. `next` is `None` when `current` is the
/// last chapter in the TOC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsChapterTransition {
    pub current: TtsChapterRef,
    /// Next chapter to preload. `None` if `current` is the last chapter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<TtsChapterRef>,
    #[serde(default)]
    pub drain_behavior: TtsQueueDrainBehavior,
}

// --- V1 method parameter types ---------------------------------------------

/// Params for `tts.slice`. Core slices chapter content into speakable
/// utterances using the requested strategy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TtsSliceParams {
    pub chapter: TtsChapterRef,
    #[serde(deserialize_with = "deserialize_non_empty_tts_text")]
    pub content: String,
    #[serde(default)]
    pub strategy: TtsSlicingStrategy,
}

/// Params for `tts.queue.status`. Core returns the current queue snapshot
/// for the given chapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TtsQueueStatusParams {
    pub chapter: TtsChapterRef,
}

/// Params for `tts.chapter.plan`. Core returns the chapter boundary
/// transition plan for the given chapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TtsChapterPlanParams {
    pub chapter: TtsChapterRef,
    #[serde(default)]
    pub drain_behavior: TtsQueueDrainBehavior,
}

// --- V1 result data types (carried by ResultEvent) -------------------------

/// Result data for `tts.slice`. Mirrors the `TtsSliceData` $def in
/// `protocol/reader-event.schema.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsSliceData {
    pub plan: TtsSlicePlan,
}

/// Result data for `tts.queue.status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsQueueStatusData {
    pub snapshot: TtsQueueSnapshot,
}

/// Result data for `tts.chapter.plan`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsChapterPlanData {
    pub transition: TtsChapterTransition,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tts_slice_plan_round_trips_through_json() {
        let plan = TtsSlicePlan {
            chapter: TtsChapterRef {
                source_id: "src-001".into(),
                book_id: "book-001".into(),
                chapter_index: 3,
                chapter_title: "Chapter 4".into(),
                chapter_url: String::new(),
            },
            strategy: TtsSlicingStrategy::ParagraphThenSentence,
            slices: vec![
                TtsSlice {
                    index: 0,
                    text: "First paragraph.".into(),
                    char_start: 0,
                    char_end: 16,
                    paragraph_index: 0,
                },
                TtsSlice {
                    index: 1,
                    text: "Second paragraph.".into(),
                    char_start: 17,
                    char_end: 35,
                    paragraph_index: 1,
                },
            ],
            source_char_count: 35,
        };

        let json = serde_json::to_value(&plan).expect("plan must serialize");
        let back: TtsSlicePlan = serde_json::from_value(json).expect("plan must round-trip");
        assert_eq!(back, plan);
        assert_eq!(back.slices.len(), 2);
        assert_eq!(back.slices[1].index, 1);
        assert_eq!(back.chapter.chapter_index, 3);
    }

    #[test]
    fn tts_queue_snapshot_round_trips_and_state_enums_serialize_as_lowercase() {
        let snapshot = TtsQueueSnapshot {
            state: TtsQueueState::Playing,
            current_slice_index: Some(2),
            total_slices: 5,
            completed_slices: 2,
            chapter: TtsChapterRef {
                source_id: "src-001".into(),
                book_id: "book-001".into(),
                chapter_index: 3,
                chapter_title: "Chapter 4".into(),
                chapter_url: String::new(),
            },
            slice_statuses: vec![
                TtsSliceStatus::Done,
                TtsSliceStatus::Done,
                TtsSliceStatus::Speaking,
                TtsSliceStatus::Pending,
                TtsSliceStatus::Pending,
            ],
        };

        let json = serde_json::to_value(&snapshot).expect("snapshot must serialize");
        // Enum variants serialize as lowercase strings.
        assert_eq!(json["state"], "playing");
        assert_eq!(json["sliceStatuses"][2], "speaking");
        let back: TtsQueueSnapshot =
            serde_json::from_value(json).expect("snapshot must round-trip");
        assert_eq!(back, snapshot);
        assert_eq!(back.state, TtsQueueState::Playing);
    }

    #[test]
    fn tts_queue_snapshot_idle_omits_optional_fields() {
        let json = serde_json::json!({
            "state": "idle",
            "totalSlices": 0,
            "completedSlices": 0,
            "chapter": {
                "sourceId": "s",
                "bookId": "b",
                "chapterIndex": 0
            }
        });
        let snap: TtsQueueSnapshot = serde_json::from_value(json).expect("idle snapshot parses");
        assert_eq!(snap.state, TtsQueueState::Idle);
        assert_eq!(snap.current_slice_index, None);
        assert!(snap.slice_statuses.is_empty());
    }

    #[test]
    fn tts_chapter_transition_round_trips_with_and_without_next() {
        let current = TtsChapterRef {
            source_id: "src-001".into(),
            book_id: "book-001".into(),
            chapter_index: 3,
            chapter_title: "Chapter 4".into(),
            chapter_url: String::new(),
        };
        let next = TtsChapterRef {
            source_id: "src-001".into(),
            book_id: "book-001".into(),
            chapter_index: 4,
            chapter_title: "Chapter 5".into(),
            chapter_url: String::new(),
        };

        let with_next = TtsChapterTransition {
            current: current.clone(),
            next: Some(next.clone()),
            drain_behavior: TtsQueueDrainBehavior::AdvanceToNext,
        };
        let json = serde_json::to_value(&with_next).expect("transition must serialize");
        assert_eq!(json["drainBehavior"], "advance-to-next");
        assert_eq!(json["next"]["chapterIndex"], 4);
        let back: TtsChapterTransition =
            serde_json::from_value(json).expect("transition must round-trip");
        assert_eq!(back, with_next);

        let no_next = TtsChapterTransition {
            current: current.clone(),
            next: None,
            drain_behavior: TtsQueueDrainBehavior::StopOnBoundary,
        };
        let json = serde_json::to_value(&no_next).expect("terminal transition must serialize");
        assert!(json.get("next").is_none());
        assert_eq!(json["drainBehavior"], "stop-on-boundary");
        let back: TtsChapterTransition =
            serde_json::from_value(json).expect("terminal transition must round-trip");
        assert_eq!(back, no_next);
        assert!(back.next.is_none());
    }
}
