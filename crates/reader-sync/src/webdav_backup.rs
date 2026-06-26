//! WebDAV backup plan → `WebDavRequest` descriptor translation.
//!
//! Maps Swift `URLSessionWebDAVAdapter.BackupService` extension methods to
//! Core-produced `WebDavRequest` sequences. Core plans; Host executes HTTP.
//!
//! Swift mapping (against `URLSessionWebDAVAdapter.swift`):
//! - `createBackup(config:items:)` (lines 201-218) → `upload_json` to
//!   `backupPackagePath`. We also emit an `MKCOL` for the parent directory
//!   (tolerated if it already exists); Swift relies on the server creating
//!   intermediate directories, but standard WebDAV servers require MKCOL —
//!   per charter red line 3 we补齐.
//! - `listBackups(config:)` (lines 220-225) → `list_directory` on the
//!   backup directory.
//! - `restoreBackup(package:policy:)` (lines 227-235) → `download_file` for
//!   the package JSON; selective filtering is a pure data decision that
//!   happens before descriptor generation.
//! - `enforceBackupRetention(config:preservingBackupID:)` (lines 362-395) →
//!   one `delete_file` per path in the precomputed `BackupRetentionPlan`.

use crate::webdav_client::{
    delete_file, download_file, list_directory, make_collection, upload_json,
};
use crate::webdav_protocol::WebDavRequest;
use crate::{BackupConfig, BackupPackage, BackupRetentionPlan, SyncError};

/// Compute the remote path for a backup package JSON file.
///
/// Mirrors Swift `URLSessionWebDAVAdapter.backupPackagePath(config:backupID:)`
/// (lines 339-343): directory = `config.target_url`, strip trailing `/`,
/// empty directory → `{backupID}.json`, else `{directory}/{backupID}.json`.
pub fn backup_package_path(config: &BackupConfig, backup_id: &str) -> Result<String, SyncError> {
    config.validate()?;
    require_non_empty(backup_id, "backup_id")?;
    let directory = config.target_url.trim();
    let normalized = directory.strip_suffix('/').unwrap_or(directory);
    Ok(if normalized.is_empty() {
        format!("{backup_id}.json")
    } else {
        format!("{normalized}/{backup_id}.json")
    })
}

/// Produce the `WebDavRequest` sequence to create a backup package remotely.
///
/// Mirrors Swift `createBackup` (lines 201-218): JSON-encode the package and
/// PUT it to `backupPackagePath`. We also emit an `MKCOL` for the parent
/// directory first (405-tolerated if it already exists); Swift lacks this but
/// standard WebDAV servers require it to create the collection.
pub fn plan_create_backup(
    config: &BackupConfig,
    package: &BackupPackage,
) -> Result<Vec<WebDavRequest>, SyncError> {
    config.validate()?;
    package.validate()?;
    let json = package.to_json()?;
    let path = backup_package_path(config, &package.manifest.backup_id)?;
    let directory = backup_directory_path(config);
    let mut requests = Vec::with_capacity(2);
    if !directory.is_empty() {
        requests.push(make_collection(&directory));
    }
    requests.push(upload_json(&path, &json));
    Ok(requests)
}

/// Produce the `WebDavRequest` to list remote backup packages.
///
/// Mirrors Swift `listBackups` (lines 220-225): PROPFIND Depth:1 on the
/// backup directory. The caller parses the multistatus response and downloads
/// each `.json` entry to reconstruct `BackupManifest` / `BackupPackage`.
pub fn plan_list_backups(config: &BackupConfig) -> Result<WebDavRequest, SyncError> {
    config.validate()?;
    let directory = backup_directory_path(config);
    require_non_empty(&directory, "target_url")?;
    Ok(list_directory(&directory))
}

/// Produce the `WebDavRequest` to download a remote backup package JSON.
///
/// Used by restore flows after `plan_list_backups` has enumerated candidates
/// and the caller has selected which package to restore.
pub fn plan_download_backup(remote_path: &str) -> Result<WebDavRequest, SyncError> {
    require_non_empty(remote_path, "remote_path")?;
    Ok(download_file(remote_path))
}

/// Produce the `WebDavRequest` to delete a remote backup package JSON.
///
/// Mirrors one iteration of Swift `enforceBackupRetention`'s delete loop
/// (lines 388-394). The 404-tolerant `delete_file` matches Swift's
/// `catch FileAccessError.notFound { continue }` behavior.
pub fn plan_delete_backup(remote_path: &str) -> Result<WebDavRequest, SyncError> {
    require_non_empty(remote_path, "remote_path")?;
    Ok(delete_file(remote_path))
}

/// Translate a retention delete plan into a DELETE `WebDavRequest` sequence.
///
/// Mirrors Swift `enforceBackupRetention` (lines 362-395): delete each path in
/// `plan.paths_to_delete`, sorted deterministically by path. The sort matches
/// Swift's `for path in pathsToDelete.sorted()` iteration order.
pub fn plan_enforce_retention(plan: &BackupRetentionPlan) -> Result<Vec<WebDavRequest>, SyncError> {
    let mut paths = plan.paths_to_delete.clone();
    paths.sort();
    let mut requests = Vec::with_capacity(paths.len());
    for path in &paths {
        require_non_empty(path, "paths_to_delete")?;
        requests.push(delete_file(path));
    }
    Ok(requests)
}

fn backup_directory_path(config: &BackupConfig) -> String {
    let directory = config.target_url.trim();
    directory.strip_suffix('/').unwrap_or(directory).to_string()
}

