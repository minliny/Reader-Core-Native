#!/usr/bin/env python3
"""Platform evidence JSON validator.

Defines and validates a canonical iOS/Android/HarmonyOS evidence JSON format
so that smoke/build/device/corpus/unit reports from different platforms are
directly comparable. Pure Python 3.9+ standard library, hand-rolled schema
(no third-party deps).

Canonical record fields:
    version      : "platform-evidence/1" (required)
    platform     : ios | android | harmony (required)
    kind         : smoke | build | device | corpus | unit (required)
    capability   : non-empty string (required)
    status       : pass | fail | skipped | unknown (required)
    timestamp    : ISO8601 string, trailing Z accepted (required)
    environment  : {"os": str, "arch": str, "toolchain"?: str} (required)
    fixture_id   : non-empty string, required when kind == "corpus"
    artifact     : optional string (repo-relative path)
    notes        : optional string

A file may also contain a BATCH: {"version": "platform-evidence/1",
"records": [<record>, ...]}.

CLI:
    python3 tools/platform-evidence-validator/platform_evidence_validator.py \
        <path> [--pretty]
"""

import argparse
import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path

SCHEMA_VERSION = "platform-evidence/1"
TOOL_VERSION = "platform-evidence-validator/1"
TOOL_NAME = "platform-evidence-validator"

_PLATFORMS = ("ios", "android", "harmony")
_KINDS = ("smoke", "build", "device", "corpus", "unit")
_STATUSES = ("pass", "fail", "skipped", "unknown")

# Allowed fields on a single evidence record.
_RECORD_FIELDS = frozenset(
    (
        "version",
        "platform",
        "kind",
        "capability",
        "status",
        "timestamp",
        "environment",
        "fixture_id",
        "artifact",
        "notes",
    )
)
# Allowed fields on a batch envelope.
_BATCH_FIELDS = frozenset(("version", "records"))


def _is_nonempty_str(value):
    return isinstance(value, str) and len(value) > 0


def _parse_timestamp(value):
    """Return True if `value` parses as ISO8601 (trailing Z accepted)."""
    if not isinstance(value, str) or value == "":
        return False
    candidate = value
    if candidate.endswith("Z"):
        candidate = candidate[:-1] + "+00:00"
    try:
        datetime.fromisoformat(candidate)
    except ValueError:
        return False
    return True


def _validate_record_fields(rec, version_required):
    """Validate a single record dict. Returns a list of error strings."""
    errors = []
    if not isinstance(rec, dict):
        errors.append("record must be a JSON object")
        return errors

    # Unknown fields.
    for key in rec:
        if key not in _RECORD_FIELDS:
            errors.append("unknown field: %s" % key)

    # version
    if "version" not in rec:
        if version_required:
            errors.append("version must be 'platform-evidence/1'")
    else:
        if rec["version"] != SCHEMA_VERSION:
            errors.append("version must be 'platform-evidence/1'")

    # platform
    if rec.get("platform") not in _PLATFORMS:
        errors.append("platform must be one of ios/android/harmony")

    # kind
    if rec.get("kind") not in _KINDS:
        errors.append("kind must be one of smoke/build/device/corpus/unit")

    # capability
    if not _is_nonempty_str(rec.get("capability")):
        errors.append("capability must be a non-empty string")

    # status
    if rec.get("status") not in _STATUSES:
        errors.append("status must be one of pass/fail/skipped/unknown")

    # timestamp
    if not _parse_timestamp(rec.get("timestamp")):
        errors.append("timestamp must be a valid ISO8601 string")

    # environment
    env = rec.get("environment")
    if not isinstance(env, dict):
        errors.append("environment must be an object with 'os' and 'arch'")
    else:
        if not _is_nonempty_str(env.get("os")):
            errors.append("environment.os must be a non-empty string")
        if not _is_nonempty_str(env.get("arch")):
            errors.append("environment.arch must be a non-empty string")
        if "toolchain" in env and not isinstance(env["toolchain"], str):
            errors.append("environment.toolchain must be a string")

    # corpus requires fixture_id
    if rec.get("kind") == "corpus":
        if not _is_nonempty_str(rec.get("fixture_id")):
            errors.append("corpus evidence requires fixture_id")

    # artifact
    if "artifact" in rec and not isinstance(rec["artifact"], str):
        errors.append("artifact must be a string")

    # notes
    if "notes" in rec and not isinstance(rec["notes"], str):
        errors.append("notes must be a string")

    return errors


