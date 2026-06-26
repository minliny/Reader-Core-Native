//! WebDAV conflict resolution with last-write-wins + audit journal.
//!
//! Extends the existing `plan_webdav_progress_push` LWW resolution with an
//! auditable journal recording which records were kept and which were
//! discarded for each conflict key.
//!
//! Swift mapping: `URLSessionWebDAVAdapter.resolveConflicts(local:remote:policy:)`
//! (lines 179-195) implements LWW as: sort by `updatedAt` desc + tiebreaker,
//! dedup by key `bookId:chapterIndex:deviceId`. We mirror this exactly and
//! additionally emit a `ConflictResolution` entry per key that had multiple
//! candidates, so callers can audit or replay conflict decisions.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    compare_progress_runtime_order, compare_progress_webdav_last_write_wins_order,
    sort_progress_cloud_records, ConflictPolicy, ProgressCloudSyncRecord,
    ProgressCloudSyncRecordKey, SyncError,
};

/// One conflict resolution decision for a single key.
///
/// Emitted only when a key had more than one candidate record under
/// `LastWriteWins`. The `kept` field is the record that survived; `discarded`
/// holds the records that lost the LWW comparison, sorted deterministically.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConflictResolution {
    pub key: ProgressCloudSyncRecordKey,
    pub policy: ConflictPolicy,
    pub resolved_at: i64,
    pub kept: ProgressCloudSyncRecord,
    pub discarded: Vec<ProgressCloudSyncRecord>,
}

/// Audit journal of conflict resolutions across one sync operation.
///
/// Empty when no conflicts were found (every key had exactly one candidate)
/// or when the policy is `DevicePriority`/`Manual`/`KeepBoth` (no resolution).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConflictJournal {
    pub created_at: i64,
    pub entries: Vec<ConflictResolution>,
}

