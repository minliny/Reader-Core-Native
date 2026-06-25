#!/usr/bin/env python3
"""Release blocker register generator.

READ-ONLY dev-time tool that derives a release-blockers list from the
capability catalog, the evidence index, and the migration ledger. It ONLY
emits a report - it never modifies business code.

Pure blocker-derivation functions take PARSED inputs (capability list +
evidence entries + migration ledger rows) so they are unit-testable with
canned data. The disk-reading ``collect(root)`` orchestrator loads files
and delegates to the pure functions.

Python 3.9+ standard library only.

CLI:
    python3 tools/release-blockers/release_blockers.py [root]
        [--evidence-index PATH] [--pretty] [--out PATH]

Exit codes:
    0  - report emitted (it is a report, not a gate)
    2  - usage / IO error
"""

import argparse
import json
import os
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

VERSION = "release-blockers/1"
TOOL = "release-blocker-register-generator"
DEFAULT_PLATFORMS = ["ios", "android", "harmony"]

_SEVERITY_ORDER = {"blocker": 0, "high": 1, "medium": 2, "low": 3}

# Markdown table row prefix.
_TABLE_ROW_RE = re.compile(r"^\s*\|")
# Separator row cell: only dashes/colons/whitespace, e.g. ":---:".
_SEP_CELL_RE = re.compile(r"^:?-+:?$")

# Owner tokens.
_OWNER_HOST_TOKENS = ("平台负责", "Platform Adapter", "platform adapter")
_OWNER_CORE_TOKENS = ("Rust Core", "rust core")
# Status tokens (Chinese capability-status matrix).
_STATUS_TOKENS = {
    "已完成": "implemented",
    "部分完成": "partial",
    "Gap": "missing",
    "gap": "missing",
}


# ---------------------------------------------------------------------------
# Pure parsers
# ---------------------------------------------------------------------------
def _split_table_row(line):
    """Split a ``| a | b | c |`` line into stripped cells (no outer pipes)."""
    stripped = line.strip()
    if stripped.startswith("|"):
        stripped = stripped[1:]
    if stripped.endswith("|"):
        stripped = stripped[:-1]
    return [cell.strip() for cell in stripped.split("|")]


def _is_separator_row(cells):
    return bool(cells) and all(
        _SEP_CELL_RE.match(c) for c in cells if c != ""
    )


def _resolve_owner_status(cells, header):
    """Resolve owner (core/host) and status from table cells.

    ``header`` is the list of header cell texts (parallel to ``cells``) or
    ``None`` if no header was seen.
    """
    owner = None
    status = None
    for idx, cell in enumerate(cells):
        if cell == "":
            continue
        has_check = "✅" in cell
        col_name = header[idx] if (header and idx < len(header)) else ""
        if has_check:
            col_lower = col_name.lower()
            if "rust core" in col_lower:
                owner = "core"
            elif "platform adapter" in col_lower or "平台" in col_name:
                owner = "host"
            if status is None:
                status = "implemented"
        for tok in _OWNER_HOST_TOKENS:
            if tok in cell:
                owner = "host"
        for tok in _OWNER_CORE_TOKENS:
            if tok in cell:
                owner = "core"
        for tok, mapped in _STATUS_TOKENS.items():
            if tok in cell:
                status = mapped
    return owner, status


def _slug(name):
    """Slugify a name into a stable id (preserves dots and unicode letters)."""
    s = re.sub(r"[^\w.]+", "-", (name or "").lower(), flags=re.UNICODE)
    return s.strip("-")


def parse_feature_matrix(text):
    """Parse markdown table rows into capability entries.

    Returns a list of dicts:
        ``{"id", "owner", "status", "platforms", "evidence", "source", "name"}``
    where ``owner`` is ``"core"|"host"|None``, ``status`` is
    ``"implemented"|"partial"|"missing"|None``, and ``platforms`` defaults to
    the standard three. Pure.
    """
    rows = []
    header = None
    for line in (text or "").splitlines():
        if not _TABLE_ROW_RE.match(line):
            continue
        cells = _split_table_row(line)
        if not cells:
            continue
        if _is_separator_row(cells):
            continue
        if header is None:
            header = cells
            continue
        name = cells[0]
        if not name:
            continue
        owner, status = _resolve_owner_status(cells, header)
        rows.append({
            "id": _slug(name),
            "owner": owner,
            "status": status,
            "platforms": list(DEFAULT_PLATFORMS),
            "evidence": ["FEATURE_MATRIX.md"],
            "source": "FEATURE_MATRIX.md",
            "name": name,
        })
    return rows


def parse_evidence_index(obj):
    """Return the list of evidence entries from a parsed evidence-index obj.

    Pure. Returns ``[]`` if ``obj`` is None or has no ``entries`` list.
    """
    if not isinstance(obj, dict):
        return []
    entries = obj.get("entries")
    if not isinstance(entries, list):
        return []
    return list(entries)


