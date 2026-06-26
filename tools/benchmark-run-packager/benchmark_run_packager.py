#!/usr/bin/env python3
"""Benchmark run packager.

Packages a single corpus benchmark *run directory* into an archivable bundle
directory (and optionally a zip) containing the run's inputs, outputs, logs and
environment information, plus a generated ``summary.json``.

A run directory is expected to look like::

    <run-dir>/
      manifest.json           # required — run metadata (runId, timestamp, ...)
      platform-result.json    # required — the platform's actual output
      canonical-result.json   # required — the canonical reference output
      diff-result.json        # required — diff between platform and canonical
      environment.json        # optional — captured environment info
      logs/                   # optional — log files
      *.log                   # optional — log files at the root
      ...                     # any other artifacts are copied through

This tool does **not** run any benchmark. It only validates and packages
already-produced results.

Output location rules (per project constraint "不写 Documents 新目录"):

  * The default bundle directory and default zip path live under
    ``/private/tmp``.
  * A user may override either with ``--out <dir>`` / ``--zip <path>``.
  * The tool refuses to write any output under ``~/Documents`` (which also
    covers the repository working tree), regardless of whether the path was
    defaulted or user-specified.

The module is intentionally stdlib-only (no third-party dependencies) and does
not touch any Core business logic.
"""

import argparse
import datetime
import hashlib
import json
import os
import platform
import re
import shutil
import socket
import sys
import tempfile
import zipfile


TOOL_NAME = "benchmark-run-packager"
TOOL_VERSION = "1.0"
SCHEMA_VERSION = 1

# (artifact key, expected filename). Order is stable for deterministic output.
REQUIRED_ARTIFACTS = [
    ("manifest", "manifest.json"),
    ("platform-result", "platform-result.json"),
    ("canonical-result", "canonical-result.json"),
    ("diff-result", "diff-result.json"),
]

PRIVATE_TMP = "/private/tmp"
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
CORE_COMMIT_RE = re.compile(r"^[0-9a-f]{7,40}$")
REQUIRED_PLATFORMS = ("cli", "ios", "android", "harmony")
BUSINESS_KERNEL = "reader-core-native-rust"
HOST_PLATFORMS = ("ios", "android", "harmony")
BUNDLE_MANIFEST_NAME = "bundle-manifest.json"
BUNDLE_MANIFEST_SHA256_NAME = "bundle-manifest.sha256"
REQUIRED_TRUE_CORPUS_PROOF_CONDITIONS = (
    "sourceManifestPresent",
    "schemaVersionBound",
    "coreIdentityBound",
    "artifactHashesBound",
    "runBindingDeclared",
    "runBindingMatches",
    "scenarioBindingDeclared",
    "scenarioBindingMatches",
    "expectedDeclared",
    "fullDiffExpectedDeclared",
    "byPlatformExpectedDeclared",
    "hostParityExpectedDeclared",
    "fourPlatformCandidatesPresent",
    "fourPlatformSummaryPresent",
    "fullDiffMatch",
    "hostParityMatch",
)


class PackagingError(Exception):
    """Raised when a run directory cannot be validated or packaged."""


# --------------------------------------------------------------------------- #
# Path helpers
# --------------------------------------------------------------------------- #
def _documents_dir():
    return os.path.realpath(os.path.expanduser("~/Documents"))


def _is_under(path, base):
    """Return True if absolute ``path`` is equal to or nested under ``base``."""
    path = os.path.realpath(path)
    base = os.path.realpath(base)
    try:
        return os.path.commonpath([path, base]) == base
    except ValueError:
        # Different drives / roots on some platforms.
        return False


def assert_safe_output(path, user_specified):
    """Enforce the output-location policy.

    Default (non-user-specified) paths must resolve under ``/private/tmp``.
    No output — default or user-specified — may be written under
    ``~/Documents``.
    """
    if not user_specified:
        if not _is_under(path, PRIVATE_TMP):
            raise PackagingError(
                "default output must live under {tmp}; got {path}".format(
                    tmp=PRIVATE_TMP, path=path
                )
            )
    docs = _documents_dir()
    if _is_under(path, docs):
        raise PackagingError(
            "refusing to write output under ~/Documents ({path}); "
            "use /private/tmp or another user-specified path".format(path=path)
        )


def sanitize_for_path(name):
    """Make ``name`` safe to use as a single path component."""
    if not name:
        return "run"
    cleaned = []
    for ch in name:
        if ch.isalnum() or ch in "-_.":
            cleaned.append(ch)
        else:
            cleaned.append("-")
    cleaned = "".join(cleaned).strip("-.")
    return cleaned or "run"


# --------------------------------------------------------------------------- #
# File / JSON helpers
# --------------------------------------------------------------------------- #
def load_json_file(path):
    """Parse a JSON file. Returns the decoded value.

    Raises :class:`PackagingError` if the file cannot be read or parsed.
    """
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return json.load(handle)
    except FileNotFoundError:
        raise PackagingError("missing file: {0}".format(path))
    except (OSError, IOError) as err:
        raise PackagingError("cannot read {0}: {1}".format(path, err))
    except json.JSONDecodeError as err:
        raise PackagingError("invalid JSON in {0}: {1}".format(path, err))


def _load_optional_json(run_dir, filename):
    """Load an optional JSON file from ``run_dir``.

    Returns the decoded value, or ``None`` if the file is absent or not valid
    JSON. Optional artifacts (e.g. ``environment.json``) must never block
    packaging.
    """
    path = os.path.join(run_dir, filename)
    if not os.path.isfile(path):
        return None
    try:
        return load_json_file(path)
    except PackagingError:
        return None


def sha256_of_file(path, chunk_size=65536):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        while True:
            chunk = handle.read(chunk_size)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def is_sha256(value):
    return isinstance(value, str) and SHA256_RE.match(value) is not None


def is_core_commit_ref(value):
    return isinstance(value, str) and CORE_COMMIT_RE.match(value) is not None


def _commit_matches_required(actual, required):
    if not is_core_commit_ref(actual) or not is_core_commit_ref(required):
        return False
    return actual == required or actual.startswith(required) or required.startswith(actual)


def list_run_files(run_dir):
    """Return a sorted list of file descriptors for every file under ``run_dir``.

    Each descriptor is ``{"path": <posix relative path>, "size": int,
    "sha256": str}``.
    """
    run_dir = os.path.abspath(run_dir)
    entries = []
    for root, _dirs, files in os.walk(run_dir):
        for name in files:
            full = os.path.join(root, name)
            rel = os.path.relpath(full, run_dir)
            entries.append(
                {
                    "path": rel.replace(os.sep, "/"),
                    "size": os.path.getsize(full),
                    "sha256": sha256_of_file(full),
                }
            )
    entries.sort(key=lambda item: item["path"])
    return entries


def _artifact_full_path(run_dir, rel_path):
    if not isinstance(rel_path, str) or not rel_path.strip():
        raise PackagingError("artifact path missing")
    if os.path.isabs(rel_path):
        raise PackagingError("artifact path must be relative: {0}".format(rel_path))
    full = os.path.realpath(os.path.join(run_dir, rel_path))
    if not _is_under(full, run_dir):
        raise PackagingError("artifact path escapes run directory: {0}".format(rel_path))
    return full