fn require_non_empty(value: &str, field: &str) -> Result<(), SyncError> {
    if value.trim().is_empty() {
        return Err(SyncError::InvalidBackup {
            field: field.into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webdav_protocol::WebDavMethod;
    use crate::{BackupArchiveFormat, BackupManifest, BackupManifestEntry};

    fn make_config(target: &str) -> BackupConfig {
        BackupConfig::new(target).unwrap()
    }

    fn make_manifest(backup_id: &str, created_at: i64) -> BackupManifest {
        BackupManifest {
            backup_id: backup_id.to_string(),
            created_at,
            entries: vec![BackupManifestEntry {
                relative_path: "book1.epub".to_string(),
                sha256: None,
                size_bytes: 100,
                modified_at: created_at,
            }],
            total_bytes: 100,
            book_count: 1,
        }
    }

    fn make_package(backup_id: &str, created_at: i64) -> BackupPackage {
        BackupPackage {
            manifest: make_manifest(backup_id, created_at),
            format: BackupArchiveFormat::Directory,
            checksum: None,
        }
    }

    #[test]
    fn backup_package_path_joins_directory_and_id() {
        let config = make_config("reader-core/backups");
        let path = backup_package_path(&config, "abc-123").unwrap();
        assert_eq!(path, "reader-core/backups/abc-123.json");
    }

    #[test]
    fn backup_package_path_strips_trailing_slash() {
        let config = make_config("reader-core/backups/");
        let path = backup_package_path(&config, "abc-123").unwrap();
        assert_eq!(path, "reader-core/backups/abc-123.json");
    }

    #[test]
    fn backup_package_path_rejects_empty_backup_id() {
        let config = make_config("reader-core/backups");
        assert!(backup_package_path(&config, "").is_err());
        assert!(backup_package_path(&config, "   ").is_err());
    }

    #[test]
    fn plan_create_backup_emits_mkcol_then_put() {
        let config = make_config("reader-core/backups");
        let package = make_package("abc-123", 1_700_000_000_000_i64);
        let requests = plan_create_backup(&config, &package).unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, WebDavMethod::Mkcol);
        assert_eq!(requests[0].path, "reader-core/backups");
        assert_eq!(requests[1].method, WebDavMethod::Put);
        assert_eq!(requests[1].path, "reader-core/backups/abc-123.json");
        assert_eq!(
            requests[1].header("Content-Type"),
            Some("application/json; charset=utf-8")
        );
        assert!(requests[1].body.is_some());
    }

    #[test]
    fn plan_create_backup_put_body_is_valid_package_json() {
        let config = make_config("backups");
        let package = make_package("id-1", 1_700_000_000_000_i64);
        let requests = plan_create_backup(&config, &package).unwrap();
        let body = std::str::from_utf8(requests[1].body.as_ref().unwrap()).unwrap();
        let decoded: BackupPackage = serde_json::from_str(body).unwrap();
        assert_eq!(decoded, package);
    }

    #[test]
    fn plan_create_backup_rejects_invalid_package() {
        let config = make_config("backups");
        let bad_manifest = BackupManifest {
            backup_id: String::new(),
            created_at: 0,
            entries: vec![],
            total_bytes: 0,
            book_count: 0,
        };
        let package = BackupPackage {
            manifest: bad_manifest,
            format: BackupArchiveFormat::Directory,
            checksum: None,
        };
        assert!(plan_create_backup(&config, &package).is_err());
    }

    #[test]
    fn plan_list_backups_is_propfind_on_directory() {
        let config = make_config("reader-core/backups");
        let request = plan_list_backups(&config).unwrap();
        assert_eq!(request.method, WebDavMethod::Propfind);
        assert_eq!(request.path, "reader-core/backups");
        assert_eq!(request.depth, Some(1));
        assert_eq!(request.accepted_status_codes, vec![207]);
    }

    #[test]
    fn plan_download_backup_is_get() {
        let request = plan_download_backup("backups/abc.json").unwrap();
        assert_eq!(request.method, WebDavMethod::Get);
        assert_eq!(request.path, "backups/abc.json");
    }

    #[test]
    fn plan_download_backup_rejects_empty_path() {
        assert!(plan_download_backup("").is_err());
    }

    #[test]
    fn plan_delete_backup_is_delete_with_404_tolerated() {
        let request = plan_delete_backup("backups/old.json").unwrap();
        assert_eq!(request.method, WebDavMethod::Delete);
        assert!(request.accepted_status_codes.contains(&404));
    }

    #[test]
    fn plan_enforce_retention_emits_sorted_deletes() {
        let plan = BackupRetentionPlan {
            paths_to_delete: vec![
                "backups/old2.json".to_string(),
                "backups/old1.json".to_string(),
                "backups/old3.json".to_string(),
            ],
        };
        let requests = plan_enforce_retention(&plan).unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].path, "backups/old1.json");
        assert_eq!(requests[1].path, "backups/old2.json");
        assert_eq!(requests[2].path, "backups/old3.json");
        for r in &requests {
            assert_eq!(r.method, WebDavMethod::Delete);
        }
    }

    #[test]
    fn plan_enforce_retention_empty_plan_yields_no_requests() {
        let plan = BackupRetentionPlan {
            paths_to_delete: vec![],
        };
        let requests = plan_enforce_retention(&plan).unwrap();
        assert!(requests.is_empty());
    }
}
