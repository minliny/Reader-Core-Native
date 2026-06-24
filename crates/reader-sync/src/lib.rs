//! Reader-Core sync — WebDAV protocol, conflict resolution, backup/restore.
//!
//! This crate owns sync data semantics, not transport. V1 models sync as
//! deterministic snapshots of opaque records so bookshelf, reading progress,
//! local books, chapter cache, RSS subscriptions, and future settings can share
//! the same merge and backup/restore rules.

use std::collections::{BTreeMap, BTreeSet};

/// Data bucket represented by a sync record.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SyncCollection {
    Bookshelf,
    ReadingProgress,
    LocalBook,
    ChapterCache,
    RssSubscription,
    RssEntry,
    Custom(String),
}

impl SyncCollection {
    pub fn as_str(&self) -> &str {
        match self {
            SyncCollection::Bookshelf => "bookshelf",
            SyncCollection::ReadingProgress => "readingProgress",
            SyncCollection::LocalBook => "localBook",
            SyncCollection::ChapterCache => "chapterCache",
            SyncCollection::RssSubscription => "rssSubscription",
            SyncCollection::RssEntry => "rssEntry",
            SyncCollection::Custom(value) => value.as_str(),
        }
    }

    pub fn custom(value: impl Into<String>) -> Result<Self, SyncError> {
        let value = normalize_required(value.into(), "collection")?;
        Ok(SyncCollection::Custom(value))
    }
}

/// Stable key for a record inside a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SyncRecordKey {
    pub collection: SyncCollection,
    pub record_id: String,
}

impl SyncRecordKey {
    pub fn new(
        collection: SyncCollection,
        record_id: impl Into<String>,
    ) -> Result<Self, SyncError> {
        Ok(Self {
            collection,
            record_id: normalize_required(record_id.into(), "record_id")?,
        })
    }
}

/// One synchronized logical row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRecord {
    pub collection: SyncCollection,
    pub record_id: String,
    /// Unix timestamp in seconds. Later records win during merge.
    pub updated_at: i64,
    /// Device or actor that produced the record.
    pub device_id: String,
    /// Monotonic per-device revision. Used as a deterministic tie-breaker.
    pub revision: u64,
    /// Opaque JSON/text payload owned by the source collection.
    pub payload: String,
    /// Tombstone marker. Deleted records keep their key and revision metadata.
    pub deleted: bool,
}

impl SyncRecord {
    pub fn upsert(
        collection: SyncCollection,
        record_id: impl Into<String>,
        payload: impl Into<String>,
        updated_at: i64,
        device_id: impl Into<String>,
        revision: u64,
    ) -> Result<Self, SyncError> {
        let payload = payload.into();
        if payload.trim().is_empty() {
            return Err(SyncError::InvalidRecord {
                field: "payload".into(),
            });
        }
        Self::build(
            collection, record_id, payload, false, updated_at, device_id, revision,
        )
    }

    pub fn tombstone(
        collection: SyncCollection,
        record_id: impl Into<String>,
        updated_at: i64,
        device_id: impl Into<String>,
        revision: u64,
    ) -> Result<Self, SyncError> {
        Self::build(
            collection,
            record_id,
            String::new(),
            true,
            updated_at,
            device_id,
            revision,
        )
    }

    pub fn key(&self) -> SyncRecordKey {
        SyncRecordKey {
            collection: self.collection.clone(),
            record_id: self.record_id.clone(),
        }
    }

    pub fn validate(&self) -> Result<(), SyncError> {
        SyncRecordKey::new(self.collection.clone(), self.record_id.clone())?;
        normalize_required(self.device_id.clone(), "device_id")?;
        if !self.deleted && self.payload.trim().is_empty() {
            return Err(SyncError::InvalidRecord {
                field: "payload".into(),
            });
        }
        Ok(())
    }

    fn build(
        collection: SyncCollection,
        record_id: impl Into<String>,
        payload: String,
        deleted: bool,
        updated_at: i64,
        device_id: impl Into<String>,
        revision: u64,
    ) -> Result<Self, SyncError> {
        let key = SyncRecordKey::new(collection, record_id)?;
        let device_id = normalize_required(device_id.into(), "device_id")?;
        let record = Self {
            collection: key.collection,
            record_id: key.record_id,
            updated_at,
            device_id,
            revision,
            payload,
            deleted,
        };
        record.validate()?;
        Ok(record)
    }
}

/// Sync export/import unit. A snapshot may contain duplicate keys from an append
/// log; normalization reduces those to the current record per key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncSnapshot {
    pub snapshot_id: String,
    pub device_id: String,
    pub created_at: i64,
    pub records: Vec<SyncRecord>,
}