def parse_migration_ledger(text):
    """Parse markdown table rows into migration ledger rows.

    Returns a list of ``{"capability", "status", "notes"}`` dicts, or ``[]``
    if ``text`` is None. Coarse: first cell is capability, second is status,
    remainder joined into notes. Pure.
    """
    if not text:
        return []
    rows = []
    header_seen = False
    for line in text.splitlines():
        if not _TABLE_ROW_RE.match(line):
            continue
        cells = _split_table_row(line)
        if not cells:
            continue
        if _is_separator_row(cells):
            continue
        if not header_seen:
            header_seen = True
            continue
        capability = cells[0] if len(cells) > 0 else ""
        status = cells[1] if len(cells) > 1 else ""
        notes = " | ".join(cells[2:]) if len(cells) > 2 else ""
        if not capability:
            continue
        rows.append({
            "capability": capability,
            "status": status,
            "notes": notes,
        })
    return rows


# ---------------------------------------------------------------------------
# Pure blocker derivation
# ---------------------------------------------------------------------------
def _blocker_slug(cap_id):
    return "rb-" + _slug(cap_id)


def _platform_field(required, passing_platforms, failing_platforms, zero_evidence):
    """Compute the ``platform`` field for a blocker entry.

    * ``"all"`` when all required platforms are affected (zero evidence, or
      every required platform is failing/missing-from-passing).
    * the specific platform when exactly one is missing.
    * ``"all"`` when multiple (but not all) are missing.
    * ``"unknown"`` as a fallback when nothing is affected.
    """
    required_set = set(required) if required else set()
    if zero_evidence:
        affected = set(required_set)
    else:
        missing_from_passing = required_set - set(passing_platforms)
        affected = set(missing_from_passing)
        if failing_platforms:
            affected |= (set(failing_platforms) & required_set)
    if not affected:
        return "unknown"
    if len(affected) == 1:
        return next(iter(affected))
    return "all"


def _evidence_status(failing, cap_evidence, passing_platforms, required, status):
    """Compute the ``evidence_status`` field."""
    if failing:
        return "fail"
    if status == "unknown":
        return "unknown"
    if not cap_evidence:
        return "missing"
    if passing_platforms and passing_platforms != set(required):
        return "partial"
    return "missing"


def _mitigation(cap, severity, missing_platforms_label):
    """Templated mitigation suggestion."""
    cap_id = cap.get("id") or "<capability>"
    status = cap.get("status")
    if severity == "blocker":
        if status == "missing":
            return "Implement %s (currently missing)" % cap_id
        if status == "partial":
            return "Complete %s (partial; core-owned with no evidence)" % cap_id
        return "Fix failing evidence for %s" % cap_id
    if severity == "high":
        return ("Add passing smoke/device evidence for %s on %s"
                % (cap_id, missing_platforms_label))
    if severity == "medium":
        if status == "implemented":
            return ("Verify implemented capability %s with at least one "
                    "passing evidence" % cap_id)
        if cap.get("owner") == "host":
            return ("Add passing evidence for %s on %s"
                    % (cap_id, missing_platforms_label))
        return ("Add passing smoke/device evidence for %s on %s"
                % (cap_id, missing_platforms_label))
    if severity == "low":
        return "Clarify status of %s and add evidence" % cap_id
    return ""


def _missing_platforms_label(required, passing_platforms):
    required_set = set(required) if required else set()
    missing = required_set - set(passing_platforms)
    if not missing:
        return "all required platforms"
    if len(missing) == 1:
        return next(iter(missing))
    return "all required platforms"


