#!/usr/bin/env python3
"""Evidence indexer.

Read-only dev-time tool that scans reports, artifacts, and tool outputs to
produce a UNIFIED evidence index. Each evidence item is tagged with a
``tier`` (smoke/build/device/corpus/unit/unknown) and a ``platform``
(ios/android/harmony/host/unknown).

This tool ONLY indexes existing evidence files. It NEVER infers whether a
capability is complete.

Python 3.9+ standard library only.

CLI:
    python3 tools/evidence-indexer/evidence_indexer.py [root] [--pretty] [--out PATH]

Exit codes:
    0  - index emitted (always, even if empty)
    2  - usage / IO error (e.g. missing root)
"""

import argparse
import hashlib
import json
import os
import sys
from datetime import datetime, timezone

INDEX_VERSION = "evidence-index/1"
TOOL_NAME = "evidence-indexer"

_TIERS = ("smoke", "build", "device", "corpus", "unit", "unknown")
_PLATFORMS = ("ios", "android", "harmony", "host", "unknown")
_STATUSES = ("pass", "fail", "skipped", "unknown")

# Directories never descended into by collect().
_FORBIDDEN_DIRS = frozenset({".git", "target", "node_modules"})

# version -> (tier, platform) defaults for one-entry-per-file reports whose
# status is "unknown" (not specified by the contract).
_SIMPLE_VERSIONS = {
    "build-env-doctor/1": ("build", "host"),
    "fixture-manifest/1": ("corpus", "host"),
    "worktree-conflict-report/1": ("unknown", "host"),
    "corpus-batch-selector/1": ("corpus", "host"),
    "protocol-schema-lint/1": ("smoke", "host"),
}


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
def _now_iso():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _entry_id(rel_path, suffix=None):
    """Stable id = first 12 hex of sha1 of the rel path (with optional suffix
    to disambiguate batch records sharing one path)."""
    raw = rel_path if suffix is None else "%s#%s" % (rel_path, suffix)
    return hashlib.sha1(raw.encode("utf-8")).hexdigest()[:12]


def _norm_tier(value):
    return value if value in _TIERS else "unknown"


def _norm_platform(value):
    return value if value in _PLATFORMS else "unknown"


def _norm_status(value):
    return value if value in _STATUSES else "unknown"


def _make_entry(rel_path, tier, platform, capability, status, timestamp):
    return {
        "id": _entry_id(rel_path),
        "source": rel_path,
        "tier": _norm_tier(tier),
        "platform": _norm_platform(platform),
        "capability": capability,
        "status": _norm_status(status),
        "path": rel_path,
        "timestamp": timestamp,
    }


# ---------------------------------------------------------------------------
# Pure classifiers
# ---------------------------------------------------------------------------
def classify_json(path, obj):
    """Classify a parsed JSON object into evidence entries.

    Pure: no disk access. ``path`` is the repo-relative path string. ``obj``
    is the parsed JSON dict. Returns a list of evidence entries (0, 1, or
    many). ``evidence-index/1`` is skipped (returns []) to avoid recursion.
    """
    if not isinstance(obj, dict):
        return []

    version = obj.get("version")
    if not isinstance(version, str):
        return []

    # Skip other evidence indexes to avoid recursion.
    if version == "evidence-index/1":
        return []

    generated_at = obj.get("generated_at")
    ts = generated_at if isinstance(generated_at, str) else None

    # platform-evidence/1: single record OR batch (records[]).
    if version == "platform-evidence/1":
        return _classify_platform_evidence(path, obj)

    if version == "protocol-schema-lint/1":
        tier, platform = _SIMPLE_VERSIONS[version]
        summary = obj.get("summary") or {}
        unexpected_invalid = summary.get("unexpected_invalid", 0) or 0
        status = "pass" if unexpected_invalid == 0 else "fail"
        return [_make_entry(path, tier, platform, None, status, ts)]

    if version == "platform-evidence-validator/1":
        summary = obj.get("summary") or {}
        invalid = summary.get("invalid", 0) or 0
        status = "pass" if invalid == 0 else "fail"
        return [_make_entry(path, "unknown", "host", None, status, ts)]

    if version == "capability-catalog/1":
        return [_make_entry(path, "unknown", "host", None, "pass", ts)]

    if version == "gate-declaration-report/1":
        return _classify_gate_declaration(path, obj, ts)

    if version in _SIMPLE_VERSIONS:
        tier, platform = _SIMPLE_VERSIONS[version]
        return [_make_entry(path, tier, platform, None, "unknown", ts)]

    # Any other JSON with a version field.
    return [_make_entry(path, "unknown", "unknown", None, "unknown", ts)]


def _classify_platform_evidence(path, obj):
    records = obj.get("records")
    if isinstance(records, list) and records:
        out = []
        for idx, rec in enumerate(records):
            if not isinstance(rec, dict):
                continue
            entry = _make_entry(
                path,
                rec.get("kind", "unknown"),
                rec.get("platform", "unknown"),
                rec.get("capability"),
                rec.get("status", "unknown"),
                rec.get("timestamp"),
            )
            # Disambiguate ids within a batch so they stay unique.
            entry["id"] = _entry_id(path, suffix=str(idx))
            out.append(entry)
        return out
    # Single record (the object itself is the record).
    return [_make_entry(
        path,
        obj.get("kind", "unknown"),
        obj.get("platform", "unknown"),
        obj.get("capability"),
        obj.get("status", "unknown"),
        obj.get("timestamp"),
    )]


