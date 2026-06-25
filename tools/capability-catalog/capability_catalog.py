#!/usr/bin/env python3
"""Capability catalog generator.

READ-ONLY dev-time scanner that walks ``protocol/**`` and
``docs/host-app-contracts/**`` to produce a capability catalog listing each
capability as implemented / missing / host-owned / core-owned / shared.

The disk-walking ``collect(root)`` orchestrator delegates to pure parsers
that take CONTENT STRINGS (so they are unit-testable with canned input):

  * ``parse_methods_from_schema(text)`` - method strings from a JSON schema.
  * ``parse_feature_matrix(text)`` - owner/status rows from a markdown table.
  * ``parse_host_contracts(file_iter)`` - coarse capability tokens from host
    contract docs.

Python 3.9+ standard library only.

CLI:
    python3 tools/capability-catalog/capability_catalog.py [root]
        [--pretty] [--out PATH]

Exit codes:
    0  - report emitted (it is a report, not a gate)
    2  - usage error
"""

import argparse
import json
import os
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

VERSION = "capability-catalog/1"
TOOL = "capability-catalog-generator"
DEFAULT_PLATFORMS = ["ios", "android", "harmony"]

# Markdown table row prefix.
_TABLE_ROW_RE = re.compile(r"^\s*\|")
# Separator row: every cell is only dashes/colons/whitespace, e.g. "|---|:---:|".
_SEP_CELL_RE = re.compile(r"^:?-+:?$")
# Method-like token: word.word (at least one dot, word chars around it).
_METHOD_TOKEN_RE = re.compile(r"\b([A-Za-z][\w]*(?:\.[A-Za-z][\w]*)+)\b")
# Backtick-quoted token inside checklist item text.
_BACKTICK_TOKEN_RE = re.compile(r"`([^`]+)`")

# Owner tokens.
_OWNER_HOST_TOKENS = ("平台负责", "Platform Adapter", "platform adapter")
_OWNER_CORE_TOKENS = ("Rust Core", "rust core")
# Status tokens.
_STATUS_TOKENS = {
    "已完成": "implemented",
    "部分完成": "partial",
    "Gap": "missing",
    "gap": "missing",
}


# ---------------------------------------------------------------------------
# Pure parsers
# ---------------------------------------------------------------------------
def _extract_strings(value):
    """Yield strings from a JSON value (str or list of str)."""
    if isinstance(value, str):
        yield value
    elif isinstance(value, list):
        for item in value:
            if isinstance(item, str):
                yield item


def parse_methods_from_schema(text):
    """Return a sorted, deduplicated list of method strings found in ``text``.

    Recognizes, inside a JSON schema:
      * ``"method": {"const": "X"}`` and ``"method": {"enum": [...]}``
        and ``"method": {"examples": [...]}``.
      * ``"x-reader-core-v1-capabilities": ["X", ...]``.
      * ``"type": {"const": "X"}`` where ``X`` looks like a method
        (contains a dot) - captures event types such as ``host.request``
        while excluding bare tokens like ``result`` / ``error``.

    Returns ``[]`` if the text is not valid JSON or contains no methods.
    """
    try:
        data = json.loads(text)
    except (json.JSONDecodeError, ValueError):
        return []

    found = set()

    def visit(node):
        if isinstance(node, dict):
            for key, val in node.items():
                if key == "method" and isinstance(val, dict):
                    for sub in ("const", "enum", "examples"):
                        if sub in val:
                            for s in _extract_strings(val[sub]):
                                found.add(s)
                elif key == "type" and isinstance(val, dict) and "const" in val:
                    s = val["const"]
                    if isinstance(s, str) and "." in s:
                        found.add(s)
                elif key == "x-reader-core-v1-capabilities":
                    for s in _extract_strings(val):
                        found.add(s)
                else:
                    visit(val)
        elif isinstance(node, list):
            for item in node:
                visit(item)

    visit(data)
    return sorted(found)


def _split_table_row(line):
    """Split a ``| a | b | c |`` line into stripped cells (no outer pipes)."""
    stripped = line.strip()
    if stripped.startswith("|"):
        stripped = stripped[1:]
    if stripped.endswith("|"):
        stripped = stripped[:-1]
    return [cell.strip() for cell in stripped.split("|")]


