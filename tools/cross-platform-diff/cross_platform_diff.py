#!/usr/bin/env python3
"""Cross-platform result diff tool.

Compares canonical JSON outputs produced by iOS, Android and HarmonyOS for the
same corpus item, and reports field-level differences:

  * ``missing``       - field present in the reference but absent in a candidate
  * ``extra``         - field present in a candidate but absent in the reference
  * ``changed``       - field present in both, same JSON type, different value
  * ``type_mismatch`` - field present in both, different JSON types

The comparison is reference-based: one platform is the reference (default
``ios``) and every other provided platform is diffed against it. Platform
metadata fields such as ``device``, ``timestamp`` and ``runtimeVersion`` can be
ignored via ``--ignore``.

This module is intentionally stdlib-only (no third-party dependencies) and does
not touch any Core business logic.
"""

import argparse
import json
import sys


def json_type(value):
    """Return the JSON type name for a Python value decoded from JSON."""
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, (int, float)):
        return "number"
    if isinstance(value, str):
        return "string"
    if isinstance(value, list):
        return "array"
    if isinstance(value, dict):
        return "object"
    return "unknown"


def _child_path(path, key):
    return "{}.{}".format(path, key)


def _index_path(path, index):
    return "{}[{}]".format(path, index)


def diff_values(a, b, path="$", ignore_fields=None):
    """Recursively diff reference value ``a`` against candidate value ``b``.

    Returns a list of diff entry dicts. Each entry has a ``category`` and a
    ``path`` (JSONPath-ish, ``$`` for the root, ``$.field`` for object fields,
    ``$[i]`` for array indices) plus value fields depending on the category:

      * ``changed``       - ``reference_value``, ``candidate_value``
      * ``type_mismatch`` - ``reference_type``, ``reference_value``,
                            ``candidate_type``, ``candidate_value``
      * ``missing``       - ``reference_value``
      * ``extra``         - ``candidate_value``

    ``ignore_fields`` is an optional set of object key names to skip anywhere
    in the tree (matched by the final path segment, i.e. the field name).
    """
    ignore = set(ignore_fields) if ignore_fields else set()
    entries = []
    ta, tb = json_type(a), json_type(b)

    if ta == "object" and tb == "object":
        for key in a:
            if key in ignore:
                continue
            child = _child_path(path, key)
            if key not in b:
                entries.append(
                    {"category": "missing", "path": child, "reference_value": a[key]}
                )
            else:
                entries.extend(diff_values(a[key], b[key], child, ignore))
        for key in b:
            if key in ignore:
                continue
            if key not in a:
                entries.append(
                    {"category": "extra", "path": _child_path(path, key),
                     "candidate_value": b[key]}
                )
        return entries

    if ta == "array" and tb == "array":
        for i in range(max(len(a), len(b))):
            child = _index_path(path, i)
            if i < len(a) and i < len(b):
                entries.extend(diff_values(a[i], b[i], child, ignore))
            elif i < len(a):
                entries.append(
                    {"category": "missing", "path": child, "reference_value": a[i]}
                )
            else:
                entries.append(
                    {"category": "extra", "path": child, "candidate_value": b[i]}
                )
        return entries

    # Either scalars, or structurally different types (e.g. object vs array).
    if ta != tb:
        entries.append({
            "category": "type_mismatch",
            "path": path,
            "reference_type": ta,
            "reference_value": a,
            "candidate_type": tb,
            "candidate_value": b,
        })
        return entries

    # Same JSON type, leaf comparison.
    if a != b:
        entries.append({
            "category": "changed",
            "path": path,
            "reference_value": a,
            "candidate_value": b,
        })
    return entries


CATEGORIES = ("missing", "extra", "changed", "type_mismatch")
TOOL_NAME = "cross-platform-diff"
TOOL_VERSION = "1.0"


def compare_platforms(platforms, reference, ignore_fields=None):
    """Diff every non-reference platform against the reference platform.

    ``platforms`` maps platform name -> parsed JSON value. ``reference`` is the
    name of the platform to diff against (must be a key in ``platforms``).
    ``ignore_fields`` is an optional list of object key names to skip anywhere.

    Returns a result dict with ``tool``, ``version``, ``reference``,
    ``candidates``, ``ignored_fields``, ``diffs`` (per-candidate buckets of
    ``missing``/``extra``/``changed``/``type_mismatch`` entries) and ``summary``
    (per-candidate counts plus ``total``).
    """
    if reference not in platforms:
        raise ValueError(
            "reference platform {!r} not among provided platforms: {}".format(
                reference, ", ".join(sorted(platforms))
            )
        )
    ignore = list(ignore_fields) if ignore_fields else []
    ignore_set = set(ignore)
    ref_value = platforms[reference]
    candidates = [name for name in platforms if name != reference]

    diffs = {}
    summary = {}
    for name in candidates:
        entries = diff_values(ref_value, platforms[name], ignore_fields=ignore_set)
        buckets = {cat: [] for cat in CATEGORIES}
        for entry in entries:
            buckets[entry["category"]].append(entry)
        diffs[name] = buckets
        summary[name] = {cat: len(buckets[cat]) for cat in CATEGORIES}
        summary[name]["total"] = len(entries)

    return {
        "tool": TOOL_NAME,
        "version": TOOL_VERSION,
        "reference": reference,
        "candidates": candidates,
        "ignored_fields": ignore,
        "diffs": diffs,
        "summary": summary,
    }