def _validate_declared_artifact(run_dir, name, rel_path, expected_hashes=None):
    expected_hashes = expected_hashes or {}
    status = {
        "path": rel_path,
        "present": False,
        "sha256": None,
        "ok": False,
        "errors": [],
    }
    try:
        full = _artifact_full_path(run_dir, rel_path)
    except PackagingError as err:
        status["errors"].append("{0}: {1}".format(name, err))
        return status

    if not os.path.isfile(full):
        status["errors"].append(
            "collector artifact missing: {0} ({1})".format(name, rel_path)
        )
        return status

    actual = sha256_of_file(full)
    status["present"] = True
    status["sha256"] = actual
    for hash_name, expected in sorted(expected_hashes.items()):
        if expected is None:
            continue
        if not is_sha256(expected):
            status["errors"].append(
                "collector artifact {0} {1} is not a lowercase sha256".format(
                    name,
                    hash_name,
                )
            )
        elif expected != actual:
            status["errors"].append(
                "collector artifact {0} {1} mismatch: expected {2}, got {3}".format(
                    name,
                    hash_name,
                    expected,
                    actual,
                )
            )
    status["ok"] = not status["errors"]
    return status


def validate_collector_artifacts(run_dir, manifest):
    """Validate manifest-declared collector artifacts, when present.

    Plain benchmark run directories are still supported. If a manifest contains
    collector evidence, however, packaging must fail closed when the declared
    artifacts are missing, escape the run directory, or no longer match their
    package hashes.
    """
    result = {
        "checked": False,
        "ok": True,
        "artifacts": {},
        "errors": [],
    }
    if not isinstance(manifest, dict):
        return result

    has_collector_manifest = isinstance(manifest.get("sourceManifest"), dict)
    has_collector_artifacts = any(
        isinstance(manifest.get(key), dict)
        for key in ("sourceManifestFile", "input", "canonical", "candidates")
    )
    if not has_collector_manifest and not has_collector_artifacts:
        return result

    result["checked"] = True
    if has_collector_manifest:
        for section in ("sourceManifestFile", "input", "canonical", "candidates", "artifacts"):
            if not isinstance(manifest.get(section), dict):
                result["errors"].append(
                    "collector artifact declaration missing: {0}".format(section)
                )
    declared = []

    source_manifest_file = manifest.get("sourceManifestFile")
    if isinstance(source_manifest_file, dict):
        declared.append((
            "sourceManifestFile",
            source_manifest_file.get("packagePath"),
            {
                "packageSha256": source_manifest_file.get("packageSha256"),
                "sourceSha256": source_manifest_file.get("sourceSha256"),
            },
        ))

    input_artifact = manifest.get("input")
    if isinstance(input_artifact, dict):
        declared.append((
            "input",
            input_artifact.get("packagePath"),
            {"packageSha256": input_artifact.get("packageSha256")},
        ))

    canonical = manifest.get("canonical")
    if isinstance(canonical, dict):
        declared.append((
            "canonical.raw",
            canonical.get("rawPath"),
            {"sourceSha256": canonical.get("sourceSha256")},
        ))
        declared.append((
            "canonical.canonicalized",
            canonical.get("packagePath"),
            {
                "packageSha256": canonical.get("packageSha256"),
                "canonicalizedFileSha256": canonical.get("canonicalizedFileSha256"),
            },
        ))

    candidates = manifest.get("candidates")
    if isinstance(candidates, dict):
        for platform_name in sorted(candidates.keys()):
            candidate = candidates.get(platform_name)
            if not isinstance(candidate, dict):
                continue
            declared.append((
                "candidate.{0}.raw".format(platform_name),
                candidate.get("rawPath"),
                {
                    "rawSha256": candidate.get("rawSha256"),
                    "sourceSha256": candidate.get("sourceSha256"),
                },
            ))
            declared.append((
                "candidate.{0}.canonicalized".format(platform_name),
                candidate.get("canonicalizedPath"),
                {
                    "canonicalizedFileSha256": candidate.get(
                        "canonicalizedFileSha256"
                    ),
                },
            ))

    artifacts = manifest.get("artifacts")
    if isinstance(artifacts, dict):
        for artifact_name, rel_path in sorted(artifacts.items()):
            if isinstance(rel_path, str):
                declared.append(("artifact.{0}".format(artifact_name), rel_path, {}))

    for name, rel_path, expected_hashes in declared:
        status = _validate_declared_artifact(
            run_dir,
            name,
            rel_path,
            expected_hashes=expected_hashes,
        )
        result["artifacts"][name] = status
        result["errors"].extend(status["errors"])

    result["ok"] = not result["errors"]
    return result


def _is_collector_manifest(manifest):
    return isinstance(manifest, dict) and (
        isinstance(manifest.get("sourceManifest"), dict)
        or any(
            isinstance(manifest.get(key), dict)
            for key in ("sourceManifestFile", "input", "canonical", "candidates")
        )
    )


def _host_parity_from_diff(diff_result):
    summary = diff_result.get("summary") if isinstance(diff_result, dict) else None
    if not isinstance(summary, dict):
        summary = {}

    by_platform = {}
    all_present = True
    all_match = True
    total = 0
    for platform_name in HOST_PLATFORMS:
        actual = summary.get(platform_name)
        present = isinstance(actual, dict)
        all_present = all_present and present
        if not present:
            all_match = False
            by_platform[platform_name] = {
                "present": False,
                "match": False,
                "total": None,
            }
            continue
        platform_match = bool(actual.get("match"))
        platform_total = actual.get("total")
        if not isinstance(platform_total, int) or isinstance(platform_total, bool):
            platform_total = None
            all_match = False
        else:
            total += platform_total
        all_match = all_match and platform_match
        by_platform[platform_name] = {
            "present": True,
            "match": platform_match,
            "total": platform_total,
        }

    return {
        "requiredPlatforms": list(HOST_PLATFORMS),
        "allPresent": all_present,
        "match": all_present and all_match,
        "total": total,
        "byPlatform": by_platform,
    }


def _register_open_state(register, run_id):
    blockers = register.get("blockers") if isinstance(register, dict) else None
    if not isinstance(blockers, list):
        return None, None
    breakdown = {}
    count = 0
    for entry in blockers:
        if not isinstance(entry, dict):
            continue
        if entry.get("status") != "open":
            continue
        if run_id is not None and entry.get("runId") != run_id:
            continue
        platform_name = entry.get("platform", "(unknown)")
        breakdown[platform_name] = breakdown.get(platform_name, 0) + 1
        count += 1
    return count, breakdown


