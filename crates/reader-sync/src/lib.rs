//! Reader-Core sync — WebDAV protocol, conflict resolution, backup/restore.
//!
//! This crate owns sync data semantics, not transport. V1 models sync as
//! deterministic snapshots of opaque records so bookshelf, reading progress,
//! local books, chapter cache, RSS subscriptions, and future settings can share
//! the same merge and backup/restore rules.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Current sync package schema version.
pub const SYNC_PACKAGE_SCHEMA_VERSION: u32 = 1;
/// Current local sync journal snapshot schema version.
pub const SYNC_JOURNAL_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

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

    pub fn parse(value: impl Into<String>) -> Result<Self, SyncError> {
        let value = normalize_required(value.into(), "collection")?;
        Ok(match value.as_str() {
            "bookshelf" => SyncCollection::Bookshelf,
            "readingProgress" => SyncCollection::ReadingProgress,
            "localBook" => SyncCollection::LocalBook,
            "chapterCache" => SyncCollection::ChapterCache,
            "rssSubscription" => SyncCollection::RssSubscription,
            "rssEntry" => SyncCollection::RssEntry,
            _ => SyncCollection::Custom(value),
        })
    }
}

impl Serialize for SyncCollection {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SyncCollection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        SyncCollection::parse(value).map_err(serde::de::Error::custom)
    }
}

/// Stable key for a record inside a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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

/// Wire package for backup/sync transports.
///
/// Packages carry one normalized snapshot plus a schema version so WebDAV,
/// local backup files, and future transport adapters can reject incompatible
/// data before applying it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncPackage {
    pub schema_version: u32,
    pub snapshot: SyncSnapshot,
}

impl SyncPackage {
    pub fn new(snapshot: SyncSnapshot) -> Result<Self, SyncError> {
        let records = snapshot.normalized_records()?;
        let snapshot = SyncSnapshot::new(
            snapshot.snapshot_id,
            snapshot.device_id,
            snapshot.created_at,
            records,
        )?;
        let package = Self {
            schema_version: SYNC_PACKAGE_SCHEMA_VERSION,
            snapshot,
        };
        package.validate()?;
        Ok(package)
    }

    pub fn validate(&self) -> Result<(), SyncError> {
        if self.schema_version != SYNC_PACKAGE_SCHEMA_VERSION {
            return Err(SyncError::InvalidPackage {
                field: "schema_version".into(),
            });
        }
        self.snapshot.validate()
    }

    pub fn to_json(&self) -> Result<String, SyncError> {
        self.validate()?;
        serde_json::to_string(self).map_err(SyncError::from_codec_error)
    }

    pub fn from_json(json: &str) -> Result<Self, SyncError> {
        let package = serde_json::from_str::<Self>(json).map_err(SyncError::from_codec_error)?;
        package.validate()?;
        Ok(package)
    }
}

/// Conflict details for records that changed differently across snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncConflict {
    pub key: SyncRecordKey,
    pub local: SyncRecord,
    pub remote: SyncRecord,
    pub winner: SyncRecord,
    pub reason: ConflictReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictReason {
    ConcurrentPayloadChange,
    DeleteVsUpdate,
    EqualTimestampTieBreak,
}

/// Merge result from two snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncMergeResult {
    pub snapshot: SyncSnapshot,
    pub conflicts: Vec<SyncConflict>,
}

/// Merge result from two sync packages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncPackageMergeResult {
    pub package: SyncPackage,
    pub conflicts: Vec<SyncConflict>,
}

/// Local pending state for one record in a sync journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncJournalEntry {
    pub sequence: u64,
    pub record: SyncRecord,
    pub status: SyncJournalEntryStatus,
    pub queued_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledged_at: Option<i64>,
}