impl SyncSnapshot {
    pub fn new(
        snapshot_id: impl Into<String>,
        device_id: impl Into<String>,
        created_at: i64,
        records: Vec<SyncRecord>,
    ) -> Result<Self, SyncError> {
        let snapshot = Self {
            snapshot_id: normalize_required(snapshot_id.into(), "snapshot_id")?,
            device_id: normalize_required(device_id.into(), "device_id")?,
            created_at,
            records,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn validate(&self) -> Result<(), SyncError> {
        normalize_required(self.snapshot_id.clone(), "snapshot_id")?;
        normalize_required(self.device_id.clone(), "device_id")?;
        for record in &self.records {
            record.validate()?;
        }
        Ok(())
    }

    /// Return one winning record per key, sorted by collection then record id.
    pub fn normalized_records(&self) -> Result<Vec<SyncRecord>, SyncError> {
        self.validate()?;
        let mut by_key = BTreeMap::<SyncRecordKey, SyncRecord>::new();
        for record in &self.records {
            let key = record.key();
            match by_key.get(&key) {
                Some(existing) if choose_record(existing, record) == RecordChoice::Existing => {}
                _ => {
                    by_key.insert(key, record.clone());
                }
            }
        }
        Ok(by_key.into_values().collect())
    }

    pub fn live_records(&self) -> Result<Vec<SyncRecord>, SyncError> {
        Ok(self
            .normalized_records()?
            .into_iter()
            .filter(|record| !record.deleted)
            .collect())
    }
}

/// Conflict details for records that changed differently across snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncConflict {
    pub key: SyncRecordKey,
    pub local: SyncRecord,
    pub remote: SyncRecord,
    pub winner: SyncRecord,
    pub reason: ConflictReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictReason {
    ConcurrentPayloadChange,
    DeleteVsUpdate,
    EqualTimestampTieBreak,
}

/// Merge result from two snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncMergeResult {
    pub snapshot: SyncSnapshot,
    pub conflicts: Vec<SyncConflict>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    InvalidRecord { field: String },
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncError::InvalidRecord { field } => write!(f, "invalid sync record field: {field}"),
        }
    }
}

impl std::error::Error for SyncError {}

/// Merge two snapshots with deterministic last-write-wins semantics.
///
/// Later `updated_at` wins. For equal timestamps, tombstones win over live
/// records; then higher `revision`; then lexicographically higher `device_id`;
/// finally lexicographically higher payload. The tie-breaks make repeated
/// backup/restore runs stable even without a transport-level vector clock.
pub fn merge_snapshots(
    local: &SyncSnapshot,
    remote: &SyncSnapshot,
    merged_snapshot_id: impl Into<String>,
    merged_device_id: impl Into<String>,
    merged_created_at: i64,
) -> Result<SyncMergeResult, SyncError> {
    let local_records = map_records(local.normalized_records()?);
    let remote_records = map_records(remote.normalized_records()?);
    let mut keys = BTreeSet::new();
    keys.extend(local_records.keys().cloned());
    keys.extend(remote_records.keys().cloned());

    let mut merged = Vec::new();
    let mut conflicts = Vec::new();
    for key in keys {
        match (local_records.get(&key), remote_records.get(&key)) {
            (Some(local), Some(remote)) => {
                let winner = match choose_record(local, remote) {
                    RecordChoice::Existing => local.clone(),
                    RecordChoice::Candidate => remote.clone(),
                };
                if let Some(reason) = conflict_reason(local, remote) {
                    conflicts.push(SyncConflict {
                        key: key.clone(),
                        local: local.clone(),
                        remote: remote.clone(),
                        winner: winner.clone(),
                        reason,
                    });
                }
                merged.push(winner);
            }
            (Some(local), None) => merged.push(local.clone()),
            (None, Some(remote)) => merged.push(remote.clone()),
            (None, None) => {}
        }
    }

    let snapshot = SyncSnapshot::new(
        merged_snapshot_id,
        merged_device_id,
        merged_created_at,
        merged,
    )?;
    Ok(SyncMergeResult {
        snapshot,
        conflicts,
    })
}

