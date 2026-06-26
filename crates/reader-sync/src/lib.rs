//! Reader-Core sync — WebDAV protocol, conflict resolution, backup/restore.
//!
//! This crate owns sync data semantics, not transport. V1 models sync as
//! deterministic snapshots of opaque records so bookshelf, reading progress,
//! local books, chapter cache, RSS subscriptions, and future settings can share
//! the same merge and backup/restore rules.

pub mod webdav_protocol;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Current sync package schema version.
pub const SYNC_PACKAGE_SCHEMA_VERSION: u32 = 1;
/// Current local sync journal snapshot schema version.
pub const SYNC_JOURNAL_SNAPSHOT_SCHEMA_VERSION: u32 = 1;
/// Legacy WebDAV remote-book import request preview upper bound.
pub const WEBDAV_REMOTE_BOOK_IMPORT_MAX_PREVIEW_LIMIT: u32 = 64;
/// Legacy WebDAV remote-book import request default candidate cap.
pub const WEBDAV_REMOTE_BOOK_IMPORT_DEFAULT_MAXIMUM_BOOK_COUNT: usize = 32;

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

/// WebDAV/local backup target settings ported from Reader-Core's backup config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupConfig {
    #[serde(rename = "targetURL")]
    pub target_url: String,
    #[serde(
        rename = "authCredentialID",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub auth_credential_id: Option<String>,
    #[serde(default)]
    pub compression_enabled: bool,
    #[serde(default)]
    pub encryption_enabled: bool,
    #[serde(default = "default_max_backups")]
    pub max_backups: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_days: Option<u32>,
}

impl BackupConfig {
    pub fn new(target_url: impl Into<String>) -> Result<Self, SyncError> {
        let config = Self {
            target_url: normalize_backup_required(target_url.into(), "target_url")?,
            auth_credential_id: None,
            compression_enabled: false,
            encryption_enabled: false,
            max_backups: default_max_backups(),
            max_age_days: None,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), SyncError> {
        validate_backup_required(&self.target_url, "target_url")?;
        validate_backup_optional_string(&self.auth_credential_id, "auth_credential_id")?;
        if self.max_backups == 0 {
            return Err(SyncError::InvalidBackup {
                field: "max_backups".into(),
            });
        }
        Ok(())
    }
}

fn default_max_backups() -> u32 {
    5
}

/// Backup schedule frequency wire values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackupFrequency {
    Manual,
    Hourly,
    Daily,
    Weekly,
}

impl Default for BackupFrequency {
    fn default() -> Self {
        Self::Manual
    }
}

/// Backup cadence settings.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupSchedule {
    #[serde(default)]
    pub frequency: BackupFrequency,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_hour: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_weekday: Option<u8>,
}

impl BackupSchedule {
    pub fn validate(&self) -> Result<(), SyncError> {
        if self
            .preferred_hour
            .is_some_and(|preferred_hour| preferred_hour > 23)
        {
            return Err(SyncError::InvalidBackup {
                field: "preferred_hour".into(),
            });
        }
        if self
            .preferred_weekday
            .is_some_and(|preferred_weekday| !(1..=7).contains(&preferred_weekday))
        {
            return Err(SyncError::InvalidBackup {
                field: "preferred_weekday".into(),
            });
        }
        Ok(())
    }
}

/// One file included in a backup manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupManifestEntry {
    pub relative_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub size_bytes: u64,
    pub modified_at: i64,
}

impl BackupManifestEntry {
    pub fn validate(&self) -> Result<(), SyncError> {
        validate_backup_relative_path(&self.relative_path)?;
        validate_backup_optional_string(&self.sha256, "sha256")?;
        Ok(())
    }
}

/// Portable manifest for one backup package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupManifest {
    #[serde(rename = "backupID")]
    pub backup_id: String,
    pub created_at: i64,
    #[serde(default)]
    pub entries: Vec<BackupManifestEntry>,
    pub total_bytes: u64,
    pub book_count: u32,
}

impl BackupManifest {
    pub fn validate(&self) -> Result<(), SyncError> {
        validate_backup_required(&self.backup_id, "backup_id")?;
        let mut paths = BTreeSet::<String>::new();
        let mut entry_total = 0u64;
        for entry in &self.entries {
            entry.validate()?;
            if !paths.insert(entry.relative_path.clone()) {
                return Err(SyncError::InvalidBackup {
                    field: "entries.relative_path".into(),
                });
            }
            entry_total = entry_total.saturating_add(entry.size_bytes);
        }
        if self.total_bytes < entry_total {
            return Err(SyncError::InvalidBackup {
                field: "total_bytes".into(),
            });
        }
        if self.book_count as usize > self.entries.len() {
            return Err(SyncError::InvalidBackup {
                field: "book_count".into(),
            });
        }
        Ok(())
    }
}

/// Backup package archive format wire values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackupArchiveFormat {
    Zip,
    Tar,
    Directory,
}

impl Default for BackupArchiveFormat {
    fn default() -> Self {
        Self::Zip
    }
}

/// Transport-neutral backup package metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupPackage {
    pub manifest: BackupManifest,
    #[serde(default)]
    pub format: BackupArchiveFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

impl BackupPackage {
    pub fn new(manifest: BackupManifest) -> Result<Self, SyncError> {
        let package = Self {
            manifest,
            format: BackupArchiveFormat::Zip,
            checksum: None,
        };
        package.validate()?;
        Ok(package)
    }

    pub fn validate(&self) -> Result<(), SyncError> {
        self.manifest.validate()?;
        validate_backup_optional_string(&self.checksum, "checksum")?;
        Ok(())
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

/// A remote backup candidate considered by retention planning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupRetentionItem {
    pub manifest: BackupManifest,
    pub remote_path: String,
}

impl BackupRetentionItem {
    pub fn validate(&self) -> Result<(), SyncError> {
        self.manifest.validate()?;
        validate_backup_required(&self.remote_path, "remote_path")?;
        Ok(())
    }
}

/// Deterministic delete plan for backup retention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupRetentionPlan {
    pub paths_to_delete: Vec<String>,
}

/// Build the backup retention delete plan without touching remote storage.
///
/// This mirrors the legacy WebDAV retention policy: preserve the newest
/// `maxBackups` packages by `createdAt`, delete packages older than
/// `maxAgeDays`, and never delete the just-created backup id.
pub fn plan_backup_retention(
    config: &BackupConfig,
    preserving_backup_id: &str,
    evaluated_at: i64,
    candidates: &[BackupRetentionItem],
) -> Result<BackupRetentionPlan, SyncError> {
    config.validate()?;
    validate_backup_required(preserving_backup_id, "preserving_backup_id")?;

    let mut candidates = candidates.to_vec();
    for candidate in &candidates {
        candidate.validate()?;
    }
    candidates.sort_by(|left, right| {
        right
            .manifest
            .created_at
            .cmp(&left.manifest.created_at)
            .then_with(|| left.manifest.backup_id.cmp(&right.manifest.backup_id))
    });

    let mut paths = BTreeSet::<String>::new();
    for candidate in candidates.iter().skip(config.max_backups as usize) {
        if candidate.manifest.backup_id != preserving_backup_id {
            paths.insert(candidate.remote_path.clone());
        }
    }

    if let Some(max_age_days) = config.max_age_days {
        let max_age_seconds = i64::from(max_age_days).saturating_mul(24 * 60 * 60);
        let cutoff = evaluated_at.saturating_sub(max_age_seconds);
        for candidate in &candidates {
            if candidate.manifest.created_at < cutoff
                && candidate.manifest.backup_id != preserving_backup_id
            {
                paths.insert(candidate.remote_path.clone());
            }
        }
    }

    Ok(BackupRetentionPlan {
        paths_to_delete: paths.into_iter().collect(),
    })
}

/// Remote WebDAV book metadata discovered before local import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteBookMetadata {
    pub remote_path: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_modified_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
}

impl RemoteBookMetadata {
    pub fn validate(&self) -> Result<(), SyncError> {
        validate_backup_required(&self.remote_path, "remote_path")?;
        validate_backup_required(&self.title, "title")?;
        validate_backup_optional_string(&self.author, "author")?;
        validate_backup_optional_string(&self.format, "format")?;
        validate_backup_optional_string(&self.etag, "etag")?;
        Ok(())
    }
}

/// Transport-neutral WebDAV remote-book import selection request.
///
/// This mirrors the legacy runtime's pre-download candidate selection without
/// performing connection tests, downloads, or local-book imports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavRemoteBookImportRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_book_directory_path: Option<String>,
    #[serde(default)]
    pub remote_books: Vec<RemoteBookMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_remote_paths: Option<Vec<String>>,
    #[serde(default = "default_webdav_remote_book_import_mode")]
    pub import_mode: WebDavRemoteBookImportMode,
    #[serde(default = "default_webdav_remote_book_import_maximum_book_count")]
    pub maximum_book_count: usize,
    #[serde(default = "default_webdav_remote_book_import_preview_limit")]
    pub preview_limit: u32,
    #[serde(default = "default_require_connection_test")]
    pub require_connection_test: bool,
}

impl Default for WebDavRemoteBookImportRequest {
    fn default() -> Self {
        Self {
            remote_book_directory_path: None,
            remote_books: Vec::new(),
            selected_remote_paths: None,
            import_mode: default_webdav_remote_book_import_mode(),
            maximum_book_count: default_webdav_remote_book_import_maximum_book_count(),
            preview_limit: default_webdav_remote_book_import_preview_limit(),
            require_connection_test: default_require_connection_test(),
        }
    }
}

impl WebDavRemoteBookImportRequest {
    pub fn effective_preview_limit(&self) -> u32 {
        self.preview_limit
            .min(WEBDAV_REMOTE_BOOK_IMPORT_MAX_PREVIEW_LIMIT)
    }

    pub fn validate(&self) -> Result<(), SyncError> {
        validate_backup_optional_string(
            &self.remote_book_directory_path,
            "remote_book_directory_path",
        )?;
        if let Some(selected_paths) = &self.selected_remote_paths {
            for path in selected_paths {
                validate_backup_required(path, "selected_remote_paths")?;
            }
        }
        for book in &self.remote_books {
            book.validate()?;
        }
        Ok(())
    }
}

fn default_webdav_remote_book_import_maximum_book_count() -> usize {
    WEBDAV_REMOTE_BOOK_IMPORT_DEFAULT_MAXIMUM_BOOK_COUNT
}

fn default_webdav_remote_book_import_preview_limit() -> u32 {
    WEBDAV_REMOTE_BOOK_IMPORT_MAX_PREVIEW_LIMIT
}

fn default_require_connection_test() -> bool {
    true
}

/// Local-book import mode requested by legacy WebDAV remote-book import.
///
/// This mirrors `ReaderCoreLocalBookLibraryImportMode` wire values without
/// coupling sync planning to the local-book crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebDavRemoteBookImportMode {
    MetadataOnly,
    IndexOnly,
    LazyContent,
    EagerFirstChapter,
    EagerAllContent,
    ValidateExisting,
    ReimportChanged,
}

fn default_webdav_remote_book_import_mode() -> WebDavRemoteBookImportMode {
    WebDavRemoteBookImportMode::LazyContent
}

/// Deterministic WebDAV remote-book import candidate plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavRemoteBookImportPlan {
    pub effective_preview_limit: u32,
    pub import_mode: WebDavRemoteBookImportMode,
    pub listed_remote_books: Vec<RemoteBookMetadata>,
    pub importable_remote_books: Vec<RemoteBookMetadata>,
    pub skipped_remote_paths: Vec<String>,
    pub operation_log: Vec<String>,
}

impl WebDavRemoteBookImportPlan {
    pub fn validate(&self) -> Result<(), SyncError> {
        if self.effective_preview_limit == 0 {
            return Err(SyncError::InvalidBackup {
                field: "effective_preview_limit".into(),
            });
        }
        let listed_paths =
            validate_remote_book_path_set(&self.listed_remote_books, "listed_remote_books")?;
        let importable_paths = validate_remote_book_path_set(
            &self.importable_remote_books,
            "importable_remote_books",
        )?;
        for path in &importable_paths {
            if !listed_paths.contains(path) {
                return Err(SyncError::InvalidBackup {
                    field: "importable_remote_books.remote_path".into(),
                });
            }
        }
        let skipped_paths =
            validate_unique_sync_string_list(&self.skipped_remote_paths, "skipped_remote_paths")?;
        for path in &skipped_paths {
            if !listed_paths.contains(path) || importable_paths.contains(path) {
                return Err(SyncError::InvalidBackup {
                    field: "skipped_remote_paths".into(),
                });
            }
        }
        for book in &self.listed_remote_books {
            book.validate()?;
        }
        for book in &self.importable_remote_books {
            book.validate()?;
        }
        validate_sync_string_list(&self.skipped_remote_paths, "skipped_remote_paths")?;
        validate_sync_string_list(&self.operation_log, "operation_log")
    }
}

/// Transport operation kind for the legacy WebDAV remote-book import runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WebDavRemoteBookImportOperationKind {
    ConnectionTest,
    ListDirectory,
    DownloadFile,
}

/// One planned host-adapter operation for remote-book import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavRemoteBookImportOperation {
    pub kind: WebDavRemoteBookImportOperationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl WebDavRemoteBookImportOperation {
    pub fn validate(&self) -> Result<(), SyncError> {
        match self.kind {
            WebDavRemoteBookImportOperationKind::ConnectionTest => {
                if self.path.is_some() {
                    return Err(SyncError::InvalidBackup {
                        field: "operation.path".into(),
                    });
                }
            }
            WebDavRemoteBookImportOperationKind::ListDirectory
            | WebDavRemoteBookImportOperationKind::DownloadFile => {
                validate_backup_required(
                    self.path.as_deref().unwrap_or_default(),
                    "operation.path",
                )?;
            }
        }
        Ok(())
    }
}

/// Candidate plan plus exact adapter operation order for WebDAV remote imports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavRemoteBookImportExecutionPlan {
    pub candidate_plan: WebDavRemoteBookImportPlan,
    pub operations: Vec<WebDavRemoteBookImportOperation>,
}

impl WebDavRemoteBookImportExecutionPlan {
    pub fn validate(&self) -> Result<(), SyncError> {
        self.candidate_plan.validate()?;
        for operation in &self.operations {
            operation.validate()?;
        }
        Ok(())
    }
}

/// Legacy WebDAV remote-book import result status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WebDavRemoteBookImportStatus {
    Success,
    PartialFailure,
    Failure,
}

/// Classify the transport/import outcome using the legacy runtime rule:
/// no errors is success, any imported book plus errors is partial failure,
/// otherwise the import failed before producing a usable local book.
pub fn classify_webdav_remote_book_import_status(
    imported_book_count: usize,
    errors: &[String],
) -> WebDavRemoteBookImportStatus {
    if errors.is_empty() {
        WebDavRemoteBookImportStatus::Success
    } else if imported_book_count > 0 {
        WebDavRemoteBookImportStatus::PartialFailure
    } else {
        WebDavRemoteBookImportStatus::Failure
    }
}

/// Metadata handed to local-book import for one downloaded WebDAV book.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavRemoteBookSourceMetadata {
    pub byte_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modification_timestamp: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_identifier_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path_checksum: Option<String>,
}

/// Transport-neutral local-book import input derived from remote metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavRemoteBookLocalImportInput {
    pub remote_book: RemoteBookMetadata,
    pub declared_filename: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_extension: Option<String>,
    #[serde(
        default,
        rename = "declaredMIMEType",
        skip_serializing_if = "Option::is_none"
    )]
    pub declared_mime_type: Option<String>,
    pub source_metadata: WebDavRemoteBookSourceMetadata,
}

/// Host-completed local import summary for one downloaded WebDAV book.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavRemoteBookImportedBook {
    pub remote_book: RemoteBookMetadata,
    pub detected_format: String,
    pub chapter_content_count_materialized: u32,
    pub downloaded_byte_count: u64,
}

impl WebDavRemoteBookImportedBook {
    pub fn validate(&self) -> Result<(), SyncError> {
        self.remote_book.validate()?;
        validate_backup_required(&self.detected_format, "detected_format")?;
        Ok(())
    }
}

/// Pure result-summary request for the legacy WebDAV remote-book import runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavRemoteBookImportResultRequest {
    #[serde(default = "default_require_connection_test")]
    pub require_connection_test: bool,
    #[serde(rename = "connectionOK")]
    pub connection_ok: bool,
    pub plan: WebDavRemoteBookImportPlan,
    #[serde(default)]
    pub imported_books: Vec<WebDavRemoteBookImportedBook>,
    #[serde(default)]
    pub import_errors: Vec<String>,
}

impl WebDavRemoteBookImportResultRequest {
    pub fn validate(&self) -> Result<(), SyncError> {
        self.plan.validate()?;
        let importable_paths = self
            .plan
            .importable_remote_books
            .iter()
            .map(|book| book.remote_path.clone())
            .collect::<BTreeSet<_>>();
        let mut imported_paths = BTreeSet::new();
        for imported_book in &self.imported_books {
            imported_book.validate()?;
            if !importable_paths.contains(&imported_book.remote_book.remote_path)
                || !imported_paths.insert(imported_book.remote_book.remote_path.clone())
            {
                return Err(SyncError::InvalidBackup {
                    field: "imported_books.remote_path".into(),
                });
            }
        }
        validate_sync_string_list(&self.import_errors, "import_errors")
    }
}

/// Transport-neutral WebDAV remote-book import result envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavRemoteBookImportResult {
    pub status: WebDavRemoteBookImportStatus,
    #[serde(rename = "connectionOK")]
    pub connection_ok: bool,
    pub listed_remote_books: Vec<RemoteBookMetadata>,
    pub imported_books: Vec<WebDavRemoteBookImportedBook>,
    pub skipped_remote_paths: Vec<String>,
    #[serde(default)]
    pub operation_log: Vec<String>,
    #[serde(default)]
    pub errors: Vec<String>,
}

impl WebDavRemoteBookImportResult {
    pub fn validate(&self) -> Result<(), SyncError> {
        let listed_paths =
            validate_remote_book_path_set(&self.listed_remote_books, "listed_remote_books")?;
        let mut imported_paths = BTreeSet::new();
        for imported_book in &self.imported_books {
            imported_book.validate()?;
            if !listed_paths.contains(&imported_book.remote_book.remote_path)
                || !imported_paths.insert(imported_book.remote_book.remote_path.clone())
            {
                return Err(SyncError::InvalidBackup {
                    field: "imported_books.remote_path".into(),
                });
            }
        }
        let skipped_paths =
            validate_unique_sync_string_list(&self.skipped_remote_paths, "skipped_remote_paths")?;
        for path in &skipped_paths {
            if !listed_paths.contains(path) || imported_paths.contains(path) {
                return Err(SyncError::InvalidBackup {
                    field: "skipped_remote_paths".into(),
                });
            }
        }
        validate_sync_string_list(&self.operation_log, "operation_log")?;
        validate_sync_string_list(&self.errors, "errors")?;
        if self.status
            != classify_webdav_remote_book_import_status(self.imported_books.len(), &self.errors)
        {
            return Err(SyncError::InvalidBackup {
                field: "status".into(),
            });
        }
        Ok(())
    }
}