def validate_collector_consistency(run_dir, manifest, diff_result):
    """Validate collector manifest summaries against packaged JSON artifacts."""
    result = {
        "checked": False,
        "ok": True,
        "errors": [],
    }
    if not _is_collector_manifest(manifest):
        return result

    result["checked"] = True
    if not isinstance(diff_result, dict):
        result["errors"].append("collector diff-result must be a JSON object")
        result["ok"] = False
        return result

    expected_diff = {
        "match": diff_result.get("match"),
        "total": diff_result.get("total"),
        "byPlatform": diff_result.get("summary"),
    }
    if manifest.get("diffSummary") != expected_diff:
        result["errors"].append(
            "collector manifest diffSummary does not match diff-result.json"
        )

    expected_host = _host_parity_from_diff(diff_result)
    if manifest.get("hostParity") != expected_host:
        result["errors"].append(
            "collector manifest hostParity does not match diff-result.json"
        )

    run_id = manifest.get("runId")
    artifacts = manifest.get("artifacts")
    register = None
    if isinstance(artifacts, dict) and isinstance(artifacts.get("blockerRegister"), str):
        try:
            register_path = _artifact_full_path(run_dir, artifacts["blockerRegister"])
            register = load_json_file(register_path)
        except PackagingError as err:
            result["errors"].append(
                "collector blocker register cannot be loaded: {0}".format(err)
            )
    else:
        result["errors"].append("collector artifacts.blockerRegister is missing")

    open_count = None
    open_breakdown = None
    if register is not None:
        open_count, open_breakdown = _register_open_state(register, run_id)
        if open_count is None:
            result["errors"].append("collector blocker register is not a valid register")
        else:
            blockers = manifest.get("blockers")
            if not isinstance(blockers, dict):
                result["errors"].append("collector manifest blockers must be an object")
            else:
                if blockers.get("open") != open_count:
                    result["errors"].append(
                        "collector manifest blockers.open does not match blocker register"
                    )
                if blockers.get("openByPlatform") != open_breakdown:
                    result["errors"].append(
                        "collector manifest blockers.openByPlatform does not match "
                        "blocker register"
                    )

    proof = manifest.get("corpusProof")
    if isinstance(proof, dict):
        conditions = proof.get("conditions")
        if isinstance(conditions, dict):
            if conditions.get("fullDiffMatch") != bool(diff_result.get("match")):
                result["errors"].append(
                    "collector corpusProof.conditions.fullDiffMatch does not match "
                    "diff-result.json"
                )
            if conditions.get("hostParityMatch") != bool(expected_host.get("match")):
                result["errors"].append(
                    "collector corpusProof.conditions.hostParityMatch does not match "
                    "hostParity"
                )
            if open_count is not None and conditions.get("openBlockers") != open_count:
                result["errors"].append(
                    "collector corpusProof.conditions.openBlockers does not match "
                    "blocker register"
                )
        if proof.get("status") == "pass":
            if not bool(diff_result.get("match")):
                result["errors"].append("collector corpusProof pass conflicts with diff mismatch")
            if not expected_host.get("match"):
                result["errors"].append(
                    "collector corpusProof pass conflicts with hostParity mismatch"
                )
            if open_count not in (None, 0):
                result["errors"].append(
                    "collector corpusProof pass conflicts with open blockers"
                )
    else:
        result["errors"].append("collector manifest corpusProof must be an object")

    result["ok"] = not result["errors"]
    return result


# --------------------------------------------------------------------------- #
# Validation
# --------------------------------------------------------------------------- #
def validate_run_dir(run_dir):
    """Validate that ``run_dir`` contains the four required artifacts as JSON.

    Returns a tuple ``(validation, loaded)`` where ``validation`` is a dict
    describing presence/validity per artifact and ``loaded`` maps the artifact
    key to its decoded JSON value (only for artifacts that parsed successfully).
    """
    if not os.path.isdir(run_dir):
        raise PackagingError("run directory does not exist: {0}".format(run_dir))

    run_dir = os.path.abspath(run_dir)
    required = {}
    missing = []
    invalid_json = []
    loaded = {}

    for key, filename in REQUIRED_ARTIFACTS:
        path = os.path.join(run_dir, filename)
        present = os.path.isfile(path)
        valid_json = False
        if present:
            try:
                loaded[key] = load_json_file(path)
                valid_json = True
            except PackagingError:
                invalid_json.append(key)
        else:
            missing.append(key)
        required[key] = {
            "filename": filename,
            "present": present,
            "validJson": valid_json,
        }

    collector_artifacts = validate_collector_artifacts(
        run_dir,
        loaded.get("manifest"),
    )
    collector_consistency = validate_collector_consistency(
        run_dir,
        loaded.get("manifest"),
        loaded.get("diff-result"),
    )
    ok = (
        not missing
        and not invalid_json
        and collector_artifacts["ok"]
        and collector_consistency["ok"]
    )
    validation = {
        "ok": ok,
        "required": required,
        "missing": missing,
        "invalidJson": invalid_json,
        "collectorArtifacts": collector_artifacts,
        "collectorConsistency": collector_consistency,
    }
    return validation, loaded


# --------------------------------------------------------------------------- #
# Summary derivation
# --------------------------------------------------------------------------- #
def derive_run_id(run_dir, manifest):
    """Determine the run id from the manifest, falling back to the dir name."""
    if isinstance(manifest, dict):
        for key in ("runId", "run_id", "id"):
            value = manifest.get(key)
            if isinstance(value, str) and value.strip():
                return value.strip()
    return os.path.basename(os.path.abspath(run_dir.rstrip(os.sep))) or "run"


def derive_diff_summary(diff_result):
    """Derive a ``{"match": bool|None, "total": int|None}`` summary from a
    diff-result document.

    Understands the shape produced by the sibling ``cross-platform-diff`` tool
    (a per-candidate ``summary`` with ``total`` counts) as well as plain
    ``{"match": bool}`` / ``{"total": int}`` documents.
    """
    match = None
    total = None

    if isinstance(diff_result, dict):
        summary = diff_result.get("summary")
        if isinstance(summary, dict) and summary:
            counted = 0
            seen = False
            for value in summary.values():
                if isinstance(value, dict) and "total" in value:
                    try:
                        counted += int(value["total"])
                        seen = True
                    except (TypeError, ValueError):
                        pass
            if seen:
                total = counted
                match = counted == 0
        elif "match" in diff_result and isinstance(diff_result["match"], bool):
            match = diff_result["match"]
        elif "total" in diff_result:
            try:
                total = int(diff_result["total"])
                match = total == 0
            except (TypeError, ValueError):
                pass

    return {"match": match, "total": total}


def derive_evidence_summary(manifest):
    """Extract release-evidence handles from a collector-style manifest.

    The full manifest is already archived in ``summary["manifest"]``. This
    compact section exists so release reviewers can quickly verify the run was
    bound to one Rust Core identity and see the four platform output hashes
    without hand-parsing the entire manifest.
    """
    if not isinstance(manifest, dict):
        return None

    source_manifest = manifest.get("sourceManifest")
    if not isinstance(source_manifest, dict):
        return None

    core_identity = source_manifest.get("coreIdentity")
    platform_runs = source_manifest.get("platformRuns")
    if not isinstance(core_identity, dict) or not isinstance(platform_runs, dict):
        return None

    candidates = manifest.get("candidates")
    if not isinstance(candidates, dict):
        candidates = {}
    host_parity = manifest.get("hostParity")
    if not isinstance(host_parity, dict):
        host_parity = None
    corpus_proof = manifest.get("corpusProof")
    if not isinstance(corpus_proof, dict):
        corpus_proof = None
    source_manifest_file = manifest.get("sourceManifestFile")
    if isinstance(source_manifest_file, dict):
        source_manifest_file = {
            "raw": source_manifest_file.get("packagePath"),
            "sourceSha256": source_manifest_file.get("sourceSha256"),
            "packageSha256": source_manifest_file.get("packageSha256"),
        }
    else:
        source_manifest_file = None

    platforms = {}
    for name in sorted(platform_runs.keys()):
        run = platform_runs.get(name)
        if not isinstance(run, dict):
            continue
        candidate = candidates.get(name)
        if not isinstance(candidate, dict):
            candidate = {}
        platforms[name] = {
            "businessKernel": run.get("businessKernel"),
            "coreCommit": run.get("coreCommit"),
            "abiVersion": run.get("abiVersion"),
            "protocolVersion": run.get("protocolVersion"),
            "raw": candidate.get("rawPath"),
            "canonicalized": candidate.get("canonicalizedPath"),
            "sourceSha256": candidate.get("sourceSha256"),
            "canonicalizedSha256": candidate.get("canonicalizedSha256"),
            "canonicalizedFileSha256": candidate.get("canonicalizedFileSha256"),
        }

    return {
        "runId": manifest.get("runId"),
        "scenario": manifest.get("scenario"),
        "coreIdentity": {
            "businessKernel": core_identity.get("businessKernel"),
            "coreCommit": core_identity.get("coreCommit"),
            "abiVersion": core_identity.get("abiVersion"),
            "protocolVersion": core_identity.get("protocolVersion"),
        },
        "sourceManifestFile": source_manifest_file,
        "hostParity": host_parity,
        "corpusProof": corpus_proof,
        "platforms": platforms,
    }


