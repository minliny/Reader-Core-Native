#!/usr/bin/env python3
"""Cross-platform corpus diff.

Compares a single canonical reference result against one or more platform
candidate results, after running every side through the sibling
``corpus_canonicalize`` normalizer. Synonymous outputs (differing only in
field order, whitespace, line endings, HTML entities, URL trailing slash,
or run-variable timestamps) therefore compare equal; only genuine
cross-platform divergence is reported as a difference.

The produced ``diff-result.json`` document is the artifact consumed by the
sibling ``benchmark-run-packager`` (which reads its ``summary`` to derive a
match / total-differences verdict) and by the ``release-blocker-register``
(which turns non-matching candidates into blockers).

Output schema (``schemaVersion`` 1)::

    {
      "schemaVersion": 1,
      "tool": "cross-platform-diff",
      "version": "1.0",
      "canonical": {"path": "...", "sha256": "..."},
      "candidates": {
        "<name>": {
          "path": "...",
          "sha256": "...",
          "match": true|false,
          "total": <int>,
          "differenceClasses": {
            "core-semantic-difference": <int>,
            "host-capability-difference": <int>,
            "platform-output-missing": <int>
          },
          "differences": [
            {
              "path": "<json-pointer-ish>",
              "kind": "...",
              "classification": "core-semantic-difference | ...",
              "canonical": <snip>,
              "candidate": <snip>
            }
          ]
        }
      },
      "summary": {
        "<name>": {"match": true|false, "total": <int>, "differenceClasses": {...}}
      },
      "releaseGate": {
        "status": "not-evaluated | passed | blocked",
        "requiredCandidates": ["ios", "android", "harmony"],
        "missingCandidates": [],
        "mismatchingCandidates": [],
        "blockedReasons": []
      },
      "match": true|false,
      "total": <int>
    }

No network access. No remote data. No Core business logic. Pure local
comparison of already-produced JSON result files.

Usage::

    # one candidate
    python3 tools/cross-platform-diff/cross_platform_diff.py \\
        canonical.json --candidate platform-a.json

    # several named candidates
    python3 tools/cross-platform-diff/cross_platform_diff.py \\
        canonical.json \\
        --candidate ios:results/ios.json \\
        --candidate android:results/android.json \\
        --candidate harmony:results/harmony.json \\
        -o diff-result.json
"""

import argparse
import hashlib
import json
import os
import sys

# Make scripts/ importable so we reuse the canonicalizer rather than
# re-implementing its normalizations (single source of truth).
_HERE = os.path.dirname(os.path.abspath(__file__))
_ROOT = os.path.abspath(os.path.join(_HERE, "..", ".."))
sys.path.insert(0, os.path.join(_ROOT, "scripts"))

import corpus_canonicalize as cc  # noqa: E402


TOOL_NAME = "cross-platform-diff"
TOOL_VERSION = "1.1"
SCHEMA_VERSION = 1
DEFAULT_RELEASE_GATE_CANDIDATES = ("ios", "android", "harmony")

CLASS_CORE_SEMANTIC = "core-semantic-difference"
CLASS_HOST_CAPABILITY = "host-capability-difference"
CLASS_PLATFORM_MISSING = "platform-output-missing"

_HOST_PATH_MARKERS = frozenset({
    "host",
    "hostrequest",
    "hostrequests",
    "capability",
    "capabilities",
    "http",
    "request",
    "requests",
    "response",
    "responses",
    "headers",
    "cookies",
    "cookie",
    "status",
    "finalurl",
    "charset",
    "charsethint",
    "bodybase64",
    "transport",
})

# Maximum number of characters of a differing scalar value to retain in the
# emitted difference record. Longer values are truncated with a sentinel so
# the diff document stays reviewable.
_VALUE_SNIPPET_LIMIT = 240


class DiffError(Exception):
    """Raised when inputs cannot be read, parsed, or compared."""