/// Plan the legacy WebDAV remote-book import candidate set without transport.
///
/// The legacy runtime sorted and de-duplicated remote books by `remotePath`,
/// applied `selectedRemotePaths`, then applied `maximumBookCount` before
/// skipping unsupported formats. This keeps the same ordering and cap semantics
/// but returns a pure plan that host transports can execute later.
pub fn plan_webdav_remote_book_import(
    request: &WebDavRemoteBookImportRequest,
) -> Result<WebDavRemoteBookImportPlan, SyncError> {
    request.validate()?;

    let selected = request
        .selected_remote_paths
        .as_ref()
        .map(|paths| paths.iter().cloned().collect::<BTreeSet<_>>());
    let listed_remote_books = unique_sorted_remote_books(&request.remote_books);
    let capped_candidates = listed_remote_books
        .iter()
        .filter(|book| {
            selected
                .as_ref()
                .map(|paths| paths.contains(&book.remote_path))
                .unwrap_or(true)
        })
        .take(request.maximum_book_count)
        .cloned()
        .collect::<Vec<_>>();

    let mut importable_remote_books = Vec::new();
    let mut skipped_remote_paths = Vec::new();
    let mut operation_log = Vec::new();
    for book in capped_candidates {
        if is_supported_webdav_remote_book(&book) {
            importable_remote_books.push(book);
        } else {
            operation_log.push(format!("remote_book:skip_unsupported:{}", book.remote_path));
            skipped_remote_paths.push(book.remote_path);
        }
    }

    Ok(WebDavRemoteBookImportPlan {
        effective_preview_limit: request.effective_preview_limit(),
        import_mode: request.import_mode,
        listed_remote_books,
        importable_remote_books,
        skipped_remote_paths,
        operation_log,
    })
}

pub fn plan_webdav_remote_book_import_execution(
    request: &WebDavRemoteBookImportRequest,
) -> Result<WebDavRemoteBookImportExecutionPlan, SyncError> {
    let candidate_plan = plan_webdav_remote_book_import(request)?;
    let mut operations = Vec::new();
    if request.require_connection_test {
        operations.push(WebDavRemoteBookImportOperation {
            kind: WebDavRemoteBookImportOperationKind::ConnectionTest,
            path: None,
        });
    }
    if let Some(directory) = request
        .remote_book_directory_path
        .as_deref()
        .and_then(|path| (!path.trim().is_empty()).then(|| path.trim().to_string()))
    {
        operations.push(WebDavRemoteBookImportOperation {
            kind: WebDavRemoteBookImportOperationKind::ListDirectory,
            path: Some(directory),
        });
    }
    operations.extend(candidate_plan.importable_remote_books.iter().map(|book| {
        WebDavRemoteBookImportOperation {
            kind: WebDavRemoteBookImportOperationKind::DownloadFile,
            path: Some(book.remote_path.clone()),
        }
    }));

    let plan = WebDavRemoteBookImportExecutionPlan {
        candidate_plan,
        operations,
    };
    plan.validate()?;
    Ok(plan)
}

pub fn webdav_remote_book_import_adapter_operations(
    plan: &WebDavRemoteBookImportExecutionPlan,
) -> Result<Vec<String>, SyncError> {
    plan.validate()?;
    plan.operations
        .iter()
        .map(|operation| match operation.kind {
            WebDavRemoteBookImportOperationKind::ConnectionTest => Ok("connectionTest".into()),
            WebDavRemoteBookImportOperationKind::ListDirectory => Ok(format!(
                "listDirectory:{}",
                operation.path.as_deref().unwrap_or_default()
            )),
            WebDavRemoteBookImportOperationKind::DownloadFile => Ok(format!(
                "downloadFile:{}",
                operation.path.as_deref().unwrap_or_default()
            )),
        })
        .collect()
}

pub fn summarize_webdav_remote_book_import_result(
    request: &WebDavRemoteBookImportResultRequest,
) -> Result<WebDavRemoteBookImportResult, SyncError> {
    request.validate()?;

    if request.require_connection_test && !request.connection_ok {
        let result = WebDavRemoteBookImportResult {
            status: WebDavRemoteBookImportStatus::Failure,
            connection_ok: false,
            listed_remote_books: Vec::new(),
            imported_books: Vec::new(),
            skipped_remote_paths: Vec::new(),
            operation_log: vec!["connection_test:failed".into()],
            errors: vec!["connection_test_failed".into()],
        };
        result.validate()?;
        return Ok(result);
    }

    let mut imported_books = request.imported_books.clone();
    imported_books.sort_by(|left, right| {
        left.remote_book
            .remote_path
            .cmp(&right.remote_book.remote_path)
    });
    let errors = request
        .import_errors
        .iter()
        .map(redact_webdav_sync_error)
        .collect::<Vec<_>>();
    let status = classify_webdav_remote_book_import_status(imported_books.len(), &errors);

    let mut operation_log = Vec::new();
    if request.require_connection_test {
        operation_log.push("connection_test:success".into());
    }
    operation_log.push(format!(
        "remote_books:list:{}",
        request.plan.listed_remote_books.len()
    ));
    operation_log.extend(request.plan.operation_log.clone());
    for imported_book in &imported_books {
        operation_log.push(format!(
            "remote_book:download:{}:{}",
            imported_book.remote_book.remote_path, imported_book.downloaded_byte_count
        ));
        operation_log.push(format!(
            "remote_book:import:{}:{}",
            imported_book.remote_book.remote_path, imported_book.detected_format
        ));
    }
    operation_log.extend(
        errors
            .iter()
            .map(|error| format!("remote_book:import_failed:{error}")),
    );

    let mut skipped_remote_paths = request.plan.skipped_remote_paths.clone();
    skipped_remote_paths.sort();

    let result = WebDavRemoteBookImportResult {
        status,
        connection_ok: request.connection_ok,
        listed_remote_books: request.plan.listed_remote_books.clone(),
        imported_books,
        skipped_remote_paths,
        operation_log,
        errors,
    };
    result.validate()?;
    Ok(result)
}

pub fn plan_webdav_remote_book_local_import_input(
    book: &RemoteBookMetadata,
    downloaded_byte_count: u64,
) -> Result<WebDavRemoteBookLocalImportInput, SyncError> {
    validate_backup_required(&book.remote_path, "remote_path")?;
    validate_backup_optional_string(&book.author, "author")?;
    validate_backup_optional_string(&book.format, "format")?;
    validate_backup_optional_string(&book.etag, "etag")?;
    if !is_supported_webdav_remote_book(book) {
        return Err(SyncError::InvalidBackup {
            field: "remote_book.format".into(),
        });
    }

    let declared_extension = webdav_remote_book_extension(book);
    let declared_mime_type = declared_extension
        .as_deref()
        .map(media_type_for_webdav_remote_book_extension)
        .map(ToString::to_string);
    let declared_filename =
        webdav_remote_book_declared_filename(book, declared_extension.as_deref());

    Ok(WebDavRemoteBookLocalImportInput {
        remote_book: book.clone(),
        declared_filename,
        declared_extension,
        declared_mime_type,
        source_metadata: WebDavRemoteBookSourceMetadata {
            byte_count: downloaded_byte_count,
            modification_timestamp: book.remote_modified_at,
            resource_identifier_hint: book.etag.clone(),
            source_path_checksum: Some(stable_webdav_import_checksum(&[
                "webdav-remote-path",
                book.remote_path.as_str(),
            ])),
        },
    })
}

pub fn is_supported_webdav_remote_book(book: &RemoteBookMetadata) -> bool {
    webdav_remote_book_extension(book)
        .as_deref()
        .is_some_and(is_supported_webdav_remote_book_extension)
}

pub fn webdav_remote_book_media_type(book: &RemoteBookMetadata) -> &'static str {
    webdav_remote_book_extension(book)
        .as_deref()
        .map(media_type_for_webdav_remote_book_extension)
        .unwrap_or("application/octet-stream")
}

fn unique_sorted_remote_books(books: &[RemoteBookMetadata]) -> Vec<RemoteBookMetadata> {
    let mut by_path = BTreeMap::<String, RemoteBookMetadata>::new();
    for book in books {
        by_path
            .entry(book.remote_path.clone())
            .or_insert_with(|| book.clone());
    }
    by_path.into_values().collect()
}

fn webdav_remote_book_extension(book: &RemoteBookMetadata) -> Option<String> {
    book.format
        .as_deref()
        .and_then(|format| {
            let format = format.trim().trim_matches('.').to_ascii_lowercase();
            (!format.is_empty()).then_some(format)
        })
        .or_else(|| remote_path_extension(&book.remote_path))
}

fn remote_path_extension(path: &str) -> Option<String> {
    let filename = path.rsplit('/').next()?.trim();
    let extension = filename.rsplit_once('.')?.1.trim().to_ascii_lowercase();
    (!extension.is_empty()).then_some(extension)
}

fn webdav_remote_book_declared_filename(
    book: &RemoteBookMetadata,
    extension: Option<&str>,
) -> String {
    let path_name = book
        .remote_path
        .rsplit('/')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("remote-book");
    let mut filename = book
        .title
        .trim()
        .is_empty()
        .then(|| path_name.to_string())
        .unwrap_or_else(|| book.title.trim().to_string());
    if filename.trim().is_empty() {
        filename = "remote-book".into();
    }
    if !filename_has_extension(&filename) {
        if let Some(extension) = extension.filter(|value| !value.trim().is_empty()) {
            filename.push('.');
            filename.push_str(extension);
        }
    }
    filename
}

fn filename_has_extension(filename: &str) -> bool {
    filename
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.').map(|(_, extension)| extension))
        .is_some_and(|extension| !extension.trim().is_empty())
}

fn is_supported_webdav_remote_book_extension(extension: &str) -> bool {
    matches!(
        extension,
        "txt"
            | "epub"
            | "pdf"
            | "mobi"
            | "azw"
            | "azw3"
            | "kf8"
            | "umd"
            | "zip"
            | "cbz"
            | "archive"
            | "tar"
            | "webdav"
            | "webdavbook"
    )
}

fn media_type_for_webdav_remote_book_extension(extension: &str) -> &'static str {
    match extension {
        "txt" => "text/plain",
        "epub" => "application/epub+zip",
        "pdf" => "application/pdf",
        "mobi" => "application/x-mobipocket-ebook",
        "azw" | "azw3" | "kf8" => "application/vnd.amazon.ebook",
        "umd" => "application/x-umd",
        "zip" | "cbz" | "archive" => "application/zip",
        "tar" => "application/x-tar",
        "webdav" | "webdavbook" => "application/vnd.reader-core.webdav-local-book+json",
        _ => "application/octet-stream",
    }
}

fn stable_webdav_import_checksum(parts: &[&str]) -> String {
    let joined = parts.join("|");
    let mut hash = 14_695_981_039_346_656_037u64;
    for byte in joined.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    format!("fnv1a64:{hash:016x}")
}

/// Legacy WebDAV sync result status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WebDavSyncStatus {
    Success,
    PartialFailure,
    Failure,
}

/// Transport-neutral WebDAV sync operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WebDavSyncOperation {
    ConnectionTest,
    ListRemoteBooks,
    PushProgress,
    PullProgress,
    CreateBackup,
    ListBackups,
    RestoreBackup,
}

/// Main WebDAV sync request shape without transport adapters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavSyncRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_book_directory_path: Option<String>,
    #[serde(default)]
    pub local_progress_records: Vec<ProgressCloudSyncRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_pull_since: Option<i64>,
    #[serde(default)]
    pub should_pull_progress: bool,
    #[serde(default)]
    pub conflict_policy: ConflictPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_config: Option<BackupConfig>,
    #[serde(default)]
    pub backup_entries_to_create: Vec<BackupManifestEntry>,
    #[serde(default)]
    pub should_list_backups: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_package: Option<BackupPackage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_policy: Option<RestorePolicy>,
    #[serde(default = "default_require_connection_test")]
    pub require_connection_test: bool,
}

impl Default for WebDavSyncRequest {
    fn default() -> Self {
        Self {
            remote_book_directory_path: None,
            local_progress_records: Vec::new(),
            progress_pull_since: None,
            should_pull_progress: false,
            conflict_policy: ConflictPolicy::LastWriteWins,
            backup_config: None,
            backup_entries_to_create: Vec::new(),
            should_list_backups: false,
            restore_package: None,
            restore_policy: None,
            require_connection_test: default_require_connection_test(),
        }
    }
}

impl WebDavSyncRequest {
    pub fn validate(&self) -> Result<(), SyncError> {
        validate_backup_optional_string(
            &self.remote_book_directory_path,
            "remote_book_directory_path",
        )?;
        for record in &self.local_progress_records {
            record.validate()?;
        }
        if let Some(config) = &self.backup_config {
            config.validate()?;
        }
        for entry in &self.backup_entries_to_create {
            entry.validate()?;
        }
        if let Some(package) = &self.restore_package {
            package.validate()?;
        }
        if let Some(policy) = &self.restore_policy {
            policy.validate()?;
        }
        Ok(())
    }
}

/// Deterministic operation plan for the main WebDAV sync runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavSyncPlan {
    pub operations: Vec<WebDavSyncOperation>,
}

/// Transport-neutral execution plan for legacy WebDAV sync side effects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavSyncExecutionPlan {
    pub operations: Vec<WebDavSyncOperation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backup_entries_to_create: Vec<BackupManifestEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_plan: Option<BackupRestorePlan>,
}

/// Counts and IDs supplied by a deterministic WebDAV sync runtime fixture.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavSyncExecutionMetrics {
    #[serde(default)]
    pub remote_book_count: usize,
    #[serde(default)]
    pub pushed_progress_count: usize,
    #[serde(default)]
    pub pulled_progress_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_backup_id: Option<String>,
    #[serde(default)]
    pub backup_manifest_count: usize,
    #[serde(default)]
    pub restored_entry_count: usize,
}

/// One operation failure injected by a transport/runtime fixture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavSyncOperationFailure {
    pub operation: WebDavSyncOperation,
    pub error: String,
}

/// Transport-neutral execution trace for legacy WebDAV sync orchestration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavSyncExecutionTrace {
    pub status: WebDavSyncStatus,
    #[serde(rename = "connectionOK")]
    pub connection_ok: bool,
    #[serde(default)]
    pub attempted_operations: Vec<WebDavSyncOperation>,
    #[serde(default)]
    pub skipped_operations: Vec<WebDavSyncOperation>,
    #[serde(default)]
    pub operation_log: Vec<String>,
    #[serde(default)]
    pub errors: Vec<String>,
}

/// Transport-neutral result shape for the legacy WebDAV sync runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavSyncResult {
    pub status: WebDavSyncStatus,
    #[serde(rename = "connectionOK")]
    pub connection_ok: bool,
    #[serde(default)]
    pub remote_books: Vec<RemoteBookMetadata>,
    pub pushed_progress_count: usize,
    #[serde(default)]
    pub pulled_progress_records: Vec<ProgressCloudSyncRecord>,
    #[serde(default)]
    pub resolved_progress_records: Vec<ProgressCloudSyncRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_backup_package: Option<BackupPackage>,
    #[serde(default)]
    pub backup_manifests: Vec<BackupManifest>,
    #[serde(default)]
    pub restored_entries: Vec<BackupManifestEntry>,
    #[serde(default)]
    pub operation_log: Vec<String>,
    #[serde(default)]
    pub errors: Vec<String>,
}

impl WebDavSyncResult {
    pub fn validate(&self) -> Result<(), SyncError> {
        for book in &self.remote_books {
            book.validate()?;
        }
        if self.pushed_progress_count != self.resolved_progress_records.len() {
            return Err(SyncError::InvalidProgress {
                field: "pushed_progress_count".into(),
            });
        }
        for record in self
            .pulled_progress_records
            .iter()
            .chain(&self.resolved_progress_records)
        {
            record.validate()?;
        }
        if let Some(package) = &self.created_backup_package {
            package.validate()?;
        }
        for manifest in &self.backup_manifests {
            manifest.validate()?;
        }
        for entry in &self.restored_entries {
            entry.validate()?;
        }
        validate_webdav_sync_text_list(&self.operation_log, "operation_log")?;
        validate_webdav_sync_text_list(&self.errors, "errors")?;
        if self.status != classify_webdav_sync_status(&self.operation_log, &self.errors) {
            return Err(SyncError::InvalidBackup {
                field: "status".into(),
            });
        }
        Ok(())
    }
}

/// Deterministic backup restore plan before any host storage mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupRestorePlan {
    pub mode: RestoreMode,
    #[serde(
        rename = "selectedBookIDs",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub selected_book_ids: Option<Vec<String>>,
    pub overwrite_existing: bool,
    pub restore_entries: Vec<BackupManifestEntry>,
    pub result_entries: Vec<BackupManifestEntry>,
}

pub fn plan_webdav_sync_operations(
    request: &WebDavSyncRequest,
) -> Result<WebDavSyncPlan, SyncError> {
    request.validate()?;

    let mut operations = Vec::new();
    if request.require_connection_test {
        operations.push(WebDavSyncOperation::ConnectionTest);
    }
    if request
        .remote_book_directory_path
        .as_deref()
        .is_some_and(|path| !path.trim().is_empty())
    {
        operations.push(WebDavSyncOperation::ListRemoteBooks);
    }
    if !request.local_progress_records.is_empty() {
        operations.push(WebDavSyncOperation::PushProgress);
    }
    if request.should_pull_progress || request.progress_pull_since.is_some() {
        operations.push(WebDavSyncOperation::PullProgress);
    }
    if request.backup_config.is_some() && !request.backup_entries_to_create.is_empty() {
        operations.push(WebDavSyncOperation::CreateBackup);
    }
    if request.backup_config.is_some() && request.should_list_backups {
        operations.push(WebDavSyncOperation::ListBackups);
    }
    if request.restore_package.is_some() && request.restore_policy.is_some() {
        operations.push(WebDavSyncOperation::RestoreBackup);
    }

    Ok(WebDavSyncPlan { operations })
}

pub fn plan_webdav_sync_execution(
    request: &WebDavSyncRequest,
) -> Result<WebDavSyncExecutionPlan, SyncError> {
    let operations = plan_webdav_sync_operations(request)?.operations;
    let mut backup_entries_to_create =
        if request.backup_config.is_some() && !request.backup_entries_to_create.is_empty() {
            request.backup_entries_to_create.clone()
        } else {
            Vec::new()
        };
    sort_backup_entries_by_relative_path(&mut backup_entries_to_create);

    let restore_plan = match (&request.restore_package, &request.restore_policy) {
        (Some(package), Some(policy)) => Some(plan_backup_restore(package, policy)?),
        _ => None,
    };

    Ok(WebDavSyncExecutionPlan {
        operations,
        backup_entries_to_create,
        restore_plan,
    })
}

pub fn trace_webdav_sync_execution(
    plan: &WebDavSyncExecutionPlan,
    metrics: &WebDavSyncExecutionMetrics,
    failure: Option<&WebDavSyncOperationFailure>,
) -> Result<WebDavSyncExecutionTrace, SyncError> {
    validate_webdav_sync_execution_plan(plan)?;
    validate_webdav_sync_execution_metrics(plan, metrics)?;
    if let Some(failure) = failure {
        validate_webdav_sync_operation_failure(plan, failure)?;
    }

    let failure_operation = failure.map(|failure| failure.operation);
    let mut attempted_operations = Vec::new();
    let mut skipped_operations = Vec::new();
    let mut operation_log = Vec::new();
    let mut stopped = false;

    for operation in &plan.operations {
        if stopped {
            skipped_operations.push(*operation);
            continue;
        }
        attempted_operations.push(*operation);
        if Some(*operation) == failure_operation {
            operation_log.push(webdav_sync_failure_log(*operation).into());
            stopped = true;
        } else {
            operation_log.push(webdav_sync_success_log(*operation, metrics)?);
        }
    }

    let errors = failure
        .map(|failure| vec![redact_webdav_sync_error(&failure.error)])
        .unwrap_or_default();
    let status = classify_webdav_sync_status(&operation_log, &errors);
    let trace = WebDavSyncExecutionTrace {
        status,
        connection_ok: !matches!(failure_operation, Some(WebDavSyncOperation::ConnectionTest)),
        attempted_operations,
        skipped_operations,
        operation_log,
        errors,
    };
    validate_webdav_sync_execution_trace(&trace)?;
    Ok(trace)
}