def collect_environment(run_dir, env_value):
    """Build the environment section of the summary.

    Always records the packager's own runtime environment and, if the run
    directory contains a valid ``environment.json`` object, nests it under
    ``run``.
    """
    section = {
        "packager": {
            "tool": TOOL_NAME,
            "version": TOOL_VERSION,
            "pythonVersion": platform.python_version(),
            "platform": platform.platform(),
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "processor": platform.processor() or None,
            "host": socket.gethostname(),
            "cpuCount": os.cpu_count(),
            "cwd": os.getcwd(),
        }
    }
    if isinstance(env_value, dict):
        section["run"] = env_value
    else:
        section["run"] = None
    return section


def build_summary(run_dir, validation, loaded, files, diff_summary, environment,
                  bundle_out, zip_path):
    """Assemble the ``summary.json`` document."""
    manifest = loaded.get("manifest") if isinstance(loaded, dict) else None
    run_id = derive_run_id(run_dir, manifest)
    now = datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds")
    return {
        "schemaVersion": SCHEMA_VERSION,
        "tool": TOOL_NAME,
        "version": TOOL_VERSION,
        "packagedAt": now,
        "runId": run_id,
        "runDir": os.path.abspath(run_dir),
        "runDirName": os.path.basename(os.path.abspath(run_dir.rstrip(os.sep))),
        "validation": validation,
        "manifest": manifest,
        "evidence": derive_evidence_summary(manifest),
        "diffSummary": diff_summary,
        "environment": environment,
        "files": files,
        "bundle": {
            "outDir": os.path.abspath(bundle_out),
            "zip": os.path.abspath(zip_path) if zip_path else None,
        },
    }


# --------------------------------------------------------------------------- #
# Packaging
# --------------------------------------------------------------------------- #
def _write_json(path, value):
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(value, handle, indent=2, ensure_ascii=False)
        handle.write("\n")


def _zip_directory(bundle_dir, zip_path):
    """Zip ``bundle_dir`` into ``zip_path`` with a single top-level folder."""
    bundle_dir = os.path.abspath(bundle_dir)
    top = os.path.basename(bundle_dir.rstrip(os.sep))
    with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as archive:
        for root, _dirs, files in os.walk(bundle_dir):
            for name in files:
                full = os.path.join(root, name)
                rel = os.path.relpath(full, bundle_dir).replace(os.sep, "/")
                arcname = "{0}/{1}".format(top, rel)
                archive.write(full, arcname)


def build_bundle_manifest(bundle_dir, summary):
    """Build a self-manifest for the packaged bundle.

    The manifest lists payload files, including ``summary.json``. The manifest
    and checksum sidecar are excluded to avoid a self-referential hash.
    """
    excluded = {BUNDLE_MANIFEST_NAME, BUNDLE_MANIFEST_SHA256_NAME}
    files = [
        entry for entry in list_run_files(bundle_dir)
        if entry["path"] not in excluded
    ]
    return {
        "schemaVersion": SCHEMA_VERSION,
        "tool": TOOL_NAME,
        "version": TOOL_VERSION,
        "runId": summary.get("runId"),
        "generatedAt": summary.get("packagedAt"),
        "bundleDir": os.path.abspath(bundle_dir),
        "hashAlgorithm": "sha256",
        "files": files,
        "self": {
            "manifestPath": BUNDLE_MANIFEST_NAME,
            "sha256Path": BUNDLE_MANIFEST_SHA256_NAME,
            "excludedFromFiles": sorted(excluded),
        },
    }


def _write_bundle_manifest(bundle_dir, summary):
    manifest_path = os.path.join(bundle_dir, BUNDLE_MANIFEST_NAME)
    checksum_path = os.path.join(bundle_dir, BUNDLE_MANIFEST_SHA256_NAME)
    manifest = build_bundle_manifest(bundle_dir, summary)
    _write_json(manifest_path, manifest)
    digest = sha256_of_file(manifest_path)
    with open(checksum_path, "w", encoding="utf-8") as handle:
        handle.write("{0}  {1}\n".format(digest, BUNDLE_MANIFEST_NAME))
    return {
        "path": BUNDLE_MANIFEST_NAME,
        "sha256Path": BUNDLE_MANIFEST_SHA256_NAME,
        "sha256": digest,
        "files": len(manifest["files"]),
    }


def _validate_required_corpus_proof(summary, result):
    evidence = summary.get("evidence") if isinstance(summary, dict) else None
    if not isinstance(evidence, dict):
        result["errors"].append(
            "bundle summary.json evidence.corpusProof.status is not pass"
        )
        return

    proof = evidence.get("corpusProof")
    if not isinstance(proof, dict) or proof.get("status") != "pass":
        result["errors"].append(
            "bundle summary.json evidence.corpusProof.status is not pass"
        )
        return

    host_parity = evidence.get("hostParity")
    if not isinstance(host_parity, dict) or host_parity.get("match") is not True:
        result["errors"].append(
            "bundle summary.json evidence.hostParity.match is not true"
        )

    conditions = proof.get("conditions")
    if not isinstance(conditions, dict):
        result["errors"].append(
            "bundle summary.json evidence.corpusProof.conditions must be an object"
        )
        return

    for name in REQUIRED_TRUE_CORPUS_PROOF_CONDITIONS:
        if conditions.get(name) is not True:
            result["errors"].append(
                "bundle summary.json evidence.corpusProof.conditions.{0} "
                "is not true".format(name)
            )

    if conditions.get("openBlockers") != 0:
        result["errors"].append(
            "bundle summary.json evidence.corpusProof.conditions.openBlockers "
            "is not 0"
        )
    if proof.get("reasons") != []:
        result["errors"].append(
            "bundle summary.json evidence.corpusProof.reasons must be empty"
        )
    if proof.get("missingCandidates") != []:
        result["errors"].append(
            "bundle summary.json evidence.corpusProof.missingCandidates must be empty"
        )
    if proof.get("missingSummary") != []:
        result["errors"].append(
            "bundle summary.json evidence.corpusProof.missingSummary must be empty"
        )