def _is_separator_row(cells):
    return bool(cells) and all(_SEP_CELL_RE.match(c) for c in cells if c != "")


def _resolve_owner_status(cells, header):
    """Resolve owner (core/host) and status from table cells.

    ``header`` is the list of header cell texts (parallel to ``cells``) or
    ``None`` if no header was seen. Owner may come from a column header
    matching "Rust Core" / "Platform Adapter" with a checkmark in the cell,
    or from an owner token appearing as a cell value.
    """
    owner = None
    status = None

    for idx, cell in enumerate(cells):
        if cell == "":
            continue
        has_check = "✅" in cell
        col_name = header[idx] if (header and idx < len(header)) else ""

        # Column-header-driven ownership (real FEATURE_MATRIX shape).
        # A checkmark marks OWNERSHIP, not completion -- it must NOT imply
        # "implemented". Status only comes from explicit status tokens
        # (已完成/部分完成/Gap) or the V1 checklist (see parse_checklist).
        if has_check:
            col_lower = col_name.lower()
            if "rust core" in col_lower:
                owner = "core"
            elif "platform adapter" in col_lower or "平台" in col_name:
                owner = "host"

        # Token-driven ownership (synthetic Owner-column shape).
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


def parse_feature_matrix(text):
    """Parse markdown table rows into capability entries.

    Returns a list of dicts:
        ``{"name", "owner", "status", "evidence"}``
    where ``owner`` is ``"core"|"host"|"shared"|None`` (caller defaults),
    ``status`` is ``"implemented"|"partial"|"missing"|None``, and
    ``evidence`` is the raw cell text of the row.
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
            "name": name,
            "owner": owner,
            "status": status,
            "evidence": " | ".join(cells),
        })
    return rows


def parse_checklist(text, section=None):
    """Parse markdown checkbox items (``- [x]`` / ``- [ ]``).

    If ``section`` is given (e.g. ``"V1 功能边界"``), only items under the
    matching ``## <section>`` heading (up to the next ``## `` heading) are
    parsed -- this avoids conflating the V1 boundary checklist with the
    migration/retirement checklist.

    Returns a list of dicts:
        ``{"text", "done": bool, "methods": [str], "keywords": [str]}``
    where ``methods`` are backtick-quoted method-like tokens (contain a
    dot) and ``keywords`` are uppercase Latin tokens (len >= 3) extracted
    from the item text outside backticks.
    """
    items = []
    lines = (text or "").splitlines()
    if section is not None:
        scoped = []
        in_section = False
        for line in lines:
            stripped = line.lstrip()
            if stripped.startswith("## "):
                in_section = stripped[3:].strip() == section
                continue
            if in_section:
                scoped.append(line)
        lines = scoped

    check_re = re.compile(r"^\s*[-*]\s*\[([ xX])\]\s*(.*)$")
    backtick_re = re.compile(r"`([^`]+)`")
    keyword_re = re.compile(r"[A-Za-z]{3,}")
    for line in lines:
        m = check_re.match(line)
        if not m:
            continue
        done = m.group(1).lower() == "x"
        body = m.group(2)
        methods = [t for t in backtick_re.findall(body) if "." in t]
        text_only = backtick_re.sub(" ", body)
        keywords = sorted({w.upper() for w in keyword_re.findall(text_only)})
        items.append({
            "text": body.strip(),
            "done": done,
            "methods": methods,
            "keywords": keywords,
        })
    return items


def parse_host_contracts(file_iter):
    """Return a sorted list of capability tokens mentioned in host contracts.

    Coarse extraction: collect ``word.word`` tokens from markdown headings
    and fenced code blocks across an iterable of ``(path, text)`` tuples.
    """
    found = set()
    for _path, text in file_iter:
        in_fence = False
        for line in (text or "").splitlines():
            stripped = line.lstrip()
            if stripped.startswith("```"):
                in_fence = not in_fence
                continue
            if stripped.startswith("#"):
                for m in _METHOD_TOKEN_RE.finditer(line):
                    found.add(m.group(1))
            elif in_fence:
                for m in _METHOD_TOKEN_RE.finditer(line):
                    found.add(m.group(1))
    return sorted(found)


# ---------------------------------------------------------------------------
# Orchestrator
# ---------------------------------------------------------------------------
def _slug(name):
    """Slugify a name into a stable id (preserves dots and unicode letters)."""
    s = re.sub(r"[^\w.]+", "-", name.lower(), flags=re.UNICODE)
    return s.strip("-")


def _humanize(id_value):
    """Humanize an id: replace dots with spaces and title-case."""
    return id_value.replace(".", " ").title()


def _read_text(path):
    try:
        with open(path, "r", encoding="utf-8") as fh:
            return fh.read()
    except (UnicodeDecodeError, OSError):
        return None


def _rel(root, path):
    """Repo-relative posix path for ``path`` under ``root``."""
    try:
        return str(Path(path).relative_to(root)).replace(os.sep, "/")
    except ValueError:
        return str(path).replace(os.sep, "/")


def collect(root):
    """Walk ``root`` and return a capability catalog dict.

    Scans ``protocol/*.schema.json``, ``protocol/compatibility.md``,
    ``FEATURE_MATRIX.md`` and ``docs/host-app-contracts/**/*.md``. Missing
    directories are skipped silently. The catalog has the shape::

        {"version", "generated_at", "tool", "capabilities": [...],
         "summary": {"total", "by_owner", "by_status"}}
    """
    root_path = Path(root)
    root_str = str(root_path)

    # capability key (id) -> mutable record
    caps = {}

    def _ensure(key, default_source):
        if key not in caps:
            caps[key] = {
                "id": key,
                "name_raw": key,
                "from_core": False,
                "from_host": False,
                "status": None,
                "evidence_files": [],
                "source": default_source,
            }
        return caps[key]

    # 1. protocol/*.schema.json -> method names (core-owned by default).
    protocol_dir = root_path / "protocol"
    if protocol_dir.is_dir():
        for schema_path in sorted(protocol_dir.glob("*.schema.json")):
            rel = _rel(root_str, schema_path)
            text = _read_text(schema_path)
            if not text:
                continue
            for method in parse_methods_from_schema(text):
                rec = _ensure(method, rel)
                rec["from_core"] = True
                if rec["status"] is None:
                    rec["status"] = "implemented"
                if rel not in rec["evidence_files"]:
                    rec["evidence_files"].append(rel)

    # 2. protocol/compatibility.md -> markdown table rows (enrich).
    compat_path = root_path / "protocol" / "compatibility.md"
    if compat_path.is_file():
        text = _read_text(compat_path)
        if text:
            rel = _rel(root_str, compat_path)
            for entry in parse_feature_matrix(text):
                key = entry["name"]
                rec = _ensure(key, rel)
                if entry["owner"] == "core":
                    rec["from_core"] = True
                elif entry["owner"] == "host":
                    rec["from_host"] = True
                if entry["status"]:
                    rec["status"] = entry["status"]
                if rel not in rec["evidence_files"]:
                    rec["evidence_files"].append(rel)

    # 3. FEATURE_MATRIX.md -> owner/status enrichment.
    matrix_path = root_path / "FEATURE_MATRIX.md"
    matrix_text = _read_text(matrix_path) if matrix_path.is_file() else None
    if matrix_text:
        rel = _rel(root_str, matrix_path)
        for entry in parse_feature_matrix(matrix_text):
            key = entry["name"]
            # Match schema-derived methods by raw name; otherwise the id
            # is the slugified name.
            if key not in caps:
                key = _slug(entry["name"])
            rec = _ensure(key, rel)
            rec["name_raw"] = entry["name"]
            if entry["owner"] == "core":
                rec["from_core"] = True
            elif entry["owner"] == "host":
                rec["from_host"] = True
            if entry["status"]:
                rec["status"] = entry["status"]
            if rel not in rec["evidence_files"]:
                rec["evidence_files"].append(rel)

    # 3b. V1 checklist (in FEATURE_MATRIX.md) -> status by method + keyword.
    # A checkmark in the ownership table marks OWNERSHIP only; the V1
    # boundary checklist is the real source of done/not-done. An undone
    # checklist item overrides any schema-default "implemented" status.
    if matrix_text:
        rel = _rel(root_str, matrix_path)
        checklist = parse_checklist(matrix_text, section="V1 功能边界")
        # Index capabilities by uppercase keyword for keyword matching.
        # Only capabilities whose name_raw contains the keyword qualify.
        for item in checklist:
            new_status = "implemented" if item["done"] else "missing"
            matched_any = False
            # Method-token match: pin a specific capability id.
            for method in item["methods"]:
                if method in caps:
                    caps[method]["status"] = new_status
                    if rel not in caps[method]["evidence_files"]:
                        caps[method]["evidence_files"].append(rel)
                    matched_any = True
            # Keyword match: an undone item with an uppercase keyword (e.g.
            # "EPUB", "TXT") marks any capability whose name_raw contains
            # that keyword as missing. Done items do not keyword-promote
            # (a done checklist item only certifies the methods it names).
            if not item["done"]:
                for kw in item["keywords"]:
                    for rec in caps.values():
                        raw = rec.get("name_raw") or ""
                        if kw and kw in raw.upper():
                            rec["status"] = "missing"
                            if rel not in rec["evidence_files"]:
                                rec["evidence_files"].append(rel)
                            matched_any = True
            # If a checklist item matched nothing it is informational only.

    # 4. docs/host-app-contracts/**/*.md -> host-owned mentions.
    host_dir = root_path / "docs" / "host-app-contracts"
    if host_dir.is_dir():
        for dirpath, _dirnames, filenames in os.walk(str(host_dir)):
            for fn in filenames:
                if not fn.endswith(".md"):
                    continue
                abs_path = os.path.join(dirpath, fn)
                rel = _rel(root_str, abs_path)
                text = _read_text(abs_path)
                if text is None:
                    continue
                for name in parse_host_contracts([(rel, text)]):
                    rec = _ensure(name, rel)
                    rec["from_host"] = True
                    if rec["status"] is None:
                        rec["status"] = "unknown"
                    if rel not in rec["evidence_files"]:
                        rec["evidence_files"].append(rel)

    # Build output capabilities.
    capabilities = []
    for key in sorted(caps.keys()):
        rec = caps[key]
        if rec["from_core"] and rec["from_host"]:
            owner = "shared"
        elif rec["from_host"]:
            owner = "host"
        else:
            owner = "core"
        status = rec["status"] or "unknown"
        capabilities.append({
            "id": rec["id"],
            "name": _humanize(rec["id"]),
            "owner": owner,
            "status": status,
            "evidence": list(rec["evidence_files"]),
            "source": rec["source"],
            "platforms": list(DEFAULT_PLATFORMS),
            "notes": "",
        })

    by_owner = {"core": 0, "host": 0, "shared": 0}
    by_status = {"implemented": 0, "partial": 0, "missing": 0, "unknown": 0}
    for cap in capabilities:
        by_owner[cap["owner"]] += 1
        by_status[cap["status"]] += 1

    return {
        "version": VERSION,
        "generated_at": _now_iso(),
        "tool": TOOL,
        "capabilities": capabilities,
        "summary": {
            "total": len(capabilities),
            "by_owner": by_owner,
            "by_status": by_status,
        },
    }


def _now_iso():
    return (
        datetime.now(timezone.utc)
        .isoformat(timespec="seconds")
        .replace("+00:00", "Z")
    )


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------
def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="capability_catalog",
        description="Generate a capability catalog JSON for a scanned root.",
    )
    parser.add_argument(
        "root", nargs="?", default=os.getcwd(),
        help="repository root to scan (default: cwd)",
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

    catalog = collect(root)

    if args.pretty:
        out = json.dumps(catalog, indent=2, sort_keys=True)
    else:
        out = json.dumps(catalog)
    sys.stdout.write(out + "\n")

    if args.out:
        out_path = Path(args.out)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(out + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