impl SyncJournalEntry {
    pub fn key(&self) -> SyncRecordKey {
        self.record.key()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncJournalEntryStatus {
    Pending,
    Acknowledged,
}

/// Persistable sync journal state for one local device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncJournalSnapshot {
    pub schema_version: u32,
    pub device_id: String,
    pub next_sequence: u64,
    pub next_revision: u64,
    #[serde(default)]
    pub entries: Vec<SyncJournalEntry>,
}

impl SyncJournalSnapshot {
    pub fn validate(&self) -> Result<(), SyncError> {
        if self.schema_version != SYNC_JOURNAL_SNAPSHOT_SCHEMA_VERSION {
            return Err(SyncError::InvalidJournal {
                field: "schema_version".into(),
            });
        }
        let device_id = normalize_required(self.device_id.clone(), "device_id")?;
        if self.next_sequence == 0 {
            return Err(SyncError::InvalidJournal {
                field: "next_sequence".into(),
            });
        }
        if self.next_revision == 0 {
            return Err(SyncError::InvalidJournal {
                field: "next_revision".into(),
            });
        }

        let mut keys = BTreeSet::<SyncRecordKey>::new();
        let mut max_sequence = 0u64;
        let mut max_revision = 0u64;
        for entry in &self.entries {
            validate_journal_entry(entry, &device_id)?;
            if !keys.insert(entry.key()) {
                return Err(SyncError::InvalidJournal {
                    field: "entries".into(),
                });
            }
            max_sequence = max_sequence.max(entry.sequence);
            max_revision = max_revision.max(entry.record.revision);
        }

        if self.next_sequence <= max_sequence {
            return Err(SyncError::InvalidJournal {
                field: "next_sequence".into(),
            });
        }
        if self.next_revision <= max_revision {
            return Err(SyncError::InvalidJournal {
                field: "next_revision".into(),
            });
        }
        Ok(())
    }
}

/// In-memory local journal for pending sync records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncJournal {
    device_id: String,
    next_sequence: u64,
    next_revision: u64,
    entries: BTreeMap<SyncRecordKey, SyncJournalEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    InvalidRecord { field: String },
    InvalidPackage { field: String },
    InvalidJournal { field: String },
    Codec { message: String },
}

impl SyncError {
    fn from_codec_error(error: serde_json::Error) -> Self {
        Self::Codec {
            message: error.to_string(),
        }
    }
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncError::InvalidRecord { field } => write!(f, "invalid sync record field: {field}"),
            SyncError::InvalidPackage { field } => {
                write!(f, "invalid sync package field: {field}")
            }
            SyncError::InvalidJournal { field } => {
                write!(f, "invalid sync journal field: {field}")
            }
            SyncError::Codec { message } => write!(f, "sync codec error: {message}"),
        }
    }
}

impl std::error::Error for SyncError {}

impl SyncJournal {
    pub fn new(device_id: impl Into<String>) -> Result<Self, SyncError> {
        Ok(Self {
            device_id: normalize_required(device_id.into(), "device_id")?,
            next_sequence: 1,
            next_revision: 1,
            entries: BTreeMap::new(),
        })
    }

    pub fn from_snapshot(snapshot: SyncJournalSnapshot) -> Result<Self, SyncError> {
        snapshot.validate()?;
        let mut entries = BTreeMap::new();
        for entry in snapshot.entries {
            entries.insert(entry.key(), entry);
        }
        Ok(Self {
            device_id: snapshot.device_id,
            next_sequence: snapshot.next_sequence,
            next_revision: snapshot.next_revision,
            entries,
        })
    }

