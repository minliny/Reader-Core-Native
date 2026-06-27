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

// --- V1 queue control params (Gap F closure) -------------------------------
//
// Maps Legado `BaseReadAloudService` control actions (play / pauseReadAloud /
// resumeReadAloud / stopSelf / nextP / prevP) into Core-owned state-machine
// transitions. Core holds the queue state; the host drives transitions by
// issuing these commands and vocalizes the slice at `currentSliceIndex`.
//
// State machine (see `reader-runtime/src/tts.rs`):
//   Idle --play(plan)--> Playing
//   Playing --pause--> Paused
//   Paused --resume--> Playing
//   Playing/Paused/Completed --stop--> Stopped
//   Playing/Paused --next--> (index++ or Playing, or Completed at last slice)
//   Playing/Paused --prev--> (index-- or error at first slice)
//   Stopped/Completed --play(plan)--> Playing (restart with a fresh plan)

/// Params for `tts.queue.play`. Loads a slice plan and starts playback from
/// `startSliceIndex` (default 0). The host obtains the plan via `tts.slice`.
///
/// Valid from: `Idle`, `Stopped`, `Completed`. Calling `play` on an active
/// (`Playing`/`Paused`) queue is an error — use `pause`/`resume` instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TtsQueuePlayParams {
    pub plan: TtsSlicePlan,
    /// Zero-based slice index to start at. Defaults to 0 (first slice).
    /// Must be `< plan.slices.len()`.
    #[serde(default)]
    pub start_slice_index: u32,
}

/// Params for `tts.queue.pause`. Requires a loaded queue in `Playing` state.
///
/// Mirrors Legado `pauseReadAloud`: the current slice cursor is preserved so
/// `resume` re-vocalizes from the same slice. Core does not signal the host
/// audio engine — the host honors the `Paused` snapshot by stopping output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TtsQueuePauseParams {
    pub chapter: TtsChapterRef,
}

/// Params for `tts.queue.resume`. Requires a loaded queue in `Paused` state.
///
/// Mirrors Legado `resumeReadAloud` → `play()`: the host re-vocalizes the
/// slice at `currentSliceIndex`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TtsQueueResumeParams {
    pub chapter: TtsChapterRef,
}

/// Params for `tts.queue.stop`. Requires a loaded, non-`Idle` queue.
///
/// Mirrors Legado `stopSelf`: terminal state. The queue retains its slice
/// history; restarting requires a fresh `tts.queue.play` with a new plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TtsQueueStopParams {
    pub chapter: TtsChapterRef,
}

/// Params for `tts.queue.next`. Advances the cursor to the next slice.
///
/// Mirrors Legado `nextP`: marks the current slice `Done` and moves the cursor
/// forward. At the last slice, the queue enters `Completed` (chapter-internal
/// boundary). Cross-chapter advance is Gap G (`tts.chapter.plan`), out of V1
/// scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TtsQueueNextParams {
    pub chapter: TtsChapterRef,
}

/// Params for `tts.queue.prev`. Moves the cursor to the previous slice.
///
/// Mirrors Legado `prevP`: moves the cursor backward. At the first slice,
/// returns an error — cross-chapter retreat is Gap G (`tts.chapter.plan`),
/// out of V1 scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TtsQueuePrevParams {
    pub chapter: TtsChapterRef,
}

// --- V1 queue control result data ------------------------------------------

/// Result data for `tts.queue.play`. Returns the snapshot after loading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsQueuePlayData {
    pub snapshot: TtsQueueSnapshot,
}

/// Result data for `tts.queue.pause`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsQueuePauseData {
    pub snapshot: TtsQueueSnapshot,
}

/// Result data for `tts.queue.resume`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsQueueResumeData {
    pub snapshot: TtsQueueSnapshot,
}

/// Result data for `tts.queue.stop`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsQueueStopData {
    pub snapshot: TtsQueueSnapshot,
}

/// Result data for `tts.queue.next`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsQueueNextData {
    pub snapshot: TtsQueueSnapshot,
}