impl ConflictJournal {
    pub fn new(created_at: i64) -> Self {
        Self {
            created_at,
            entries: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Resolve conflicts and produce an audit journal.
///
/// Mirrors Swift `resolveConflicts` (lines 179-195):
/// - `LastWriteWins`: sort by `updatedAt` desc + tiebreaker, dedup by key.
///   For each key with multiple candidates, emit a `ConflictResolution` entry
///   recording the kept record and the discarded candidates.
/// - `DevicePriority`/`Manual`/`KeepBoth`: no conflict resolution, returns
///   `local + remote` with an empty journal.
///
/// The returned record vector is sorted exactly as the legacy runtime result
/// (by `bookId`, `chapterIndex`, `deviceId`, `updatedAt`, `syncVersion`).
pub fn resolve_with_journal(
    local: &[ProgressCloudSyncRecord],
    remote: &[ProgressCloudSyncRecord],
    policy: ConflictPolicy,
    resolved_at: i64,
) -> Result<(Vec<ProgressCloudSyncRecord>, ConflictJournal), SyncError> {
    for record in local.iter().chain(remote) {
        record.validate()?;
    }

    let mut journal = ConflictJournal::new(resolved_at);

    let mut resolved = match policy {
        ConflictPolicy::LastWriteWins => {
            let mut records: Vec<ProgressCloudSyncRecord> =
                local.iter().chain(remote).cloned().collect();
            records.sort_by(compare_progress_webdav_last_write_wins_order);

            // Group by key, preserving LWW order (first = newest).
            let mut groups: BTreeMap<ProgressCloudSyncRecordKey, Vec<ProgressCloudSyncRecord>> =
                BTreeMap::new();
            for record in records {
                groups.entry(record.key()).or_default().push(record);
            }

            let mut kept_records = Vec::with_capacity(groups.len());
            for (key, group) in groups {
                let mut iter = group.into_iter();
                let kept = iter.next().expect("non-empty group");
                let discarded: Vec<ProgressCloudSyncRecord> = iter.collect();
                if !discarded.is_empty() {
                    let mut discarded = discarded;
                    discarded.sort_by(compare_progress_runtime_order);
                    journal.entries.push(ConflictResolution {
                        key,
                        policy,
                        resolved_at,
                        kept: kept.clone(),
                        discarded,
                    });
                }
                kept_records.push(kept);
            }
            kept_records
        }
        ConflictPolicy::DevicePriority | ConflictPolicy::Manual | ConflictPolicy::KeepBoth => {
            local.iter().chain(remote).cloned().collect()
        }
    };

    sort_progress_cloud_records(&mut resolved);
    Ok((resolved, journal))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(book: &str, chapter: u32, device: &str, updated_at: i64) -> ProgressCloudSyncRecord {
        ProgressCloudSyncRecord::new(book, chapter, 0.5, updated_at, device).unwrap()
    }

    #[test]
    fn lww_no_conflict_yields_empty_journal() {
        let local = vec![record("b1", 1, "d1", 100)];
        let remote = vec![record("b2", 1, "d1", 200)];
        let (resolved, journal) =
            resolve_with_journal(&local, &remote, ConflictPolicy::LastWriteWins, 300).unwrap();
        assert_eq!(resolved.len(), 2);
        assert!(journal.is_empty());
    }

    #[test]
    fn lww_same_key_keeps_newest() {
        let local = vec![record("b1", 1, "d1", 100)];
        let remote = vec![record("b1", 1, "d1", 200)];
        let (resolved, journal) =
            resolve_with_journal(&local, &remote, ConflictPolicy::LastWriteWins, 300).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].updated_at, 200);
        assert_eq!(journal.len(), 1);
        let entry = &journal.entries[0];
        assert_eq!(entry.kept.updated_at, 200);
        assert_eq!(entry.discarded.len(), 1);
        assert_eq!(entry.discarded[0].updated_at, 100);
        assert_eq!(entry.policy, ConflictPolicy::LastWriteWins);
        assert_eq!(entry.resolved_at, 300);
    }

    #[test]
    fn lww_three_way_conflict_records_two_discarded() {
        let local = vec![record("b1", 1, "d1", 100), record("b1", 1, "d1", 300)];
        let remote = vec![record("b1", 1, "d1", 200)];
        let (resolved, journal) =
            resolve_with_journal(&local, &remote, ConflictPolicy::LastWriteWins, 400).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].updated_at, 300);
        assert_eq!(journal.len(), 1);
        let entry = &journal.entries[0];
        assert_eq!(entry.kept.updated_at, 300);
        assert_eq!(entry.discarded.len(), 2);
        // discarded sorted by runtime order (updatedAt asc)
        assert_eq!(entry.discarded[0].updated_at, 100);
        assert_eq!(entry.discarded[1].updated_at, 200);
    }

    #[test]
    fn lww_tiebreak_uses_runtime_order() {
        // Same updatedAt — tiebreaker is bookId/chapterIndex/deviceId asc.
        let local = vec![record("b1", 1, "d1", 100)];
        let remote = vec![record("b1", 1, "d2", 100)];
        let (resolved, journal) =
            resolve_with_journal(&local, &remote, ConflictPolicy::LastWriteWins, 200).unwrap();
        // Different device IDs = different keys = no conflict.
        assert_eq!(resolved.len(), 2);
        assert!(journal.is_empty());
    }

    #[test]
    fn keep_both_policy_no_resolution() {
        let local = vec![record("b1", 1, "d1", 100)];
        let remote = vec![record("b1", 1, "d1", 200)];
        let (resolved, journal) =
            resolve_with_journal(&local, &remote, ConflictPolicy::KeepBoth, 300).unwrap();
        assert_eq!(resolved.len(), 2);
        assert!(journal.is_empty());
    }

    #[test]
    fn device_priority_policy_no_resolution() {
        let local = vec![record("b1", 1, "d1", 100)];
        let remote = vec![record("b1", 1, "d1", 200)];
        let (resolved, journal) =
            resolve_with_journal(&local, &remote, ConflictPolicy::DevicePriority, 300).unwrap();
        assert_eq!(resolved.len(), 2);
        assert!(journal.is_empty());
    }

    #[test]
    fn manual_policy_no_resolution() {
        let local = vec![record("b1", 1, "d1", 100)];
        let remote = vec![record("b1", 1, "d1", 200)];
        let (resolved, journal) =
            resolve_with_journal(&local, &remote, ConflictPolicy::Manual, 300).unwrap();
        assert_eq!(resolved.len(), 2);
        assert!(journal.is_empty());
    }

    #[test]
    fn resolved_records_sorted_by_runtime_order() {
        let local = vec![record("b2", 1, "d1", 100), record("b1", 2, "d1", 200)];
        let remote = vec![];
        let (resolved, _) =
            resolve_with_journal(&local, &remote, ConflictPolicy::LastWriteWins, 300).unwrap();
        assert_eq!(resolved[0].book_id, "b1");
        assert_eq!(resolved[1].book_id, "b2");
    }

    #[test]
    fn empty_inputs_yield_empty_resolved_and_journal() {
        let (resolved, journal) =
            resolve_with_journal(&[], &[], ConflictPolicy::LastWriteWins, 100).unwrap();
        assert!(resolved.is_empty());
        assert!(journal.is_empty());
    }

    #[test]
    fn journal_serde_roundtrip() {
        let local = vec![record("b1", 1, "d1", 100)];
        let remote = vec![record("b1", 1, "d1", 200)];
        let (_, journal) =
            resolve_with_journal(&local, &remote, ConflictPolicy::LastWriteWins, 300).unwrap();
        let json = serde_json::to_string(&journal).unwrap();
        let decoded: ConflictJournal = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, journal);
    }

    #[test]
    fn invalid_record_rejected() {
        let bad = ProgressCloudSyncRecord {
            book_id: String::new(),
            chapter_index: 0,
            chapter_title: None,
            progress_fraction: 0.5,
            updated_at: 100,
            device_id: "d1".into(),
            sync_version: 1,
        };
        assert!(resolve_with_journal(&[bad], &[], ConflictPolicy::LastWriteWins, 200).is_err());
    }
}