    pub fn export_snapshot(&self) -> Result<SyncJournalSnapshot, SyncError> {
        let snapshot = SyncJournalSnapshot {
            schema_version: SYNC_JOURNAL_SNAPSHOT_SCHEMA_VERSION,
            device_id: self.device_id.clone(),
            next_sequence: self.next_sequence,
            next_revision: self.next_revision,
            entries: self.entries.values().cloned().collect(),
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn record_upsert(
        &mut self,
        collection: SyncCollection,
        record_id: impl Into<String>,
        payload: impl Into<String>,
        updated_at: i64,
        queued_at: i64,
    ) -> Result<SyncJournalEntry, SyncError> {
        let revision = self.next_revision;
        let record = SyncRecord::upsert(
            collection,
            record_id,
            payload,
            updated_at,
            self.device_id.clone(),
            revision,
        )?;
        self.next_revision += 1;
        self.record_change(record, queued_at)
    }

    pub fn record_tombstone(
        &mut self,
        collection: SyncCollection,
        record_id: impl Into<String>,
        updated_at: i64,
        queued_at: i64,
    ) -> Result<SyncJournalEntry, SyncError> {
        let revision = self.next_revision;
        let record = SyncRecord::tombstone(
            collection,
            record_id,
            updated_at,
            self.device_id.clone(),
            revision,
        )?;
        self.next_revision += 1;
        self.record_change(record, queued_at)
    }

    pub fn pending_records(&self) -> Vec<SyncRecord> {
        self.entries
            .values()
            .filter(|entry| entry.status == SyncJournalEntryStatus::Pending)
            .map(|entry| entry.record.clone())
            .collect()
    }

    pub fn pending_package(
        &self,
        snapshot_id: impl Into<String>,
        created_at: i64,
    ) -> Result<SyncPackage, SyncError> {
        SyncPackage::new(SyncSnapshot::new(
            snapshot_id,
            self.device_id.clone(),
            created_at,
            self.pending_records(),
        )?)
    }

    /// Mark matching pending records as acknowledged.
    ///
    /// Acknowledgement is exact: if a newer local change has replaced a record
    /// after an older package was sent, acknowledging the old package will not
    /// clear the newer pending entry.
    pub fn acknowledge_package(
        &mut self,
        package: &SyncPackage,
        acknowledged_at: i64,
    ) -> Result<usize, SyncError> {
        package.validate()?;
        let records = package.snapshot.normalized_records()?;
        let mut acknowledged = 0usize;
        for record in records {
            let key = record.key();
            let Some(entry) = self.entries.get_mut(&key) else {
                continue;
            };
            if entry.status == SyncJournalEntryStatus::Pending && entry.record == record {
                entry.status = SyncJournalEntryStatus::Acknowledged;
                entry.acknowledged_at = Some(acknowledged_at);
                acknowledged += 1;
            }
        }
        Ok(acknowledged)
    }

    pub fn entries(&self) -> Vec<SyncJournalEntry> {
        self.entries.values().cloned().collect()
    }

    fn record_change(
        &mut self,
        record: SyncRecord,
        queued_at: i64,
    ) -> Result<SyncJournalEntry, SyncError> {
        if record.device_id != self.device_id {
            return Err(SyncError::InvalidJournal {
                field: "record.device_id".into(),
            });
        }
        let entry = SyncJournalEntry {
            sequence: self.take_sequence(),
            record,
            status: SyncJournalEntryStatus::Pending,
            queued_at,
            acknowledged_at: None,
        };
        validate_journal_entry(&entry, &self.device_id)?;
        self.entries.insert(entry.key(), entry.clone());
        Ok(entry)
    }

    fn take_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        sequence
    }
}

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

/// Merge two wire packages and return a normalized package plus conflicts.
pub fn merge_packages(
    local: &SyncPackage,
    remote: &SyncPackage,
    merged_snapshot_id: impl Into<String>,
    merged_device_id: impl Into<String>,
    merged_created_at: i64,
) -> Result<SyncPackageMergeResult, SyncError> {
    local.validate()?;
    remote.validate()?;
    let result = merge_snapshots(
        &local.snapshot,
        &remote.snapshot,
        merged_snapshot_id,
        merged_device_id,
        merged_created_at,
    )?;
    Ok(SyncPackageMergeResult {
        package: SyncPackage::new(result.snapshot)?,
        conflicts: result.conflicts,
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

fn validate_journal_entry(entry: &SyncJournalEntry, device_id: &str) -> Result<(), SyncError> {
    if entry.sequence == 0 {
        return Err(SyncError::InvalidJournal {
            field: "entries.sequence".into(),
        });
    }
    entry.record.validate()?;
    if entry.record.device_id != device_id {
        return Err(SyncError::InvalidJournal {
            field: "entries.record.device_id".into(),
        });
    }
    match entry.status {
        SyncJournalEntryStatus::Pending if entry.acknowledged_at.is_some() => {
            Err(SyncError::InvalidJournal {
                field: "entries.acknowledged_at".into(),
            })
        }
        SyncJournalEntryStatus::Acknowledged if entry.acknowledged_at.is_none() => {
            Err(SyncError::InvalidJournal {
                field: "entries.acknowledged_at".into(),
            })
        }
        _ => Ok(()),
    }
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
    fn collection_json_uses_stable_strings_and_accepts_custom_buckets() {
        assert_eq!(
            serde_json::to_string(&SyncCollection::ReadingProgress).unwrap(),
            r#""readingProgress""#
        );
        assert_eq!(
            serde_json::from_str::<SyncCollection>(r#""rssEntry""#).unwrap(),
            SyncCollection::RssEntry
        );
        assert_eq!(
            serde_json::from_str::<SyncCollection>(r#""deviceSettings""#).unwrap(),
            SyncCollection::Custom("deviceSettings".into())
        );
        assert!(serde_json::from_str::<SyncCollection>(r#""   ""#).is_err());
    }

    #[test]
    fn record_json_round_trips_and_denies_unknown_fields() {
        let record = rec(SyncCollection::Bookshelf, "s1/b1", r#"{"title":"A"}"#, 10);

        let json = serde_json::to_string(&record).unwrap();
        assert_eq!(serde_json::from_str::<SyncRecord>(&json).unwrap(), record);
        assert!(json.contains(r#""collection":"bookshelf""#));

        let unknown = r#"{"collection":"bookshelf","recordId":"b","updatedAt":1,"deviceId":"d","revision":1,"payload":"{}","deleted":false,"bogus":true}"#;
        assert!(serde_json::from_str::<SyncRecord>(unknown).is_err());
    }

    #[test]
    fn sync_package_normalizes_records_and_json_round_trips() {
        let old = rec(SyncCollection::Bookshelf, "b1", r#"{"title":"old"}"#, 1);
        let latest = rec(SyncCollection::Bookshelf, "b1", r#"{"title":"new"}"#, 2);
        let rss = rec(
            SyncCollection::RssSubscription,
            "feed",
            r#"{"url":"https://example.test/feed.xml"}"#,
            1,
        );
        let snapshot = snap("snapshot", vec![rss.clone(), old, latest.clone()]);

        let package = SyncPackage::new(snapshot).unwrap();

        assert_eq!(package.schema_version, SYNC_PACKAGE_SCHEMA_VERSION);
        assert_eq!(package.snapshot.records, vec![latest, rss]);
        let json = package.to_json().unwrap();
        assert!(json.contains(r#""schemaVersion":1"#));
        assert_eq!(SyncPackage::from_json(&json).unwrap(), package);
    }

    #[test]
    fn sync_package_rejects_schema_unknown_fields_and_invalid_nested_records() {
        let mut package = SyncPackage::new(snap("snapshot", Vec::new())).unwrap();
        package.schema_version = 2;
        assert_eq!(
            package.validate().unwrap_err(),
            SyncError::InvalidPackage {
                field: "schema_version".into()
            }
        );

        let unknown_package_field = r#"{"schemaVersion":1,"snapshot":{"snapshotId":"s","deviceId":"d","createdAt":1,"records":[]},"bogus":true}"#;
        assert!(matches!(
            SyncPackage::from_json(unknown_package_field),
            Err(SyncError::Codec { .. })
        ));

        let invalid_record = r#"{"schemaVersion":1,"snapshot":{"snapshotId":"s","deviceId":"d","createdAt":1,"records":[{"collection":"bookshelf","recordId":"b","updatedAt":1,"deviceId":"d","revision":1,"payload":"   ","deleted":false}]}}"#;
        assert_eq!(
            SyncPackage::from_json(invalid_record).unwrap_err(),
            SyncError::InvalidRecord {
                field: "payload".into()
            }
        );
    }

    #[test]
    fn merge_packages_returns_normalized_package_and_conflicts() {
        let local = SyncPackage::new(snap(
            "local",
            vec![rec(
                SyncCollection::ReadingProgress,
                "s1/b1",
                r#"{"chapter":1}"#,
                10,
            )],
        ))
        .unwrap();
        let mut remote_record = rec(
            SyncCollection::ReadingProgress,
            "s1/b1",
            r#"{"chapter":2}"#,
            11,
        );
        remote_record.device_id = "device-b".into();
        let remote = SyncPackage::new(snap("remote", vec![remote_record.clone()])).unwrap();

        let result = merge_packages(&local, &remote, "merged", "merge-device", 12).unwrap();

        assert_eq!(result.package.schema_version, SYNC_PACKAGE_SCHEMA_VERSION);
        assert_eq!(result.package.snapshot.snapshot_id, "merged");
        assert_eq!(result.package.snapshot.records, vec![remote_record.clone()]);
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(
            result.conflicts[0].reason,
            ConflictReason::ConcurrentPayloadChange
        );
        assert_eq!(
            SyncPackage::from_json(&result.package.to_json().unwrap()).unwrap(),
            result.package
        );
    }

    #[test]
    fn sync_journal_records_pending_changes_and_builds_package() {
        let mut journal = SyncJournal::new("device-a").unwrap();

        let first = journal
            .record_upsert(SyncCollection::Bookshelf, "b2", r#"{"title":"B"}"#, 10, 100)
            .unwrap();
        let second = journal
            .record_upsert(SyncCollection::Bookshelf, "b1", r#"{"title":"A"}"#, 11, 101)
            .unwrap();
        let deleted = journal
            .record_tombstone(SyncCollection::RssEntry, "entry-1", 12, 102)
            .unwrap();

        assert_eq!(first.sequence, 1);
        assert_eq!(first.record.revision, 1);
        assert_eq!(second.sequence, 2);
        assert_eq!(second.record.revision, 2);
        assert_eq!(deleted.sequence, 3);
        assert_eq!(deleted.record.revision, 3);

        let pending = journal.pending_records();
        assert_eq!(
            pending
                .iter()
                .map(|record| (record.collection.as_str(), record.record_id.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("bookshelf", "b1"),
                ("bookshelf", "b2"),
                ("rssEntry", "entry-1")
            ]
        );

        let package = journal.pending_package("pending-1", 200).unwrap();
        assert_eq!(package.snapshot.snapshot_id, "pending-1");
        assert_eq!(package.snapshot.device_id, "device-a");
        assert_eq!(package.snapshot.created_at, 200);
        assert_eq!(package.snapshot.records, pending);
        assert_eq!(
            SyncPackage::from_json(&package.to_json().unwrap()).unwrap(),
            package
        );
    }

    #[test]
    fn sync_journal_acknowledges_only_exact_pending_records() {
        let mut journal = SyncJournal::new("device-a").unwrap();
        journal
            .record_upsert(
                SyncCollection::ReadingProgress,
                "book",
                r#"{"chapter":1}"#,
                10,
                100,
            )
            .unwrap();
        let stale_package = journal.pending_package("stale", 101).unwrap();

        journal
            .record_upsert(
                SyncCollection::ReadingProgress,
                "book",
                r#"{"chapter":2}"#,
                11,
                102,
            )
            .unwrap();

        assert_eq!(
            journal.acknowledge_package(&stale_package, 200).unwrap(),
            0,
            "acknowledging an older sent package must not clear a newer pending change"
        );
        let pending = journal.pending_records();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].revision, 2);
        assert_eq!(pending[0].payload, r#"{"chapter":2}"#);

        let current_package = journal.pending_package("current", 201).unwrap();
        assert_eq!(
            journal.acknowledge_package(&current_package, 202).unwrap(),
            1
        );
        assert!(journal.pending_records().is_empty());
        let entries = journal.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, SyncJournalEntryStatus::Acknowledged);
        assert_eq!(entries[0].acknowledged_at, Some(202));
    }

    #[test]
    fn sync_journal_snapshot_round_trips_and_json_denies_unknown_fields() {
        let mut journal = SyncJournal::new("device-a").unwrap();
        journal
            .record_upsert(
                SyncCollection::LocalBook,
                "local-1",
                r#"{"title":"Local"}"#,
                10,
                100,
            )
            .unwrap();
        let package = journal.pending_package("pending", 101).unwrap();
        journal.acknowledge_package(&package, 102).unwrap();

        let snapshot = journal.export_snapshot().unwrap();

        assert_eq!(
            snapshot.schema_version,
            SYNC_JOURNAL_SNAPSHOT_SCHEMA_VERSION
        );
        assert_eq!(snapshot.device_id, "device-a");
        assert_eq!(snapshot.next_sequence, 2);
        assert_eq!(snapshot.next_revision, 2);
        assert_eq!(
            snapshot.entries[0].status,
            SyncJournalEntryStatus::Acknowledged
        );

        let json = serde_json::to_string(&snapshot).unwrap();
        let back: SyncJournalSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snapshot);
        assert_eq!(
            SyncJournal::from_snapshot(back)
                .unwrap()
                .export_snapshot()
                .unwrap(),
            snapshot
        );

        let unknown = r#"{"schemaVersion":1,"deviceId":"device-a","nextSequence":1,"nextRevision":1,"entries":[],"bogus":true}"#;
        assert!(serde_json::from_str::<SyncJournalSnapshot>(unknown).is_err());
    }

    #[test]
    fn sync_journal_snapshot_rejects_schema_duplicates_and_invalid_status() {
        let mut journal = SyncJournal::new("device-a").unwrap();
        let entry = journal
            .record_upsert(
                SyncCollection::Bookshelf,
                "book",
                r#"{"title":"A"}"#,
                10,
                100,
            )
            .unwrap();

        let mut wrong_schema = journal.export_snapshot().unwrap();
        wrong_schema.schema_version = 2;
        assert_eq!(
            wrong_schema.validate().unwrap_err(),
            SyncError::InvalidJournal {
                field: "schema_version".into()
            }
        );

        let mut duplicate = journal.export_snapshot().unwrap();
        duplicate.next_sequence = 3;
        duplicate.next_revision = 3;
        duplicate.entries.push(entry.clone());
        assert_eq!(
            duplicate.validate().unwrap_err(),
            SyncError::InvalidJournal {
                field: "entries".into()
            }
        );

        let mut invalid_status = journal.export_snapshot().unwrap();
        invalid_status.entries[0].acknowledged_at = Some(200);
        assert_eq!(
            invalid_status.validate().unwrap_err(),
            SyncError::InvalidJournal {
                field: "entries.acknowledged_at".into()
            }
        );

        let mut device_mismatch = journal.export_snapshot().unwrap();
        device_mismatch.entries[0].record.device_id = "other-device".into();
        assert_eq!(
            device_mismatch.validate().unwrap_err(),
            SyncError::InvalidJournal {
                field: "entries.record.device_id".into()
            }
        );
    }

    #[test]
    fn sync_journal_invalid_update_does_not_consume_revision_or_sequence() {
        let mut journal = SyncJournal::new("device-a").unwrap();

        assert_eq!(
            journal
                .record_upsert(SyncCollection::Bookshelf, "book", "   ", 10, 100)
                .unwrap_err(),
            SyncError::InvalidRecord {
                field: "payload".into()
            }
        );

        let entry = journal
            .record_upsert(
                SyncCollection::Bookshelf,
                "book",
                r#"{"title":"A"}"#,
                11,
                101,
            )
            .unwrap();
        assert_eq!(entry.sequence, 1);
        assert_eq!(entry.record.revision, 1);
        let snapshot = journal.export_snapshot().unwrap();
        assert_eq!(snapshot.next_sequence, 2);
        assert_eq!(snapshot.next_revision, 2);
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