/// Result data for `tts.queue.prev`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsQueuePrevData {
    pub snapshot: TtsQueueSnapshot,
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

    fn sample_plan() -> TtsSlicePlan {
        TtsSlicePlan {
            chapter: TtsChapterRef {
                source_id: "src-1".into(),
                book_id: "book-1".into(),
                chapter_index: 2,
                chapter_title: "第二章".into(),
                chapter_url: String::new(),
            },
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
            ],
            source_char_count: 8,
        }
    }

    #[test]
    fn tts_queue_play_params_round_trips_and_defaults_start_index_to_zero() {
        let plan = sample_plan();
        let params = TtsQueuePlayParams {
            plan: plan.clone(),
            start_slice_index: 0,
        };
        let json = serde_json::to_value(&params).expect("play params must serialize");
        assert_eq!(json["startSliceIndex"], 0);
        let back: TtsQueuePlayParams =
            serde_json::from_value(json).expect("play params must round-trip");
        assert_eq!(back, params);
        assert_eq!(back.plan, plan);

        // Omitting startSliceIndex defaults to 0.
        let json = serde_json::json!({
            "plan": serde_json::to_value(&plan).unwrap()
        });
        let back: TtsQueuePlayParams =
            serde_json::from_value(json).expect("play params must default startSliceIndex");
        assert_eq!(back.start_slice_index, 0);
    }

    #[test]
    fn tts_queue_play_params_rejects_unknown_fields() {
        let plan = sample_plan();
        let json = serde_json::json!({
            "plan": serde_json::to_value(&plan).unwrap(),
            "unexpectedField": true
        });
        let err = serde_json::from_value::<TtsQueuePlayParams>(json);
        assert!(
            err.is_err(),
            "deny_unknown_fields must reject unknown field"
        );
    }

    #[test]
    fn tts_queue_control_params_round_trip_with_chapter() {
        let chapter = TtsChapterRef {
            source_id: "src-1".into(),
            book_id: "book-1".into(),
            chapter_index: 2,
            chapter_title: String::new(),
            chapter_url: String::new(),
        };
        for (label, json_value) in [
            (
                "pause",
                serde_json::json!({ "chapter": serde_json::to_value(&chapter).unwrap() }),
            ),
            (
                "resume",
                serde_json::json!({ "chapter": serde_json::to_value(&chapter).unwrap() }),
            ),
            (
                "stop",
                serde_json::json!({ "chapter": serde_json::to_value(&chapter).unwrap() }),
            ),
            (
                "next",
                serde_json::json!({ "chapter": serde_json::to_value(&chapter).unwrap() }),
            ),
            (
                "prev",
                serde_json::json!({ "chapter": serde_json::to_value(&chapter).unwrap() }),
            ),
        ] {
            let _ = label;
            // Each control params type parses from the chapter-only payload.
            serde_json::from_value::<TtsQueuePauseParams>(json_value.clone())
                .expect("pause params must parse");
            serde_json::from_value::<TtsQueueResumeParams>(json_value.clone())
                .expect("resume params must parse");
            serde_json::from_value::<TtsQueueStopParams>(json_value.clone())
                .expect("stop params must parse");
            serde_json::from_value::<TtsQueueNextParams>(json_value.clone())
                .expect("next params must parse");
            serde_json::from_value::<TtsQueuePrevParams>(json_value.clone())
                .expect("prev params must parse");
        }
    }

    #[test]
    fn tts_queue_control_data_types_round_trip_with_snapshot() {
        let snapshot = TtsQueueSnapshot {
            state: TtsQueueState::Playing,
            current_slice_index: Some(1),
            total_slices: 2,
            completed_slices: 1,
            chapter: TtsChapterRef {
                source_id: "src-1".into(),
                book_id: "book-1".into(),
                chapter_index: 2,
                chapter_title: String::new(),
                chapter_url: String::new(),
            },
            slice_statuses: vec![TtsSliceStatus::Done, TtsSliceStatus::Speaking],
        };
        for (label, json_value) in [
            (
                "play",
                serde_json::to_value(TtsQueuePlayData {
                    snapshot: snapshot.clone(),
                })
                .unwrap(),
            ),
            (
                "pause",
                serde_json::to_value(TtsQueuePauseData {
                    snapshot: snapshot.clone(),
                })
                .unwrap(),
            ),
            (
                "resume",
                serde_json::to_value(TtsQueueResumeData {
                    snapshot: snapshot.clone(),
                })
                .unwrap(),
            ),
            (
                "stop",
                serde_json::to_value(TtsQueueStopData {
                    snapshot: snapshot.clone(),
                })
                .unwrap(),
            ),
            (
                "next",
                serde_json::to_value(TtsQueueNextData {
                    snapshot: snapshot.clone(),
                })
                .unwrap(),
            ),
            (
                "prev",
                serde_json::to_value(TtsQueuePrevData {
                    snapshot: snapshot.clone(),
                })
                .unwrap(),
            ),
        ] {
            let _ = label;
            assert_eq!(json_value["snapshot"]["state"], "playing");
            assert_eq!(json_value["snapshot"]["currentSliceIndex"], 1);
        }
    }
}