# --------------------------------------------------------------------------- #
# Helpers
# --------------------------------------------------------------------------- #
def sha256_of_file(path, chunk_size=65536):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        while True:
            chunk = handle.read(chunk_size)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def load_canonical_object(path):
    """Parse ``path`` as JSON and return its *canonicalized* form."""
    try:
        with open(path, "r", encoding="utf-8") as handle:
            raw = json.load(handle)
    except FileNotFoundError:
        raise DiffError("canonical file not found: {0}".format(path))
    except (OSError, IOError) as err:
        raise DiffError("cannot read canonical {0}: {1}".format(path, err))
    except json.JSONDecodeError as err:
        raise DiffError("invalid JSON in canonical {0}: {1}".format(path, err))
    return cc.canonicalize(raw)


def _snippet(value):
    """Render a scalar/collection value as a reviewable snippet."""
    try:
        text = json.dumps(value, sort_keys=True, ensure_ascii=False)
    except (TypeError, ValueError):
        text = repr(value)
    if len(text) > _VALUE_SNIPPET_LIMIT:
        return text[:_VALUE_SNIPPET_LIMIT] + "…<truncated>"
    return text


def _join_ptr(prefix, key):
    if prefix == "":
        return str(key)
    return "{0}.{1}".format(prefix, key)


def _path_markers(path):
    lowered = path.lower()
    token = ""
    markers = []
    for ch in lowered:
        if ch.isalnum():
            token += ch
        else:
            if token:
                markers.append(token)
                token = ""
    if token:
        markers.append(token)
    return markers


def classify_difference(diff):
    kind = diff.get("kind") if isinstance(diff, dict) else None
    if kind == CLASS_PLATFORM_MISSING:
        return CLASS_PLATFORM_MISSING

    path = diff.get("path", "") if isinstance(diff, dict) else ""
    for marker in _path_markers(path):
        if marker in _HOST_PATH_MARKERS:
            return CLASS_HOST_CAPABILITY
    return CLASS_CORE_SEMANTIC


def _class_counts(diffs):
    counts = {
        CLASS_CORE_SEMANTIC: 0,
        CLASS_HOST_CAPABILITY: 0,
        CLASS_PLATFORM_MISSING: 0,
    }
    for diff in diffs:
        classification = classify_difference(diff)
        diff["classification"] = classification
        counts[classification] = counts.get(classification, 0) + 1
    return counts


def collect_differences(canonical, candidate, prefix=""):
    """Walk ``canonical`` and ``candidate`` together, returning a list of
    difference records at the paths where they diverge.

    Differences are reported from the canonical side's perspective:
      * a key present in canonical but absent in candidate  → missing in candidate
      * a key present in candidate but absent in canonical  → unexpected in candidate
      * a scalar / type mismatch                            → value difference
    Lists are compared element-wise by index (after canonicalization, list
    order is the original order; the canonicalizer does not reorder lists).
    """
    diffs = []

    if isinstance(canonical, dict) and isinstance(candidate, dict):
        c_keys = set(canonical.keys())
        v_keys = set(candidate.keys())
        for key in sorted(c_keys | v_keys):
            ptr = _join_ptr(prefix, key)
            if key not in candidate:
                diffs.append({
                    "path": ptr,
                    "kind": "missing-in-candidate",
                    "canonical": _snippet(canonical[key]),
                    "candidate": None,
                })
            elif key not in canonical:
                diffs.append({
                    "path": ptr,
                    "kind": "unexpected-in-candidate",
                    "canonical": None,
                    "candidate": _snippet(candidate[key]),
                })
            else:
                diffs.extend(collect_differences(canonical[key], candidate[key], ptr))
        return diffs

    if isinstance(canonical, list) and isinstance(candidate, list):
        length = max(len(canonical), len(candidate))
        for index in range(length):
            ptr = "{0}[{1}]".format(prefix, index)
            if index >= len(canonical):
                diffs.append({
                    "path": ptr,
                    "kind": "unexpected-in-candidate",
                    "canonical": None,
                    "candidate": _snippet(candidate[index]),
                })
            elif index >= len(candidate):
                diffs.append({
                    "path": ptr,
                    "kind": "missing-in-candidate",
                    "canonical": _snippet(canonical[index]),
                    "candidate": None,
                })
            else:
                diffs.extend(collect_differences(canonical[index], candidate[index], ptr))
        return diffs

    # Scalar / type-mismatch leaf.
    if canonical != candidate:
        diffs.append({
            "path": prefix or "<root>",
            "kind": "value-mismatch",
            "canonical": _snippet(canonical),
            "candidate": _snippet(candidate),
        })
    return diffs