def render_summary(result):
    """Render a human-readable text summary of a ``compare_platforms`` result."""
    lines = ["Cross-platform diff summary"]
    lines.append("Reference platform: {}".format(result["reference"]))
    if result["ignored_fields"]:
        lines.append("Ignored fields: {}".format(", ".join(result["ignored_fields"])))
    lines.append("")

    any_diff = False
    for name in result["candidates"]:
        s = result["summary"][name]
        lines.append("{} vs {}:".format(name, result["reference"]))
        lines.append(
            "  missing={} extra={} changed={} type_mismatch={} total={}".format(
                s["missing"], s["extra"], s["changed"], s["type_mismatch"], s["total"]
            )
        )
        if s["total"] == 0:
            lines.append("  no differences")
        else:
            any_diff = True
            for cat in CATEGORIES:
                for entry in result["diffs"][name][cat]:
                    lines.append("  [{}] {}".format(cat, entry["path"]))
        lines.append("")

    if not any_diff:
        lines.append("All candidates match the reference (no differences).")

    return "\n".join(lines).rstrip() + "\n"


PLATFORM_NAMES = ("ios", "android", "harmony")


def load_json_file(path):
    """Load and parse a JSON file, raising FileNotFoundError/JSONDecodeError."""
    with open(path, "r", encoding="utf-8") as fh:
        return json.load(fh)


def main(argv=None, stdout=None, stderr=None):
    """CLI entry point. Returns a process exit code (0/1/2).

    0 = no differences, 1 = differences found, 2 = usage / IO / parse error.
    """
    if stdout is None:
        stdout = sys.stdout
    if stderr is None:
        stderr = sys.stderr

    parser = argparse.ArgumentParser(
        prog="cross_platform_diff",
        description="Diff canonical JSON outputs from iOS, Android and HarmonyOS.",
    )
    parser.add_argument("--ios", help="iOS canonical JSON file")
    parser.add_argument("--android", help="Android canonical JSON file")
    parser.add_argument("--harmony", help="HarmonyOS canonical JSON file")
    parser.add_argument(
        "--reference", choices=list(PLATFORM_NAMES), default="ios",
        help="platform to diff against (default: ios)",
    )
    parser.add_argument(
        "--ignore", nargs="+", default=None, metavar="FIELD",
        help="object field names to skip anywhere in the tree",
    )
    parser.add_argument(
        "--format", choices=["json", "summary", "both"], default="both",
        help="json = machine-readable to stdout; summary = human-readable to "
             "stdout; both (default) = json to stdout + summary to stderr",
    )
    args = parser.parse_args(argv)

    # Collect provided platform files in a canonical order.
    platform_files = {}
    for name in PLATFORM_NAMES:
        path = getattr(args, name)
        if path:
            platform_files[name] = path

    if not platform_files:
        print("error: provide at least one platform file via "
              "--ios/--android/--harmony", file=stderr)
        return 2
    if args.reference not in platform_files:
        print("error: reference platform {!r} was not provided via --{}".format(
            args.reference, args.reference), file=stderr)
        return 2
    if len(platform_files) < 2:
        print("error: at least one candidate platform besides the reference "
              "is required", file=stderr)
        return 2

    platforms = {}
    for name, path in platform_files.items():
        try:
            platforms[name] = load_json_file(path)
        except FileNotFoundError:
            print("error: file not found for {!r}: {}".format(name, path),
                  file=stderr)
            return 2
        except json.JSONDecodeError as exc:
            print("error: invalid JSON in {!r} ({}): {}".format(name, path, exc),
                  file=stderr)
            return 2

    result = compare_platforms(
        platforms, reference=args.reference, ignore_fields=args.ignore
    )

    if args.format in ("json", "both"):
        print(json.dumps(result, ensure_ascii=False, indent=2), file=stdout)
    if args.format in ("summary", "both"):
        text = render_summary(result)
        target = stderr if args.format == "both" else stdout
        print(text, end="", file=target)

    any_diff = any(result["summary"][n]["total"] > 0 for n in result["candidates"])
    return 1 if any_diff else 0


if __name__ == "__main__":
    sys.exit(main())