def _validate_required_core_commit(summary, result, required_core_commit):
    if required_core_commit is None:
        return
    if not is_core_commit_ref(required_core_commit):
        result["errors"].append(
            "required core commit must be lowercase hex with 7 to 40 characters"
        )
        return

    evidence = summary.get("evidence") if isinstance(summary, dict) else None
    if not isinstance(evidence, dict):
        result["errors"].append(
            "bundle summary.json evidence.coreIdentity.coreCommit is missing"
        )
        return

    core_identity = evidence.get("coreIdentity")
    if not isinstance(core_identity, dict):
        result["errors"].append(
            "bundle summary.json evidence.coreIdentity must be an object"
        )
    else:
        if core_identity.get("businessKernel") != BUSINESS_KERNEL:
            result["errors"].append(
                "bundle summary.json evidence.coreIdentity.businessKernel is not "
                "{0}".format(BUSINESS_KERNEL)
            )
        actual = core_identity.get("coreCommit")
        if not _commit_matches_required(actual, required_core_commit):
            result["errors"].append(
                "bundle summary.json evidence.coreIdentity.coreCommit does not "
                "match required core commit: expected {0}, got {1}".format(
                    required_core_commit,
                    actual,
                )
            )

    platforms = evidence.get("platforms")
    if not isinstance(platforms, dict):
        result["errors"].append(
            "bundle summary.json evidence.platforms must be an object"
        )
        return

    actual_platforms = set(platforms.keys())
    required_platforms = set(REQUIRED_PLATFORMS)
    missing = sorted(required_platforms - actual_platforms)
    extra = sorted(actual_platforms - required_platforms)
    if missing:
        result["errors"].append(
            "bundle summary.json evidence.platforms missing required platforms: "
            "{0}".format(", ".join(missing))
        )
    if extra:
        result["errors"].append(
            "bundle summary.json evidence.platforms has unexpected platforms: "
            "{0}".format(", ".join(extra))
        )

    for platform_name in REQUIRED_PLATFORMS:
        platform = platforms.get(platform_name)
        if not isinstance(platform, dict):
            continue
        if platform.get("businessKernel") != BUSINESS_KERNEL:
            result["errors"].append(
                "bundle summary.json evidence.platforms.{0}.businessKernel is not "
                "{1}".format(platform_name, BUSINESS_KERNEL)
            )
        actual = platform.get("coreCommit")
        if not _commit_matches_required(actual, required_core_commit):
            result["errors"].append(
                "bundle summary.json evidence.platforms.{0}.coreCommit does not "
                "match required core commit: expected {1}, got {2}".format(
                    platform_name,
                    required_core_commit,
                    actual,
                )
            )


def _validate_required_run_binding(
    summary,
    result,
    required_run_id=None,
    required_scenario=None,
):
    if required_run_id is not None:
        actual = summary.get("runId") if isinstance(summary, dict) else None
        if actual != required_run_id:
            result["errors"].append(
                "bundle summary.json runId does not match required runId: "
                "expected {0}, got {1}".format(required_run_id, actual)
            )

    if required_scenario is None:
        return

    manifest = summary.get("manifest") if isinstance(summary, dict) else None
    evidence = summary.get("evidence") if isinstance(summary, dict) else None
    manifest_scenario = manifest.get("scenario") if isinstance(manifest, dict) else None
    evidence_scenario = evidence.get("scenario") if isinstance(evidence, dict) else None
    if manifest_scenario != required_scenario:
        result["errors"].append(
            "bundle summary.json manifest.scenario does not match required "
            "scenario: expected {0}, got {1}".format(
                required_scenario,
                manifest_scenario,
            )
        )
    if evidence_scenario != required_scenario:
        result["errors"].append(
            "bundle summary.json evidence.scenario does not match required "
            "scenario: expected {0}, got {1}".format(
                required_scenario,
                evidence_scenario,
            )
        )


def _validate_bundle_summary(
    summary,
    bundle_manifest,
    result,
    require_corpus_proof_pass=False,
    required_core_commit=None,
    required_run_id=None,
    required_scenario=None,
):
    if not isinstance(summary, dict):
        result["errors"].append("bundle summary.json must be an object")
        return
    if summary.get("tool") != TOOL_NAME:
        result["errors"].append("bundle summary.json tool does not match packager")
    if summary.get("runId") != bundle_manifest.get("runId"):
        result["errors"].append("bundle summary.json runId does not match bundle manifest")

    validation = summary.get("validation")
    if not isinstance(validation, dict):
        result["errors"].append("bundle summary.json validation must be an object")
    elif validation.get("ok") is not True:
        result["errors"].append("bundle summary.json validation.ok is not true")
    else:
        collector_artifacts = validation.get("collectorArtifacts")
        if isinstance(collector_artifacts, dict):
            if collector_artifacts.get("checked") and collector_artifacts.get("ok") is not True:
                result["errors"].append(
                    "bundle summary.json collectorArtifacts validation is not true"
                )
        collector_consistency = validation.get("collectorConsistency")
        if isinstance(collector_consistency, dict):
            if collector_consistency.get("checked") and collector_consistency.get("ok") is not True:
                result["errors"].append(
                    "bundle summary.json collectorConsistency validation is not true"
                )

    bundle = summary.get("bundle")
    if not isinstance(bundle, dict):
        result["errors"].append("bundle summary.json bundle must be an object")
        return
    manifest_ref = bundle.get("manifest")
    if not isinstance(manifest_ref, dict):
        result["errors"].append("bundle summary.json bundle.manifest must be an object")
        return
    if manifest_ref.get("path") != BUNDLE_MANIFEST_NAME:
        result["errors"].append("bundle summary.json bundle.manifest.path mismatch")
    if manifest_ref.get("sha256Path") != BUNDLE_MANIFEST_SHA256_NAME:
        result["errors"].append("bundle summary.json bundle.manifest.sha256Path mismatch")
    if require_corpus_proof_pass:
        _validate_required_corpus_proof(summary, result)
    if required_core_commit is not None:
        _validate_required_core_commit(summary, result, required_core_commit)
    if required_run_id is not None or required_scenario is not None:
        _validate_required_run_binding(
            summary,
            result,
            required_run_id=required_run_id,
            required_scenario=required_scenario,
        )


def _append_payload_validation_errors(validation, result):
    if validation.get("ok") is True:
        return
    result["errors"].append("bundle payload validation.ok is not true")
    for key in ("missing", "invalidJson"):
        values = validation.get(key)
        if values:
            result["errors"].append(
                "bundle payload validation {0}: {1}".format(
                    key,
                    ", ".join(values),
                )
            )
    for section_name in ("collectorArtifacts", "collectorConsistency"):
        section = validation.get(section_name)
        if isinstance(section, dict):
            for error in section.get("errors") or []:
                result["errors"].append(
                    "bundle payload {0}: {1}".format(section_name, error)
                )


def _validate_bundle_payload_consistency(bundle_dir, summary, result):
    """Recompute payload validation from bundle files instead of trusting summary."""
    if not isinstance(summary, dict):
        return
    try:
        validation, loaded = validate_run_dir(bundle_dir)
    except PackagingError as err:
        result["errors"].append("bundle payload validation failed: {0}".format(err))
        return

    _append_payload_validation_errors(validation, result)
    if summary.get("validation") != validation:
        result["errors"].append(
            "bundle summary.json validation does not match payload validation"
        )

    manifest = loaded.get("manifest")
    if summary.get("manifest") != manifest:
        result["errors"].append(
            "bundle summary.json manifest does not match payload manifest.json"
        )

    diff_summary = derive_diff_summary(loaded.get("diff-result"))
    if summary.get("diffSummary") != diff_summary:
        result["errors"].append(
            "bundle summary.json diffSummary does not match payload diff-result.json"
        )

    evidence = derive_evidence_summary(manifest)
    if summary.get("evidence") != evidence:
        result["errors"].append(
            "bundle summary.json evidence does not match payload manifest.json"
        )


def _validate_zip_payload_consistency(archive, prefix, summary, result):
    with tempfile.TemporaryDirectory(prefix="brp-verify-", dir=PRIVATE_TMP) as temp_dir:
        for info in archive.infolist():
            if info.is_dir():
                continue
            try:
                rel_path = _zip_rel_path(info.filename, prefix)
            except PackagingError as err:
                result["errors"].append(str(err))
                continue
            if rel_path is None:
                continue
            full = os.path.join(temp_dir, rel_path)
            os.makedirs(os.path.dirname(full), exist_ok=True)
            with open(full, "wb") as handle:
                handle.write(archive.read(info.filename))
        _validate_bundle_payload_consistency(temp_dir, summary, result)