def _derive_one(cap, cap_evidence):
    """Derive a single blocker entry for a capability, or None if not a blocker."""
    cap_id = cap.get("id")
    status = cap.get("status")
    owner = cap.get("owner")
    required = cap.get("platforms") or list(DEFAULT_PLATFORMS)

    passing = [e for e in cap_evidence if e.get("status") == "pass"]
    failing = [e for e in cap_evidence if e.get("status") == "fail"]
    passing_platforms = {
        e.get("platform") for e in passing
        if e.get("platform") in required
    }
    failing_platforms = {
        e.get("platform") for e in failing
        if e.get("platform") in required
    }
    zero_evidence = not cap_evidence
    has_passing = bool(passing_platforms)

    severity = None
    # --- blocker tier ---
    if failing:
        severity = "blocker"
    elif status == "missing" and not has_passing:
        severity = "blocker"
    elif status in ("partial", "missing") and owner == "core" and zero_evidence:
        severity = "blocker"
    # --- high tier ---
    elif status == "partial" and not has_passing:
        severity = "high"
    # --- medium tier ---
    elif status == "partial" and passing_platforms and passing_platforms != set(required):
        severity = "medium"
    elif owner == "host" and zero_evidence:
        severity = "medium"
    elif status == "implemented" and zero_evidence:
        severity = "medium"
    # --- low tier ---
    elif status == "unknown" and zero_evidence:
        severity = "low"

    if severity is None:
        return None

    evidence_status = _evidence_status(
        failing, cap_evidence, passing_platforms, required, status
    )
    platform = _platform_field(
        required, passing_platforms, failing_platforms, zero_evidence
    )
    missing_label = _missing_platforms_label(required, passing_platforms)

    # Sources: capability's evidence list + matched evidence entry paths.
    sources = []
    for src in (cap.get("evidence") or []):
        if src and src not in sources:
            sources.append(src)
    cap_source = cap.get("source")
    if cap_source and cap_source not in sources:
        sources.append(cap_source)
    for e in cap_evidence:
        for key in ("path", "source"):
            val = e.get(key)
            if val and val not in sources:
                sources.append(val)

    reason = _reason(cap, severity, status, owner, failing, has_passing,
                     passing_platforms, required, zero_evidence)
    mitigation = _mitigation(cap, severity, missing_label)

    return {
        "id": _blocker_slug(cap_id),
        "capability": cap_id,
        "platform": platform,
        "severity": severity,
        "reason": reason,
        "sources": sources,
        "evidence_status": evidence_status,
        "mitigation": mitigation,
    }


def _reason(cap, severity, status, owner, failing, has_passing,
            passing_platforms, required, zero_evidence):
    """Short human-readable reason string."""
    cap_id = cap.get("id") or "<capability>"
    if failing:
        return ("%s has %d failing evidence entr(y|ies)"
                % (cap_id, len(failing)))
    if severity == "blocker":
        if status == "missing":
            return "%s is missing with zero passing evidence" % cap_id
        if status == "partial":
            return ("%s is partial, core-owned, with zero evidence" % cap_id)
        return "%s is a release blocker" % cap_id
    if severity == "high":
        return "%s is partial with zero passing evidence" % cap_id
    if severity == "medium":
        if status == "implemented" and zero_evidence:
            return "%s is implemented but unverified (zero evidence)" % cap_id
        if owner == "host" and zero_evidence:
            return "%s is host-owned with zero evidence" % cap_id
        missing = set(required) - passing_platforms
        return ("%s is partial; missing passing evidence on %s"
                % (cap_id, ", ".join(sorted(missing)) if missing else "all"))
    if severity == "low":
        return "%s has unknown status with zero evidence" % cap_id
    return "%s flagged" % cap_id


def derive_blockers(capabilities, evidence_entries, ledger_rows):
    """Derive the release-blockers list from parsed inputs.

    Pure. ``capabilities`` is a list of capability dicts (with ``id``,
    ``owner``, ``status``, ``platforms``, ``evidence``). ``evidence_entries``
    is a list of evidence entries (with ``capability``, ``status``,
    ``platform``, ``path``). ``ledger_rows`` is accepted for contract
    completeness but not currently consulted by the rules.

    Returns a list of blocker entry dicts sorted by severity (blocker > high >
    medium > low) then by capability id.
    """
    by_cap = {}
    for entry in evidence_entries or []:
        cap_id = entry.get("capability")
        if cap_id is None:
            continue
        by_cap.setdefault(cap_id, []).append(entry)

    blockers = []
    for cap in capabilities or []:
        cap_id = cap.get("id")
        if cap_id is None:
            continue
        cap_evidence = by_cap.get(cap_id, [])
        entry = _derive_one(cap, cap_evidence)
        if entry is not None:
            blockers.append(entry)

    blockers.sort(
        key=lambda b: (_SEVERITY_ORDER.get(b["severity"], 99),
                       b.get("capability") or "")
    )
    return blockers


# ---------------------------------------------------------------------------
# Orchestrator
# ---------------------------------------------------------------------------
def _read_text(path):
    try:
        with open(path, "r", encoding="utf-8") as fh:
            return fh.read()
    except (UnicodeDecodeError, OSError):
        return None


def _read_json(path):
    """Read and parse a JSON file, returning None on any error."""
    try:
        with open(path, "rb") as fh:
            raw = fh.read()
    except OSError:
        return None
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        return None
    if "\x00" in text:
        return None
    try:
        return json.loads(text)
    except (ValueError, json.JSONDecodeError):
        return None


def _rel(root, path):
    try:
        return str(Path(path).relative_to(root)).replace(os.sep, "/")
    except ValueError:
        return str(path).replace(os.sep, "/")