#[allow(clippy::too_many_arguments)]
pub fn summarize_webdav_sync_result(
    connection_ok: bool,
    remote_books: &[RemoteBookMetadata],
    progress_push: Option<&WebDavProgressPushPlan>,
    pulled_progress_records: &[ProgressCloudSyncRecord],
    created_backup_package: Option<&BackupPackage>,
    backup_manifests: &[BackupManifest],
    restored_entries: &[BackupManifestEntry],
    operation_log: &[String],
    errors: &[String],
) -> Result<WebDavSyncResult, SyncError> {
    validate_webdav_sync_text_list(operation_log, "operation_log")?;
    validate_webdav_sync_text_list(errors, "errors")?;

    let mut remote_books = remote_books.to_vec();
    for book in &remote_books {
        book.validate()?;
    }
    remote_books.sort_by(|left, right| left.remote_path.cmp(&right.remote_path));

    let mut resolved_progress_records = match progress_push {
        Some(plan) => {
            validate_webdav_progress_push_plan(plan)?;
            plan.resolved_progress_records.clone()
        }
        None => Vec::new(),
    };
    sort_progress_cloud_records(&mut resolved_progress_records);
    let pushed_progress_count = resolved_progress_records.len();

    let mut pulled_progress_records = pulled_progress_records.to_vec();
    for record in &pulled_progress_records {
        record.validate()?;
    }
    sort_progress_cloud_records(&mut pulled_progress_records);

    let created_backup_package = created_backup_package.cloned();
    if let Some(package) = &created_backup_package {
        package.validate()?;
    }

    let mut backup_manifests = backup_manifests.to_vec();
    for manifest in &backup_manifests {
        manifest.validate()?;
    }
    backup_manifests.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.backup_id.cmp(&right.backup_id))
    });

    let mut restored_entries = restored_entries.to_vec();
    for entry in &restored_entries {
        entry.validate()?;
    }
    sort_backup_entries_by_relative_path(&mut restored_entries);

    let errors = errors
        .iter()
        .map(redact_webdav_sync_error)
        .collect::<Vec<_>>();
    let operation_log = operation_log.to_vec();
    let status = classify_webdav_sync_status(&operation_log, &errors);
    let result = WebDavSyncResult {
        status,
        connection_ok,
        remote_books,
        pushed_progress_count,
        pulled_progress_records,
        resolved_progress_records,
        created_backup_package,
        backup_manifests,
        restored_entries,
        operation_log,
        errors,
    };
    result.validate()?;
    Ok(result)
}

pub fn plan_backup_restore(
    package: &BackupPackage,
    policy: &RestorePolicy,
) -> Result<BackupRestorePlan, SyncError> {
    package.validate()?;
    policy.validate()?;

    let restore_entries = match policy.mode {
        RestoreMode::DryRun => package.manifest.entries.clone(),
        RestoreMode::Full | RestoreMode::Selective => match &policy.selected_book_ids {
            Some(selected_book_ids) => package
                .manifest
                .entries
                .iter()
                .filter(|entry| selected_book_ids.contains(&entry.relative_path))
                .cloned()
                .collect(),
            None => package.manifest.entries.clone(),
        },
    };
    let mut result_entries = restore_entries.clone();
    sort_backup_entries_by_relative_path(&mut result_entries);

    Ok(BackupRestorePlan {
        mode: policy.mode,
        selected_book_ids: policy.selected_book_ids.clone(),
        overwrite_existing: policy.overwrite_existing,
        restore_entries,
        result_entries,
    })
}

pub fn classify_webdav_sync_status(
    operation_log: &[String],
    errors: &[String],
) -> WebDavSyncStatus {
    if errors.is_empty() {
        return WebDavSyncStatus::Success;
    }
    if operation_log
        .iter()
        .any(|entry| is_successful_webdav_sync_log(entry))
    {
        WebDavSyncStatus::PartialFailure
    } else {
        WebDavSyncStatus::Failure
    }
}

fn validate_webdav_progress_push_plan(plan: &WebDavProgressPushPlan) -> Result<(), SyncError> {
    if plan.pushed_progress_count != plan.resolved_progress_records.len() {
        return Err(SyncError::InvalidProgress {
            field: "pushed_progress_count".into(),
        });
    }
    for record in &plan.resolved_progress_records {
        record.validate()?;
    }
    Ok(())
}

fn validate_webdav_sync_execution_plan(plan: &WebDavSyncExecutionPlan) -> Result<(), SyncError> {
    for entry in &plan.backup_entries_to_create {
        entry.validate()?;
    }
    if let Some(restore_plan) = &plan.restore_plan {
        for entry in restore_plan
            .restore_entries
            .iter()
            .chain(&restore_plan.result_entries)
        {
            entry.validate()?;
        }
        if restore_plan
            .selected_book_ids
            .as_ref()
            .is_some_and(|ids| ids.iter().any(|id| id.trim().is_empty()))
        {
            return Err(SyncError::InvalidRestore {
                field: "selected_book_ids".into(),
            });
        }
    }
    Ok(())
}

fn validate_webdav_sync_execution_metrics(
    _plan: &WebDavSyncExecutionPlan,
    metrics: &WebDavSyncExecutionMetrics,
) -> Result<(), SyncError> {
    if metrics
        .created_backup_id
        .as_deref()
        .is_some_and(|id| id.trim().is_empty())
    {
        return Err(SyncError::InvalidBackup {
            field: "created_backup_id".into(),
        });
    }
    Ok(())
}

fn validate_webdav_sync_operation_failure(
    plan: &WebDavSyncExecutionPlan,
    failure: &WebDavSyncOperationFailure,
) -> Result<(), SyncError> {
    if !plan.operations.contains(&failure.operation) {
        return Err(SyncError::InvalidBackup {
            field: "failure.operation".into(),
        });
    }
    if failure.error.trim().is_empty() {
        return Err(SyncError::InvalidBackup {
            field: "failure.error".into(),
        });
    }
    Ok(())
}

fn validate_webdav_sync_execution_trace(trace: &WebDavSyncExecutionTrace) -> Result<(), SyncError> {
    validate_webdav_sync_text_list(&trace.operation_log, "operation_log")?;
    validate_webdav_sync_text_list(&trace.errors, "errors")?;
    if trace.status != classify_webdav_sync_status(&trace.operation_log, &trace.errors) {
        return Err(SyncError::InvalidBackup {
            field: "status".into(),
        });
    }
    Ok(())
}

fn webdav_sync_success_log(
    operation: WebDavSyncOperation,
    metrics: &WebDavSyncExecutionMetrics,
) -> Result<String, SyncError> {
    Ok(match operation {
        WebDavSyncOperation::ConnectionTest => "connection_test:success".into(),
        WebDavSyncOperation::ListRemoteBooks => {
            format!("remote_books:list:{}", metrics.remote_book_count)
        }
        WebDavSyncOperation::PushProgress => {
            format!("progress:push:{}", metrics.pushed_progress_count)
        }
        WebDavSyncOperation::PullProgress => {
            format!("progress:pull:{}", metrics.pulled_progress_count)
        }
        WebDavSyncOperation::CreateBackup => format!(
            "backup:create:{}",
            metrics.created_backup_id.as_deref().ok_or_else(|| {
                SyncError::InvalidBackup {
                    field: "created_backup_id".into(),
                }
            })?
        ),
        WebDavSyncOperation::ListBackups => {
            format!("backup:list:{}", metrics.backup_manifest_count)
        }
        WebDavSyncOperation::RestoreBackup => {
            format!("backup:restore:{}", metrics.restored_entry_count)
        }
    })
}

fn webdav_sync_failure_log(operation: WebDavSyncOperation) -> &'static str {
    match operation {
        WebDavSyncOperation::ConnectionTest => "connection_test:failed",
        WebDavSyncOperation::ListRemoteBooks => "remote_books:list_failed",
        WebDavSyncOperation::PushProgress => "progress:push_failed",
        WebDavSyncOperation::PullProgress => "progress:pull_failed",
        WebDavSyncOperation::CreateBackup => "backup:create_failed",
        WebDavSyncOperation::ListBackups => "backup:list_failed",
        WebDavSyncOperation::RestoreBackup => "backup:restore_failed",
    }
}

fn validate_webdav_sync_text_list(values: &[String], field: &str) -> Result<(), SyncError> {
    if values.iter().any(|value| value.trim().is_empty()) {
        return Err(SyncError::InvalidBackup {
            field: field.into(),
        });
    }
    Ok(())
}

pub fn redact_webdav_sync_error(message: impl AsRef<str>) -> String {
    let message = redact_auth_scheme(message.as_ref(), "Basic");
    let message = redact_auth_scheme(&message, "Bearer");
    redact_sensitive_key_values(&message)
}

fn is_successful_webdav_sync_log(entry: &str) -> bool {
    entry.ends_with(":success")
        || entry.contains(":list:")
        || entry.contains(":push:")
        || entry.contains(":create:")
        || entry.contains(":restore:")
}

fn redact_auth_scheme(input: &str, scheme: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while index < bytes.len() {
        if starts_with_ignore_ascii_case(bytes, index, scheme.as_bytes())
            && bytes
                .get(index + scheme.len())
                .is_some_and(u8::is_ascii_whitespace)
        {
            output.push_str(scheme);
            output.push_str(" REDACTED");
            index += scheme.len();
            while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
                index += 1;
            }
            while bytes
                .get(index)
                .is_some_and(|byte| is_auth_token_byte(*byte))
            {
                index += 1;
            }
        } else {
            let ch = input[index..]
                .chars()
                .next()
                .expect("index is inside input");
            output.push(ch);
            index += ch.len_utf8();
        }
    }
    output
}

fn redact_sensitive_key_values(input: &str) -> String {
    const KEYS: [&str; 4] = ["password", "token", "cookie", "authorization"];

    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while index < bytes.len() {
        let matched = KEYS.iter().find_map(|key| {
            let key_bytes = key.as_bytes();
            let value_start = index + key_bytes.len() + 1;
            if starts_with_ignore_ascii_case(bytes, index, key_bytes)
                && bytes.get(index + key_bytes.len()) == Some(&b'=')
                && bytes
                    .get(value_start)
                    .is_some_and(|byte| !is_sensitive_value_terminator(*byte))
            {
                Some((*key, value_start))
            } else {
                None
            }
        });

        if let Some((key, mut value_end)) = matched {
            output.push_str(&input[index..index + key.len()]);
            output.push_str("=REDACTED");
            while bytes
                .get(value_end)
                .is_some_and(|byte| !is_sensitive_value_terminator(*byte))
            {
                value_end += 1;
            }
            index = value_end;
        } else {
            let ch = input[index..]
                .chars()
                .next()
                .expect("index is inside input");
            output.push(ch);
            index += ch.len_utf8();
        }
    }
    output
}

fn starts_with_ignore_ascii_case(bytes: &[u8], index: usize, needle: &[u8]) -> bool {
    bytes
        .get(index..index + needle.len())
        .is_some_and(|window| window.eq_ignore_ascii_case(needle))
}

fn is_auth_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=' | b'.' | b'_' | b':' | b'-')
}

fn is_sensitive_value_terminator(byte: u8) -> bool {
    byte.is_ascii_whitespace() || matches!(byte, b',' | b';')
}

/// Stable key used by legacy WebDAV progress sync.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgressCloudSyncRecordKey {
    #[serde(rename = "bookId")]
    pub book_id: String,
    pub chapter_index: u32,
    #[serde(rename = "deviceId")]
    pub device_id: String,
}

/// Reading progress row carried by cloud sync.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgressCloudSyncRecord {
    #[serde(rename = "bookId")]
    pub book_id: String,
    pub chapter_index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_title: Option<String>,
    pub progress_fraction: f64,
    pub updated_at: i64,
    #[serde(rename = "deviceId")]
    pub device_id: String,
    #[serde(default = "default_sync_version")]
    pub sync_version: u32,
}

/// Resolved progress payload that a WebDAV sync runtime would push.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavProgressPushPlan {
    #[serde(default)]
    pub resolved_progress_records: Vec<ProgressCloudSyncRecord>,
    pub pushed_progress_count: usize,
}

/// Transport-neutral steps for the legacy WebDAV progress push runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WebDavProgressPushExecutionStep {
    PullRemoteBeforePush,
    ResolveConflicts,
    PushResolvedRecords,
}

/// Core-owned model of the old WebDAV progress push sequence:
/// pull remote progress with `since = nil`, resolve conflicts, then push.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavProgressPushExecutionPlan {
    pub steps: Vec<WebDavProgressPushExecutionStep>,
    pub remote_pull_since: Option<i64>,
    pub remote_before_push_count: usize,
    pub conflict_policy: ConflictPolicy,
    pub push_plan: WebDavProgressPushPlan,
}

impl ProgressCloudSyncRecord {
    pub fn new(
        book_id: impl Into<String>,
        chapter_index: u32,
        progress_fraction: f64,
        updated_at: i64,
        device_id: impl Into<String>,
    ) -> Result<Self, SyncError> {
        let record = Self {
            book_id: normalize_progress_required(book_id.into(), "book_id")?,
            chapter_index,
            chapter_title: None,
            progress_fraction,
            updated_at,
            device_id: normalize_progress_required(device_id.into(), "device_id")?,
            sync_version: default_sync_version(),
        };
        record.validate()?;
        Ok(record)
    }

    pub fn key(&self) -> ProgressCloudSyncRecordKey {
        ProgressCloudSyncRecordKey {
            book_id: self.book_id.clone(),
            chapter_index: self.chapter_index,
            device_id: self.device_id.clone(),
        }
    }

    pub fn validate(&self) -> Result<(), SyncError> {
        validate_progress_required(&self.book_id, "book_id")?;
        validate_progress_required(&self.device_id, "device_id")?;
        validate_progress_optional_string(&self.chapter_title, "chapter_title")?;
        if !self.progress_fraction.is_finite() || !(0.0..=1.0).contains(&self.progress_fraction) {
            return Err(SyncError::InvalidProgress {
                field: "progress_fraction".into(),
            });
        }
        if self.sync_version == 0 {
            return Err(SyncError::InvalidProgress {
                field: "sync_version".into(),
            });
        }
        Ok(())
    }
}

fn default_sync_version() -> u32 {
    1
}

/// Conflict strategy wire values from Reader-Core progress sync config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProgressConflictStrategy {
    LastWriteWins,
    DevicePriority,
    Manual,
}

impl Default for ProgressConflictStrategy {
    fn default() -> Self {
        Self::LastWriteWins
    }
}

/// Progress sync scheduling and retention settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgressCloudSyncConfig {
    #[serde(default = "default_sync_interval_minutes")]
    pub sync_interval_minutes: u32,
    #[serde(default)]
    pub conflict_strategy: ProgressConflictStrategy,
    #[serde(default = "default_auto_sync_enabled")]
    pub auto_sync_enabled: bool,
    #[serde(default = "default_max_records_per_book")]
    pub max_records_per_book: usize,
}

impl Default for ProgressCloudSyncConfig {
    fn default() -> Self {
        Self {
            sync_interval_minutes: default_sync_interval_minutes(),
            conflict_strategy: ProgressConflictStrategy::LastWriteWins,
            auto_sync_enabled: default_auto_sync_enabled(),
            max_records_per_book: default_max_records_per_book(),
        }
    }
}

impl ProgressCloudSyncConfig {
    pub fn validate(&self) -> Result<(), SyncError> {
        if self.sync_interval_minutes == 0 {
            return Err(SyncError::InvalidProgress {
                field: "sync_interval_minutes".into(),
            });
        }
        if self.max_records_per_book == 0 {
            return Err(SyncError::InvalidProgress {
                field: "max_records_per_book".into(),
            });
        }
        Ok(())
    }
}

fn default_sync_interval_minutes() -> u32 {
    15
}

fn default_auto_sync_enabled() -> bool {
    true
}

fn default_max_records_per_book() -> usize {
    50
}

/// Filter progress records using the legacy `updatedAt > since` rule.
pub fn progress_cloud_records_since(
    records: &[ProgressCloudSyncRecord],
    since: Option<i64>,
) -> Result<Vec<ProgressCloudSyncRecord>, SyncError> {
    for record in records {
        record.validate()?;
    }
    let mut records = records
        .iter()
        .filter(|record| match since {
            Some(since) => record.updated_at > since,
            None => true,
        })
        .cloned()
        .collect::<Vec<_>>();
    sort_progress_cloud_records(&mut records);
    Ok(records)
}

/// Resolve the WebDAV progress push payload using the legacy SyncTransport
/// conflict policy, then sort it exactly as the runtime result does.
pub fn plan_webdav_progress_push(
    local: &[ProgressCloudSyncRecord],
    remote: &[ProgressCloudSyncRecord],
    policy: ConflictPolicy,
) -> Result<WebDavProgressPushPlan, SyncError> {
    for record in local.iter().chain(remote) {
        record.validate()?;
    }

    let mut resolved_progress_records = match policy {
        ConflictPolicy::LastWriteWins => {
            let mut records = local
                .iter()
                .chain(remote)
                .cloned()
                .collect::<Vec<ProgressCloudSyncRecord>>();
            records.sort_by(compare_progress_webdav_last_write_wins_order);
            let mut seen = BTreeSet::<ProgressCloudSyncRecordKey>::new();
            records
                .into_iter()
                .filter(|record| seen.insert(record.key()))
                .collect::<Vec<_>>()
        }
        ConflictPolicy::DevicePriority | ConflictPolicy::Manual | ConflictPolicy::KeepBoth => {
            local.iter().chain(remote).cloned().collect()
        }
    };
    sort_progress_cloud_records(&mut resolved_progress_records);

    Ok(WebDavProgressPushPlan {
        pushed_progress_count: resolved_progress_records.len(),
        resolved_progress_records,
    })
}

pub fn plan_webdav_progress_push_execution(
    local: &[ProgressCloudSyncRecord],
    remote_before_push: &[ProgressCloudSyncRecord],
    policy: ConflictPolicy,
) -> Result<WebDavProgressPushExecutionPlan, SyncError> {
    for record in local {
        record.validate()?;
    }

    if local.is_empty() {
        return Ok(WebDavProgressPushExecutionPlan {
            steps: Vec::new(),
            remote_pull_since: None,
            remote_before_push_count: 0,
            conflict_policy: policy,
            push_plan: WebDavProgressPushPlan {
                resolved_progress_records: Vec::new(),
                pushed_progress_count: 0,
            },
        });
    }

    let push_plan = plan_webdav_progress_push(local, remote_before_push, policy)?;
    Ok(WebDavProgressPushExecutionPlan {
        steps: vec![
            WebDavProgressPushExecutionStep::PullRemoteBeforePush,
            WebDavProgressPushExecutionStep::ResolveConflicts,
            WebDavProgressPushExecutionStep::PushResolvedRecords,
        ],
        remote_pull_since: None,
        remote_before_push_count: remote_before_push.len(),
        conflict_policy: policy,
        push_plan,
    })
}

/// Merge local and remote progress rows under the configured conflict strategy.
///
/// The stable identity is `(bookId, chapterIndex, deviceId)`, matching the
/// legacy WebDAV adapter. Last-write-wins uses `updatedAt`, then
/// `syncVersion`, then deterministic tie-breaks. Manual strategy refuses
/// divergent same-key records instead of silently picking a winner.
pub fn merge_progress_cloud_records(
    local: &[ProgressCloudSyncRecord],
    remote: &[ProgressCloudSyncRecord],
    config: &ProgressCloudSyncConfig,
) -> Result<Vec<ProgressCloudSyncRecord>, SyncError> {
    config.validate()?;
    let mut by_key = BTreeMap::<ProgressCloudSyncRecordKey, ProgressCloudSyncRecord>::new();
    for record in local.iter().chain(remote) {
        record.validate()?;
        let key = record.key();
        match by_key.get(&key) {
            Some(existing)
                if config.conflict_strategy == ProgressConflictStrategy::Manual
                    && !progress_records_equivalent(existing, record) =>
            {
                return Err(SyncError::UnresolvedConflict {
                    record_id: progress_record_key_string(&key),
                });
            }
            Some(existing)
                if choose_progress_record(existing, record) == RecordChoice::Existing => {}
            _ => {
                by_key.insert(key, record.clone());
            }
        }
    }

    let records =
        apply_progress_record_limit(by_key.into_values().collect(), config.max_records_per_book);
    Ok(records)
}