fn map_records(records: Vec<SyncRecord>) -> BTreeMap<SyncRecordKey, SyncRecord> {
    records
        .into_iter()
        .map(|record| (record.key(), record))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordChoice {
    Existing,
    Candidate,
}

fn choose_record(existing: &SyncRecord, candidate: &SyncRecord) -> RecordChoice {
    if candidate.updated_at != existing.updated_at {
        return if candidate.updated_at > existing.updated_at {
            RecordChoice::Candidate
        } else {
            RecordChoice::Existing
        };
    }

    if candidate.deleted != existing.deleted {
        return if candidate.deleted {
            RecordChoice::Candidate
        } else {
            RecordChoice::Existing
        };
    }

    if candidate.revision != existing.revision {
        return if candidate.revision > existing.revision {
            RecordChoice::Candidate
        } else {
            RecordChoice::Existing
        };
    }

    if candidate.device_id != existing.device_id {
        return if candidate.device_id > existing.device_id {
            RecordChoice::Candidate
        } else {
            RecordChoice::Existing
        };
    }

    if candidate.payload > existing.payload {
        RecordChoice::Candidate
    } else {
        RecordChoice::Existing
    }
}

fn conflict_reason(local: &SyncRecord, remote: &SyncRecord) -> Option<ConflictReason> {
    if records_equivalent(local, remote) {
        return None;
    }
    if local.deleted != remote.deleted {
        return Some(ConflictReason::DeleteVsUpdate);
    }
    if local.updated_at == remote.updated_at {
        return Some(ConflictReason::EqualTimestampTieBreak);
    }
    if local.payload != remote.payload {
        return Some(ConflictReason::ConcurrentPayloadChange);
    }
    None
}

fn records_equivalent(left: &SyncRecord, right: &SyncRecord) -> bool {
    left.deleted == right.deleted
        && left.updated_at == right.updated_at
        && left.revision == right.revision
        && left.payload == right.payload
}

fn normalize_required(value: String, field: &str) -> Result<String, SyncError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(SyncError::InvalidRecord {
            field: field.into(),
        });
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(collection: SyncCollection, id: &str, payload: &str, ts: i64) -> SyncRecord {
        SyncRecord::upsert(collection, id, payload, ts, "device-a", 1).unwrap()
    }

    fn snap(id: &str, records: Vec<SyncRecord>) -> SyncSnapshot {
        SyncSnapshot::new(id, "device-a", 1000, records).unwrap()
    }

    #[test]
    fn record_rejects_empty_required_fields_and_payload() {
        assert_eq!(
            SyncRecord::upsert(SyncCollection::Bookshelf, "", "{}", 1, "device", 1).unwrap_err(),
            SyncError::InvalidRecord {
                field: "record_id".into()
            }
        );
        assert_eq!(
            SyncRecord::upsert(SyncCollection::Bookshelf, "book", "  ", 1, "device", 1)
                .unwrap_err(),
            SyncError::InvalidRecord {
                field: "payload".into()
            }
        );
        assert_eq!(
            SyncRecord::upsert(SyncCollection::Bookshelf, "book", "{}", 1, " ", 1).unwrap_err(),
            SyncError::InvalidRecord {
                field: "device_id".into()
            }
        );
        assert!(SyncCollection::custom("  ").is_err());
    }

    #[test]
    fn tombstone_allows_empty_payload() {
        let record =
            SyncRecord::tombstone(SyncCollection::ChapterCache, "src/book/1", 10, "device", 2)
                .unwrap();

        assert!(record.deleted);
        assert!(record.payload.is_empty());
        assert_eq!(record.key().record_id, "src/book/1");
    }

    #[test]
    fn snapshot_rejects_empty_metadata() {
        assert_eq!(
            SyncSnapshot::new("", "device", 1, Vec::new()).unwrap_err(),
            SyncError::InvalidRecord {
                field: "snapshot_id".into()
            }
        );
        assert_eq!(
            SyncSnapshot::new("snapshot", " ", 1, Vec::new()).unwrap_err(),
            SyncError::InvalidRecord {
                field: "device_id".into()
            }
        );
    }

    #[test]
    fn normalized_records_keep_latest_per_key_and_sort_by_key() {
        let old = rec(SyncCollection::Bookshelf, "b1", r#"{"title":"old"}"#, 1);
        let latest = rec(SyncCollection::Bookshelf, "b1", r#"{"title":"new"}"#, 2);
        let progress = rec(SyncCollection::ReadingProgress, "b1", r#"{"chapter":2}"#, 1);
        let snapshot = snap("s1", vec![latest.clone(), progress.clone(), old]);

        let records = snapshot.normalized_records().unwrap();

        assert_eq!(records, vec![latest, progress]);
    }

    #[test]
    fn live_records_filter_tombstones_after_normalization() {
        let live = rec(SyncCollection::Bookshelf, "b1", r#"{"title":"live"}"#, 1);
        let deleted =
            SyncRecord::tombstone(SyncCollection::Bookshelf, "b1", 2, "device-a", 2).unwrap();
        let snapshot = snap("s1", vec![live, deleted]);

        assert!(snapshot.live_records().unwrap().is_empty());
    }

    #[test]
    fn merge_preserves_records_present_on_one_side() {
        let local = snap(
            "local",
            vec![rec(SyncCollection::Bookshelf, "b1", r#"{"title":"A"}"#, 1)],
        );
        let remote = snap(
            "remote",
            vec![rec(
                SyncCollection::RssSubscription,
                "feed",
                r#"{"url":"https://example.test"}"#,
                1,
            )],
        );

        let result = merge_snapshots(&local, &remote, "merged", "merge-device", 3).unwrap();

        assert!(result.conflicts.is_empty());
        assert_eq!(result.snapshot.records.len(), 2);
        assert_eq!(result.snapshot.snapshot_id, "merged");
        assert_eq!(result.snapshot.device_id, "merge-device");
    }

    #[test]
    fn merge_later_update_wins_and_reports_payload_conflict() {
        let local = snap(
            "local",
            vec![rec(
                SyncCollection::ReadingProgress,
                "book",
                r#"{"chapter":1}"#,
                10,
            )],
        );
        let mut remote_record = rec(
            SyncCollection::ReadingProgress,
            "book",
            r#"{"chapter":2}"#,
            11,
        );
        remote_record.device_id = "device-b".into();
        let remote = snap("remote", vec![remote_record.clone()]);

        let result = merge_snapshots(&local, &remote, "merged", "merge-device", 12).unwrap();

        assert_eq!(result.snapshot.records, vec![remote_record.clone()]);
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(
            result.conflicts[0].reason,
            ConflictReason::ConcurrentPayloadChange
        );
        assert_eq!(result.conflicts[0].winner, remote_record);
    }

    #[test]
    fn merge_delete_wins_on_equal_timestamp() {
        let live = rec(
            SyncCollection::ChapterCache,
            "src/book/1",
            r#"{"body":"x"}"#,
            20,
        );
        let tombstone = SyncRecord::tombstone(
            SyncCollection::ChapterCache,
            "src/book/1",
            20,
            "device-b",
            1,
        )
        .unwrap();
        let local = snap("local", vec![live]);
        let remote = snap("remote", vec![tombstone.clone()]);

        let result = merge_snapshots(&local, &remote, "merged", "merge-device", 21).unwrap();

        assert_eq!(result.snapshot.records, vec![tombstone.clone()]);
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].reason, ConflictReason::DeleteVsUpdate);
        assert!(result.snapshot.live_records().unwrap().is_empty());
    }

    #[test]
    fn merge_equal_timestamp_uses_revision_then_device_tie_breaks() {
        let local_record = SyncRecord::upsert(
            SyncCollection::LocalBook,
            "local-1",
            r#"{"rev":1}"#,
            30,
            "a",
            1,
        )
        .unwrap();
        let remote_record = SyncRecord::upsert(
            SyncCollection::LocalBook,
            "local-1",
            r#"{"rev":2}"#,
            30,
            "b",
            1,
        )
        .unwrap();
        let local = snap("local", vec![local_record]);
        let remote = snap("remote", vec![remote_record.clone()]);

        let result = merge_snapshots(&local, &remote, "merged", "merge-device", 31).unwrap();

        assert_eq!(result.snapshot.records, vec![remote_record]);
        assert_eq!(
            result.conflicts[0].reason,
            ConflictReason::EqualTimestampTieBreak
        );

        let newer_revision = SyncRecord::upsert(
            SyncCollection::LocalBook,
            "local-1",
            r#"{"rev":3}"#,
            30,
            "a",
            2,
        )
        .unwrap();
        let result = merge_snapshots(
            &snap("local", vec![newer_revision.clone()]),
            &remote,
            "m2",
            "d",
            32,
        )
        .unwrap();
        assert_eq!(result.snapshot.records, vec![newer_revision]);
    }

    #[test]
    fn merge_identical_records_are_not_conflicts() {
        let record = rec(SyncCollection::RssEntry, "entry", r#"{"title":"same"}"#, 40);
        let local = snap("local", vec![record.clone()]);
        let remote = snap("remote", vec![record.clone()]);

        let result = merge_snapshots(&local, &remote, "merged", "merge-device", 41).unwrap();

        assert!(result.conflicts.is_empty());
        assert_eq!(result.snapshot.records, vec![record]);
    }
}