# --------------------------------------------------------------------------- #
# Core comparison
# --------------------------------------------------------------------------- #
def compare_candidate(canonical_obj, candidate_path):
    """Compare an already-canonicalized reference to a candidate file.

    Returns ``(match, differences)`` where ``differences`` is the list
    produced by :func:`collect_differences`.
    """
    candidate_obj = load_canonical_object(candidate_path)
    diffs = collect_differences(canonical_obj, candidate_obj)
    _class_counts(diffs)
    return (len(diffs) == 0), diffs


def _missing_candidate_result(name):
    diffs = [{
        "path": "<candidate>",
        "kind": CLASS_PLATFORM_MISSING,
        "classification": CLASS_PLATFORM_MISSING,
        "canonical": "required platform output present in canonical reference",
        "candidate": None,
    }]
    return {
        "path": None,
        "sha256": None,
        "match": False,
        "total": 1,
        "differenceClasses": _class_counts(diffs),
        "differences": diffs,
    }


def _build_release_gate(candidate_results, required_candidates):
    required = list(required_candidates or [])
    if not required:
        return {
            "status": "not-evaluated",
            "requiredCandidates": [],
            "presentCandidates": [],
            "matchingCandidates": [],
            "missingCandidates": [],
            "mismatchingCandidates": [],
            "blockedReasons": [],
        }

    present = []
    matching = []
    missing = []
    mismatching = []

    for name in required:
        info = candidate_results.get(name)
        if not info or info.get("path") is None:
            missing.append(name)
            continue
        present.append(name)
        if info.get("match") is True:
            matching.append(name)
        else:
            mismatching.append(name)

    blocked_reasons = []
    if missing:
        blocked_reasons.append(
            "missing required platform output: {0}".format(", ".join(missing))
        )
    if mismatching:
        blocked_reasons.append(
            "required platform output differs: {0}".format(", ".join(mismatching))
        )

    return {
        "status": "passed" if not blocked_reasons else "blocked",
        "requiredCandidates": required,
        "presentCandidates": present,
        "matchingCandidates": matching,
        "missingCandidates": missing,
        "mismatchingCandidates": mismatching,
        "blockedReasons": blocked_reasons,
    }


def build_diff_result(canonical_path, candidates, required_candidates=None):
    """Build the full diff-result document.

    ``candidates`` is an ordered list of ``(name, path)`` tuples.
    """
    canonical_obj = load_canonical_object(canonical_path)
    canonical_sha = sha256_of_file(canonical_path)

    candidate_results = {}
    summary = {}
    overall_match = True
    overall_total = 0

    required_candidates = list(required_candidates or [])

    for name, path in candidates:
        match, diffs = compare_candidate(canonical_obj, path)
        total = len(diffs)
        classes = _class_counts(diffs)
        candidate_results[name] = {
            "path": os.path.abspath(path),
            "sha256": sha256_of_file(path),
            "match": match,
            "total": total,
            "differenceClasses": classes,
            "differences": diffs,
        }
        summary[name] = {
            "match": match,
            "total": total,
            "differenceClasses": classes,
        }
        overall_match = overall_match and match
        overall_total += total

    for name in required_candidates:
        if name in candidate_results:
            continue
        missing = _missing_candidate_result(name)
        candidate_results[name] = missing
        summary[name] = {
            "match": False,
            "total": missing["total"],
            "differenceClasses": missing["differenceClasses"],
        }
        overall_match = False
        overall_total += missing["total"]

    release_gate = _build_release_gate(candidate_results, required_candidates)

    return {
        "schemaVersion": SCHEMA_VERSION,
        "tool": TOOL_NAME,
        "version": TOOL_VERSION,
        "canonical": {
            "path": os.path.abspath(canonical_path),
            "sha256": canonical_sha,
        },
        "candidates": candidate_results,
        "summary": summary,
        "match": overall_match,
        "total": overall_total,
        "releaseGate": release_gate,
    }