/// Restore mode wire values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RestoreMode {
    Full,
    Selective,
    DryRun,
}

impl Default for RestoreMode {
    fn default() -> Self {
        Self::Full
    }
}

/// Restore policy for applying a backup package.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestorePolicy {
    #[serde(default)]
    pub mode: RestoreMode,
    #[serde(
        rename = "selectedBookIDs",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub selected_book_ids: Option<Vec<String>>,
    #[serde(default)]
    pub overwrite_existing: bool,
}

impl RestorePolicy {
    pub fn validate(&self) -> Result<(), SyncError> {
        if self.mode == RestoreMode::Selective
            && match &self.selected_book_ids {
                Some(selected_book_ids) => selected_book_ids.is_empty(),
                None => true,
            }
        {
            return Err(SyncError::InvalidRestore {
                field: "selected_book_ids".into(),
            });
        }

        let mut ids = BTreeSet::<String>::new();
        for book_id in self.selected_book_ids.iter().flatten() {
            let normalized = book_id.trim();
            if normalized.is_empty() || !ids.insert(normalized.to_string()) {
                return Err(SyncError::InvalidRestore {
                    field: "selected_book_ids".into(),
                });
            }
        }
        Ok(())
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

/// Conflict policy carried by WebDAV/progress sync settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictPolicy {
    LastWriteWins,
    Manual,
    KeepBoth,
    DevicePriority,
}

impl Default for ConflictPolicy {
    fn default() -> Self {
        Self::LastWriteWins
    }
}

/// Merge policy for conflict-aware snapshot merges.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncMergePolicy {
    #[serde(default)]
    pub conflict_policy: ConflictPolicy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub device_priority: Vec<String>,
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
    InvalidBackup { field: String },
    InvalidProgress { field: String },
    InvalidRestore { field: String },
    InvalidJournal { field: String },
    UnresolvedConflict { record_id: String },
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
            SyncError::InvalidBackup { field } => {
                write!(f, "invalid backup field: {field}")
            }
            SyncError::InvalidProgress { field } => {
                write!(f, "invalid progress sync field: {field}")
            }
            SyncError::InvalidRestore { field } => {
                write!(f, "invalid restore field: {field}")
            }
            SyncError::InvalidJournal { field } => {
                write!(f, "invalid sync journal field: {field}")
            }
            SyncError::UnresolvedConflict { record_id } => {
                write!(f, "sync conflict requires manual resolution: {record_id}")
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
    merge_snapshots_with_policy(
        local,
        remote,
        merged_snapshot_id,
        merged_device_id,
        merged_created_at,
        &SyncMergePolicy::default(),
    )
}

/// Merge two snapshots with an explicit conflict policy.
///
/// `Manual` refuses divergent same-key records so callers can route the pair to
/// a user-visible resolver. `DevicePriority` chooses the configured higher
/// priority device when possible, then falls back to deterministic
/// last-write-wins. `KeepBoth` keeps the deterministic winner at the original
/// key and copies the losing live record under a conflict copy id.
pub fn merge_snapshots_with_policy(
    local: &SyncSnapshot,
    remote: &SyncSnapshot,
    merged_snapshot_id: impl Into<String>,
    merged_device_id: impl Into<String>,
    merged_created_at: i64,
    policy: &SyncMergePolicy,
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
                let (winner, extra_records) =
                    resolve_records_with_policy(&key, local, remote, policy)?;
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
                merged.extend(extra_records);
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
    merge_packages_with_policy(
        local,
        remote,
        merged_snapshot_id,
        merged_device_id,
        merged_created_at,
        &SyncMergePolicy::default(),
    )
}

/// Merge two wire packages with an explicit conflict policy.
pub fn merge_packages_with_policy(
    local: &SyncPackage,
    remote: &SyncPackage,
    merged_snapshot_id: impl Into<String>,
    merged_device_id: impl Into<String>,
    merged_created_at: i64,
    policy: &SyncMergePolicy,
) -> Result<SyncPackageMergeResult, SyncError> {
    local.validate()?;
    remote.validate()?;
    let result = merge_snapshots_with_policy(
        &local.snapshot,
        &remote.snapshot,
        merged_snapshot_id,
        merged_device_id,
        merged_created_at,
        policy,
    )?;
    Ok(SyncPackageMergeResult {
        package: SyncPackage::new(result.snapshot)?,
        conflicts: result.conflicts,
    })
}

fn apply_progress_record_limit(
    records: Vec<ProgressCloudSyncRecord>,
    max_records_per_book: usize,
) -> Vec<ProgressCloudSyncRecord> {
    let mut by_book = BTreeMap::<String, Vec<ProgressCloudSyncRecord>>::new();
    for record in records {
        by_book
            .entry(record.book_id.clone())
            .or_default()
            .push(record);
    }

    let mut kept = Vec::new();
    for records in by_book.values_mut() {
        records.sort_by(compare_progress_retention_order);
        records.truncate(max_records_per_book);
        kept.append(records);
    }
    sort_progress_cloud_records(&mut kept);
    kept
}

fn compare_progress_retention_order(
    left: &ProgressCloudSyncRecord,
    right: &ProgressCloudSyncRecord,
) -> std::cmp::Ordering {
    right
        .updated_at
        .cmp(&left.updated_at)
        .then_with(|| right.sync_version.cmp(&left.sync_version))
        .then_with(|| right.chapter_index.cmp(&left.chapter_index))
        .then_with(|| right.device_id.cmp(&left.device_id))
}

fn sort_progress_cloud_records(records: &mut [ProgressCloudSyncRecord]) {
    records.sort_by(|left, right| {
        left.book_id
            .cmp(&right.book_id)
            .then_with(|| left.chapter_index.cmp(&right.chapter_index))
            .then_with(|| left.device_id.cmp(&right.device_id))
            .then_with(|| left.updated_at.cmp(&right.updated_at))
            .then_with(|| left.sync_version.cmp(&right.sync_version))
    });
}

fn compare_progress_webdav_last_write_wins_order(
    left: &ProgressCloudSyncRecord,
    right: &ProgressCloudSyncRecord,
) -> std::cmp::Ordering {
    right
        .updated_at
        .cmp(&left.updated_at)
        .then_with(|| compare_progress_runtime_order(left, right))
}

fn compare_progress_runtime_order(
    left: &ProgressCloudSyncRecord,
    right: &ProgressCloudSyncRecord,
) -> std::cmp::Ordering {
    left.book_id
        .cmp(&right.book_id)
        .then_with(|| left.chapter_index.cmp(&right.chapter_index))
        .then_with(|| left.device_id.cmp(&right.device_id))
        .then_with(|| left.updated_at.cmp(&right.updated_at))
        .then_with(|| left.sync_version.cmp(&right.sync_version))
}

fn sort_backup_entries_by_relative_path(entries: &mut [BackupManifestEntry]) {
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
}

fn choose_progress_record(
    existing: &ProgressCloudSyncRecord,
    candidate: &ProgressCloudSyncRecord,
) -> RecordChoice {
    if candidate.updated_at != existing.updated_at {
        return if candidate.updated_at > existing.updated_at {
            RecordChoice::Candidate
        } else {
            RecordChoice::Existing
        };
    }

    if candidate.sync_version != existing.sync_version {
        return if candidate.sync_version > existing.sync_version {
            RecordChoice::Candidate
        } else {
            RecordChoice::Existing
        };
    }

    match candidate
        .progress_fraction
        .total_cmp(&existing.progress_fraction)
    {
        std::cmp::Ordering::Greater => return RecordChoice::Candidate,
        std::cmp::Ordering::Less => return RecordChoice::Existing,
        std::cmp::Ordering::Equal => {}
    }

    if candidate.chapter_title != existing.chapter_title {
        return if candidate.chapter_title > existing.chapter_title {
            RecordChoice::Candidate
        } else {
            RecordChoice::Existing
        };
    }

    RecordChoice::Existing
}

fn progress_records_equivalent(
    left: &ProgressCloudSyncRecord,
    right: &ProgressCloudSyncRecord,
) -> bool {
    left.updated_at == right.updated_at
        && left.sync_version == right.sync_version
        && left.progress_fraction == right.progress_fraction
        && left.chapter_title == right.chapter_title
}

fn progress_record_key_string(key: &ProgressCloudSyncRecordKey) -> String {
    format!("{}:{}:{}", key.book_id, key.chapter_index, key.device_id)
}

fn resolve_records_with_policy(
    key: &SyncRecordKey,
    local: &SyncRecord,
    remote: &SyncRecord,
    policy: &SyncMergePolicy,
) -> Result<(SyncRecord, Vec<SyncRecord>), SyncError> {
    let conflict = conflict_reason(local, remote).is_some();
    if conflict && policy.conflict_policy == ConflictPolicy::Manual {
        return Err(SyncError::UnresolvedConflict {
            record_id: key.record_id.clone(),
        });
    }

    let winner = match policy.conflict_policy {
        ConflictPolicy::DevicePriority if conflict => {
            choose_record_by_device_priority(local, remote, &policy.device_priority)
                .unwrap_or_else(|| choose_record(local, remote))
        }
        _ => choose_record(local, remote),
    };
    let winner = match winner {
        RecordChoice::Existing => local.clone(),
        RecordChoice::Candidate => remote.clone(),
    };

    let mut extra_records = Vec::new();
    if conflict && policy.conflict_policy == ConflictPolicy::KeepBoth {
        let loser = if winner == *local { remote } else { local };
        if !loser.deleted {
            extra_records.push(conflict_copy_record(key, loser)?);
        }
    }

    Ok((winner, extra_records))
}

fn choose_record_by_device_priority(
    existing: &SyncRecord,
    candidate: &SyncRecord,
    device_priority: &[String],
) -> Option<RecordChoice> {
    let existing_rank = device_priority_rank(&existing.device_id, device_priority)?;
    let candidate_rank = device_priority_rank(&candidate.device_id, device_priority)?;
    if existing_rank == candidate_rank {
        None
    } else if candidate_rank < existing_rank {
        Some(RecordChoice::Candidate)
    } else {
        Some(RecordChoice::Existing)
    }
}

fn device_priority_rank(device_id: &str, device_priority: &[String]) -> Option<usize> {
    device_priority
        .iter()
        .position(|candidate| candidate == device_id)
}

fn conflict_copy_record(
    key: &SyncRecordKey,
    losing_record: &SyncRecord,
) -> Result<SyncRecord, SyncError> {
    let copy_id = format!(
        "{}#conflict:{}:{}",
        key.record_id, losing_record.device_id, losing_record.revision
    );
    let copy = SyncRecord {
        collection: key.collection.clone(),
        record_id: copy_id,
        updated_at: losing_record.updated_at,
        device_id: losing_record.device_id.clone(),
        revision: losing_record.revision,
        payload: losing_record.payload.clone(),
        deleted: false,
    };
    copy.validate()?;
    Ok(copy)
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

fn normalize_backup_required(value: String, field: &str) -> Result<String, SyncError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(SyncError::InvalidBackup {
            field: field.into(),
        });
    }
    Ok(value)
}

fn validate_backup_required(value: &str, field: &str) -> Result<(), SyncError> {
    if value.trim().is_empty() {
        return Err(SyncError::InvalidBackup {
            field: field.into(),
        });
    }
    Ok(())
}

fn validate_backup_optional_string(value: &Option<String>, field: &str) -> Result<(), SyncError> {
    if value.as_ref().is_some_and(|value| value.trim().is_empty()) {
        return Err(SyncError::InvalidBackup {
            field: field.into(),
        });
    }
    Ok(())
}

fn validate_sync_string_list(values: &[String], field: &str) -> Result<(), SyncError> {
    if values.iter().any(|value| value.trim().is_empty()) {
        return Err(SyncError::InvalidBackup {
            field: field.into(),
        });
    }
    Ok(())
}

fn validate_unique_sync_string_list(
    values: &[String],
    field: &str,
) -> Result<BTreeSet<String>, SyncError> {
    validate_sync_string_list(values, field)?;
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value.clone()) {
            return Err(SyncError::InvalidBackup {
                field: field.into(),
            });
        }
    }
    Ok(seen)
}

fn validate_remote_book_path_set(
    books: &[RemoteBookMetadata],
    field: &str,
) -> Result<BTreeSet<String>, SyncError> {
    let mut seen = BTreeSet::new();
    for book in books {
        book.validate()?;
        if !seen.insert(book.remote_path.clone()) {
            return Err(SyncError::InvalidBackup {
                field: format!("{field}.remote_path"),
            });
        }
    }
    Ok(seen)
}

fn validate_backup_relative_path(relative_path: &str) -> Result<(), SyncError> {
    validate_backup_required(relative_path, "relative_path")?;
    let relative_path = relative_path.trim();
    if relative_path.starts_with('/')
        || relative_path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(SyncError::InvalidBackup {
            field: "relative_path".into(),
        });
    }
    Ok(())
}

fn normalize_progress_required(value: String, field: &str) -> Result<String, SyncError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(SyncError::InvalidProgress {
            field: field.into(),
        });
    }
    Ok(value)
}

fn validate_progress_required(value: &str, field: &str) -> Result<(), SyncError> {
    if value.trim().is_empty() {
        return Err(SyncError::InvalidProgress {
            field: field.into(),
        });
    }
    Ok(())
}

