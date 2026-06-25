#!/usr/bin/env python3
"""Gate declaration checker.

Verifies that each platform (iOS/Android/HarmonyOS) has its release gates
DECLARED: ``simulator``, ``device``, ``corpus``. When any of those is missing
for any platform the run FAILS CLOSED (CLI exit 1).

Declaration sources, in priority order:
  1. Explicit file ``docs/ci-gates/gates.json`` (version ``gate-declaration/1``).
  2. Heuristic scan of ``scripts/*.sh`` for platform + gate keywords.
  3. Otherwise the gate is ``declared=false`` with ``notes="missing"``.

Pure Python 3.9+ standard library only.

CLI:
    python3 tools/gate-declaration/gate_declaration.py [root] [--pretty]

Exit codes: 0 = all gates declared (fail_closed true); 1 = some gate missing
(fail_closed false, FAIL CLOSED); 2 = usage / IO error.
"""

import argparse
import json
import os
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

DECL_VERSION = "gate-declaration/1"
REPORT_VERSION = "gate-declaration-report/1"
TOOL_NAME = "gate-declaration-checker"

PLATFORMS = ("ios", "android", "harmony")
GATES = ("simulator", "device", "corpus")

# Heuristic keyword patterns (case-insensitive substring via regex alternation).
_GATE_KEYWORDS = {
    "simulator": re.compile(r"sim|simulator|xcframework|host-sim", re.IGNORECASE),
    "device": re.compile(r"device|真机|hardware|on-device", re.IGNORECASE),
    "corpus": re.compile(r"corpus|fixture|conformance", re.IGNORECASE),
}


def _missing():
    """Default entry for an undeclared gate."""
    return {"declared": False, "source": "", "notes": "missing"}


def _platform_of(path):
    """Map a script path to a platform by name. ``ohos`` -> ``harmony``."""
    name = str(path).lower()
    if "ios" in name:
        return "ios"
    if "android" in name:
        return "android"
    if "harmony" in name:
        return "harmony"
    if "ohos" in name:
        return "harmony"
    return None


def parse_explicit_decl(text):
    """Parse an explicit gates.json document.

    Returns ``{platform: {gate: {"declared","source","notes"}}}`` or ``None``
    if ``text`` is not valid JSON or the version is not ``gate-declaration/1``.
    Pure.
    """
    if not isinstance(text, str):
        return None
    try:
        obj = json.loads(text)
    except (json.JSONDecodeError, ValueError):
        return None
    if not isinstance(obj, dict):
        return None
    if obj.get("version") != DECL_VERSION:
        return None
    platforms = obj.get("platforms")
    if not isinstance(platforms, dict):
        return {}
    return platforms


def heuristic_scan(scripts):
    """Scan ``scripts`` (iterable of (path, content)) for declared gates.

    Returns ``{platform: {gate: {"declared":True,"source":path,"notes":"heuristic"}}}``
    for matches only; absent (platform, gate) pairs are omitted. Pure.
    """
    out = {}
    for path, content in scripts:
        plat = _platform_of(path)
        if plat is None:
            continue
        if not isinstance(content, str):
            content = str(content)
        for gate, pattern in _GATE_KEYWORDS.items():
            if pattern.search(content):
                out.setdefault(plat, {})[gate] = {
                    "declared": True,
                    "source": path,
                    "notes": "heuristic",
                }
    return out


def merge(explicit, heuristic):
    """Combine explicit + heuristic declarations.

    Explicit wins per (platform, gate); heuristic fills the rest; anything
    missing from both becomes the ``missing`` default. Always returns the full
    3x3 grid (all platforms, all gates). Pure.
    """
    explicit = explicit or {}
    heuristic = heuristic or {}
    merged = {}
    for plat in PLATFORMS:
        merged[plat] = {}
        e_plat = explicit.get(plat) or {}
        h_plat = heuristic.get(plat) or {}
        for gate in GATES:
            entry = e_plat.get(gate)
            if entry is None:
                entry = h_plat.get(gate)
            if entry is None:
                entry = _missing()
            else:
                entry = dict(entry)
                entry.setdefault("declared", False)
                entry.setdefault("source", "")
                entry.setdefault("notes", "")
            merged[plat][gate] = entry
    return merged