def verify_bundle_manifest(
    bundle_dir,
    require_corpus_proof_pass=False,
    required_core_commit=None,
    required_run_id=None,
    required_scenario=None,
):
    """Verify a bundle against ``bundle-manifest.json`` and its checksum."""
    bundle_dir = os.path.abspath(bundle_dir)
    manifest_path = os.path.join(bundle_dir, BUNDLE_MANIFEST_NAME)
    checksum_path = os.path.join(bundle_dir, BUNDLE_MANIFEST_SHA256_NAME)
    result = {
        "ok": True,
        "errors": [],
        "manifestSha256": None,
        "filesChecked": 0,
        "requiredCorpusProofPass": bool(require_corpus_proof_pass),
        "requiredCoreCommit": required_core_commit,
        "requiredRunId": required_run_id,
        "requiredScenario": required_scenario,
    }
    if not os.path.isfile(manifest_path):
        result["errors"].append("bundle manifest missing")
    if not os.path.isfile(checksum_path):
        result["errors"].append("bundle manifest checksum missing")
    if result["errors"]:
        result["ok"] = False
        return result

    manifest_sha = sha256_of_file(manifest_path)
    result["manifestSha256"] = manifest_sha
    try:
        with open(checksum_path, "r", encoding="utf-8") as handle:
            checksum_line = handle.read().strip()
    except (OSError, IOError) as err:
        result["errors"].append("cannot read bundle manifest checksum: {0}".format(err))
        result["ok"] = False
        return result

    expected_line = "{0}  {1}".format(manifest_sha, BUNDLE_MANIFEST_NAME)
    if checksum_line != expected_line:
        result["errors"].append("bundle manifest checksum mismatch")

    try:
        manifest = load_json_file(manifest_path)
    except PackagingError as err:
        result["errors"].append(str(err))
        result["ok"] = False
        return result
    if not isinstance(manifest, dict):
        result["errors"].append("bundle manifest must be an object")
        result["ok"] = False
        return result

    declared_files = manifest.get("files")
    if not isinstance(declared_files, list):
        result["errors"].append("bundle manifest files must be a list")
        result["ok"] = False
        return result
    excluded = {BUNDLE_MANIFEST_NAME, BUNDLE_MANIFEST_SHA256_NAME}
    actual = {
        entry["path"]: entry
        for entry in list_run_files(bundle_dir)
        if entry["path"] not in excluded
    }
    declared_paths = set()
    for entry in declared_files:
        if not isinstance(entry, dict):
            result["errors"].append("bundle manifest file entry must be an object")
            continue
        rel_path = entry.get("path")
        declared_paths.add(rel_path)
        if rel_path not in actual:
            result["errors"].append("bundle manifest file missing: {0}".format(rel_path))
            continue
        actual_entry = actual[rel_path]
        if entry.get("size") != actual_entry["size"]:
            result["errors"].append("bundle manifest size mismatch: {0}".format(rel_path))
        if entry.get("sha256") != actual_entry["sha256"]:
            result["errors"].append(
                "bundle manifest sha256 mismatch: {0}".format(rel_path)
            )
        result["filesChecked"] += 1

    for rel_path in sorted(set(actual.keys()) - declared_paths):
        result["errors"].append("bundle manifest missing extra file: {0}".format(rel_path))

    summary_path = os.path.join(bundle_dir, "summary.json")
    if not os.path.isfile(summary_path):
        result["errors"].append("bundle summary.json missing")
    else:
        try:
            summary = load_json_file(summary_path)
        except PackagingError as err:
            result["errors"].append(str(err))
        else:
            _validate_bundle_summary(
                summary,
                manifest,
                result,
                require_corpus_proof_pass=require_corpus_proof_pass,
                required_core_commit=required_core_commit,
                required_run_id=required_run_id,
                required_scenario=required_scenario,
            )
            _validate_bundle_payload_consistency(bundle_dir, summary, result)

    result["ok"] = not result["errors"]
    return result


def _zip_rel_path(name, prefix):
    if not name.startswith(prefix):
        return None
    rel = name[len(prefix):]
    if not rel or rel.endswith("/"):
        return None
    if rel.startswith("/") or rel == "." or rel.startswith("../") or "/../" in rel:
        raise PackagingError("zip entry escapes bundle root: {0}".format(name))
    return rel


def verify_bundle_zip(
    zip_path,
    require_corpus_proof_pass=False,
    required_core_commit=None,
    required_run_id=None,
    required_scenario=None,
):
    """Verify a zipped bundle, including a disposable payload consistency check."""
    zip_path = os.path.abspath(zip_path)
    result = {
        "ok": True,
        "errors": [],
        "manifestSha256": None,
        "filesChecked": 0,
        "sourceType": "zip",
        "sourcePath": zip_path,
        "requiredCorpusProofPass": bool(require_corpus_proof_pass),
        "requiredCoreCommit": required_core_commit,
        "requiredRunId": required_run_id,
        "requiredScenario": required_scenario,
    }
    if not zipfile.is_zipfile(zip_path):
        result["errors"].append("bundle zip is not a valid zip archive")
        result["ok"] = False
        return result

    try:
        with zipfile.ZipFile(zip_path, "r") as archive:
            names = [name for name in archive.namelist() if not name.endswith("/")]
            top_levels = {
                name.split("/", 1)[0]
                for name in names
                if "/" in name and name.split("/", 1)[0]
            }
            if len(top_levels) != 1:
                result["errors"].append("bundle zip must contain one top-level directory")
                result["ok"] = False
                return result
            prefix = next(iter(top_levels)) + "/"
            manifest_name = prefix + BUNDLE_MANIFEST_NAME
            checksum_name = prefix + BUNDLE_MANIFEST_SHA256_NAME
            if manifest_name not in names:
                result["errors"].append("bundle manifest missing")
            if checksum_name not in names:
                result["errors"].append("bundle manifest checksum missing")
            if result["errors"]:
                result["ok"] = False
                return result

            manifest_bytes = archive.read(manifest_name)
            manifest_sha = hashlib.sha256(manifest_bytes).hexdigest()
            result["manifestSha256"] = manifest_sha
            checksum_line = archive.read(checksum_name).decode("utf-8").strip()
            expected_line = "{0}  {1}".format(manifest_sha, BUNDLE_MANIFEST_NAME)
            if checksum_line != expected_line:
                result["errors"].append("bundle manifest checksum mismatch")
            try:
                manifest = json.loads(manifest_bytes.decode("utf-8"))
            except (UnicodeDecodeError, json.JSONDecodeError) as err:
                result["errors"].append("invalid JSON in bundle manifest: {0}".format(err))
                result["ok"] = False
                return result
            if not isinstance(manifest, dict):
                result["errors"].append("bundle manifest must be an object")
                result["ok"] = False
                return result
            declared_files = manifest.get("files")
            if not isinstance(declared_files, list):
                result["errors"].append("bundle manifest files must be a list")
                result["ok"] = False
                return result

            excluded = {BUNDLE_MANIFEST_NAME, BUNDLE_MANIFEST_SHA256_NAME}
            actual = {}
            for info in archive.infolist():
                if info.is_dir():
                    continue
                try:
                    rel_path = _zip_rel_path(info.filename, prefix)
                except PackagingError as err:
                    result["errors"].append(str(err))
                    continue
                if rel_path is None or rel_path in excluded:
                    continue
                data = archive.read(info.filename)
                actual[rel_path] = {
                    "path": rel_path,
                    "size": len(data),
                    "sha256": hashlib.sha256(data).hexdigest(),
                }

            declared_paths = set()
            for entry in declared_files:
                if not isinstance(entry, dict):
                    result["errors"].append("bundle manifest file entry must be an object")
                    continue
                rel_path = entry.get("path")
                declared_paths.add(rel_path)
                if rel_path not in actual:
                    result["errors"].append(
                        "bundle manifest file missing: {0}".format(rel_path)
                    )
                    continue
                actual_entry = actual[rel_path]
                if entry.get("size") != actual_entry["size"]:
                    result["errors"].append(
                        "bundle manifest size mismatch: {0}".format(rel_path)
                    )
                if entry.get("sha256") != actual_entry["sha256"]:
                    result["errors"].append(
                        "bundle manifest sha256 mismatch: {0}".format(rel_path)
                    )
                result["filesChecked"] += 1

            for rel_path in sorted(set(actual.keys()) - declared_paths):
                result["errors"].append(
                    "bundle manifest missing extra file: {0}".format(rel_path)
                )

            summary_name = prefix + "summary.json"
            if summary_name not in names:
                result["errors"].append("bundle summary.json missing")
            else:
                try:
                    summary = json.loads(archive.read(summary_name).decode("utf-8"))
                except (UnicodeDecodeError, json.JSONDecodeError) as err:
                    result["errors"].append(
                        "invalid JSON in bundle summary.json: {0}".format(err)
                    )
                else:
                    _validate_bundle_summary(
                        summary,
                        manifest,
                        result,
                        require_corpus_proof_pass=require_corpus_proof_pass,
                        required_core_commit=required_core_commit,
                        required_run_id=required_run_id,
                        required_scenario=required_scenario,
                    )
                    _validate_zip_payload_consistency(
                        archive,
                        prefix,
                        summary,
                        result,
                    )
    except (OSError, IOError, zipfile.BadZipFile) as err:
        result["errors"].append("cannot read bundle zip: {0}".format(err))

    result["ok"] = not result["errors"]
    return result


