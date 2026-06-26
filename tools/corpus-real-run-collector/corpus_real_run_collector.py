#!/usr/bin/env python3
"""Corpus real-run collector.

Collects already-produced CLI / iOS / Android / HarmonyOS JSON result files
into one candidate package, then wires that package through the existing
corpus canonicalizer, cross-platform diff, and release blocker register.

This tool does not run Core business logic, does not run platform adapters,
and does not certify a release. It only copies local run artifacts, emits a
packager-compatible run directory, validates source-manifest provenance when
provided, and records diff-derived blockers.

Package layout::

    <out>/
      manifest.json
      platform-result.json
      canonical-result.json
      diff-result.json
      environment.json
      corpus-blocker-register.json
      input.json
      raw/
        cli-result.json
        ios-result.json
        android-result.json
        harmony-result.json
        canonical-result.json
      candidates/
        cli-result.json
        ios-result.json
        android-result.json
        harmony-result.json

Usage::

    python3 tools/corpus-real-run-collector/corpus_real_run_collector.py \\
      --run-id 2026-06-25-real-001 \\
      --scenario authorized-corpus-search \\
      --input input.json \\
      --canonical canonical-result.json \\
      --candidate cli:cli-result.json \\
      --candidate ios:ios-result.json \\
      --candidate android:android-result.json \\
      --candidate harmony:harmony-result.json
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


_HERE = os.path.dirname(os.path.abspath(__file__))
_ROOT = os.path.abspath(os.path.join(_HERE, "..", ".."))
sys.path.insert(0, os.path.join(_ROOT, "scripts"))
sys.path.insert(0, os.path.join(_ROOT, "tools", "cross-platform-diff"))
sys.path.insert(0, os.path.join(_ROOT, "tools", "release-blocker-register"))

import corpus_canonicalize as cc  # noqa: E402
import cross_platform_diff as cpd  # noqa: E402
import release_blocker_register as rbr  # noqa: E402


TOOL_NAME = "corpus-real-run-collector"
TOOL_VERSION = "1.0"
SCHEMA_VERSION = 1

PRIVATE_TMP = "/private/tmp"
REQUIRED_PLATFORMS = ("cli", "ios", "android", "harmony")
HOST_PLATFORMS = ("ios", "android", "harmony")
BUSINESS_KERNEL = "reader-core-native-rust"
_PLATFORM_RE = re.compile(r"^[a-z][a-z0-9_.-]*$")
_CORE_COMMIT_RE = re.compile(r"^[0-9a-f]{7,40}$")
_SHA256_RE = re.compile(r"^[0-9a-f]{64}$")


class CollectorError(Exception):
    """Raised when the collector cannot validate inputs or write outputs."""


def _now_iso():
    return datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds")


def _documents_dir():
    return os.path.realpath(os.path.expanduser("~/Documents"))


def _is_under(path, base):
    path = os.path.realpath(path)
    base = os.path.realpath(base)
    try:
        return os.path.commonpath([path, base]) == base
    except ValueError:
        return False


def sanitize_for_path(name):
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


def assert_safe_output(path, user_specified):
    if not user_specified and not _is_under(path, PRIVATE_TMP):
        raise CollectorError(
            "default output must live under {0}; got {1}".format(PRIVATE_TMP, path)
        )
    if _is_under(path, _documents_dir()):
        raise CollectorError(
            "refusing to write output under ~/Documents ({0}); "
            "use /private/tmp or another disposable path".format(path)
        )


def resolve_output_path(run_id, out_dir):
    if out_dir is None:
        path = os.path.join(PRIVATE_TMP, "{0}-candidate".format(sanitize_for_path(run_id)))
        user_specified = False
    else:
        path = os.path.abspath(out_dir)
        user_specified = True
    assert_safe_output(path, user_specified)
    return path


def sha256_of_file(path, chunk_size=65536):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        while True:
            chunk = handle.read(chunk_size)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def _write_json(path, value):
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(value, handle, indent=2, ensure_ascii=False, sort_keys=True)
        handle.write("\n")


def _load_json(path, label):
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return json.load(handle)
    except FileNotFoundError:
        raise CollectorError("{0} file not found: {1}".format(label, path))
    except (OSError, IOError) as err:
        raise CollectorError("cannot read {0} {1}: {2}".format(label, path, err))
    except json.JSONDecodeError as err:
        raise CollectorError("invalid JSON in {0} {1}: {2}".format(label, path, err))


def _copy_file(src, dst):
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    shutil.copyfile(src, dst)


def parse_candidate_spec(spec):
    if ":" not in spec:
        raise CollectorError(
            "candidate must use NAME:PATH form; got {0}".format(spec)
        )
    name, path = spec.split(":", 1)
    name = name.strip()
    path = path.strip()
    if not name:
        raise CollectorError("empty candidate platform in spec: {0}".format(spec))
    if not _PLATFORM_RE.match(name):
        raise CollectorError("invalid candidate platform name: {0}".format(name))
    if not path:
        raise CollectorError("empty candidate path for platform: {0}".format(name))
    return name, path


def parse_candidate_specs(specs, required_platforms=REQUIRED_PLATFORMS):
    candidates = {}
    for spec in specs:
        name, path = parse_candidate_spec(spec)
        if name in candidates:
            raise CollectorError("duplicate candidate platform: {0}".format(name))
        candidates[name] = os.path.abspath(path)

    required = set(required_platforms)
    seen = set(candidates.keys())
    missing = sorted(required - seen)
    extra = sorted(seen - required)
    if missing:
        raise CollectorError("missing required candidate platform(s): " + ", ".join(missing))
    if extra:
        raise CollectorError("unknown candidate platform(s): " + ", ".join(extra))
    return {name: candidates[name] for name in required_platforms}


def canonicalize_file(src, dst, label):
    data = _load_json(src, label)
    _write_json(dst, cc.canonicalize(data))
    return {
        "sourcePath": os.path.abspath(src),
        "packagePath": os.path.relpath(dst, os.path.dirname(os.path.dirname(dst))).replace(os.sep, "/"),
        "sourceSha256": sha256_of_file(src),
        "packageSha256": sha256_of_file(dst),
    }


def _artifact_rel(out_dir, path):
    return os.path.relpath(path, out_dir).replace(os.sep, "/")


def build_platform_result(run_id, scenario, input_rel, candidate_records):
    return {
        "schemaVersion": SCHEMA_VERSION,
        "tool": TOOL_NAME,
        "version": TOOL_VERSION,
        "type": "four-platform-real-run-candidate-package",
        "runId": run_id,
        "scenario": scenario,
        "input": input_rel,
        "platforms": {
            platform: {
                "raw": record["rawPath"],
                "canonicalized": record["canonicalizedPath"],
                "sourcePath": record["sourcePath"],
                "sourceSha256": record["sourceSha256"],
                "canonicalizedSha256": record["canonicalizedSha256"],
                "canonicalizedFileSha256": record["canonicalizedFileSha256"],
            }
            for platform, record in candidate_records.items()
        },
    }


def build_environment(cwd):
    return {
        "collector": {
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
            "cwd": cwd,
        }
    }


def _load_optional_source_manifest(path):
    if path is None:
        return None
    return _load_json(path, "source manifest")


def _require_object(parent, key, label):
    value = parent.get(key) if isinstance(parent, dict) else None
    if not isinstance(value, dict):
        raise CollectorError("source manifest {0} must be an object".format(label))
    return value


def _require_string(parent, key, label):
    value = parent.get(key) if isinstance(parent, dict) else None
    if not isinstance(value, str) or not value.strip():
        raise CollectorError("source manifest {0} must be a non-empty string".format(label))
    return value.strip()


def _require_positive_int(parent, key, label):
    value = parent.get(key) if isinstance(parent, dict) else None
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise CollectorError("source manifest {0} must be a positive integer".format(label))
    return value


def _manifest_artifact_path(source_manifest_path, value, label):
    if not isinstance(value, str) or not value.strip():
        raise CollectorError("source manifest {0} must be a non-empty path".format(label))
    if os.path.isabs(value):
        return os.path.realpath(value)
    return os.path.realpath(os.path.join(os.path.dirname(source_manifest_path), value))


def _assert_manifest_path(source_manifest_path, manifest_value, actual_path, label):
    expected = _manifest_artifact_path(source_manifest_path, manifest_value, label)
    actual = os.path.realpath(actual_path)
    if expected != actual:
        raise CollectorError(
            "source manifest {0} path mismatch: expected {1}, got {2}".format(
                label,
                expected,
                actual,
            )
        )


def _assert_manifest_sha256(manifest_value, actual_path, label):
    if not isinstance(manifest_value, str) or not _SHA256_RE.match(manifest_value):
        raise CollectorError(
            "source manifest {0} sha256 must be a 64 character lowercase hex digest".format(
                label
            )
        )
    actual = sha256_of_file(actual_path)
    if manifest_value != actual:
        raise CollectorError(
            "source manifest {0} sha256 mismatch: expected {1}, got {2}".format(
                label,
                manifest_value,
                actual,
            )
        )


def _validate_core_identity_doc(doc, label, expected=None):
    business_kernel = _require_string(doc, "businessKernel", label + ".businessKernel")
    if business_kernel != BUSINESS_KERNEL:
        raise CollectorError(
            "source manifest {0}.businessKernel must be {1}; got {2}".format(
                label,
                BUSINESS_KERNEL,
                business_kernel,
            )
        )

    core_commit = _require_string(doc, "coreCommit", label + ".coreCommit")
    if not _CORE_COMMIT_RE.match(core_commit):
        raise CollectorError(
            "source manifest {0}.coreCommit must be a 7-40 character lowercase hex git SHA".format(
                label
            )
        )
    abi_version = _require_positive_int(doc, "abiVersion", label + ".abiVersion")
    protocol_version = _require_positive_int(
        doc,
        "protocolVersion",
        label + ".protocolVersion",
    )

    identity = {
        "businessKernel": business_kernel,
        "coreCommit": core_commit,
        "abiVersion": abi_version,
        "protocolVersion": protocol_version,
    }
    if expected is not None and identity != expected:
        raise CollectorError(
            "source manifest {0} does not match coreIdentity".format(label)
        )
    return identity


def validate_core_identity(source_manifest):
    core_identity = _validate_core_identity_doc(
        _require_object(source_manifest, "coreIdentity", "coreIdentity"),
        "coreIdentity",
    )

    platform_runs = _require_object(source_manifest, "platformRuns", "platformRuns")
    platform_names = set(platform_runs.keys())
    required_names = set(REQUIRED_PLATFORMS)
    if platform_names != required_names:
        missing = sorted(required_names - platform_names)
        extra = sorted(platform_names - required_names)
        details = []
        if missing:
            details.append("missing " + ", ".join(missing))
        if extra:
            details.append("extra " + ", ".join(extra))
        raise CollectorError(
            "source manifest platformRuns must contain exactly cli, ios, android, harmony"
            + (" ({0})".format("; ".join(details)) if details else "")
        )
    for platform_name in REQUIRED_PLATFORMS:
        _validate_core_identity_doc(
            _require_object(
                platform_runs,
                platform_name,
                "platformRuns.{0}".format(platform_name),
            ),
            "platformRuns.{0}".format(platform_name),
            expected=core_identity,
        )
    return core_identity


def validate_source_manifest(source_manifest, source_manifest_path, input_path,
                             canonical_path, candidates, run_id=None, scenario=None):
    if source_manifest is None:
        return
    if not isinstance(source_manifest, dict):
        raise CollectorError("source manifest must be a JSON object")
    if "schemaVersion" in source_manifest and source_manifest["schemaVersion"] != SCHEMA_VERSION:
        raise CollectorError(
            "source manifest schemaVersion must be {0}; got {1}".format(
                SCHEMA_VERSION,
                source_manifest["schemaVersion"],
            )
        )

    if "runId" in source_manifest:
        manifest_run_id = _require_string(source_manifest, "runId", "runId")
        if run_id is not None and manifest_run_id != run_id:
            raise CollectorError(
                "source manifest runId={0} does not match actual {1}".format(
                    manifest_run_id,
                    run_id,
                )
            )
    if "scenario" in source_manifest:
        manifest_scenario = _require_string(source_manifest, "scenario", "scenario")
        if scenario is not None and manifest_scenario != scenario:
            raise CollectorError(
                "source manifest scenario={0} does not match actual {1}".format(
                    manifest_scenario,
                    scenario,
                )
            )

    _assert_manifest_path(
        source_manifest_path,
        source_manifest.get("input"),
        input_path,
        "input",
    )
    _assert_manifest_path(
        source_manifest_path,
        source_manifest.get("canonical"),
        canonical_path,
        "canonical",
    )

    manifest_candidates = source_manifest.get("candidates")
    if not isinstance(manifest_candidates, dict):
        raise CollectorError("source manifest candidates must be an object")
    manifest_names = set(manifest_candidates.keys())
    required_names = set(REQUIRED_PLATFORMS)
    if manifest_names != required_names:
        missing = sorted(required_names - manifest_names)
        extra = sorted(manifest_names - required_names)
        details = []
        if missing:
            details.append("missing " + ", ".join(missing))
        if extra:
            details.append("extra " + ", ".join(extra))
        raise CollectorError(
            "source manifest candidates must contain exactly cli, ios, android, harmony"
            + (" ({0})".format("; ".join(details)) if details else "")
        )
    for platform_name in REQUIRED_PLATFORMS:
        _assert_manifest_path(
            source_manifest_path,
            manifest_candidates[platform_name],
            candidates[platform_name],
            "candidate {0}".format(platform_name),
        )
    validate_core_identity(source_manifest)

    artifact_hashes = source_manifest.get("artifacts")
    if artifact_hashes is not None:
        artifact_hashes = _require_object(source_manifest, "artifacts", "artifacts")
        _assert_manifest_sha256(artifact_hashes.get("input"), input_path, "artifacts.input")
        _assert_manifest_sha256(
            artifact_hashes.get("canonical"),
            canonical_path,
            "artifacts.canonical",
        )
        candidate_hashes = _require_object(
            artifact_hashes,
            "candidates",
            "artifacts.candidates",
        )
        if set(candidate_hashes.keys()) != set(REQUIRED_PLATFORMS):
            raise CollectorError(
                "source manifest artifacts.candidates must contain exactly "
                "cli, ios, android, harmony"
            )
        for platform_name in REQUIRED_PLATFORMS:
            _assert_manifest_sha256(
                candidate_hashes.get(platform_name),
                candidates[platform_name],
                "artifacts.candidates.{0}".format(platform_name),
            )


def validate_manifest_expected_diff(source_manifest, diff_result):
    if source_manifest is None:
        return
    expected = source_manifest.get("expected")
    if expected is None:
        return
    if not isinstance(expected, dict):
        raise CollectorError("source manifest expected must be an object")

    if "match" in expected and not isinstance(expected["match"], bool):
        raise CollectorError("source manifest expected.match must be a boolean")
    if "match" in expected and expected["match"] != bool(diff_result["match"]):
        raise CollectorError(
            "source manifest expected.match={0} does not match actual {1}".format(
                expected["match"],
                diff_result["match"],
            )
        )
    if (
        "total" in expected
        and (
            not isinstance(expected["total"], int)
            or isinstance(expected["total"], bool)
        )
    ):
        raise CollectorError("source manifest expected.total must be an integer")
    if "total" in expected and expected["total"] != int(diff_result["total"]):
        raise CollectorError(
            "source manifest expected.total={0} does not match actual {1}".format(
                expected["total"],
                diff_result["total"],
            )
        )
    validate_manifest_expected_by_platform(expected, diff_result)
    validate_manifest_expected_host_parity(expected, build_host_parity(diff_result))
    validate_manifest_expected_blocker(expected, diff_result)


def build_host_parity(diff_result):
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
        actual_total = actual.get("total")
        if not isinstance(actual_total, int) or isinstance(actual_total, bool):
            actual_total = None
            all_match = False
        else:
            total += actual_total
        all_match = all_match and platform_match
        by_platform[platform_name] = {
            "present": True,
            "match": platform_match,
            "total": actual_total,
        }

    return {
        "requiredPlatforms": list(HOST_PLATFORMS),
        "allPresent": all_present,
        "match": all_present and all_match,
        "total": total,
        "byPlatform": by_platform,
    }


def validate_manifest_expected_host_parity(expected, host_parity):
    if "hostParity" not in expected:
        return
    expected_host = expected["hostParity"]
    if not isinstance(expected_host, dict):
        raise CollectorError("source manifest expected.hostParity must be an object")
    if "match" in expected_host:
        if not isinstance(expected_host["match"], bool):
            raise CollectorError(
                "source manifest expected.hostParity.match must be a boolean"
            )
        if expected_host["match"] != bool(host_parity["match"]):
            raise CollectorError(
                "source manifest expected.hostParity.match={0} "
                "does not match actual {1}".format(
                    expected_host["match"],
                    host_parity["match"],
                )
            )
    if "total" in expected_host:
        if (
            not isinstance(expected_host["total"], int)
            or isinstance(expected_host["total"], bool)
        ):
            raise CollectorError(
                "source manifest expected.hostParity.total must be an integer"
            )
        if expected_host["total"] != int(host_parity["total"]):
            raise CollectorError(
                "source manifest expected.hostParity.total={0} "
                "does not match actual {1}".format(
                    expected_host["total"],
                    host_parity["total"],
                )
            )


def build_corpus_proof(source_manifest, diff_result, host_parity, open_count,
                       run_id=None, scenario=None):
    candidates = diff_result.get("candidates") if isinstance(diff_result, dict) else None
    if not isinstance(candidates, dict):
        candidates = {}
    summary = diff_result.get("summary") if isinstance(diff_result, dict) else None
    if not isinstance(summary, dict):
        summary = {}

    missing_candidates = sorted(set(REQUIRED_PLATFORMS) - set(candidates.keys()))
    missing_summary = sorted(set(REQUIRED_PLATFORMS) - set(summary.keys()))
    full_diff_total = diff_result.get("total") if isinstance(diff_result, dict) else None
    full_diff_match = (
        isinstance(full_diff_total, int)
        and not isinstance(full_diff_total, bool)
        and full_diff_total == 0
        and bool(diff_result.get("match"))
    )
    host_total = host_parity.get("total") if isinstance(host_parity, dict) else None
    host_match = (
        isinstance(host_total, int)
        and not isinstance(host_total, bool)
        and host_total == 0
        and bool(host_parity.get("match"))
    )
    source_manifest_present = isinstance(source_manifest, dict)
    schema_version_bound = (
        source_manifest_present
        and source_manifest.get("schemaVersion") == SCHEMA_VERSION
    )
    core_identity_bound = (
        source_manifest_present
        and isinstance(source_manifest.get("coreIdentity"), dict)
        and isinstance(source_manifest.get("platformRuns"), dict)
    )
    artifacts = source_manifest.get("artifacts") if source_manifest_present else None
    artifact_hashes_bound = isinstance(artifacts, dict)
    if artifact_hashes_bound:
        artifact_hashes_bound = (
            isinstance(artifacts.get("input"), str)
            and _SHA256_RE.match(artifacts.get("input", "")) is not None
            and isinstance(artifacts.get("canonical"), str)
            and _SHA256_RE.match(artifacts.get("canonical", "")) is not None
            and isinstance(artifacts.get("candidates"), dict)
            and set(artifacts.get("candidates", {}).keys()) == set(REQUIRED_PLATFORMS)
        )
    if artifact_hashes_bound:
        for platform_name in REQUIRED_PLATFORMS:
            value = artifacts["candidates"].get(platform_name)
            if not isinstance(value, str) or _SHA256_RE.match(value) is None:
                artifact_hashes_bound = False
                break
    manifest_run_id = (
        source_manifest.get("runId") if source_manifest_present else None
    )
    manifest_scenario = (
        source_manifest.get("scenario") if source_manifest_present else None
    )
    run_binding_declared = isinstance(manifest_run_id, str) and bool(
        manifest_run_id.strip()
    )
    scenario_binding_declared = isinstance(manifest_scenario, str) and bool(
        manifest_scenario.strip()
    )
    run_binding_matches = run_binding_declared and (
        run_id is None or manifest_run_id.strip() == run_id
    )
    scenario_binding_matches = scenario_binding_declared and (
        scenario is None or manifest_scenario.strip() == scenario
    )
    expected = source_manifest.get("expected") if source_manifest_present else None
    expected_declared = isinstance(expected, dict)
    full_expected_declared = (
        expected_declared
        and isinstance(expected.get("match"), bool)
        and isinstance(expected.get("total"), int)
        and not isinstance(expected.get("total"), bool)
    )
    by_platform = expected.get("byPlatform") if expected_declared else None
    by_platform_expected_declared = isinstance(by_platform, dict) and set(
        by_platform.keys()
    ) == set(REQUIRED_PLATFORMS)
    if by_platform_expected_declared:
        for platform_name in REQUIRED_PLATFORMS:
            value = by_platform.get(platform_name)
            if not (
                isinstance(value, dict)
                and isinstance(value.get("match"), bool)
                and isinstance(value.get("total"), int)
                and not isinstance(value.get("total"), bool)
            ):
                by_platform_expected_declared = False
                break
    expected_host_parity = expected.get("hostParity") if expected_declared else None
    host_parity_expected_declared = (
        isinstance(expected_host_parity, dict)
        and isinstance(expected_host_parity.get("match"), bool)
        and isinstance(expected_host_parity.get("total"), int)
        and not isinstance(expected_host_parity.get("total"), bool)
    )

    reasons = []
    if not source_manifest_present:
        reasons.append("source-manifest-missing")
    if not schema_version_bound:
        reasons.append("schema-version-not-bound")
    if not core_identity_bound:
        reasons.append("core-identity-not-bound")
    if not artifact_hashes_bound:
        reasons.append("artifact-hashes-not-bound")
    if not run_binding_declared:
        reasons.append("run-id-not-bound")
    elif not run_binding_matches:
        reasons.append("run-id-mismatch")
    if not scenario_binding_declared:
        reasons.append("scenario-not-bound")
    elif not scenario_binding_matches:
        reasons.append("scenario-mismatch")
    if not expected_declared:
        reasons.append("expected-missing")
    if not full_expected_declared:
        reasons.append("expected-full-diff-missing")
    if not by_platform_expected_declared:
        reasons.append("expected-by-platform-missing")
    if not host_parity_expected_declared:
        reasons.append("expected-host-parity-missing")
    if missing_candidates:
        reasons.append("missing-candidates:" + ",".join(missing_candidates))
    if missing_summary:
        reasons.append("missing-summary:" + ",".join(missing_summary))
    if not full_diff_match:
        reasons.append("full-diff-mismatch")
    if not host_match:
        reasons.append("host-parity-mismatch")
    if open_count:
        reasons.append("open-blockers:{0}".format(open_count))

    return {
        "type": "corpus-same-result-proof",
        "status": "pass" if not reasons else "blocked",
        "reasons": reasons,
        "conditions": {
            "sourceManifestPresent": source_manifest_present,
            "schemaVersionBound": schema_version_bound,
            "coreIdentityBound": core_identity_bound,
            "artifactHashesBound": artifact_hashes_bound,
            "runBindingDeclared": run_binding_declared,
            "runBindingMatches": run_binding_matches,
            "scenarioBindingDeclared": scenario_binding_declared,
            "scenarioBindingMatches": scenario_binding_matches,
            "expectedDeclared": expected_declared,
            "fullDiffExpectedDeclared": full_expected_declared,
            "byPlatformExpectedDeclared": by_platform_expected_declared,
            "hostParityExpectedDeclared": host_parity_expected_declared,
            "fourPlatformCandidatesPresent": not missing_candidates,
            "fourPlatformSummaryPresent": not missing_summary,
            "fullDiffMatch": full_diff_match,
            "hostParityMatch": host_match,
            "openBlockers": open_count,
        },
        "missingCandidates": missing_candidates,
        "missingSummary": missing_summary,
    }


def validate_manifest_expected_by_platform(expected, diff_result):
    if "byPlatform" not in expected:
        return
    by_platform = expected["byPlatform"]
    if not isinstance(by_platform, dict):
        raise CollectorError("source manifest expected.byPlatform must be an object")
    names = set(by_platform.keys())
    required = set(REQUIRED_PLATFORMS)
    if names != required:
        missing = sorted(required - names)
        extra = sorted(names - required)
        details = []
        if missing:
            details.append("missing: " + ", ".join(missing))
        if extra:
            details.append("extra: " + ", ".join(extra))
        raise CollectorError(
            "source manifest expected.byPlatform must contain exactly "
            "cli, ios, android, harmony ({0})".format("; ".join(details))
        )
    summary = diff_result.get("summary")
    if not isinstance(summary, dict):
        raise CollectorError("diff-result summary must be an object")

    for platform_name in REQUIRED_PLATFORMS:
        platform_expected = by_platform[platform_name]
        if not isinstance(platform_expected, dict):
            raise CollectorError(
                "source manifest expected.byPlatform.{0} must be an object".format(
                    platform_name
                )
            )
        actual = summary.get(platform_name)
        if not isinstance(actual, dict):
            raise CollectorError(
                "diff-result summary.{0} must be an object".format(platform_name)
            )
        if "match" in platform_expected:
            if not isinstance(platform_expected["match"], bool):
                raise CollectorError(
                    "source manifest expected.byPlatform.{0}.match "
                    "must be a boolean".format(platform_name)
                )
            if platform_expected["match"] != bool(actual.get("match")):
                raise CollectorError(
                    "source manifest expected.byPlatform.{0}.match={1} "
                    "does not match actual {2}".format(
                        platform_name,
                        platform_expected["match"],
                        actual.get("match"),
                    )
                )
        if "total" in platform_expected:
            if (
                not isinstance(platform_expected["total"], int)
                or isinstance(platform_expected["total"], bool)
            ):
                raise CollectorError(
                    "source manifest expected.byPlatform.{0}.total "
                    "must be an integer".format(platform_name)
                )
            actual_total = actual.get("total")
            if not isinstance(actual_total, int) or isinstance(actual_total, bool):
                raise CollectorError(
                    "diff-result summary.{0}.total must be an integer".format(
                        platform_name
                    )
                )
            if platform_expected["total"] != actual_total:
                raise CollectorError(
                    "source manifest expected.byPlatform.{0}.total={1} "
                    "does not match actual {2}".format(
                        platform_name,
                        platform_expected["total"],
                        actual_total,
                    )
                )


def validate_manifest_expected_blocker(expected, diff_result):
    if "blockerPath" in expected and "blockerPlatform" not in expected:
        raise CollectorError(
            "source manifest expected.blockerPath requires blockerPlatform"
        )
    if "blockerPlatform" not in expected:
        return
    platform_name = expected["blockerPlatform"]
    if not isinstance(platform_name, str) or platform_name not in REQUIRED_PLATFORMS:
        raise CollectorError(
            "source manifest expected.blockerPlatform must be one of "
            "cli, ios, android, harmony"
        )
    candidates = diff_result.get("candidates")
    if not isinstance(candidates, dict):
        raise CollectorError("diff-result candidates must be an object")
    actual = candidates.get(platform_name)
    if not isinstance(actual, dict):
        raise CollectorError(
            "diff-result candidates.{0} must be an object".format(platform_name)
        )
    if actual.get("match"):
        raise CollectorError(
            "source manifest expected.blockerPlatform={0} does not match "
            "a mismatching candidate".format(platform_name)
        )
    if "blockerPath" not in expected:
        return
    blocker_path = expected["blockerPath"]
    if not isinstance(blocker_path, str) or not blocker_path:
        raise CollectorError(
            "source manifest expected.blockerPath must be a non-empty string"
        )
    differences = actual.get("differences")
    if not isinstance(differences, list):
        raise CollectorError(
            "diff-result candidates.{0}.differences must be a list".format(
                platform_name
            )
        )
    if not any(
        isinstance(diff, dict) and diff.get("path") == blocker_path
        for diff in differences
    ):
        raise CollectorError(
            "source manifest expected.blockerPath={0} was not found in "
            "{1} differences".format(blocker_path, platform_name)
        )


def collect_real_run(run_id, scenario, input_path, canonical_path, candidates,
                     out_dir=None, register_path=None, severity="high",
                     source_manifest_path=None, cwd=None):
    """Collect artifacts and return a summary dict.

    ``candidates`` must be an ordered mapping for ``cli``, ``ios``, ``android``,
    and ``harmony``. Output is a packager-compatible run directory.
    """
    if not run_id or not run_id.strip():
        raise CollectorError("run id is required")
    run_id = run_id.strip()
    scenario = (scenario or "").strip() or "corpus-real-run"
    cwd = cwd or os.getcwd()

    input_path = os.path.abspath(input_path)
    canonical_path = os.path.abspath(canonical_path)
    candidates = {name: os.path.abspath(path) for name, path in candidates.items()}
    source_manifest_path = (
        os.path.abspath(source_manifest_path) if source_manifest_path else None
    )

    # Validate JSON before cleaning the output directory.
    input_doc = _load_json(input_path, "input")
    source_manifest = _load_optional_source_manifest(source_manifest_path)
    _load_json(canonical_path, "canonical")
    for platform_name, path in candidates.items():
        _load_json(path, "{0} candidate".format(platform_name))
    validate_source_manifest(
        source_manifest,
        source_manifest_path,
        input_path,
        canonical_path,
        candidates,
        run_id=run_id,
        scenario=scenario,
    )

    out_user_specified = out_dir is not None
    out_dir = resolve_output_path(run_id, out_dir)
    register_user_specified = register_path is not None or out_user_specified
    if register_path is None:
        register_path = os.path.join(out_dir, rbr.DEFAULT_REGISTER_NAME)
    register_path = os.path.abspath(register_path)
    rbr.assert_safe_register(register_path, user_specified=register_user_specified)

    if os.path.exists(out_dir):
        if os.path.isdir(out_dir):
            shutil.rmtree(out_dir)
        else:
            os.remove(out_dir)
    os.makedirs(os.path.join(out_dir, "raw"), exist_ok=True)
    os.makedirs(os.path.join(out_dir, "candidates"), exist_ok=True)

    input_out = os.path.join(out_dir, "input.json")
    _write_json(input_out, input_doc)
    input_rel = _artifact_rel(out_dir, input_out)

    source_manifest_file = None
    if source_manifest_path is not None:
        source_manifest_out = os.path.join(out_dir, "raw", "source-manifest.json")
        _copy_file(source_manifest_path, source_manifest_out)
        source_manifest_file = {
            "sourcePath": source_manifest_path,
            "packagePath": _artifact_rel(out_dir, source_manifest_out),
            "sourceSha256": sha256_of_file(source_manifest_path),
            "packageSha256": sha256_of_file(source_manifest_out),
        }

    raw_canonical = os.path.join(out_dir, "raw", "canonical-result.json")
    _copy_file(canonical_path, raw_canonical)
    canonical_out = os.path.join(out_dir, "canonical-result.json")
    canonical_record = canonicalize_file(canonical_path, canonical_out, "canonical")
    canonical_record["rawPath"] = _artifact_rel(out_dir, raw_canonical)
    canonical_record["packagePath"] = _artifact_rel(out_dir, canonical_out)

    candidate_records = {}
    for platform_name in REQUIRED_PLATFORMS:
        src = candidates[platform_name]
        raw_dst = os.path.join(out_dir, "raw", "{0}-result.json".format(platform_name))
        canon_dst = os.path.join(
            out_dir, "candidates", "{0}-result.json".format(platform_name)
        )
        _copy_file(src, raw_dst)
        canonicalize_file(src, canon_dst, "{0} candidate".format(platform_name))
        candidate_records[platform_name] = {
            "sourcePath": src,
            "rawPath": _artifact_rel(out_dir, raw_dst),
            "canonicalizedPath": _artifact_rel(out_dir, canon_dst),
            "sourceSha256": sha256_of_file(src),
            "rawSha256": sha256_of_file(raw_dst),
            "canonicalizedFileSha256": sha256_of_file(canon_dst),
        }

    diff_candidates = [
        (
            platform_name,
            os.path.join(out_dir, "candidates", "{0}-result.json".format(platform_name)),
        )
        for platform_name in REQUIRED_PLATFORMS
    ]
    diff_result = cpd.build_diff_result(canonical_out, diff_candidates)
    host_parity = build_host_parity(diff_result)
    validate_manifest_expected_diff(source_manifest, diff_result)
    canonical_record["canonicalizedSha256"] = (
        diff_result["canonical"]["canonicalizedSha256"]
    )
    canonical_record["canonicalizedFileSha256"] = canonical_record["packageSha256"]
    for platform_name in REQUIRED_PLATFORMS:
        candidate_records[platform_name]["canonicalizedSha256"] = (
            diff_result["candidates"][platform_name]["canonicalizedSha256"]
        )
    diff_out = os.path.join(out_dir, "diff-result.json")
    _write_json(diff_out, diff_result)

    platform_result = build_platform_result(
        run_id,
        scenario,
        input_rel,
        candidate_records,
    )
    platform_out = os.path.join(out_dir, "platform-result.json")
    _write_json(platform_out, platform_result)

    environment = build_environment(cwd)
    environment_out = os.path.join(out_dir, "environment.json")
    _write_json(environment_out, environment)

    register = rbr.load_register(register_path)
    added_blockers = rbr.add_blockers_from_diff(
        register,
        diff_result,
        run_id=run_id,
        severity=severity,
    )
    rbr.save_register(register_path, register)
    open_count, open_breakdown = rbr.gate_evaluate(register, run_id=run_id)
    register_artifact_path = register_path
    local_register_artifact = os.path.join(out_dir, rbr.DEFAULT_REGISTER_NAME)
    if os.path.realpath(register_path) != os.path.realpath(local_register_artifact):
        _copy_file(register_path, local_register_artifact)
        register_artifact_path = local_register_artifact
    corpus_proof = build_corpus_proof(
        source_manifest,
        diff_result,
        host_parity,
        open_count,
        run_id=run_id,
        scenario=scenario,
    )

    manifest = {
        "schemaVersion": SCHEMA_VERSION,
        "tool": TOOL_NAME,
        "version": TOOL_VERSION,
        "runId": run_id,
        "scenario": scenario,
        "createdAt": _now_iso(),
        "sourceManifest": source_manifest,
        "sourceManifestFile": source_manifest_file,
        "input": {
            "sourcePath": input_path,
            "packagePath": input_rel,
            "sourceSha256": sha256_of_file(input_path),
            "packageSha256": sha256_of_file(input_out),
        },
        "canonical": canonical_record,
        "candidates": candidate_records,
        "artifacts": {
            "platformResult": _artifact_rel(out_dir, platform_out),
            "canonicalResult": _artifact_rel(out_dir, canonical_out),
            "diffResult": _artifact_rel(out_dir, diff_out),
            "environment": _artifact_rel(out_dir, environment_out),
            "blockerRegister": _artifact_rel(out_dir, register_artifact_path),
        },
        "diffSummary": {
            "match": diff_result["match"],
            "total": diff_result["total"],
            "byPlatform": diff_result["summary"],
        },
        "hostParity": host_parity,
        "corpusProof": corpus_proof,
        "blockers": {
            "registerPath": register_path,
            "added": len(added_blockers),
            "open": open_count,
            "openByPlatform": open_breakdown,
        },
        "releaseCertification": "not-assessed",
        "notes": [
            "Collector only packages already-produced local JSON run artifacts.",
            "Diff-derived blockers must be resolved or waived by separate review.",
        ],
    }
    manifest_out = os.path.join(out_dir, "manifest.json")
    _write_json(manifest_out, manifest)

    return {
        "runId": run_id,
        "scenario": scenario,
        "outDir": out_dir,
        "manifest": manifest_out,
        "diffResult": diff_out,
        "register": register_path,
        "match": diff_result["match"],
        "total": diff_result["total"],
        "hostParity": host_parity,
        "corpusProof": corpus_proof,
        "blockersAdded": len(added_blockers),
        "openBlockers": open_count,
        "openByPlatform": open_breakdown,
    }


def render_summary(summary):
    lines = [
        "Corpus real-run collector summary",
        "  tool: {0} v{1}".format(TOOL_NAME, TOOL_VERSION),
        "  runId: {0}".format(summary["runId"]),
        "  scenario: {0}".format(summary["scenario"]),
        "  outDir: {0}".format(summary["outDir"]),
        "  diff: {0} (total differences: {1})".format(
            "match" if summary["match"] else "mismatch",
            summary["total"],
        ),
        "  host parity: {0} (total differences: {1})".format(
            "match" if summary["hostParity"]["match"] else "mismatch",
            summary["hostParity"]["total"],
        ),
        "  corpus proof: {0}".format(summary["corpusProof"]["status"]),
        "  blockers added: {0}".format(summary["blockersAdded"]),
        "  open blockers: {0}".format(summary["openBlockers"]),
        "  register: {0}".format(summary["register"]),
    ]
    if summary["openByPlatform"]:
        lines.append("  open by platform:")
        for platform_name in sorted(summary["openByPlatform"].keys()):
            lines.append(
                "    {0}: {1}".format(platform_name, summary["openByPlatform"][platform_name])
            )
    return "\n".join(lines) + "\n"


def parse_args(argv):
    parser = argparse.ArgumentParser(
        prog=TOOL_NAME,
        description=(
            "Collect already-produced CLI / iOS / Android / HarmonyOS corpus "
            "JSON results into one candidate package, then run canonicalizer, "
            "cross-platform diff, and blocker registration."
        ),
    )
    parser.add_argument("--run-id", required=True, help="Stable id for this corpus run.")
    parser.add_argument("--scenario", default="corpus-real-run")
    parser.add_argument("--input", required=True, help="Path to the shared input JSON.")
    parser.add_argument(
        "--source-manifest",
        default=None,
        help="Optional source run manifest to embed in the collector manifest.",
    )
    parser.add_argument(
        "--canonical",
        required=True,
        help="Path to the canonical reference JSON result.",
    )
    parser.add_argument(
        "--candidate",
        action="append",
        default=[],
        metavar="PLATFORM:PATH",
        help=(
            "Candidate result JSON. Repeat for cli, ios, android, and harmony. "
            "All four are required."
        ),
    )
    parser.add_argument(
        "--out",
        dest="out_dir",
        default=None,
        help="Output package directory. Default: /private/tmp/<run-id>-candidate.",
    )
    parser.add_argument(
        "--register",
        default=None,
        help=(
            "Blocker register path. Default: <out>/corpus-blocker-register.json. "
            "Must not be under ~/Documents."
        ),
    )
    parser.add_argument(
        "--severity",
        choices=(rbr.SEVERITY_HIGH, rbr.SEVERITY_MEDIUM, rbr.SEVERITY_LOW),
        default=rbr.SEVERITY_HIGH,
        help="Severity for blockers derived from this run's diff.",
    )
    return parser.parse_args(argv)


def main(argv=None):
    if argv is None:
        argv = sys.argv[1:]
    args = parse_args(argv)
    try:
        candidates = parse_candidate_specs(args.candidate)
        summary = collect_real_run(
            run_id=args.run_id,
            scenario=args.scenario,
            input_path=args.input,
            canonical_path=args.canonical,
            candidates=candidates,
            out_dir=args.out_dir,
            register_path=args.register,
            severity=args.severity,
            source_manifest_path=args.source_manifest,
        )
    except (CollectorError, rbr.RegisterError, cpd.DiffError) as err:
        sys.stderr.write("error: {0}\n".format(err))
        return 2
    sys.stdout.write(render_summary(summary))
    return 0


if __name__ == "__main__":
    sys.exit(main())