def _find_evidence_index(root_path):
    """Scan ``reports/`` for a JSON file whose version is ``evidence-index/1``.

    Returns the parsed dict or None.
    """
    reports_dir = root_path / "reports"
    if not reports_dir.is_dir():
        return None
    for dirpath, _dirnames, filenames in os.walk(str(reports_dir)):
        for fn in sorted(filenames):
            if not fn.endswith(".json"):
                continue
            full = os.path.join(dirpath, fn)
            obj = _read_json(full)
            if isinstance(obj, dict) and obj.get("version") == "evidence-index/1":
                return obj
    return None


def _now_iso():
    return (
        datetime.now(timezone.utc)
        .isoformat(timespec="seconds")
        .replace("+00:00", "Z")
    )


def _build_summary(blockers):
    by_severity = {"blocker": 0, "high": 0, "medium": 0, "low": 0}
    by_platform = {"ios": 0, "android": 0, "harmony": 0, "all": 0, "unknown": 0}
    for b in blockers:
        sev = b.get("severity")
        if sev in by_severity:
            by_severity[sev] += 1
        plat = b.get("platform")
        if plat in by_platform:
            by_platform[plat] += 1
        else:
            by_platform["unknown"] += 1
    return {
        "total": len(blockers),
        "by_severity": by_severity,
        "by_platform": by_platform,
    }


def collect(root, evidence_index_path=None):
    """Orchestrator: load inputs from ``root`` and derive the blockers list.

    * Loads ``reports/tooling/capability-catalog.json`` if present, else
      falls back to parsing ``FEATURE_MATRIX.md``.
    * Loads the evidence index from ``evidence_index_path`` if given, else
      scans ``reports/`` for an ``evidence-index/1`` file. If none found,
      evidence is empty.
    * Loads the migration ledger from
      ``reports/legado-migration-master-audit/branch-integration-ledger.md``
      if present; absent ledger degrades to an empty list (never crashes).

    Returns the release-blockers report dict.
    """
    root_path = Path(root)

    # 1. Capabilities.
    capabilities = []
    catalog_path = root_path / "reports" / "tooling" / "capability-catalog.json"
    if catalog_path.is_file():
        obj = _read_json(catalog_path)
        if isinstance(obj, dict):
            caps = obj.get("capabilities")
            if isinstance(caps, list):
                capabilities = caps
    if not capabilities:
        matrix_path = root_path / "FEATURE_MATRIX.md"
        if matrix_path.is_file():
            text = _read_text(matrix_path)
            if text is not None:
                capabilities = parse_feature_matrix(text)

    # 2. Evidence index.
    evidence_entries = []
    if evidence_index_path:
        ev_obj = _read_json(evidence_index_path)
        if isinstance(ev_obj, dict):
            evidence_entries = parse_evidence_index(ev_obj)
    else:
        ev_obj = _find_evidence_index(root_path)
        if ev_obj is not None:
            evidence_entries = parse_evidence_index(ev_obj)

    # 3. Migration ledger (absent -> []).
    ledger_path = (root_path / "reports" / "legado-migration-master-audit"
                   / "branch-integration-ledger.md")
    ledger_text = None
    if ledger_path.is_file():
        ledger_text = _read_text(ledger_path)
    ledger_rows = parse_migration_ledger(ledger_text)

    blockers = derive_blockers(capabilities, evidence_entries, ledger_rows)
    summary = _build_summary(blockers)

    return {
        "version": VERSION,
        "generated_at": _now_iso(),
        "tool": TOOL,
        "blockers": blockers,
        "summary": summary,
    }


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------
def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="release_blockers",
        description="Generate a release-blockers register JSON from the "
        "feature matrix, evidence index, and migration ledger.",
    )
    parser.add_argument(
        "root", nargs="?", default=os.getcwd(),
        help="repository root to scan (default: cwd)",
    )
    parser.add_argument(
        "--evidence-index", default=None, metavar="PATH",
        help="path to an evidence-index/1 JSON file (default: scan reports/)",
    )
    parser.add_argument(
        "--pretty", action="store_true",
        help="indent=2, sorted keys, trailing newline",
    )
    parser.add_argument(
        "--out", default=None, metavar="PATH",
        help="also write the report to this file",
    )
    args = parser.parse_args(argv)

    root = Path(args.root)
    if not root.is_dir():
        sys.stderr.write(
            "error: scan root not found or not a directory: %s\n" % root
        )
        return 2

    try:
        report = collect(root, evidence_index_path=args.evidence_index)
    except OSError as exc:
        sys.stderr.write("error: %s\n" % exc)
        return 2

    if args.pretty:
        out = json.dumps(report, indent=2, sort_keys=True)
    else:
        out = json.dumps(report)
    sys.stdout.write(out + "\n")

    if args.out:
        try:
            out_path = Path(args.out)
            out_path.parent.mkdir(parents=True, exist_ok=True)
            out_path.write_text(out + "\n", encoding="utf-8")
        except OSError as exc:
            sys.stderr.write("error: %s\n" % exc)
            return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