def verify_bundle_path(
    path,
    require_corpus_proof_pass=False,
    required_core_commit=None,
    required_run_id=None,
    required_scenario=None,
):
    path = os.path.abspath(path)
    if os.path.isdir(path):
        result = verify_bundle_manifest(
            path,
            require_corpus_proof_pass=require_corpus_proof_pass,
            required_core_commit=required_core_commit,
            required_run_id=required_run_id,
            required_scenario=required_scenario,
        )
        result["sourceType"] = "directory"
        result["sourcePath"] = path
        return result
    if os.path.isfile(path):
        return verify_bundle_zip(
            path,
            require_corpus_proof_pass=require_corpus_proof_pass,
            required_core_commit=required_core_commit,
            required_run_id=required_run_id,
            required_scenario=required_scenario,
        )
    return {
        "ok": False,
        "errors": ["bundle path does not exist: {0}".format(path)],
        "manifestSha256": None,
        "filesChecked": 0,
        "sourceType": None,
        "sourcePath": path,
        "requiredCorpusProofPass": bool(require_corpus_proof_pass),
        "requiredCoreCommit": required_core_commit,
        "requiredRunId": required_run_id,
        "requiredScenario": required_scenario,
    }


def package_run(run_dir, out_dir=None, make_zip=False, zip_path=None):
    """Validate and package a benchmark run directory.

    ``run_dir`` is the source run directory. ``out_dir`` overrides the bundle
    directory (default ``/private/tmp/<run-id>-bundle``). When ``make_zip`` is
    true, a zip is also produced; ``zip_path`` overrides its location (default
    ``/private/tmp/<run-id>-bundle.zip``).

    Returns the generated summary dict. Raises :class:`PackagingError` on any
    validation or IO failure.
    """
    if not os.path.isdir(run_dir):
        raise PackagingError("run directory does not exist: {0}".format(run_dir))
    run_dir = os.path.abspath(run_dir)

    validation, loaded = validate_run_dir(run_dir)
    if not validation["ok"]:
        problems = []
        problems.extend("{0} missing".format(k) for k in validation["missing"])
        problems.extend("{0} not valid JSON".format(k) for k in validation["invalidJson"])
        problems.extend(validation["collectorArtifacts"].get("errors", []))
        problems.extend(validation["collectorConsistency"].get("errors", []))
        raise PackagingError(
            "run directory failed validation ({0}): {1}".format(
                run_dir, "; ".join(problems)
            )
        )

    manifest = loaded.get("manifest")
    run_id = derive_run_id(run_dir, manifest)
    safe_id = sanitize_for_path(run_id)

    # Resolve + guard the bundle directory.
    if out_dir is None:
        bundle_out = os.path.join(PRIVATE_TMP, "{0}-bundle".format(safe_id))
        bundle_user_specified = False
    else:
        bundle_out = os.path.abspath(out_dir)
        bundle_user_specified = True
    assert_safe_output(bundle_out, bundle_user_specified)

    # Resolve + guard the zip path.
    resolved_zip_path = None
    if make_zip:
        if zip_path is None:
            resolved_zip_path = os.path.join(
                PRIVATE_TMP, "{0}-bundle.zip".format(safe_id)
            )
            zip_user_specified = False
        else:
            resolved_zip_path = os.path.abspath(zip_path)
            zip_user_specified = True
        assert_safe_output(resolved_zip_path, zip_user_specified)

    # Fresh bundle directory (output lives in /private/tmp or a user-chosen
    # disposable location, so overwriting a previous bundle is safe).
    if os.path.exists(bundle_out):
        if os.path.isdir(bundle_out):
            shutil.rmtree(bundle_out)
        else:
            os.remove(bundle_out)
    shutil.copytree(run_dir, bundle_out)

    # Build the summary from the *source* run directory inventory.
    files = list_run_files(run_dir)
    diff_summary = derive_diff_summary(loaded.get("diff-result"))
    environment = collect_environment(run_dir, _load_optional_json(run_dir, "environment.json"))
    summary = build_summary(
        run_dir,
        validation,
        loaded,
        files,
        diff_summary,
        environment,
        bundle_out,
        resolved_zip_path,
    )
    summary["bundle"]["manifest"] = {
        "path": BUNDLE_MANIFEST_NAME,
        "sha256Path": BUNDLE_MANIFEST_SHA256_NAME,
    }

    # Write summary.json into the bundle, then generate a manifest for the
    # final payload files. The returned summary includes the manifest digest;
    # the on-disk summary intentionally points to the manifest/checksum files
    # without embedding their self-referential hash.
    _write_json(os.path.join(bundle_out, "summary.json"), summary)
    bundle_manifest = _write_bundle_manifest(bundle_out, summary)
    bundle_validation = verify_bundle_manifest(bundle_out)
    if not bundle_validation["ok"]:
        raise PackagingError(
            "bundle manifest validation failed ({0}): {1}".format(
                bundle_out,
                "; ".join(bundle_validation["errors"]),
            )
        )
    summary["bundle"]["manifest"] = bundle_manifest

    if resolved_zip_path is not None:
        if os.path.exists(resolved_zip_path):
            os.remove(resolved_zip_path)
        _zip_directory(bundle_out, resolved_zip_path)

    return summary