def validate_record(rec):
    """Validate a standalone single evidence record. Empty list = valid."""
    return _validate_record_fields(rec, version_required=True)


def validate(obj):
    """Validate either a single record or a batch envelope.

    Returns a list of error strings (empty = valid). For batch records,
    record-level errors are prefixed with ``[records[i]] ``.
    """
    if not isinstance(obj, dict):
        return ["root must be a JSON object"]

    if "records" in obj:
        return _validate_batch(obj)
    return _validate_record_fields(obj, version_required=True)


def _validate_batch(obj):
    errors = []
    for key in obj:
        if key not in _BATCH_FIELDS:
            errors.append("unknown field: %s" % key)

    if obj.get("version") != SCHEMA_VERSION:
        errors.append("version must be 'platform-evidence/1'")

    records = obj.get("records")
    if not isinstance(records, list):
        errors.append("records must be a non-empty list")
        return errors
    if len(records) == 0:
        errors.append("records must be a non-empty list")
        return errors

    for i, rec in enumerate(records):
        rec_errors = _validate_record_fields(rec, version_required=False)
        for msg in rec_errors:
            errors.append("[records[%d]] %s" % (i, msg))
    return errors


def validate_file(path):
    """Read+parse JSON from `path`. Returns (obj_or_None, errors).

    On JSON parse error returns (None, ["invalid JSON: <msg>"]).
    """
    try:
        with open(path, "r", encoding="utf-8") as fh:
            text = fh.read()
    except OSError as exc:
        return None, ["invalid JSON: %s" % exc]

    try:
        obj = json.loads(text)
    except (json.JSONDecodeError, ValueError) as exc:
        return None, ["invalid JSON: %s" % exc]

    return obj, validate(obj)


def validate_dir(path):
    """Recursively validate every ``*.json`` under `path`.

    Returns a list of ``{"path": rel, "valid": bool, "errors": [...]}``
    sorted by relative path (forward slashes).
    """
    root = Path(path)
    results = []
    for file_path in sorted(root.rglob("*.json")):
        rel = file_path.relative_to(root)
        rel_str = str(rel).replace(os.sep, "/")
        _obj, errs = validate_file(file_path)
        results.append(
            {"path": rel_str, "valid": len(errs) == 0, "errors": errs}
        )
    return results


def summarize(results):
    """Summarize validate_dir-style results into total/valid/invalid counts."""
    total = len(results)
    valid = sum(1 for r in results if r.get("valid"))
    return {"total": total, "valid": valid, "invalid": total - valid}


def _report(results):
    return {
        "version": TOOL_VERSION,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "tool": TOOL_NAME,
        "results": results,
        "summary": summarize(results),
    }


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="platform_evidence_validator",
        description="Validate platform evidence JSON files against the "
        "platform-evidence/1 schema.",
    )
    parser.add_argument("path", help="file or directory to validate")
    parser.add_argument(
        "--pretty",
        action="store_true",
        help="indent=2, sorted keys, trailing newline",
    )
    args = parser.parse_args(argv)

    target = Path(args.path)
    if target.is_file():
        _obj, errs = validate_file(target)
        results = [
            {"path": target.name, "valid": len(errs) == 0, "errors": errs}
        ]
    elif target.is_dir():
        results = validate_dir(target)
    else:
        sys.stderr.write(
            "error: path not found or not a file/directory: %s\n" % target
        )
        return 2

    report = _report(results)
    if args.pretty:
        out = json.dumps(report, indent=2, sort_keys=True)
    else:
        out = json.dumps(report)
    sys.stdout.write(out + "\n")

    if report["summary"]["invalid"] > 0:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