fn validate_progress_optional_string(value: &Option<String>, field: &str) -> Result<(), SyncError> {
    if value.as_ref().is_some_and(|value| value.trim().is_empty()) {
        return Err(SyncError::InvalidProgress {
            field: field.into(),
        });
    }
    Ok(())
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

    fn rec_from_device(
        collection: SyncCollection,
        id: &str,
        payload: &str,
        ts: i64,
        device_id: &str,
        revision: u64,
    ) -> SyncRecord {
        SyncRecord::upsert(collection, id, payload, ts, device_id, revision).unwrap()
    }

    fn snap(id: &str, records: Vec<SyncRecord>) -> SyncSnapshot {
        SyncSnapshot::new(id, "device-a", 1000, records).unwrap()
    }

    fn backup_entry(relative_path: &str, size_bytes: u64) -> BackupManifestEntry {
        BackupManifestEntry {
            relative_path: relative_path.into(),
            sha256: Some(format!("sha256-{size_bytes}")),
            size_bytes,
            modified_at: 1_700_000_000,
        }
    }

    fn backup_manifest(entries: Vec<BackupManifestEntry>) -> BackupManifest {
        let total_bytes = entries.iter().map(|entry| entry.size_bytes).sum();
        let book_count = entries.len() as u32;
        BackupManifest {
            backup_id: "bkp-001".into(),
            created_at: 1_700_000_100,
            entries,
            total_bytes,
            book_count,
        }
    }

    fn retention_item(backup_id: &str, created_at: i64) -> BackupRetentionItem {
        let mut manifest =
            backup_manifest(vec![backup_entry(&format!("books/{backup_id}.json"), 64)]);
        manifest.backup_id = backup_id.into();
        manifest.created_at = created_at;
        BackupRetentionItem {
            manifest,
            remote_path: format!("/dav/backups/{backup_id}.json"),
        }
    }

    fn progress_record(
        book_id: &str,
        chapter_index: u32,
        device_id: &str,
        updated_at: i64,
        sync_version: u32,
        progress_fraction: f64,
    ) -> ProgressCloudSyncRecord {
        ProgressCloudSyncRecord {
            book_id: book_id.into(),
            chapter_index,
            chapter_title: Some(format!("Chapter {chapter_index}")),
            progress_fraction,
            updated_at,
            device_id: device_id.into(),
            sync_version,
        }
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
    fn conflict_policy_wire_values_match_legacy_reader_core() {
        let cases = [
            (ConflictPolicy::LastWriteWins, "lastWriteWins"),
            (ConflictPolicy::Manual, "manual"),
            (ConflictPolicy::KeepBoth, "keepBoth"),
            (ConflictPolicy::DevicePriority, "devicePriority"),
        ];

        for (policy, expected) in cases {
            let json = serde_json::to_string(&policy).unwrap();
            assert_eq!(json, format!(r#""{expected}""#));
            let decoded: ConflictPolicy = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, policy);
        }

        let merge_policy = SyncMergePolicy {
            conflict_policy: ConflictPolicy::DevicePriority,
            device_priority: vec!["phone".into(), "tablet".into()],
        };
        let json = serde_json::to_value(&merge_policy).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "conflictPolicy": "devicePriority",
                "devicePriority": ["phone", "tablet"]
            })
        );
        assert_eq!(
            serde_json::from_value::<SyncMergePolicy>(json).unwrap(),
            merge_policy
        );
        assert!(serde_json::from_str::<ConflictPolicy>(r#""serverWins""#).is_err());
    }

    #[test]
    fn backup_config_defaults_and_wire_keys_match_legacy_reader_core() {
        let config = BackupConfig::new(" https://dav.example.com/backup ").unwrap();

        assert_eq!(config.target_url, "https://dav.example.com/backup");
        assert_eq!(config.max_backups, 5);
        assert!(!config.compression_enabled);
        assert!(!config.encryption_enabled);
        assert_eq!(
            serde_json::to_value(&config).unwrap(),
            serde_json::json!({
                "targetURL": "https://dav.example.com/backup",
                "compressionEnabled": false,
                "encryptionEnabled": false,
                "maxBackups": 5
            })
        );

        let full = BackupConfig {
            target_url: "https://dav.example.com".into(),
            auth_credential_id: Some("cred1".into()),
            compression_enabled: true,
            encryption_enabled: false,
            max_backups: 10,
            max_age_days: Some(30),
        };
        full.validate().unwrap();
        let json = serde_json::to_string(&full).unwrap();
        assert!(json.contains(r#""targetURL":"https://dav.example.com""#));
        assert!(json.contains(r#""authCredentialID":"cred1""#));
        let decoded: BackupConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, full);

        let mut invalid = full.clone();
        invalid.max_backups = 0;
        assert_eq!(
            invalid.validate().unwrap_err(),
            SyncError::InvalidBackup {
                field: "max_backups".into()
            }
        );
        assert_eq!(
            BackupConfig::new(" ").unwrap_err(),
            SyncError::InvalidBackup {
                field: "target_url".into()
            }
        );
        assert!(serde_json::from_str::<BackupConfig>(
            r#"{"targetURL":"x","compressionEnabled":false,"encryptionEnabled":false,"maxBackups":5,"bogus":true}"#
        )
        .is_err());
    }

    #[test]
    fn backup_schedule_frequency_wire_values_and_validation() {
        let cases = [
            (BackupFrequency::Manual, "manual"),
            (BackupFrequency::Hourly, "hourly"),
            (BackupFrequency::Daily, "daily"),
            (BackupFrequency::Weekly, "weekly"),
        ];

        for (frequency, expected) in cases {
            let json = serde_json::to_string(&frequency).unwrap();
            assert_eq!(json, format!(r#""{expected}""#));
            assert_eq!(
                serde_json::from_str::<BackupFrequency>(&json).unwrap(),
                frequency
            );
        }

        let schedule = BackupSchedule {
            frequency: BackupFrequency::Daily,
            preferred_hour: Some(3),
            preferred_weekday: Some(1),
        };
        schedule.validate().unwrap();
        assert_eq!(
            serde_json::to_value(&schedule).unwrap(),
            serde_json::json!({
                "frequency": "daily",
                "preferredHour": 3,
                "preferredWeekday": 1
            })
        );
        assert_eq!(BackupSchedule::default().frequency, BackupFrequency::Manual);

        let invalid_hour = BackupSchedule {
            frequency: BackupFrequency::Daily,
            preferred_hour: Some(24),
            preferred_weekday: None,
        };
        assert_eq!(
            invalid_hour.validate().unwrap_err(),
            SyncError::InvalidBackup {
                field: "preferred_hour".into()
            }
        );
        let invalid_weekday = BackupSchedule {
            frequency: BackupFrequency::Weekly,
            preferred_hour: None,
            preferred_weekday: Some(0),
        };
        assert_eq!(
            invalid_weekday.validate().unwrap_err(),
            SyncError::InvalidBackup {
                field: "preferred_weekday".into()
            }
        );
        assert!(serde_json::from_str::<BackupFrequency>(r#""monthly""#).is_err());
    }

    #[test]
    fn backup_manifest_package_validate_and_round_trip() {
        let manifest = backup_manifest(vec![
            backup_entry("books/a.json", 1024),
            backup_entry("books/b.json", 2048),
        ]);
        manifest.validate().unwrap();

        let mut package = BackupPackage::new(manifest.clone()).unwrap();
        assert_eq!(package.format, BackupArchiveFormat::Zip);
        package.format = BackupArchiveFormat::Tar;
        package.checksum = Some("sha256:def".into());
        package.validate().unwrap();
        assert_eq!(
            serde_json::to_value(&package).unwrap(),
            serde_json::json!({
                "manifest": {
                    "backupID": "bkp-001",
                    "createdAt": 1700000100,
                    "entries": [
                        {
                            "relativePath": "books/a.json",
                            "sha256": "sha256-1024",
                            "sizeBytes": 1024,
                            "modifiedAt": 1700000000
                        },
                        {
                            "relativePath": "books/b.json",
                            "sha256": "sha256-2048",
                            "sizeBytes": 2048,
                            "modifiedAt": 1700000000
                        }
                    ],
                    "totalBytes": 3072,
                    "bookCount": 2
                },
                "format": "tar",
                "checksum": "sha256:def"
            })
        );
        let json = package.to_json().unwrap();
        assert_eq!(BackupPackage::from_json(&json).unwrap(), package);

        let mut duplicate = manifest.clone();
        duplicate.entries.push(duplicate.entries[0].clone());
        duplicate.total_bytes += duplicate.entries[0].size_bytes;
        assert_eq!(
            duplicate.validate().unwrap_err(),
            SyncError::InvalidBackup {
                field: "entries.relative_path".into()
            }
        );

        let mut invalid_path = manifest.clone();
        invalid_path.entries[0].relative_path = "/books/a.json".into();
        assert_eq!(
            invalid_path.validate().unwrap_err(),
            SyncError::InvalidBackup {
                field: "relative_path".into()
            }
        );

        let mut invalid_total = manifest.clone();
        invalid_total.total_bytes = 1;
        assert_eq!(
            invalid_total.validate().unwrap_err(),
            SyncError::InvalidBackup {
                field: "total_bytes".into()
            }
        );

        let mut invalid_book_count = manifest;
        invalid_book_count.book_count = 3;
        assert_eq!(
            invalid_book_count.validate().unwrap_err(),
            SyncError::InvalidBackup {
                field: "book_count".into()
            }
        );
        assert!(serde_json::from_str::<BackupArchiveFormat>(r#""rar""#).is_err());
    }

    #[test]
    fn backup_retention_deletes_count_overflow_and_expired_packages() {
        let day = 24 * 60 * 60;
        let now = 100 * day;
        let config = BackupConfig {
            target_url: "backups".into(),
            auth_credential_id: None,
            compression_enabled: false,
            encryption_enabled: false,
            max_backups: 2,
            max_age_days: Some(30),
        };
        let candidates = vec![
            retention_item("count-overflow", 80 * day),
            retention_item("expired", 10 * day),
            retention_item("new-backup", now),
            retention_item("recent", 90 * day),
        ];

        let plan = plan_backup_retention(&config, "new-backup", now, &candidates).unwrap();

        assert_eq!(
            plan.paths_to_delete,
            vec![
                "/dav/backups/count-overflow.json",
                "/dav/backups/expired.json"
            ]
        );
        assert_eq!(
            serde_json::to_value(&plan).unwrap(),
            serde_json::json!({
                "pathsToDelete": [
                    "/dav/backups/count-overflow.json",
                    "/dav/backups/expired.json"
                ]
            })
        );
    }

    #[test]
    fn backup_retention_preserves_requested_backup_and_validates_candidates() {
        let day = 24 * 60 * 60;
        let config = BackupConfig {
            target_url: "backups".into(),
            auth_credential_id: None,
            compression_enabled: false,
            encryption_enabled: false,
            max_backups: 1,
            max_age_days: Some(1),
        };
        let candidates = vec![
            retention_item("preserved", 1),
            retention_item("newer", 10 * day),
            retention_item("expired", 2),
        ];

        let plan = plan_backup_retention(&config, "preserved", 10 * day, &candidates).unwrap();

        assert_eq!(plan.paths_to_delete, vec!["/dav/backups/expired.json"]);

        let mut invalid_candidate = retention_item("bad", 1);
        invalid_candidate.remote_path = " ".into();
        assert_eq!(
            plan_backup_retention(&config, "preserved", 10 * day, &[invalid_candidate])
                .unwrap_err(),
            SyncError::InvalidBackup {
                field: "remote_path".into()
            }
        );
        assert_eq!(
            plan_backup_retention(&config, " ", 10 * day, &[]).unwrap_err(),
            SyncError::InvalidBackup {
                field: "preserving_backup_id".into()
            }
        );
    }

    #[test]
    fn remote_book_metadata_round_trips_etag_and_validates_required_fields() {
        let metadata = RemoteBookMetadata {
            remote_path: "/books/test.epub".into(),
            title: "Test Book".into(),
            author: Some("Author".into()),
            format: Some("epub".into()),
            file_size: Some(4096),
            remote_modified_at: Some(1_700_000_000),
            etag: Some(r#""abc123""#.into()),
        };

        metadata.validate().unwrap();
        let json = serde_json::to_string(&metadata).unwrap();
        assert!(json.contains(r#""remotePath":"/books/test.epub""#));
        assert!(json.contains(r#""etag":"\"abc123\"""#));
        assert_eq!(
            serde_json::from_str::<RemoteBookMetadata>(&json).unwrap(),
            metadata
        );

        let mut missing_title = metadata.clone();
        missing_title.title = " ".into();
        assert_eq!(
            missing_title.validate().unwrap_err(),
            SyncError::InvalidBackup {
                field: "title".into()
            }
        );
        assert!(serde_json::from_str::<RemoteBookMetadata>(
            r#"{"remotePath":"/books/test.epub","title":"Test","fileSize":-1}"#
        )
        .is_err());
    }

    #[test]
    fn webdav_remote_book_import_mode_wire_values_match_legacy_local_book_runtime() {
        let cases = [
            (WebDavRemoteBookImportMode::MetadataOnly, "metadata_only"),
            (WebDavRemoteBookImportMode::IndexOnly, "index_only"),
            (WebDavRemoteBookImportMode::LazyContent, "lazy_content"),
            (
                WebDavRemoteBookImportMode::EagerFirstChapter,
                "eager_first_chapter",
            ),
            (
                WebDavRemoteBookImportMode::EagerAllContent,
                "eager_all_content",
            ),
            (
                WebDavRemoteBookImportMode::ValidateExisting,
                "validate_existing",
            ),
            (
                WebDavRemoteBookImportMode::ReimportChanged,
                "reimport_changed",
            ),
        ];

        for (mode, expected_wire_value) in cases {
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(json, format!(r#""{expected_wire_value}""#));
            assert_eq!(
                serde_json::from_str::<WebDavRemoteBookImportMode>(&json).unwrap(),
                mode
            );
        }
        assert!(serde_json::from_str::<WebDavRemoteBookImportMode>(r#""lazyContent""#).is_err());
    }

    #[test]
    fn webdav_remote_book_import_plan_matches_legacy_selection_rules() {
        let request = WebDavRemoteBookImportRequest {
            remote_book_directory_path: Some("/books".into()),
            remote_books: vec![
                RemoteBookMetadata {
                    remote_path: "/books/z.txt".into(),
                    title: "Z".into(),
                    format: Some("txt".into()),
                    author: None,
                    file_size: Some(48),
                    remote_modified_at: Some(2_000),
                    etag: Some("etag-z".into()),
                },
                RemoteBookMetadata {
                    remote_path: "/books/archive.tar".into(),
                    title: "Archive".into(),
                    format: Some("tar".into()),
                    author: None,
                    file_size: Some(3_072),
                    remote_modified_at: Some(1_000),
                    etag: Some("etag-tar".into()),
                },
                RemoteBookMetadata {
                    remote_path: "/books/ignore.bin".into(),
                    title: "Ignore".into(),
                    format: Some("bin".into()),
                    author: None,
                    file_size: None,
                    remote_modified_at: None,
                    etag: None,
                },
                RemoteBookMetadata {
                    remote_path: "/books/z.txt".into(),
                    title: "Duplicate Z".into(),
                    format: Some("txt".into()),
                    author: None,
                    file_size: None,
                    remote_modified_at: None,
                    etag: None,
                },
            ],
            selected_remote_paths: Some(vec![
                "/books/z.txt".into(),
                "/books/archive.tar".into(),
                "/books/ignore.bin".into(),
                "/books/missing.epub".into(),
            ]),
            import_mode: WebDavRemoteBookImportMode::EagerFirstChapter,
            maximum_book_count: 3,
            preview_limit: 256,
            require_connection_test: true,
        };

        let plan = plan_webdav_remote_book_import(&request).unwrap();

        assert_eq!(plan.effective_preview_limit, 64);
        assert_eq!(
            plan.import_mode,
            WebDavRemoteBookImportMode::EagerFirstChapter
        );
        assert_eq!(
            plan.listed_remote_books
                .iter()
                .map(|book| (book.remote_path.as_str(), book.title.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("/books/archive.tar", "Archive"),
                ("/books/ignore.bin", "Ignore"),
                ("/books/z.txt", "Z")
            ]
        );
        assert_eq!(
            plan.importable_remote_books
                .iter()
                .map(|book| book.remote_path.as_str())
                .collect::<Vec<_>>(),
            vec!["/books/archive.tar", "/books/z.txt"]
        );
        assert_eq!(plan.skipped_remote_paths, vec!["/books/ignore.bin"]);
        assert_eq!(
            plan.operation_log,
            vec!["remote_book:skip_unsupported:/books/ignore.bin"]
        );
    }

    #[test]
    fn webdav_remote_book_import_execution_plan_matches_legacy_adapter_operations() {
        let request = WebDavRemoteBookImportRequest {
            remote_book_directory_path: Some("/books".into()),
            remote_books: vec![
                RemoteBookMetadata {
                    remote_path: "/books/z.txt".into(),
                    title: "Z".into(),
                    format: Some("txt".into()),
                    author: None,
                    file_size: Some(48),
                    remote_modified_at: Some(2_000),
                    etag: Some("etag-z".into()),
                },
                RemoteBookMetadata {
                    remote_path: "/books/archive.tar".into(),
                    title: "Archive".into(),
                    format: Some("tar".into()),
                    author: None,
                    file_size: Some(3_072),
                    remote_modified_at: Some(1_000),
                    etag: Some("etag-tar".into()),
                },
                RemoteBookMetadata {
                    remote_path: "/books/ignore.bin".into(),
                    title: "Ignore".into(),
                    format: Some("bin".into()),
                    author: None,
                    file_size: None,
                    remote_modified_at: None,
                    etag: None,
                },
            ],
            selected_remote_paths: Some(vec![
                "/books/z.txt".into(),
                "/books/archive.tar".into(),
                "/books/ignore.bin".into(),
            ]),
            import_mode: WebDavRemoteBookImportMode::EagerFirstChapter,
            require_connection_test: true,
            ..WebDavRemoteBookImportRequest::default()
        };

        let plan = plan_webdav_remote_book_import_execution(&request).unwrap();

        assert_eq!(
            plan.candidate_plan
                .importable_remote_books
                .iter()
                .map(|book| book.remote_path.as_str())
                .collect::<Vec<_>>(),
            vec!["/books/archive.tar", "/books/z.txt"]
        );
        assert_eq!(
            webdav_remote_book_import_adapter_operations(&plan).unwrap(),
            vec![
                "connectionTest",
                "listDirectory:/books",
                "downloadFile:/books/archive.tar",
                "downloadFile:/books/z.txt"
            ]
        );
        let json = serde_json::to_value(&plan).unwrap();
        assert_eq!(json["operations"][0]["kind"], "connectionTest");
        assert_eq!(json["operations"][1]["kind"], "listDirectory");
        assert_eq!(json["operations"][1]["path"], "/books");
        assert_eq!(json["operations"][2]["kind"], "downloadFile");
        assert_eq!(json["operations"][2]["path"], "/books/archive.tar");
        assert_eq!(
            serde_json::from_value::<WebDavRemoteBookImportExecutionPlan>(json).unwrap(),
            plan
        );
    }

    #[test]
    fn webdav_remote_book_import_execution_plan_respects_no_connection_or_directory_mode() {
        let request = WebDavRemoteBookImportRequest {
            remote_books: vec![RemoteBookMetadata {
                remote_path: "/books/private.txt".into(),
                title: "Private".into(),
                format: Some("txt".into()),
                author: None,
                file_size: None,
                remote_modified_at: None,
                etag: None,
            }],
            require_connection_test: false,
            ..WebDavRemoteBookImportRequest::default()
        };

        let plan = plan_webdav_remote_book_import_execution(&request).unwrap();

        assert_eq!(
            webdav_remote_book_import_adapter_operations(&plan).unwrap(),
            vec!["downloadFile:/books/private.txt"]
        );
        let invalid_operation = WebDavRemoteBookImportExecutionPlan {
            candidate_plan: plan.candidate_plan,
            operations: vec![WebDavRemoteBookImportOperation {
                kind: WebDavRemoteBookImportOperationKind::ConnectionTest,
                path: Some("/books".into()),
            }],
        };
        assert_eq!(
            invalid_operation.validate().unwrap_err(),
            SyncError::InvalidBackup {
                field: "operation.path".into()
            }
        );
        assert!(
            serde_json::from_value::<WebDavRemoteBookImportExecutionPlan>(serde_json::json!({
                "candidatePlan": {
                    "effectivePreviewLimit": 64,
                    "importMode": "lazy_content",
                    "listedRemoteBooks": [],
                    "importableRemoteBooks": [],
                    "skippedRemotePaths": [],
                    "operationLog": []
                },
                "operations": [],
                "unexpected": true
            }))
            .is_err()
        );
    }

    #[test]
    fn webdav_remote_book_import_plan_applies_maximum_before_skip() {
        let request = WebDavRemoteBookImportRequest {
            remote_books: vec![
                RemoteBookMetadata {
                    remote_path: "/books/c.txt".into(),
                    title: "C".into(),
                    author: None,
                    format: None,
                    file_size: None,
                    remote_modified_at: None,
                    etag: None,
                },
                RemoteBookMetadata {
                    remote_path: "/books/a.bin".into(),
                    title: "A".into(),
                    author: None,
                    format: None,
                    file_size: None,
                    remote_modified_at: None,
                    etag: None,
                },
                RemoteBookMetadata {
                    remote_path: "/books/b.epub".into(),
                    title: "B".into(),
                    author: None,
                    format: None,
                    file_size: None,
                    remote_modified_at: None,
                    etag: None,
                },
            ],
            maximum_book_count: 2,
            ..WebDavRemoteBookImportRequest::default()
        };

        let plan = plan_webdav_remote_book_import(&request).unwrap();

        assert_eq!(plan.import_mode, WebDavRemoteBookImportMode::LazyContent);
        assert_eq!(
            plan.listed_remote_books
                .iter()
                .map(|book| book.remote_path.as_str())
                .collect::<Vec<_>>(),
            vec!["/books/a.bin", "/books/b.epub", "/books/c.txt"]
        );
        assert_eq!(
            plan.importable_remote_books
                .iter()
                .map(|book| book.remote_path.as_str())
                .collect::<Vec<_>>(),
            vec!["/books/b.epub"]
        );
        assert_eq!(plan.skipped_remote_paths, vec!["/books/a.bin"]);
    }

    #[test]
    fn webdav_remote_book_media_types_match_legacy_runtime() {
        let cases = [
            ("txt", "text/plain"),
            ("epub", "application/epub+zip"),
            ("pdf", "application/pdf"),
            ("mobi", "application/x-mobipocket-ebook"),
            ("azw", "application/vnd.amazon.ebook"),
            ("azw3", "application/vnd.amazon.ebook"),
            ("kf8", "application/vnd.amazon.ebook"),
            ("umd", "application/x-umd"),
            ("zip", "application/zip"),
            ("cbz", "application/zip"),
            ("archive", "application/zip"),
            ("tar", "application/x-tar"),
            (
                "webdavbook",
                "application/vnd.reader-core.webdav-local-book+json",
            ),
        ];

        for (extension, expected_media_type) in cases {
            let by_format = RemoteBookMetadata {
                remote_path: "/books/without-extension".into(),
                title: "Book".into(),
                author: None,
                format: Some(format!(".{}", extension.to_ascii_uppercase())),
                file_size: None,
                remote_modified_at: None,
                etag: None,
            };
            assert!(is_supported_webdav_remote_book(&by_format));
            assert_eq!(
                webdav_remote_book_media_type(&by_format),
                expected_media_type
            );

            let by_path = RemoteBookMetadata {
                remote_path: format!("/books/book.{extension}"),
                title: "Book".into(),
                author: None,
                format: None,
                file_size: None,
                remote_modified_at: None,
                etag: None,
            };
            assert!(is_supported_webdav_remote_book(&by_path));
            assert_eq!(webdav_remote_book_media_type(&by_path), expected_media_type);
        }

        let unsupported = RemoteBookMetadata {
            remote_path: "/books/file.bin".into(),
            title: "Unsupported".into(),
            author: None,
            format: None,
            file_size: None,
            remote_modified_at: None,
            etag: None,
        };
        assert!(!is_supported_webdav_remote_book(&unsupported));
        assert_eq!(
            webdav_remote_book_media_type(&unsupported),
            "application/octet-stream"
        );
    }

    #[test]
    fn webdav_remote_book_local_import_input_matches_legacy_metadata_plan() {
        let remote = RemoteBookMetadata {
            remote_path: "/books/z.txt".into(),
            title: "Remote Z".into(),
            author: Some("Author".into()),
            format: None,
            file_size: Some(48),
            remote_modified_at: Some(2_000),
            etag: Some("etag-z".into()),
        };

        let plan = plan_webdav_remote_book_local_import_input(&remote, 51).unwrap();

        assert_eq!(plan.remote_book, remote);
        assert_eq!(plan.declared_filename, "Remote Z.txt");
        assert_eq!(plan.declared_extension.as_deref(), Some("txt"));
        assert_eq!(plan.declared_mime_type.as_deref(), Some("text/plain"));
        assert_eq!(plan.source_metadata.byte_count, 51);
        assert_eq!(plan.source_metadata.modification_timestamp, Some(2_000));
        assert_eq!(
            plan.source_metadata.resource_identifier_hint.as_deref(),
            Some("etag-z")
        );
        assert_eq!(
            plan.source_metadata.source_path_checksum.as_deref(),
            Some("fnv1a64:08bcf0b86541943b")
        );
        assert_eq!(
            serde_json::to_value(&plan).unwrap()["declaredMIMEType"],
            "text/plain"
        );
    }

    #[test]
    fn webdav_remote_book_local_import_input_preserves_filename_extension_or_falls_back_to_path() {
        let title_with_extension = RemoteBookMetadata {
            remote_path: "/books/no-extension".into(),
            title: "Already.epub".into(),
            author: None,
            format: Some("epub".into()),
            file_size: None,
            remote_modified_at: None,
            etag: None,
        };
        let title_plan =
            plan_webdav_remote_book_local_import_input(&title_with_extension, 4096).unwrap();
        assert_eq!(title_plan.declared_filename, "Already.epub");
        assert_eq!(title_plan.declared_extension.as_deref(), Some("epub"));
        assert_eq!(
            title_plan.declared_mime_type.as_deref(),
            Some("application/epub+zip")
        );

        let blank_title = RemoteBookMetadata {
            remote_path: "/books/archive.tar".into(),
            title: "  ".into(),
            author: None,
            format: None,
            file_size: None,
            remote_modified_at: None,
            etag: None,
        };
        let path_plan = plan_webdav_remote_book_local_import_input(&blank_title, 3072).unwrap();
        assert_eq!(path_plan.declared_filename, "archive.tar");
        assert_eq!(path_plan.declared_extension.as_deref(), Some("tar"));
        assert_eq!(
            path_plan.declared_mime_type.as_deref(),
            Some("application/x-tar")
        );

        let unsupported = RemoteBookMetadata {
            remote_path: "/books/private.bin".into(),
            title: "Private".into(),
            author: None,
            format: None,
            file_size: None,
            remote_modified_at: None,
            etag: None,
        };
        assert_eq!(
            plan_webdav_remote_book_local_import_input(&unsupported, 1).unwrap_err(),
            SyncError::InvalidBackup {
                field: "remote_book.format".into()
            }
        );
    }

    #[test]
    fn webdav_remote_book_import_request_round_trips_and_validates_shape() {
        let request = WebDavRemoteBookImportRequest {
            remote_book_directory_path: Some("/books".into()),
            remote_books: vec![RemoteBookMetadata {
                remote_path: "/books/a.txt".into(),
                title: "A".into(),
                author: None,
                format: None,
                file_size: None,
                remote_modified_at: None,
                etag: None,
            }],
            selected_remote_paths: Some(vec!["/books/a.txt".into()]),
            import_mode: WebDavRemoteBookImportMode::MetadataOnly,
            maximum_book_count: 32,
            preview_limit: 64,
            require_connection_test: false,
        };
        request.validate().unwrap();
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "remoteBookDirectoryPath": "/books",
                "remoteBooks": [{
                    "remotePath": "/books/a.txt",
                    "title": "A"
                }],
                "selectedRemotePaths": ["/books/a.txt"],
                "importMode": "metadata_only",
                "maximumBookCount": 32,
                "previewLimit": 64,
                "requireConnectionTest": false
            })
        );
        assert_eq!(
            serde_json::from_value::<WebDavRemoteBookImportRequest>(json).unwrap(),
            request
        );

        let mut invalid = request;
        invalid.selected_remote_paths = Some(vec![" ".into()]);
        assert_eq!(
            invalid.validate().unwrap_err(),
            SyncError::InvalidBackup {
                field: "selected_remote_paths".into()
            }
        );
    }

    #[test]
    fn webdav_remote_book_import_status_wire_values_match_legacy_runtime() {
        let cases = [
            (WebDavRemoteBookImportStatus::Success, "success"),
            (
                WebDavRemoteBookImportStatus::PartialFailure,
                "partialFailure",
            ),
            (WebDavRemoteBookImportStatus::Failure, "failure"),
        ];

        for (status, expected_wire_value) in cases {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, format!(r#""{expected_wire_value}""#));
            assert_eq!(
                serde_json::from_str::<WebDavRemoteBookImportStatus>(&json).unwrap(),
                status
            );
        }
        assert!(
            serde_json::from_str::<WebDavRemoteBookImportStatus>(r#""partial_failure""#).is_err()
        );
    }

    #[test]
    fn webdav_remote_book_import_status_classification_matches_legacy_runtime() {
        assert_eq!(
            classify_webdav_remote_book_import_status(0, &[]),
            WebDavRemoteBookImportStatus::Success
        );
        assert_eq!(
            classify_webdav_remote_book_import_status(
                2,
                &["remote_book_import_error:/books/bad.epub:timeout".into()]
            ),
            WebDavRemoteBookImportStatus::PartialFailure
        );
        assert_eq!(
            classify_webdav_remote_book_import_status(0, &["connection_test_failed".into()]),
            WebDavRemoteBookImportStatus::Failure
        );

        let redacted = redact_webdav_sync_error(
            "remote_book_import_error:/books/private.txt:Bearer book-token password=secret",
        );
        assert!(redacted.contains("Bearer REDACTED"));
        assert!(redacted.contains("password=REDACTED"));
        assert!(!redacted.contains("book-token"));
        assert!(!redacted.contains("secret"));
    }

    #[test]
    fn webdav_remote_book_import_result_summarizes_download_and_commit_envelope() {
        let plan = plan_webdav_remote_book_import(&WebDavRemoteBookImportRequest {
            remote_book_directory_path: Some("/books".into()),
            remote_books: vec![
                RemoteBookMetadata {
                    remote_path: "/books/z.txt".into(),
                    title: "Z".into(),
                    author: None,
                    format: Some("txt".into()),
                    file_size: Some(48),
                    remote_modified_at: Some(2_000),
                    etag: Some("etag-z".into()),
                },
                RemoteBookMetadata {
                    remote_path: "/books/archive.tar".into(),
                    title: "Archive".into(),
                    author: None,
                    format: Some("tar".into()),
                    file_size: Some(3_072),
                    remote_modified_at: Some(1_000),
                    etag: Some("etag-tar".into()),
                },
                RemoteBookMetadata {
                    remote_path: "/books/ignore.bin".into(),
                    title: "Ignore".into(),
                    author: None,
                    format: Some("bin".into()),
                    file_size: None,
                    remote_modified_at: None,
                    etag: None,
                },
            ],
            selected_remote_paths: Some(vec![
                "/books/z.txt".into(),
                "/books/archive.tar".into(),
                "/books/ignore.bin".into(),
            ]),
            import_mode: WebDavRemoteBookImportMode::EagerFirstChapter,
            maximum_book_count: 3,
            preview_limit: 256,
            require_connection_test: true,
        })
        .unwrap();
        let archive = plan.importable_remote_books[0].clone();
        let z = plan.importable_remote_books[1].clone();

        let result =
            summarize_webdav_remote_book_import_result(&WebDavRemoteBookImportResultRequest {
                require_connection_test: true,
                connection_ok: true,
                plan,
                imported_books: vec![
                    WebDavRemoteBookImportedBook {
                        remote_book: z,
                        detected_format: "txt".into(),
                        chapter_content_count_materialized: 1,
                        downloaded_byte_count: 48,
                    },
                    WebDavRemoteBookImportedBook {
                        remote_book: archive,
                        detected_format: "archive".into(),
                        chapter_content_count_materialized: 1,
                        downloaded_byte_count: 3_072,
                    },
                ],
                import_errors: Vec::new(),
            })
            .unwrap();

        assert_eq!(result.status, WebDavRemoteBookImportStatus::Success);
        assert!(result.connection_ok);
        assert_eq!(
            result
                .listed_remote_books
                .iter()
                .map(|book| book.remote_path.as_str())
                .collect::<Vec<_>>(),
            vec!["/books/archive.tar", "/books/ignore.bin", "/books/z.txt"]
        );
        assert_eq!(
            result
                .imported_books
                .iter()
                .map(|book| {
                    (
                        book.remote_book.remote_path.as_str(),
                        book.detected_format.as_str(),
                        book.chapter_content_count_materialized,
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                ("/books/archive.tar", "archive", 1),
                ("/books/z.txt", "txt", 1)
            ]
        );
        assert_eq!(result.skipped_remote_paths, vec!["/books/ignore.bin"]);
        assert_eq!(
            result.operation_log,
            vec![
                "connection_test:success",
                "remote_books:list:3",
                "remote_book:skip_unsupported:/books/ignore.bin",
                "remote_book:download:/books/archive.tar:3072",
                "remote_book:import:/books/archive.tar:archive",
                "remote_book:download:/books/z.txt:48",
                "remote_book:import:/books/z.txt:txt",
            ]
        );
        assert!(result.errors.is_empty());
        assert_eq!(
            serde_json::to_value(&result).unwrap()["connectionOK"],
            serde_json::json!(true)
        );
        assert_eq!(
            serde_json::to_value(&result).unwrap()["status"],
            serde_json::json!("success")
        );
        result.validate().unwrap();
    }

    #[test]
    fn webdav_remote_book_import_result_redacts_errors_and_fails_without_imports() {
        let plan = plan_webdav_remote_book_import(&WebDavRemoteBookImportRequest {
            remote_books: vec![RemoteBookMetadata {
                remote_path: "/books/private.txt".into(),
                title: "Private".into(),
                author: None,
                format: Some("txt".into()),
                file_size: Some(128),
                remote_modified_at: Some(3_000),
                etag: Some("private-etag".into()),
            }],
            maximum_book_count: 1,
            require_connection_test: false,
            ..WebDavRemoteBookImportRequest::default()
        })
        .unwrap();

        let result = summarize_webdav_remote_book_import_result(
            &WebDavRemoteBookImportResultRequest {
                require_connection_test: false,
                connection_ok: true,
                plan,
                imported_books: Vec::new(),
                import_errors: vec![
                    "remote_book_download_error:/books/private.txt:Authorization=Basic abc123 password=hunter2 token=abc cookie=session-secret"
                        .into(),
                ],
            },
        )
        .unwrap();

        assert_eq!(result.status, WebDavRemoteBookImportStatus::Failure);
        assert!(result.connection_ok);
        assert!(result.imported_books.is_empty());
        assert_eq!(result.listed_remote_books.len(), 1);
        assert_eq!(result.operation_log[0], "remote_books:list:1");
        assert!(result.operation_log[1].starts_with("remote_book:import_failed:"));
        assert!(result.errors[0].contains("Authorization=REDACTED"));
        assert!(result.errors[0].contains("password=REDACTED"));
        assert!(result.errors[0].contains("token=REDACTED"));
        assert!(result.errors[0].contains("cookie=REDACTED"));
        assert!(!result.errors[0].contains("abc123"));
        assert!(!result.errors[0].contains("hunter2"));
        assert!(!result.errors[0].contains("session-secret"));
    }

    #[test]
    fn webdav_remote_book_import_result_stops_on_connection_failure() {
        let plan = plan_webdav_remote_book_import(&WebDavRemoteBookImportRequest {
            remote_books: vec![RemoteBookMetadata {
                remote_path: "/books/a.txt".into(),
                title: "A".into(),
                author: None,
                format: Some("txt".into()),
                file_size: None,
                remote_modified_at: None,
                etag: None,
            }],
            ..WebDavRemoteBookImportRequest::default()
        })
        .unwrap();

        let result =
            summarize_webdav_remote_book_import_result(&WebDavRemoteBookImportResultRequest {
                require_connection_test: true,
                connection_ok: false,
                plan,
                imported_books: Vec::new(),
                import_errors: Vec::new(),
            })
            .unwrap();

        assert_eq!(result.status, WebDavRemoteBookImportStatus::Failure);
        assert!(!result.connection_ok);
        assert!(result.listed_remote_books.is_empty());
        assert!(result.imported_books.is_empty());
        assert!(result.skipped_remote_paths.is_empty());
        assert_eq!(result.operation_log, vec!["connection_test:failed"]);
        assert_eq!(result.errors, vec!["connection_test_failed"]);
    }

    #[test]
    fn webdav_remote_book_import_plan_rejects_drifted_membership() {
        let plan = plan_webdav_remote_book_import(&WebDavRemoteBookImportRequest {
            remote_books: vec![
                RemoteBookMetadata {
                    remote_path: "/books/a.txt".into(),
                    title: "A".into(),
                    author: None,
                    format: Some("txt".into()),
                    file_size: None,
                    remote_modified_at: None,
                    etag: None,
                },
                RemoteBookMetadata {
                    remote_path: "/books/skip.bin".into(),
                    title: "Skip".into(),
                    author: None,
                    format: Some("bin".into()),
                    file_size: None,
                    remote_modified_at: None,
                    etag: None,
                },
            ],
            maximum_book_count: 2,
            ..WebDavRemoteBookImportRequest::default()
        })
        .unwrap();
        plan.validate().unwrap();

        let mut unlisted_importable = plan.clone();
        unlisted_importable
            .importable_remote_books
            .push(RemoteBookMetadata {
                remote_path: "/books/ghost.txt".into(),
                title: "Ghost".into(),
                author: None,
                format: Some("txt".into()),
                file_size: None,
                remote_modified_at: None,
                etag: None,
            });
        assert_eq!(
            unlisted_importable.validate().unwrap_err(),
            SyncError::InvalidBackup {
                field: "importable_remote_books.remote_path".into()
            }
        );

        let mut duplicate_skipped = plan;
        duplicate_skipped
            .skipped_remote_paths
            .push("/books/skip.bin".into());
        assert_eq!(
            duplicate_skipped.validate().unwrap_err(),
            SyncError::InvalidBackup {
                field: "skipped_remote_paths".into()
            }
        );
    }

    #[test]
    fn webdav_remote_book_import_result_rejects_unplanned_or_duplicate_imports() {
        let plan = plan_webdav_remote_book_import(&WebDavRemoteBookImportRequest {
            remote_books: vec![RemoteBookMetadata {
                remote_path: "/books/a.txt".into(),
                title: "A".into(),
                author: None,
                format: Some("txt".into()),
                file_size: Some(16),
                remote_modified_at: None,
                etag: None,
            }],
            maximum_book_count: 1,
            ..WebDavRemoteBookImportRequest::default()
        })
        .unwrap();
        let imported = WebDavRemoteBookImportedBook {
            remote_book: plan.importable_remote_books[0].clone(),
            detected_format: "txt".into(),
            chapter_content_count_materialized: 1,
            downloaded_byte_count: 16,
        };
        let unplanned = WebDavRemoteBookImportedBook {
            remote_book: RemoteBookMetadata {
                remote_path: "/books/unplanned.txt".into(),
                title: "Unplanned".into(),
                author: None,
                format: Some("txt".into()),
                file_size: Some(16),
                remote_modified_at: None,
                etag: None,
            },
            detected_format: "txt".into(),
            chapter_content_count_materialized: 1,
            downloaded_byte_count: 16,
        };

        assert_eq!(
            summarize_webdav_remote_book_import_result(&WebDavRemoteBookImportResultRequest {
                require_connection_test: true,
                connection_ok: true,
                plan: plan.clone(),
                imported_books: vec![unplanned],
                import_errors: Vec::new(),
            })
            .unwrap_err(),
            SyncError::InvalidBackup {
                field: "imported_books.remote_path".into()
            }
        );
        assert_eq!(
            summarize_webdav_remote_book_import_result(&WebDavRemoteBookImportResultRequest {
                require_connection_test: true,
                connection_ok: true,
                plan,
                imported_books: vec![imported.clone(), imported],
                import_errors: Vec::new(),
            })
            .unwrap_err(),
            SyncError::InvalidBackup {
                field: "imported_books.remote_path".into()
            }
        );
    }

    #[test]
    fn webdav_remote_book_import_result_validate_rejects_drifted_envelopes() {
        let plan = plan_webdav_remote_book_import(&WebDavRemoteBookImportRequest {
            remote_books: vec![RemoteBookMetadata {
                remote_path: "/books/a.txt".into(),
                title: "A".into(),
                author: None,
                format: Some("txt".into()),
                file_size: Some(16),
                remote_modified_at: None,
                etag: None,
            }],
            maximum_book_count: 1,
            ..WebDavRemoteBookImportRequest::default()
        })
        .unwrap();
        let imported = WebDavRemoteBookImportedBook {
            remote_book: plan.importable_remote_books[0].clone(),
            detected_format: "txt".into(),
            chapter_content_count_materialized: 1,
            downloaded_byte_count: 16,
        };
        let result =
            summarize_webdav_remote_book_import_result(&WebDavRemoteBookImportResultRequest {
                require_connection_test: true,
                connection_ok: true,
                plan,
                imported_books: vec![imported],
                import_errors: Vec::new(),
            })
            .unwrap();
        result.validate().unwrap();

        let mut drifted_status = result.clone();
        drifted_status.status = WebDavRemoteBookImportStatus::Failure;
        assert_eq!(
            drifted_status.validate().unwrap_err(),
            SyncError::InvalidBackup {
                field: "status".into()
            }
        );

        let mut unlisted_import = result.clone();
        unlisted_import.imported_books[0].remote_book.remote_path = "/books/ghost.txt".into();
        assert_eq!(
            unlisted_import.validate().unwrap_err(),
            SyncError::InvalidBackup {
                field: "imported_books.remote_path".into()
            }
        );

        let mut skipped_imported = result;
        skipped_imported
            .skipped_remote_paths
            .push("/books/a.txt".into());
        assert_eq!(
            skipped_imported.validate().unwrap_err(),
            SyncError::InvalidBackup {
                field: "skipped_remote_paths".into()
            }
        );
    }

    #[test]
    fn webdav_sync_operation_plan_matches_legacy_runtime_order() {
        let backup_entries = vec![
            backup_entry("books/z.epub", 20),
            backup_entry("books/a.txt", 10),
        ];
        let request = WebDavSyncRequest {
            remote_book_directory_path: Some("/books".into()),
            local_progress_records: vec![
                progress_record("book-a", 1, "phone", 2_000, 2, 0.8),
                progress_record("book-b", 2, "tablet", 1_500, 1, 0.4),
            ],
            progress_pull_since: Some(1_200),
            should_pull_progress: false,
            conflict_policy: ConflictPolicy::LastWriteWins,
            backup_config: Some(BackupConfig::new("/backups").unwrap()),
            backup_entries_to_create: backup_entries.clone(),
            should_list_backups: true,
            restore_package: Some(BackupPackage {
                manifest: backup_manifest(backup_entries),
                format: BackupArchiveFormat::Directory,
                checksum: Some("restore-checksum".into()),
            }),
            restore_policy: Some(RestorePolicy {
                mode: RestoreMode::DryRun,
                selected_book_ids: None,
                overwrite_existing: false,
            }),
            require_connection_test: true,
        };

        let plan = plan_webdav_sync_operations(&request).unwrap();

        assert_eq!(
            plan.operations,
            vec![
                WebDavSyncOperation::ConnectionTest,
                WebDavSyncOperation::ListRemoteBooks,
                WebDavSyncOperation::PushProgress,
                WebDavSyncOperation::PullProgress,
                WebDavSyncOperation::CreateBackup,
                WebDavSyncOperation::ListBackups,
                WebDavSyncOperation::RestoreBackup,
            ]
        );
        assert_eq!(
            serde_json::to_value(&plan).unwrap(),
            serde_json::json!({
                "operations": [
                    "connectionTest",
                    "listRemoteBooks",
                    "pushProgress",
                    "pullProgress",
                    "createBackup",
                    "listBackups",
                    "restoreBackup"
                ]
            })
        );
    }

    #[test]
    fn webdav_sync_execution_plan_preserves_legacy_backup_and_restore_ordering() {
        let backup_entries = vec![
            backup_entry("books/z.epub", 20),
            backup_entry("books/a.txt", 10),
        ];
        let restore_entries = vec![
            backup_entry("books/z.epub", 20),
            backup_entry("books/skip.pdf", 30),
            backup_entry("books/a.txt", 10),
        ];
        let request = WebDavSyncRequest {
            backup_config: Some(BackupConfig::new("/backups").unwrap()),
            backup_entries_to_create: backup_entries,
            restore_package: Some(BackupPackage {
                manifest: backup_manifest(restore_entries),
                format: BackupArchiveFormat::Directory,
                checksum: Some("restore-checksum".into()),
            }),
            restore_policy: Some(RestorePolicy {
                mode: RestoreMode::Full,
                selected_book_ids: Some(vec!["books/a.txt".into(), "books/z.epub".into()]),
                overwrite_existing: true,
            }),
            require_connection_test: false,
            ..WebDavSyncRequest::default()
        };

        let plan = plan_webdav_sync_execution(&request).unwrap();

        assert_eq!(
            plan.operations,
            vec![
                WebDavSyncOperation::CreateBackup,
                WebDavSyncOperation::RestoreBackup
            ]
        );
        assert_eq!(
            plan.backup_entries_to_create
                .iter()
                .map(|entry| entry.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["books/a.txt", "books/z.epub"]
        );
        let restore_plan = plan.restore_plan.as_ref().unwrap();
        assert_eq!(restore_plan.mode, RestoreMode::Full);
        assert!(restore_plan.overwrite_existing);
        assert_eq!(
            restore_plan
                .restore_entries
                .iter()
                .map(|entry| entry.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["books/z.epub", "books/a.txt"]
        );
        assert_eq!(
            restore_plan
                .result_entries
                .iter()
                .map(|entry| entry.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["books/a.txt", "books/z.epub"]
        );

        let json = serde_json::to_value(&plan).unwrap();
        assert_eq!(
            json["operations"],
            serde_json::json!(["createBackup", "restoreBackup"])
        );
        assert_eq!(
            json["backupEntriesToCreate"][0]["relativePath"],
            "books/a.txt"
        );
        assert_eq!(
            json["restorePlan"]["selectedBookIDs"],
            serde_json::json!(["books/a.txt", "books/z.epub"])
        );
        assert_eq!(
            json["restorePlan"]["restoreEntries"][0]["relativePath"],
            "books/z.epub"
        );
        assert_eq!(
            json["restorePlan"]["resultEntries"][0]["relativePath"],
            "books/a.txt"
        );
        assert_eq!(
            serde_json::from_value::<WebDavSyncExecutionPlan>(json).unwrap(),
            plan
        );
    }

    #[test]
    fn webdav_sync_execution_trace_matches_legacy_success_operation_log() {
        let backup_entries = vec![
            backup_entry("books/z.epub", 20),
            backup_entry("books/a.txt", 10),
        ];
        let request = WebDavSyncRequest {
            remote_book_directory_path: Some("/books".into()),
            local_progress_records: vec![
                progress_record("book-a", 1, "phone", 2_000, 2, 0.8),
                progress_record("book-b", 2, "tablet", 1_500, 1, 0.4),
            ],
            progress_pull_since: Some(1_200),
            backup_config: Some(BackupConfig::new("/backups").unwrap()),
            backup_entries_to_create: backup_entries.clone(),
            should_list_backups: true,
            restore_package: Some(BackupPackage {
                manifest: backup_manifest(backup_entries),
                format: BackupArchiveFormat::Directory,
                checksum: Some("restore-checksum".into()),
            }),
            restore_policy: Some(RestorePolicy {
                mode: RestoreMode::DryRun,
                selected_book_ids: None,
                overwrite_existing: false,
            }),
            require_connection_test: true,
            ..WebDavSyncRequest::default()
        };
        let plan = plan_webdav_sync_execution(&request).unwrap();
        let trace = trace_webdav_sync_execution(
            &plan,
            &WebDavSyncExecutionMetrics {
                remote_book_count: 2,
                pushed_progress_count: 2,
                pulled_progress_count: 2,
                created_backup_id: Some("backup-created-1".into()),
                backup_manifest_count: 1,
                restored_entry_count: 2,
            },
            None,
        )
        .unwrap();

        assert_eq!(trace.status, WebDavSyncStatus::Success);
        assert!(trace.connection_ok);
        assert_eq!(trace.attempted_operations, plan.operations);
        assert!(trace.skipped_operations.is_empty());
        assert_eq!(
            trace.operation_log,
            vec![
                "connection_test:success",
                "remote_books:list:2",
                "progress:push:2",
                "progress:pull:2",
                "backup:create:backup-created-1",
                "backup:list:1",
                "backup:restore:2",
            ]
        );
        assert_eq!(
            serde_json::to_value(&trace).unwrap()["connectionOK"],
            serde_json::json!(true)
        );
    }

    #[test]
    fn webdav_sync_execution_trace_stops_on_connection_failure() {
        let request = WebDavSyncRequest {
            remote_book_directory_path: Some("/books".into()),
            local_progress_records: vec![progress_record("book", 1, "phone", 1, 1, 0.1)],
            backup_config: Some(BackupConfig::new("/backups").unwrap()),
            backup_entries_to_create: vec![backup_entry("book", 1)],
            require_connection_test: true,
            ..WebDavSyncRequest::default()
        };
        let plan = plan_webdav_sync_execution(&request).unwrap();
        let trace = trace_webdav_sync_execution(
            &plan,
            &WebDavSyncExecutionMetrics::default(),
            Some(&WebDavSyncOperationFailure {
                operation: WebDavSyncOperation::ConnectionTest,
                error: "connection_test_failed".into(),
            }),
        )
        .unwrap();

        assert_eq!(trace.status, WebDavSyncStatus::Failure);
        assert!(!trace.connection_ok);
        assert_eq!(
            trace.attempted_operations,
            vec![WebDavSyncOperation::ConnectionTest]
        );
        assert_eq!(
            trace.skipped_operations,
            vec![
                WebDavSyncOperation::ListRemoteBooks,
                WebDavSyncOperation::PushProgress,
                WebDavSyncOperation::CreateBackup,
            ]
        );
        assert_eq!(trace.operation_log, vec!["connection_test:failed"]);
        assert_eq!(trace.errors, vec!["connection_test_failed"]);
    }

    #[test]
    fn webdav_sync_execution_trace_records_partial_failure_and_rejects_drift() {
        let request = WebDavSyncRequest {
            remote_book_directory_path: Some("/books".into()),
            local_progress_records: vec![progress_record("book-a", 1, "phone", 2, 1, 0.8)],
            progress_pull_since: Some(1),
            require_connection_test: true,
            ..WebDavSyncRequest::default()
        };
        let plan = plan_webdav_sync_execution(&request).unwrap();
        let trace = trace_webdav_sync_execution(
            &plan,
            &WebDavSyncExecutionMetrics {
                remote_book_count: 2,
                ..WebDavSyncExecutionMetrics::default()
            },
            Some(&WebDavSyncOperationFailure {
                operation: WebDavSyncOperation::PushProgress,
                error: "progress_push_error:Authorization=Basic abc123 password=hunter2".into(),
            }),
        )
        .unwrap();

        assert_eq!(trace.status, WebDavSyncStatus::PartialFailure);
        assert!(trace.connection_ok);
        assert_eq!(
            trace.operation_log,
            vec![
                "connection_test:success",
                "remote_books:list:2",
                "progress:push_failed",
            ]
        );
        assert_eq!(
            trace.skipped_operations,
            vec![WebDavSyncOperation::PullProgress]
        );
        assert!(trace.errors[0].contains("Authorization=REDACTED"));
        assert!(trace.errors[0].contains("password=REDACTED"));
        assert!(!trace.errors[0].contains("abc123"));
        assert!(!trace.errors[0].contains("hunter2"));
        assert!(
            serde_json::from_value::<WebDavSyncExecutionTrace>(serde_json::json!({
                "status": "failure",
                "connectionOK": false,
                "attemptedOperations": ["connectionTest"],
                "skippedOperations": [],
                "operationLog": ["connection_test:failed"],
                "errors": ["connection_test_failed"],
                "hostAdapterInvoked": true
            }))
            .is_err()
        );
        assert_eq!(
            trace_webdav_sync_execution(
                &plan,
                &WebDavSyncExecutionMetrics::default(),
                Some(&WebDavSyncOperationFailure {
                    operation: WebDavSyncOperation::RestoreBackup,
                    error: "restore failed".into(),
                }),
            )
            .unwrap_err(),
            SyncError::InvalidBackup {
                field: "failure.operation".into()
            }
        );
    }

    #[test]
    fn backup_restore_plan_matches_legacy_policy_filtering_and_runtime_result_sort() {
        let package = BackupPackage::new(backup_manifest(vec![
            backup_entry("books/z.epub", 20),
            backup_entry("books/a.txt", 10),
            backup_entry("books/b.pdf", 30),
        ]))
        .unwrap();

        let dry_run = plan_backup_restore(
            &package,
            &RestorePolicy {
                mode: RestoreMode::DryRun,
                selected_book_ids: Some(vec!["books/a.txt".into()]),
                overwrite_existing: false,
            },
        )
        .unwrap();
        assert_eq!(
            dry_run
                .restore_entries
                .iter()
                .map(|entry| entry.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["books/z.epub", "books/a.txt", "books/b.pdf"]
        );
        assert_eq!(
            dry_run
                .result_entries
                .iter()
                .map(|entry| entry.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["books/a.txt", "books/b.pdf", "books/z.epub"]
        );

        let selective = plan_backup_restore(
            &package,
            &RestorePolicy {
                mode: RestoreMode::Selective,
                selected_book_ids: Some(vec!["books/b.pdf".into(), "books/a.txt".into()]),
                overwrite_existing: false,
            },
        )
        .unwrap();
        assert_eq!(
            selective
                .restore_entries
                .iter()
                .map(|entry| entry.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["books/a.txt", "books/b.pdf"]
        );
        assert_eq!(
            selective
                .result_entries
                .iter()
                .map(|entry| entry.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["books/a.txt", "books/b.pdf"]
        );

        assert_eq!(
            plan_backup_restore(
                &package,
                &RestorePolicy {
                    mode: RestoreMode::Selective,
                    selected_book_ids: None,
                    overwrite_existing: false,
                },
            )
            .unwrap_err(),
            SyncError::InvalidRestore {
                field: "selected_book_ids".into()
            }
        );
    }

    #[test]
    fn webdav_sync_operation_plan_skips_unrequested_transport_work() {
        let request = WebDavSyncRequest {
            should_pull_progress: true,
            backup_entries_to_create: vec![backup_entry("books/a.txt", 10)],
            should_list_backups: true,
            restore_package: Some(
                BackupPackage::new(backup_manifest(vec![backup_entry("books/a.txt", 10)])).unwrap(),
            ),
            require_connection_test: false,
            ..WebDavSyncRequest::default()
        };

        let plan = plan_webdav_sync_operations(&request).unwrap();

        assert_eq!(plan.operations, vec![WebDavSyncOperation::PullProgress]);

        let mut invalid = request;
        invalid.local_progress_records = vec![ProgressCloudSyncRecord {
            book_id: " ".into(),
            chapter_index: 1,
            chapter_title: None,
            progress_fraction: 0.5,
            updated_at: 1,
            device_id: "phone".into(),
            sync_version: 1,
        }];
        assert_eq!(
            plan_webdav_sync_operations(&invalid).unwrap_err(),
            SyncError::InvalidProgress {
                field: "book_id".into()
            }
        );
    }

    #[test]
    fn webdav_sync_status_classification_matches_legacy_operation_log_heuristic() {
        assert_eq!(
            classify_webdav_sync_status(&[], &[]),
            WebDavSyncStatus::Success
        );
        assert_eq!(
            classify_webdav_sync_status(
                &["connection_test:failed".into()],
                &["connection_test_failed".into()]
            ),
            WebDavSyncStatus::Failure
        );
        assert_eq!(
            classify_webdav_sync_status(
                &[
                    "connection_test:success".into(),
                    "backup:create_failed".into()
                ],
                &["backup_create_error:Basic REDACTED".into()]
            ),
            WebDavSyncStatus::PartialFailure
        );
        assert_eq!(
            classify_webdav_sync_status(
                &["remote_books:list:2".into(), "progress:push_failed".into()],
                &["progress_push_error:timeout".into()]
            ),
            WebDavSyncStatus::PartialFailure
        );
        assert_eq!(
            classify_webdav_sync_status(
                &["progress:pull:2".into(), "backup:list_failed".into()],
                &["backup_list_error:timeout".into()]
            ),
            WebDavSyncStatus::Failure
        );
    }

    #[test]
    fn webdav_sync_result_summarizes_legacy_runtime_success_fixture() {
        let local_progress = progress_record("book-a", 1, "phone", 2_000, 2, 0.8);
        let second_progress = progress_record("book-b", 2, "tablet", 1_500, 1, 0.4);
        let remote_progress = progress_record("book-a", 1, "phone", 1_000, 1, 0.2);
        let push_plan = plan_webdav_progress_push(
            &[local_progress.clone(), second_progress.clone()],
            &[remote_progress],
            ConflictPolicy::LastWriteWins,
        )
        .unwrap();
        let created_backup_manifest = BackupManifest {
            backup_id: "backup-created-1".into(),
            created_at: 3_000,
            entries: vec![
                backup_entry("books/a.txt", 10),
                backup_entry("books/z.epub", 20),
            ],
            total_bytes: 30,
            book_count: 2,
        };
        let created_backup_package = BackupPackage {
            manifest: created_backup_manifest.clone(),
            format: BackupArchiveFormat::Zip,
            checksum: None,
        };
        let result = summarize_webdav_sync_result(
            true,
            &[
                RemoteBookMetadata {
                    remote_path: "/books/z.epub".into(),
                    title: "Z".into(),
                    author: None,
                    format: Some("epub".into()),
                    file_size: None,
                    remote_modified_at: None,
                    etag: None,
                },
                RemoteBookMetadata {
                    remote_path: "/books/a.txt".into(),
                    title: "A".into(),
                    author: None,
                    format: Some("txt".into()),
                    file_size: None,
                    remote_modified_at: None,
                    etag: None,
                },
            ],
            Some(&push_plan),
            &[second_progress.clone(), local_progress.clone()],
            Some(&created_backup_package),
            &[created_backup_manifest],
            &[
                backup_entry("books/z.epub", 20),
                backup_entry("books/a.txt", 10),
            ],
            &[
                "connection_test:success".into(),
                "remote_books:list:2".into(),
                "progress:push:2".into(),
                "progress:pull:2".into(),
                "backup:create:backup-created-1".into(),
                "backup:list:1".into(),
                "backup:restore:2".into(),
            ],
            &[],
        )
        .unwrap();

        assert_eq!(result.status, WebDavSyncStatus::Success);
        assert!(result.connection_ok);
        assert_eq!(
            result
                .remote_books
                .iter()
                .map(|book| book.remote_path.as_str())
                .collect::<Vec<_>>(),
            vec!["/books/a.txt", "/books/z.epub"]
        );
        assert_eq!(result.pushed_progress_count, 2);
        assert_eq!(
            result
                .resolved_progress_records
                .iter()
                .map(|record| (record.book_id.as_str(), record.progress_fraction))
                .collect::<Vec<_>>(),
            vec![("book-a", 0.8), ("book-b", 0.4)]
        );
        assert_eq!(
            result
                .pulled_progress_records
                .iter()
                .map(|record| record.book_id.as_str())
                .collect::<Vec<_>>(),
            vec!["book-a", "book-b"]
        );
        assert_eq!(
            result
                .restored_entries
                .iter()
                .map(|entry| entry.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["books/a.txt", "books/z.epub"]
        );
        assert_eq!(
            serde_json::to_value(&result).unwrap()["connectionOK"],
            serde_json::json!(true)
        );
        assert_eq!(
            serde_json::to_value(&result).unwrap()["status"],
            serde_json::json!("success")
        );
        result.validate().unwrap();
    }

    #[test]
    fn webdav_sync_result_redacts_errors_and_classifies_failure_modes() {
        let failed = summarize_webdav_sync_result(
            false,
            &[],
            None,
            &[],
            None,
            &[],
            &[],
            &["connection_test:failed".into()],
            &["connection_test_failed".into()],
        )
        .unwrap();
        assert_eq!(failed.status, WebDavSyncStatus::Failure);
        assert_eq!(failed.errors, vec!["connection_test_failed"]);

        let partial = summarize_webdav_sync_result(
            true,
            &[],
            None,
            &[],
            None,
            &[],
            &[],
            &["remote_books:list:2".into(), "progress:push_failed".into()],
            &[
                "progress_push_error:Authorization=Basic abc123 password=hunter2 token=abc cookie=session-secret"
                    .into(),
            ],
        )
        .unwrap();
        assert_eq!(partial.status, WebDavSyncStatus::PartialFailure);
        assert!(partial.errors[0].contains("Authorization=REDACTED"));
        assert!(partial.errors[0].contains("password=REDACTED"));
        assert!(partial.errors[0].contains("token=REDACTED"));
        assert!(partial.errors[0].contains("cookie=REDACTED"));
        assert!(!partial.errors[0].contains("abc123"));
        assert!(!partial.errors[0].contains("hunter2"));
        assert!(!partial.errors[0].contains("session-secret"));

        let pull_only_with_error = summarize_webdav_sync_result(
            true,
            &[],
            None,
            &[progress_record("book-a", 1, "phone", 1, 1, 0.1)],
            None,
            &[],
            &[],
            &["progress:pull:1".into(), "backup:list_failed".into()],
            &["backup_list_error:timeout".into()],
        )
        .unwrap();
        assert_eq!(pull_only_with_error.status, WebDavSyncStatus::Failure);
    }

    #[test]
    fn webdav_sync_result_rejects_drifted_counts_and_blank_logs() {
        let mut push_plan = WebDavProgressPushPlan {
            resolved_progress_records: vec![progress_record("book-a", 1, "phone", 1, 1, 0.1)],
            pushed_progress_count: 2,
        };
        assert_eq!(
            summarize_webdav_sync_result(
                true,
                &[],
                Some(&push_plan),
                &[],
                None,
                &[],
                &[],
                &["progress:push:2".into()],
                &[],
            )
            .unwrap_err(),
            SyncError::InvalidProgress {
                field: "pushed_progress_count".into()
            }
        );

        push_plan.pushed_progress_count = 1;
        assert_eq!(
            summarize_webdav_sync_result(
                true,
                &[],
                Some(&push_plan),
                &[],
                None,
                &[],
                &[],
                &[" ".into()],
                &[],
            )
            .unwrap_err(),
            SyncError::InvalidBackup {
                field: "operation_log".into()
            }
        );
        assert_eq!(
            summarize_webdav_sync_result(true, &[], None, &[], None, &[], &[], &[], &[" ".into()],)
                .unwrap_err(),
            SyncError::InvalidBackup {
                field: "errors".into()
            }
        );
    }

    #[test]
    fn webdav_sync_error_redaction_matches_legacy_runtime() {
        assert_eq!(
            redact_webdav_sync_error("backup_create_error:Basic abc123 Bearer token-456"),
            "backup_create_error:Basic REDACTED Bearer REDACTED"
        );

        let redacted = redact_webdav_sync_error(
            "remote_book_import_error:/books/private.txt:Authorization=Basic abc123 password=hunter2 token=abc cookie=session-secret",
        );

        assert!(redacted.contains("Authorization=REDACTED"));
        assert!(redacted.contains("password=REDACTED"));
        assert!(redacted.contains("token=REDACTED"));
        assert!(redacted.contains("cookie=REDACTED"));
        assert!(!redacted.contains("abc123"));
        assert!(!redacted.contains("hunter2"));
        assert!(!redacted.contains("session-secret"));

        let case_insensitive = redact_webdav_sync_error(
            "progress_pull_error:bearer secret.token PASSWORD=OpenSesame Authorization=DAV:private; next",
        );
        assert!(case_insensitive.contains("Bearer REDACTED"));
        assert!(case_insensitive.contains("PASSWORD=REDACTED"));
        assert!(case_insensitive.contains("Authorization=REDACTED"));
        assert!(!case_insensitive.contains("secret.token"));
        assert!(!case_insensitive.contains("OpenSesame"));
        assert!(!case_insensitive.contains("DAV:private"));
    }

    #[test]
    fn webdav_sync_request_round_trips_legacy_wire_shape() {
        let request = WebDavSyncRequest {
            remote_book_directory_path: Some("/books".into()),
            local_progress_records: vec![progress_record("book-a", 1, "phone", 2_000, 2, 0.8)],
            progress_pull_since: Some(1_200),
            should_pull_progress: true,
            conflict_policy: ConflictPolicy::DevicePriority,
            backup_config: Some(BackupConfig::new("/backups").unwrap()),
            backup_entries_to_create: vec![backup_entry("books/a.txt", 10)],
            should_list_backups: true,
            restore_package: None,
            restore_policy: Some(RestorePolicy::default()),
            require_connection_test: false,
        };

        request.validate().unwrap();
        let json = serde_json::to_value(&request).unwrap();

        assert_eq!(json["remoteBookDirectoryPath"], "/books");
        assert_eq!(json["progressPullSince"], 1_200);
        assert_eq!(json["shouldPullProgress"], true);
        assert_eq!(json["conflictPolicy"], "devicePriority");
        assert_eq!(json["shouldListBackups"], true);
        assert_eq!(json["requireConnectionTest"], false);
        assert_eq!(json["localProgressRecords"][0]["bookId"], "book-a");
        assert_eq!(
            serde_json::from_value::<WebDavSyncRequest>(json).unwrap(),
            request
        );
    }

    #[test]
    fn webdav_progress_push_plan_matches_legacy_last_write_wins_resolution() {
        let local = vec![
            progress_record("book-b", 1, "phone", 1_000, 1, 0.2),
            progress_record("book-a", 1, "phone", 2_000, 1, 0.4),
        ];
        let remote = vec![
            progress_record("book-a", 1, "phone", 2_500, 1, 0.8),
            progress_record("book-c", 2, "tablet", 1_500, 1, 0.6),
        ];

        let plan =
            plan_webdav_progress_push(&local, &remote, ConflictPolicy::LastWriteWins).unwrap();

        assert_eq!(plan.pushed_progress_count, 3);
        assert_eq!(
            plan.resolved_progress_records
                .iter()
                .map(|record| (
                    record.book_id.as_str(),
                    record.chapter_index,
                    record.device_id.as_str(),
                    record.updated_at,
                    record.progress_fraction
                ))
                .collect::<Vec<_>>(),
            vec![
                ("book-a", 1, "phone", 2_500, 0.8),
                ("book-b", 1, "phone", 1_000, 0.2),
                ("book-c", 2, "tablet", 1_500, 0.6),
            ]
        );
        assert_eq!(
            serde_json::to_value(&plan).unwrap(),
            serde_json::json!({
                "resolvedProgressRecords": [
                    {
                        "bookId": "book-a",
                        "chapterIndex": 1,
                        "chapterTitle": "Chapter 1",
                        "progressFraction": 0.8,
                        "updatedAt": 2500,
                        "deviceId": "phone",
                        "syncVersion": 1
                    },
                    {
                        "bookId": "book-b",
                        "chapterIndex": 1,
                        "chapterTitle": "Chapter 1",
                        "progressFraction": 0.2,
                        "updatedAt": 1000,
                        "deviceId": "phone",
                        "syncVersion": 1
                    },
                    {
                        "bookId": "book-c",
                        "chapterIndex": 2,
                        "chapterTitle": "Chapter 2",
                        "progressFraction": 0.6,
                        "updatedAt": 1500,
                        "deviceId": "tablet",
                        "syncVersion": 1
                    }
                ],
                "pushedProgressCount": 3
            })
        );
    }

    #[test]
    fn webdav_progress_push_execution_models_legacy_pull_resolve_push_sequence() {
        let local = vec![
            progress_record("book-a", 1, "phone", 2_000, 2, 0.8),
            progress_record("book-b", 2, "tablet", 1_500, 1, 0.4),
        ];
        let remote_before_push = vec![progress_record("book-a", 1, "phone", 1_000, 1, 0.2)];

        let plan = plan_webdav_progress_push_execution(
            &local,
            &remote_before_push,
            ConflictPolicy::LastWriteWins,
        )
        .unwrap();

        assert_eq!(
            plan.steps,
            vec![
                WebDavProgressPushExecutionStep::PullRemoteBeforePush,
                WebDavProgressPushExecutionStep::ResolveConflicts,
                WebDavProgressPushExecutionStep::PushResolvedRecords,
            ]
        );
        assert_eq!(plan.remote_pull_since, None);
        assert_eq!(plan.remote_before_push_count, 1);
        assert_eq!(plan.conflict_policy, ConflictPolicy::LastWriteWins);
        assert_eq!(plan.push_plan.pushed_progress_count, 2);
        assert_eq!(
            plan.push_plan
                .resolved_progress_records
                .iter()
                .map(|record| (record.book_id.as_str(), record.progress_fraction))
                .collect::<Vec<_>>(),
            vec![("book-a", 0.8), ("book-b", 0.4)]
        );

        let json = serde_json::to_value(&plan).unwrap();
        assert_eq!(
            json["steps"],
            serde_json::json!([
                "pullRemoteBeforePush",
                "resolveConflicts",
                "pushResolvedRecords"
            ])
        );
        assert!(json["remotePullSince"].is_null());
        assert_eq!(json["remoteBeforePushCount"], 1);
        assert_eq!(json["conflictPolicy"], "lastWriteWins");
        assert_eq!(json["pushPlan"]["pushedProgressCount"], 2);
        assert_eq!(
            json["pushPlan"]["resolvedProgressRecords"][0]["bookId"],
            "book-a"
        );
        assert_eq!(
            serde_json::from_value::<WebDavProgressPushExecutionPlan>(json).unwrap(),
            plan
        );
    }

    #[test]
    fn webdav_progress_push_execution_skips_transport_steps_without_local_changes() {
        let remote_before_push = vec![ProgressCloudSyncRecord {
            book_id: " ".into(),
            chapter_index: 1,
            chapter_title: None,
            progress_fraction: 0.5,
            updated_at: 1,
            device_id: "phone".into(),
            sync_version: 1,
        }];

        let plan = plan_webdav_progress_push_execution(
            &[],
            &remote_before_push,
            ConflictPolicy::DevicePriority,
        )
        .unwrap();

        assert!(plan.steps.is_empty());
        assert_eq!(plan.remote_pull_since, None);
        assert_eq!(plan.remote_before_push_count, 0);
        assert_eq!(plan.conflict_policy, ConflictPolicy::DevicePriority);
        assert_eq!(plan.push_plan.pushed_progress_count, 0);
        assert!(plan.push_plan.resolved_progress_records.is_empty());

        let invalid_local = vec![ProgressCloudSyncRecord {
            book_id: "book-a".into(),
            chapter_index: 1,
            chapter_title: None,
            progress_fraction: 1.5,
            updated_at: 1,
            device_id: "phone".into(),
            sync_version: 1,
        }];
        assert_eq!(
            plan_webdav_progress_push_execution(
                &invalid_local,
                &[],
                ConflictPolicy::LastWriteWins,
            )
            .unwrap_err(),
            SyncError::InvalidProgress {
                field: "progress_fraction".into()
            }
        );
    }

    #[test]
    fn webdav_progress_push_plan_preserves_non_lww_conflict_policy_payloads() {
        let local = vec![progress_record("book-a", 1, "phone", 2_000, 1, 0.4)];
        let remote = vec![progress_record("book-a", 1, "phone", 2_500, 1, 0.8)];

        for policy in [
            ConflictPolicy::DevicePriority,
            ConflictPolicy::Manual,
            ConflictPolicy::KeepBoth,
        ] {
            let plan = plan_webdav_progress_push(&local, &remote, policy).unwrap();

            assert_eq!(plan.pushed_progress_count, 2);
            assert_eq!(
                plan.resolved_progress_records
                    .iter()
                    .map(|record| record.updated_at)
                    .collect::<Vec<_>>(),
                vec![2_000, 2_500]
            );
        }

        let invalid = vec![ProgressCloudSyncRecord {
            book_id: "book-a".into(),
            chapter_index: 1,
            chapter_title: None,
            progress_fraction: 1.5,
            updated_at: 1,
            device_id: "phone".into(),
            sync_version: 1,
        }];
        assert_eq!(
            plan_webdav_progress_push(&invalid, &[], ConflictPolicy::LastWriteWins).unwrap_err(),
            SyncError::InvalidProgress {
                field: "progress_fraction".into()
            }
        );
    }

    #[test]
    fn progress_sync_config_defaults_and_wire_values_match_legacy_reader_core() {
        let config = ProgressCloudSyncConfig::default();

        assert_eq!(config.sync_interval_minutes, 15);
        assert_eq!(
            config.conflict_strategy,
            ProgressConflictStrategy::LastWriteWins
        );
        assert!(config.auto_sync_enabled);
        assert_eq!(config.max_records_per_book, 50);
        assert_eq!(
            serde_json::to_value(&config).unwrap(),
            serde_json::json!({
                "syncIntervalMinutes": 15,
                "conflictStrategy": "lastWriteWins",
                "autoSyncEnabled": true,
                "maxRecordsPerBook": 50
            })
        );

        let cases = [
            (ProgressConflictStrategy::LastWriteWins, "lastWriteWins"),
            (ProgressConflictStrategy::DevicePriority, "devicePriority"),
            (ProgressConflictStrategy::Manual, "manual"),
        ];
        for (strategy, expected) in cases {
            let json = serde_json::to_string(&strategy).unwrap();
            assert_eq!(json, format!(r#""{expected}""#));
            assert_eq!(
                serde_json::from_str::<ProgressConflictStrategy>(&json).unwrap(),
                strategy
            );
        }

        let mut invalid = config;
        invalid.sync_interval_minutes = 0;
        assert_eq!(
            invalid.validate().unwrap_err(),
            SyncError::InvalidProgress {
                field: "sync_interval_minutes".into()
            }
        );
        assert!(serde_json::from_str::<ProgressConflictStrategy>(r#""keepBoth""#).is_err());
    }

    #[test]
    fn progress_record_round_trips_and_rejects_invalid_state() {
        let mut record =
            ProgressCloudSyncRecord::new(" book-a ", 5, 0.42, 1_700_000_000, " phone ").unwrap();
        record.chapter_title = Some("Ch5".into());
        record.sync_version = 2;

        assert_eq!(record.book_id, "book-a");
        assert_eq!(record.device_id, "phone");
        assert_eq!(
            record.key(),
            ProgressCloudSyncRecordKey {
                book_id: "book-a".into(),
                chapter_index: 5,
                device_id: "phone".into()
            }
        );
        let json = serde_json::to_string(&record).unwrap();
        assert!(json.contains(r#""bookId":"book-a""#));
        assert!(json.contains(r#""deviceId":"phone""#));
        assert_eq!(
            serde_json::from_str::<ProgressCloudSyncRecord>(&json).unwrap(),
            record
        );

        let mut invalid_fraction = record.clone();
        invalid_fraction.progress_fraction = 1.1;
        assert_eq!(
            invalid_fraction.validate().unwrap_err(),
            SyncError::InvalidProgress {
                field: "progress_fraction".into()
            }
        );
        let mut invalid_version = record.clone();
        invalid_version.sync_version = 0;
        assert_eq!(
            invalid_version.validate().unwrap_err(),
            SyncError::InvalidProgress {
                field: "sync_version".into()
            }
        );
        assert!(serde_json::from_str::<ProgressCloudSyncRecord>(
            r#"{"bookId":"b","chapterIndex":1,"progressFraction":0.5,"updatedAt":1,"deviceId":"d","syncVersion":1,"bogus":true}"#
        )
        .is_err());
    }

    #[test]
    fn progress_since_filter_uses_updated_at_and_stable_sort() {
        let records = vec![
            progress_record("book-b", 2, "tablet", 2_000, 1, 0.2),
            progress_record("book-a", 2, "phone", 1_500, 1, 0.3),
            progress_record("book-a", 1, "phone", 1_000, 1, 0.1),
        ];

        let pulled = progress_cloud_records_since(&records, Some(1_200)).unwrap();

        assert_eq!(
            pulled
                .iter()
                .map(|record| (
                    record.book_id.as_str(),
                    record.chapter_index,
                    record.device_id.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![("book-a", 2, "phone"), ("book-b", 2, "tablet")]
        );
        assert_eq!(
            progress_cloud_records_since(&records, Some(2_000))
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn progress_last_write_wins_dedupes_by_book_chapter_and_device() {
        let old = progress_record("book-a", 1, "phone", 1_000, 1, 0.2);
        let newer = progress_record("book-a", 1, "phone", 2_000, 2, 0.8);
        let other_device = progress_record("book-a", 1, "tablet", 1_500, 1, 0.4);

        let merged = merge_progress_cloud_records(
            &[old],
            &[newer.clone(), other_device.clone()],
            &ProgressCloudSyncConfig::default(),
        )
        .unwrap();

        assert_eq!(merged, vec![newer, other_device]);
    }

    #[test]
    fn progress_manual_strategy_rejects_divergent_same_key() {
        let local = progress_record("book-a", 1, "phone", 1_000, 1, 0.2);
        let remote = progress_record("book-a", 1, "phone", 2_000, 2, 0.8);
        let config = ProgressCloudSyncConfig {
            conflict_strategy: ProgressConflictStrategy::Manual,
            ..ProgressCloudSyncConfig::default()
        };

        assert_eq!(
            merge_progress_cloud_records(&[local], &[remote], &config).unwrap_err(),
            SyncError::UnresolvedConflict {
                record_id: "book-a:1:phone".into()
            }
        );
    }

    #[test]
    fn progress_retention_keeps_latest_records_per_book() {
        let config = ProgressCloudSyncConfig {
            max_records_per_book: 2,
            ..ProgressCloudSyncConfig::default()
        };
        let records = vec![
            progress_record("book-a", 1, "phone", 1_000, 1, 0.1),
            progress_record("book-a", 2, "phone", 3_000, 1, 0.5),
            progress_record("book-a", 3, "phone", 2_000, 1, 0.3),
            progress_record("book-b", 1, "phone", 1_500, 1, 0.2),
        ];

        let merged = merge_progress_cloud_records(&records, &[], &config).unwrap();

        assert_eq!(
            merged
                .iter()
                .map(|record| (record.book_id.as_str(), record.chapter_index))
                .collect::<Vec<_>>(),
            vec![("book-a", 2), ("book-a", 3), ("book-b", 1)]
        );
    }

    #[test]
    fn restore_policy_modes_and_selective_validation_match_legacy_reader_core() {
        let cases = [
            (RestoreMode::Full, "full"),
            (RestoreMode::Selective, "selective"),
            (RestoreMode::DryRun, "dryRun"),
        ];

        for (mode, expected) in cases {
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(json, format!(r#""{expected}""#));
            assert_eq!(serde_json::from_str::<RestoreMode>(&json).unwrap(), mode);
        }

        let default_policy = RestorePolicy::default();
        default_policy.validate().unwrap();
        assert_eq!(default_policy.mode, RestoreMode::Full);
        assert!(!default_policy.overwrite_existing);
        assert_eq!(
            serde_json::to_value(&default_policy).unwrap(),
            serde_json::json!({
                "mode": "full",
                "overwriteExisting": false
            })
        );

        let selective = RestorePolicy {
            mode: RestoreMode::Selective,
            selected_book_ids: Some(vec!["b1".into(), "b2".into()]),
            overwrite_existing: false,
        };
        selective.validate().unwrap();
        let value = serde_json::to_value(&selective).unwrap();
        assert_eq!(value["selectedBookIDs"], serde_json::json!(["b1", "b2"]));
        assert_eq!(
            serde_json::from_value::<RestorePolicy>(value).unwrap(),
            selective
        );

        let missing_selection = RestorePolicy {
            mode: RestoreMode::Selective,
            selected_book_ids: None,
            overwrite_existing: false,
        };
        assert_eq!(
            missing_selection.validate().unwrap_err(),
            SyncError::InvalidRestore {
                field: "selected_book_ids".into()
            }
        );
        let duplicate_selection = RestorePolicy {
            mode: RestoreMode::Selective,
            selected_book_ids: Some(vec!["b1".into(), "b1".into()]),
            overwrite_existing: false,
        };
        assert_eq!(
            duplicate_selection.validate().unwrap_err(),
            SyncError::InvalidRestore {
                field: "selected_book_ids".into()
            }
        );
        assert!(serde_json::from_str::<RestoreMode>(r#""preview""#).is_err());
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
    fn manual_conflict_policy_rejects_automatic_payload_conflict() {
        let local = snap(
            "local",
            vec![rec_from_device(
                SyncCollection::ReadingProgress,
                "book",
                r#"{"chapter":1}"#,
                10,
                "phone",
                1,
            )],
        );
        let remote = snap(
            "remote",
            vec![rec_from_device(
                SyncCollection::ReadingProgress,
                "book",
                r#"{"chapter":2}"#,
                11,
                "tablet",
                2,
            )],
        );
        let policy = SyncMergePolicy {
            conflict_policy: ConflictPolicy::Manual,
            device_priority: Vec::new(),
        };

        assert_eq!(
            merge_snapshots_with_policy(&local, &remote, "merged", "merge-device", 12, &policy)
                .unwrap_err(),
            SyncError::UnresolvedConflict {
                record_id: "book".into()
            }
        );
    }

    #[test]
    fn device_priority_policy_can_choose_older_preferred_device_record() {
        let phone = rec_from_device(
            SyncCollection::ReadingProgress,
            "book",
            r#"{"chapter":1}"#,
            10,
            "phone",
            1,
        );
        let tablet = rec_from_device(
            SyncCollection::ReadingProgress,
            "book",
            r#"{"chapter":2}"#,
            20,
            "tablet",
            2,
        );
        let policy = SyncMergePolicy {
            conflict_policy: ConflictPolicy::DevicePriority,
            device_priority: vec!["phone".into(), "tablet".into()],
        };

        let result = merge_snapshots_with_policy(
            &snap("local", vec![phone.clone()]),
            &snap("remote", vec![tablet]),
            "merged",
            "merge-device",
            21,
            &policy,
        )
        .unwrap();

        assert_eq!(result.snapshot.records, vec![phone.clone()]);
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].winner, phone);
        assert_eq!(
            result.conflicts[0].reason,
            ConflictReason::ConcurrentPayloadChange
        );
    }

    #[test]
    fn keep_both_policy_preserves_losing_live_record_as_conflict_copy() {
        let local = rec_from_device(
            SyncCollection::LocalBook,
            "local-1",
            r#"{"title":"Local A"}"#,
            10,
            "phone",
            1,
        );
        let remote = rec_from_device(
            SyncCollection::LocalBook,
            "local-1",
            r#"{"title":"Local B"}"#,
            11,
            "tablet",
            2,
        );
        let policy = SyncMergePolicy {
            conflict_policy: ConflictPolicy::KeepBoth,
            device_priority: Vec::new(),
        };

        let result = merge_snapshots_with_policy(
            &snap("local", vec![local.clone()]),
            &snap("remote", vec![remote.clone()]),
            "merged",
            "merge-device",
            12,
            &policy,
        )
        .unwrap();

        assert_eq!(result.snapshot.records.len(), 2);
        assert_eq!(result.snapshot.records[0], remote);
        assert_eq!(
            result.snapshot.records[1].collection,
            SyncCollection::LocalBook
        );
        assert_eq!(
            result.snapshot.records[1].record_id,
            "local-1#conflict:phone:1"
        );
        assert_eq!(result.snapshot.records[1].payload, local.payload);
        assert_eq!(result.conflicts.len(), 1);
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