# --------------------------------------------------------------------------- #
# Rendering + CLI
# --------------------------------------------------------------------------- #
def render_summary(summary):
    """Render a human-readable text summary of a packaging result."""
    lines = [
        "Benchmark run packager summary",
        "  tool: {0} v{1}".format(summary["tool"], summary["version"]),
        "  runId: {0}".format(summary["runId"]),
        "  runDir: {0}".format(summary["runDir"]),
        "  packagedAt: {0}".format(summary["packagedAt"]),
        "  validation: {0}".format(
            "OK" if summary["validation"]["ok"] else "FAILED"
        ),
    ]
    for key, info in summary["validation"]["required"].items():
        state = "missing"
        if info["present"] and info["validJson"]:
            state = "ok"
        elif info["present"]:
            state = "invalid-json"
        lines.append("    {0}: {1}".format(key, state))

    diff = summary["diffSummary"]
    if diff["match"] is None:
        diff_text = "unknown"
    else:
        diff_text = "match" if diff["match"] else "mismatch"
    if diff["total"] is not None:
        diff_text += " (total differences: {0})".format(diff["total"])
    lines.append("  diff: {0}".format(diff_text))

    evidence = summary.get("evidence")
    if isinstance(evidence, dict):
        core_identity = evidence.get("coreIdentity")
        if isinstance(core_identity, dict):
            lines.append(
                "  core: {0} {1} (abi {2}, protocol {3})".format(
                    core_identity.get("businessKernel"),
                    core_identity.get("coreCommit"),
                    core_identity.get("abiVersion"),
                    core_identity.get("protocolVersion"),
                )
            )
        platforms = evidence.get("platforms")
        if isinstance(platforms, dict) and platforms:
            lines.append(
                "  platforms: {0}".format(", ".join(sorted(platforms.keys())))
            )
        host_parity = evidence.get("hostParity")
        if isinstance(host_parity, dict):
            lines.append(
                "  host parity: {0} (total differences: {1})".format(
                    "match" if host_parity.get("match") else "mismatch",
                    host_parity.get("total"),
                )
            )
        corpus_proof = evidence.get("corpusProof")
        if isinstance(corpus_proof, dict):
            lines.append("  corpus proof: {0}".format(
                corpus_proof.get("status", "unknown")
            ))

    lines.append("  files packaged: {0}".format(len(summary["files"])))
    lines.append("  bundle dir: {0}".format(summary["bundle"]["outDir"]))
    manifest = summary["bundle"].get("manifest")
    if isinstance(manifest, dict):
        lines.append("  bundle manifest: {0}".format(manifest.get("path")))
    if summary["bundle"]["zip"]:
        lines.append("  zip: {0}".format(summary["bundle"]["zip"]))
    else:
        lines.append("  zip: (not created)")
    return "\n".join(lines) + "\n"


def render_bundle_verification(path, result):
    lines = [
        "Benchmark bundle verification",
        "  path: {0}".format(os.path.abspath(path)),
        "  type: {0}".format(result.get("sourceType") or "unknown"),
        "  status: {0}".format("OK" if result.get("ok") else "FAILED"),
        "  files checked: {0}".format(result.get("filesChecked", 0)),
    ]
    if result.get("requiredCorpusProofPass"):
        lines.append("  required corpus proof: pass")
    if result.get("requiredCoreCommit"):
        lines.append("  required core commit: {0}".format(result["requiredCoreCommit"]))
    if result.get("requiredRunId"):
        lines.append("  required runId: {0}".format(result["requiredRunId"]))
    if result.get("requiredScenario"):
        lines.append("  required scenario: {0}".format(result["requiredScenario"]))
    if result.get("manifestSha256"):
        lines.append("  manifest sha256: {0}".format(result["manifestSha256"]))
    errors = result.get("errors") or []
    if errors:
        lines.append("  errors:")
        for error in errors:
            lines.append("    - {0}".format(error))
    return "\n".join(lines) + "\n"


def parse_args(argv):
    parser = argparse.ArgumentParser(
        prog=TOOL_NAME,
        description=(
            "Package a benchmark run directory (manifest + platform/canonical/"
            "diff results + logs + environment) into an archivable bundle and "
            "optional zip. Does not run any benchmark."
        ),
    )
    parser.add_argument(
        "run_dir",
        nargs="?",
        help="Path to the run directory to package.",
    )
    parser.add_argument(
        "--verify-bundle",
        dest="verify_bundle",
        default=None,
        help=(
            "Verify an existing bundle directory or bundle zip using "
            "bundle-manifest.json and bundle-manifest.sha256, then exit."
        ),
    )
    parser.add_argument(
        "--require-corpus-proof-pass",
        action="store_true",
        help=(
            "When used with --verify-bundle, require summary.json evidence to "
            "declare corpusProof.status=pass, host parity match, full diff match, "
            "and zero open blockers."
        ),
    )
    parser.add_argument(
        "--require-core-commit",
        dest="required_core_commit",
        default=None,
        help=(
            "When used with --verify-bundle, require summary.json evidence to "
            "bind coreIdentity and all cli/ios/android/harmony platform runs to "
            "this Rust Core commit (7-40 lowercase hex characters)."
        ),
    )
    parser.add_argument(
        "--require-run-id",
        dest="required_run_id",
        default=None,
        help=(
            "When used with --verify-bundle, require summary.json runId to match "
            "this corpus run id."
        ),
    )
    parser.add_argument(
        "--require-scenario",
        dest="required_scenario",
        default=None,
        help=(
            "When used with --verify-bundle, require collector manifest/evidence "
            "scenario to match this corpus scenario."
        ),
    )
    parser.add_argument(
        "--out",
        dest="out_dir",
        default=None,
        help=(
            "Bundle output directory. Default: /private/tmp/<run-id>-bundle. "
            "Must not be under ~/Documents."
        ),
    )
    parser.add_argument(
        "--zip",
        dest="zip_path",
        nargs="?",
        const="",
        default=None,
        help=(
            "Also produce a zip archive. Pass an explicit path to override the "
            "default (/private/tmp/<run-id>-bundle.zip). Must not be under "
            "~/Documents."
        ),
    )
    return parser.parse_args(argv)


def main(argv=None):
    if argv is None:
        argv = sys.argv[1:]
    args = parse_args(argv)

    if args.verify_bundle is not None:
        result = verify_bundle_path(
            args.verify_bundle,
            require_corpus_proof_pass=args.require_corpus_proof_pass,
            required_core_commit=args.required_core_commit,
            required_run_id=args.required_run_id,
            required_scenario=args.required_scenario,
        )
        sys.stdout.write(render_bundle_verification(args.verify_bundle, result))
        return 0 if result["ok"] else 2

    if args.require_corpus_proof_pass:
        sys.stderr.write(
            "error: --require-corpus-proof-pass requires --verify-bundle\n"
        )
        return 2
    if args.required_core_commit is not None:
        sys.stderr.write("error: --require-core-commit requires --verify-bundle\n")
        return 2
    if args.required_run_id is not None:
        sys.stderr.write("error: --require-run-id requires --verify-bundle\n")
        return 2
    if args.required_scenario is not None:
        sys.stderr.write("error: --require-scenario requires --verify-bundle\n")
        return 2

    if args.run_dir is None:
        sys.stderr.write("error: run_dir is required unless --verify-bundle is used\n")
        return 2

    make_zip = args.zip_path is not None
    zip_path = args.zip_path if args.zip_path else None

    try:
        summary = package_run(
            args.run_dir,
            out_dir=args.out_dir,
            make_zip=make_zip,
            zip_path=zip_path,
        )
    except PackagingError as err:
        sys.stderr.write("error: {0}\n".format(err))
        return 2

    sys.stdout.write(render_summary(summary))
    return 0


if __name__ == "__main__":
    sys.exit(main())