def _classify_gate_declaration(path, obj, ts):
    gates = obj.get("gates")
    if isinstance(gates, list) and gates:
        out = []
        for gate in gates:
            if not isinstance(gate, dict):
                continue
            platform = gate.get("platform", "unknown")
            fail_closed = gate.get("fail_closed")
            status = "pass" if fail_closed else "fail"
            out.append(_make_entry(path, "smoke", platform, None, status, ts))
        return out
    return [_make_entry(path, "smoke", "unknown", None, "unknown", ts)]


# Markdown tier keyword priority (first match wins).
_MD_TIER_KEYWORDS = (
    ("smoke", "smoke"),
    ("build", "build"),
    ("device", "device"),
    ("corpus", "corpus"),
)
_MD_PLATFORM_KEYWORDS = (
    ("ios", "ios"),
    ("android", "android"),
    ("harmony", "harmony"),
)


def classify_md(path, text):
    """Classify a markdown file into one evidence entry, or None if ``path``
    is not under ``reports/`` or ``evidence/``.

    Pure: no disk access. ``path`` is the repo-relative path string.
    """
    if not _under_reports_or_evidence(path):
        return None
    name = path.rsplit("/", 1)[-1].lower()
    tier = "unknown"
    for keyword, label in _MD_TIER_KEYWORDS:
        if keyword in name:
            tier = label
            break
    platform = "unknown"
    for keyword, label in _MD_PLATFORM_KEYWORDS:
        if keyword in name:
            platform = label
            break
    return _make_entry(path, tier, platform, None, "unknown", None)


def _under_reports_or_evidence(path):
    if not isinstance(path, str):
        return False
    norm = path.replace(os.sep, "/").lstrip("./")
    return norm.startswith("reports/") or norm.startswith("evidence/")


# ---------------------------------------------------------------------------
# Disk-walking orchestrator
# ---------------------------------------------------------------------------
def collect(root):
    """Walk ``root`` and return the unified evidence index dict.

    Raises FileNotFoundError if ``root`` does not exist.
    """
    root_path = os.path.abspath(str(root))
    if not os.path.isdir(root_path):
        raise FileNotFoundError("root not found or not a directory: %s" % root_path)

    entries = []
    for dirpath, dirnames, filenames in os.walk(root_path):
        # Prune forbidden + dot directories in-place.
        pruned = []
        for d in dirnames:
            if d.startswith(".") or d in _FORBIDDEN_DIRS:
                continue
            pruned.append(d)
        dirnames[:] = pruned

        for fname in filenames:
            if fname.startswith("."):
                continue
            full = os.path.join(dirpath, fname)
            rel = os.path.relpath(full, root_path).replace(os.sep, "/")
            if fname.endswith(".json"):
                _collect_json(full, rel, entries)
            elif fname.endswith(".md"):
                entry = classify_md(rel, None)
                if entry is not None:
                    entries.append(entry)

    entries.sort(key=lambda e: (e["source"], e["id"]))
    return _build_index(entries)


def _collect_json(full, rel, entries):
    """Read+parse a JSON file; append classified entries. Skip on any error."""
    try:
        with open(full, "rb") as f:
            raw = f.read()
    except OSError:
        return
    # Skip binary files (null bytes / undecodable).
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        return
    if "\x00" in text:
        return
    try:
        obj = json.loads(text)
    except (ValueError, json.JSONDecodeError):
        return
    entries.extend(classify_json(rel, obj))


def _build_index(entries):
    by_tier = {}
    by_platform = {}
    by_status = {}
    for e in entries:
        by_tier[e["tier"]] = by_tier.get(e["tier"], 0) + 1
        by_platform[e["platform"]] = by_platform.get(e["platform"], 0) + 1
        by_status[e["status"]] = by_status.get(e["status"], 0) + 1
    return {
        "version": INDEX_VERSION,
        "generated_at": _now_iso(),
        "tool": TOOL_NAME,
        "entries": entries,
        "summary": {
            "total": len(entries),
            "by_tier": by_tier,
            "by_platform": by_platform,
            "by_status": by_status,
        },
    }


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------
def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="evidence_indexer",
        description="Read-only evidence indexer. Scans reports, artifacts, "
        "and tool outputs to emit a unified evidence index.",
    )
    parser.add_argument(
        "root",
        nargs="?",
        default=os.getcwd(),
        help="Root directory to scan (default: cwd).",
    )
    parser.add_argument(
        "--pretty",
        action="store_true",
        help="Pretty-print JSON (indent=2, sorted keys, trailing newline).",
    )
    parser.add_argument(
        "--out",
        default=None,
        help="Write index to this file instead of stdout.",
    )
    args = parser.parse_args(argv)

    try:
        index = collect(args.root)
    except FileNotFoundError as e:
        sys.stderr.write("error: %s\n" % e)
        return 2
    except OSError as e:
        sys.stderr.write("error: %s\n" % e)
        return 2

    if args.pretty:
        text = json.dumps(index, indent=2, sort_keys=True) + "\n"
    else:
        text = json.dumps(index)

    if args.out:
        try:
            with open(args.out, "w", encoding="utf-8") as f:
                f.write(text)
        except OSError as e:
            sys.stderr.write("error: %s\n" % e)
            return 2
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
