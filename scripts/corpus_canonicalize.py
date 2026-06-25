#!/usr/bin/env python3
"""Corpus canonicalizer.

Normalizes JSON result files (chapter content, TOC / directory, search
results, book detail) produced by different platforms into a single
comparable canonical JSON form, so that synonymous outputs collapse to
the same bytes.

Normalizations applied:
  1. Field order      — object keys sorted alphabetically (recursively).
  2. Whitespace       — runs of non-newline whitespace collapsed to one
                        space; each line stripped; leading/trailing blank
                        lines removed.
  3. Newlines         — CRLF / CR normalized to LF.
  4. HTML entities    — named (&amp; &lt; &nbsp; ...) and numeric
                        (&#65; &#x42;) entities decoded.
  5. URL trailing     — a single trailing '/' on the path component of
     slash             http(s) URLs is removed (root and query preserved).
  6. Variable fields  — values of known run-volatile fields (timestamps,
                        request/trace ids) are replaced with a sentinel so
                        two runs that differ only in these fields compare
                        equal.

No network access. No remote data. Pure local text transformation.

Usage:
    python3 scripts/corpus_canonicalize.py input.json
    python3 scripts/corpus_canonicalize.py input.json -o output.json
"""

import argparse
import html
import json
import re
import sys


# Field names whose values are run-variable (timestamps, request/trace ids).
# Values of these fields are replaced with a constant sentinel so that two
# runs that differ only in these volatile fields compare equal.
VARIABLE_FIELDS = frozenset({
    "timestamp", "time", "ts", "t",
    "created_at", "updated_at", "created", "updated",
    "date", "datetime",
    "request_id", "req_id", "requestId",
    "trace_id", "traceId",
})

VARIABLE_SENTINEL = "<normalized>"

_URL_RE = re.compile(r"^https?://", re.IGNORECASE)
# Whitespace that is not a newline (includes space, tab, nbsp, etc.).
_NON_NL_WS_RE = re.compile(r"[^\S\n]+")


def _normalize_string(s):
    # 1. Decode HTML entities (named + numeric).
    s = html.unescape(s)
    # 2. Normalize line endings to LF.
    s = s.replace("\r\n", "\n").replace("\r", "\n")
    # 3. Collapse runs of non-newline whitespace to a single space.
    s = _NON_NL_WS_RE.sub(" ", s)
    # 4. Strip each line (normalizes per-line indentation) and rejoin.
    s = "\n".join(line.strip() for line in s.split("\n"))
    # 5. Remove leading/trailing blank lines.
    s = s.strip()
    # 6. URL trailing-slash normalization on the path component.
    if _URL_RE.match(s):
        if "?" in s:
            path, query = s.split("?", 1)
            s = path.rstrip("/") + "?" + query
        else:
            stripped = s.rstrip("/")
            # Guard: never strip the "//" in "scheme://" (degenerate input).
            if _URL_RE.match(stripped):
                s = stripped
    return s


def canonicalize(obj):
    """Return a canonicalized copy of ``obj`` (dict / list / scalar)."""
    if isinstance(obj, dict):
        out = {}
        for key in sorted(obj.keys()):
            if key in VARIABLE_FIELDS:
                out[key] = VARIABLE_SENTINEL
            else:
                out[key] = canonicalize(obj[key])
        return out
    if isinstance(obj, list):
        return [canonicalize(item) for item in obj]
    if isinstance(obj, str):
        return _normalize_string(obj)
    return obj


def serialize(obj):
    """Serialize a canonicalized object to stable JSON text."""
    return json.dumps(obj, sort_keys=True, indent=2, ensure_ascii=False)


def canonicalize_text(text):
    """Parse JSON text and return its canonical JSON serialization."""
    return serialize(canonicalize(json.loads(text)))


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Canonicalize a corpus JSON result file for comparison.",
    )
    parser.add_argument("input", help="path to input JSON file")
    parser.add_argument(
        "-o", "--output",
        help="path to output file (default: write to stdout)",
    )
    args = parser.parse_args(argv)

    with open(args.input, "r", encoding="utf-8") as f:
        data = json.load(f)

    result = serialize(canonicalize(data))

    if args.output:
        with open(args.output, "w", encoding="utf-8") as f:
            f.write(result + "\n")
    else:
        sys.stdout.write(result + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