def evaluate(decl):
    """Add per-platform ``fail_closed`` + ``issues`` and overall ``fail_closed``.

    Returns ``{"platforms": [...], "fail_closed": bool}``. Pure.
    """
    decl = decl or {}
    platforms_out = []
    overall = True
    for plat in PLATFORMS:
        gates_out = {}
        issues = []
        plat_decl = decl.get(plat) or {}
        for gate in GATES:
            entry = plat_decl.get(gate)
            if not isinstance(entry, dict):
                entry = _missing()
            declared = bool(entry.get("declared", False))
            gates_out[gate] = {
                "declared": declared,
                "source": entry.get("source", "") or "",
                "notes": entry.get("notes", "") or "",
            }
            if not declared:
                issues.append("missing %s gate" % gate)
        fail_closed = not issues
        if not fail_closed:
            overall = False
        platforms_out.append(
            {
                "platform": plat,
                "gates": gates_out,
                "fail_closed": fail_closed,
                "issues": issues,
            }
        )
    return {"platforms": platforms_out, "fail_closed": overall}


def collect(root):
    """Orchestrator: read explicit gates.json if present, else/also scan
    ``scripts/*.sh`` heuristically, merge, evaluate, and return the report.

    Always includes all three platforms. Read-only over the worktree.
    """
    root = Path(root)

    explicit = None
    gates_file = root / "docs" / "ci-gates" / "gates.json"
    if gates_file.is_file():
        try:
            text = gates_file.read_text(encoding="utf-8")
        except OSError:
            text = ""
        explicit = parse_explicit_decl(text)

    scripts = []
    scripts_dir = root / "scripts"
    if scripts_dir.is_dir():
        for sh in sorted(scripts_dir.glob("*.sh")):
            try:
                content = sh.read_text(encoding="utf-8")
            except OSError:
                content = ""
            rel = sh.relative_to(root)
            rel_str = str(rel).replace(os.sep, "/")
            scripts.append((rel_str, content))
    heuristic = heuristic_scan(scripts)

    merged = merge(explicit, heuristic)
    ev = evaluate(merged)

    missing_gates = sum(
        1 for p in ev["platforms"] for g in p["gates"].values() if not g["declared"]
    )
    fail_closed_count = sum(1 for p in ev["platforms"] if p["fail_closed"])

    return {
        "version": REPORT_VERSION,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "tool": TOOL_NAME,
        "platforms": ev["platforms"],
        "fail_closed": ev["fail_closed"],
        "summary": {
            "platforms": len(ev["platforms"]),
            "missing_gates": missing_gates,
            "fail_closed_count": fail_closed_count,
        },
    }


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="gate_declaration",
        description="Verify each platform declares simulator/device/corpus "
        "release gates. Fails closed (exit 1) when any gate is missing.",
    )
    parser.add_argument(
        "root", nargs="?", default=".", help="repo root (default: cwd)"
    )
    parser.add_argument(
        "--pretty",
        action="store_true",
        help="indent=2, sorted keys, trailing newline",
    )
    args = parser.parse_args(argv)

    root = Path(args.root)
    if not root.is_dir():
        sys.stderr.write(
            "error: root not found or not a directory: %s\n" % root
        )
        return 2

    try:
        report = collect(root)
    except OSError as exc:
        sys.stderr.write("error: %s\n" % exc)
        return 2

    if args.pretty:
        out = json.dumps(report, indent=2, sort_keys=True)
    else:
        out = json.dumps(report)
    sys.stdout.write(out + "\n")

    return 0 if report["fail_closed"] else 1


if __name__ == "__main__":
    sys.exit(main())