# --------------------------------------------------------------------------- #
# CLI
# --------------------------------------------------------------------------- #
def _parse_candidate_spec(spec):
    """Parse a ``name:path`` or bare ``path`` candidate spec.

    A bare path uses the file's stem (without extension) as the candidate
    name. A duplicate name is rejected.
    """
    if ":" in spec:
        name, path = spec.split(":", 1)
        name = name.strip()
        if not name:
            raise DiffError("empty candidate name in spec: {0}".format(spec))
    else:
        path = spec
        stem = os.path.basename(path)
        name = os.path.splitext(stem)[0] or "candidate"
    return name, path


def parse_args(argv):
    parser = argparse.ArgumentParser(
        prog=TOOL_NAME,
        description=(
            "Compare a canonical corpus result against one or more platform "
            "candidate results (each canonicalized first), producing a "
            "diff-result.json consumed by the benchmark run packager and the "
            "release blocker register."
        ),
    )
    parser.add_argument(
        "canonical",
        help="Path to the canonical reference JSON file.",
    )
    parser.add_argument(
        "--candidate",
        action="append",
        default=[],
        metavar="NAME:PATH | PATH",
        help=(
            "A candidate result. Repeat for multiple platforms. Use the form "
            "NAME:PATH to name the candidate; a bare PATH uses the file stem "
            "as the name."
        ),
    )
    parser.add_argument(
        "-o", "--output",
        default=None,
        help="Path to write the diff-result JSON (default: write to stdout).",
    )
    parser.add_argument(
        "--required-candidate",
        action="append",
        default=[],
        metavar="NAME",
        help=(
            "Candidate name required for release-gate evidence. Missing or "
            "mismatching required candidates mark releaseGate.status=blocked."
        ),
    )
    parser.add_argument(
        "--release-gate",
        action="store_true",
        help=(
            "Require the default three app candidates: ios, android, harmony."
        ),
    )
    return parser.parse_args(argv)


def main(argv=None):
    if argv is None:
        argv = sys.argv[1:]
    args = parse_args(argv)

    if not args.candidate:
        sys.stderr.write("error: at least one --candidate is required\n")
        return 2

    seen_names = set()
    candidates = []
    for spec in args.candidate:
        name, path = _parse_candidate_spec(spec)
        if name in seen_names:
            sys.stderr.write("error: duplicate candidate name: {0}\n".format(name))
            return 2
        seen_names.add(name)
        candidates.append((name, path))

    required_candidates = list(args.required_candidate)
    if args.release_gate:
        for name in DEFAULT_RELEASE_GATE_CANDIDATES:
            if name not in required_candidates:
                required_candidates.append(name)

    try:
        result = build_diff_result(
            args.canonical,
            candidates,
            required_candidates=required_candidates,
        )
    except DiffError as err:
        sys.stderr.write("error: {0}\n".format(err))
        return 2

    text = json.dumps(result, sort_keys=True, indent=2, ensure_ascii=False) + "\n"
    if args.output:
        with open(args.output, "w", encoding="utf-8") as handle:
            handle.write(text)
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
