#!/usr/bin/env python3
"""Corpus batch selector.

Divides a fixture corpus (consumed from a `fixture-manifest/1` JSON object, or
scanned directly from a corpus root) into P0/P1/P2 priority batches and emits a
three-platform (iOS/Android/HarmonyOS) benchmark input list.

The selector does NOT modify the parser or any fixture content — it only
classifies existing fixtures into batches and produces a manifest describing
which fixtures should run on which platform.

Pure Python 3.9+ standard library.

CLI:
    python3 tools/corpus-batch-selector/corpus_batch_selector.py \\
        [--manifest PATH | --root PATH] [--pretty] [--out PATH]
"""

import argparse
import hashlib
import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path

VERSION = "corpus-batch-selector/1"
TOOL = "corpus-batch-selector"
MANIFEST_VERSION = "fixture-manifest/1"

DEFAULT_PLATFORMS = ["ios", "android", "harmony"]

P0_SOURCE_TYPES = {"book-source", "web-page"}
P1_SOURCE_TYPES = {"json-api", "xml-feed", "rss-feed"}

RATIONALES = {
    "P0": "Core reading chain: book-source/web-page synthetic fixture must run on all 3 platforms.",
    "P1": "Secondary feed/api fixture: should run on at least 2 platforms (all of fixture.platforms).",
    "P2": "Nice-to-have fixture (local-book, unknown source_type, or non-synthetic): runs on first platform only.",
}


# ---------------------------------------------------------------------------
# Pure helpers
# ---------------------------------------------------------------------------

def assign_batch(fixture):
    """Return the priority batch ("P0" | "P1" | "P2") for a single fixture.

    Pure. Rules:
      - P0: source_type in {book-source, web-page} AND sanitization == "synthetic".
      - P1: source_type in {json-api, xml-feed, rss-feed} AND sanitization == "synthetic".
      - P2: everything else (local-book, unknown, or non-synthetic).
    """
    source_type = fixture.get("source_type", "unknown")
    sanitization = fixture.get("sanitization", "unknown")
    if sanitization == "synthetic":
        if source_type in P0_SOURCE_TYPES:
            return "P0"
        if source_type in P1_SOURCE_TYPES:
            return "P1"
    return "P2"


def build_entry(fixture, batch):
    """Build a batch entry dict for a fixture, including a human-readable rationale."""
    return {
        "fixture_id": fixture.get("id"),
        "path": fixture.get("path"),
        "source_type": fixture.get("source_type", "unknown"),
        "capability_tags": list(fixture.get("capability_tags", [])),
        "platforms": list(fixture.get("platforms", [])),
        "rationale": RATIONALES.get(batch, ""),
    }


def platform_targets(fixture, batch):
    """Return the list of platforms a fixture should run on, given its batch.

    Rules:
      - P0 → all 3 platforms (ios, android, harmony).
      - P1 → all platforms in fixture.platforms (default all 3 if empty).
      - P2 → first platform in fixture.platforms only (default ios if empty).
    """
    platforms = list(fixture.get("platforms") or [])
    if not platforms:
        platforms = list(DEFAULT_PLATFORMS)
    if batch == "P0":
        return list(DEFAULT_PLATFORMS)
    if batch == "P1":
        return list(platforms)
    # P2: first platform only
    return [platforms[0]]


def select(manifest_obj):
    """Build a batch-selector report from a parsed fixture-manifest/1 object.

    Pure (does not touch disk). Raises ValueError if the manifest version is
    not "fixture-manifest/1".
    """
    if not isinstance(manifest_obj, dict):
        raise ValueError("manifest must be a dict, got %r" % type(manifest_obj))
    if manifest_obj.get("version") != MANIFEST_VERSION:
        raise ValueError(
            "unsupported manifest version: %r (expected %r)"
            % (manifest_obj.get("version"), MANIFEST_VERSION)
        )

    fixtures = manifest_obj.get("fixtures") or []

    batches = {"P0": [], "P1": [], "P2": []}
    platform_inputs = {"ios": [], "android": [], "harmony": []}

    for fx in fixtures:
        batch = assign_batch(fx)
        entry = build_entry(fx, batch)
        batches[batch].append(entry)
        for p in platform_targets(fx, batch):
            if p in platform_inputs and fx.get("id") is not None:
                platform_inputs[p].append(fx["id"])

    # Sort entries within each batch by fixture_id.
    for batch_name in batches:
        batches[batch_name].sort(key=lambda e: e["fixture_id"] or "")
    # Sort platform_inputs lists.
    for p in platform_inputs:
        platform_inputs[p] = sorted(set(platform_inputs[p]))

    p0 = len(batches["P0"])
    p1 = len(batches["P1"])
    p2 = len(batches["P2"])
    summary = {
        "p0": p0,
        "p1": p1,
        "p2": p2,
        "total": p0 + p1 + p2,
        "platform_counts": {
            "ios": len(platform_inputs["ios"]),
            "android": len(platform_inputs["android"]),
            "harmony": len(platform_inputs["harmony"]),
        },
    }

    return {
        "version": VERSION,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "tool": TOOL,
        "batches": batches,
        "platform_inputs": platform_inputs,
        "summary": summary,
    }


# ---------------------------------------------------------------------------
# Disk-touching convenience
# ---------------------------------------------------------------------------

# Minimal in-tree classification for roots without a manifest. We reuse the
# same classification rules as the fixture-manifest generator (kept here as a
# self-contained fallback so this tool has no cross-tool import dependency).
_SHORT_CODES = {
    "book-source": "bs",
    "web-page": "wp",
    "json-api": "ja",
    "xml-feed": "xf",
    "rss-feed": "rf",
    "local-book": "lb",
    "unknown": "un",
}
_CAPABILITY_TAGS = {
    "book-source": ["search", "toc", "chapter-content"],
    "web-page": ["chapter-content"],
    "json-api": ["search"],
    "xml-feed": ["toc"],
    "rss-feed": ["toc", "search"],
    "local-book": ["chapter-content"],
    "unknown": [],
}


def _read_text(path):
    try:
        with open(path, "r", encoding="utf-8") as fh:
            return fh.read()
    except (UnicodeDecodeError, OSError):
        return None


def _slug(rel_path):
    base = str(rel_path.with_suffix("")).replace(os.sep, "/").replace("/", "-")
    return base.lstrip("-")


def _classify_file(file_path, rel_path):
    """Classify a single file into a fixture dict (or None if unrecognized)."""
    rel_str = str(rel_path).replace(os.sep, "/")
    ext = file_path.suffix.lower()
    source_type = "unknown"
    fmt = "unknown"
    text = None
    content_id = None

    if ext == ".json":
        fmt = "json"
        text = _read_text(file_path)
        if text is not None:
            try:
                data = json.loads(text)
            except (json.JSONDecodeError, ValueError):
                data = None
            if isinstance(data, dict):
                st = data.get("source_type")
                if isinstance(st, str):
                    source_type = st
                elif any(k in data for k in ("data", "items", "list")):
                    source_type = "json-api"
                if isinstance(data.get("id"), str):
                    content_id = data["id"]
    elif ext in (".html", ".htm"):
        fmt = "html"
        source_type = "web-page"
        text = _read_text(file_path)
    elif ext == ".xml":
        fmt = "xml"
        text = _read_text(file_path)
        if text is not None:
            head = text.lstrip()[:512].lower()
            if "<rss" in head or "<feed" in head:
                source_type = "rss-feed"
            else:
                source_type = "xml-feed"
    elif ext == ".txt":
        fmt = "text"
        source_type = "local-book"
        text = _read_text(file_path)
    else:
        return None

    try:
        raw = file_path.read_bytes()
    except OSError:
        return None
    sha = hashlib.sha256(raw).hexdigest()
    size = len(raw)

    if content_id:
        fid = content_id
    else:
        code = _SHORT_CODES.get(source_type, "un")
        fid = "%s-%s" % (code, _slug(rel_path))

    parts = set(rel_path.parts)
    synthetic = ("samples" in parts) or ("tooling" in parts)
    if not synthetic and text is not None and "example.test" in text:
        synthetic = True
    sanitization = "synthetic" if synthetic else "unknown"

    return {
        "id": fid,
        "path": rel_str,
        "source_type": source_type,
        "format": fmt,
        "platforms": list(DEFAULT_PLATFORMS),
        "capability_tags": list(_CAPABILITY_TAGS.get(source_type, [])),
        "sanitization": sanitization,
        "bytes": size,
        "sha256": sha,
    }


def _scan_root(root):
    """Scan `root` directly and build an in-memory fixture-manifest/1 object."""
    root_abs = root.resolve()
    fixtures = []
    for dirpath, dirnames, filenames in os.walk(str(root)):
        for fn in filenames:
            if fn.startswith(".") or fn.endswith(".manifest.json"):
                continue
            fp = Path(dirpath) / fn
            rel = fp.relative_to(root)
            fixture = _classify_file(fp, rel)
            if fixture is not None:
                fixtures.append(fixture)
    fixtures.sort(key=lambda f: f["id"])
    return {
        "version": MANIFEST_VERSION,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "tool": "fixture-manifest-generator",
        "root": str(root_abs),
        "fixtures": fixtures,
        "summary": {"total": len(fixtures)},
    }


def select_from_root(root):
    """Convenience: load a sibling manifest if present, else scan `root` directly.

    Looks for `<root>/fixture-manifest.json`. If found, parses and selects.
    Otherwise, scans `root` directly (simple classification) and selects.
    Raises FileNotFoundError if `root` does not exist.
    """
    root = Path(root)
    if not root.exists():
        raise FileNotFoundError("root not found: %s" % root)

    manifest_path = root / "fixture-manifest.json"
    if manifest_path.is_file():
        with open(manifest_path, "r", encoding="utf-8") as fh:
            manifest_obj = json.load(fh)
    else:
        manifest_obj = _scan_root(root)
    return select(manifest_obj)


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def _emit(report, pretty=False, out_path=None):
    if pretty:
        text = json.dumps(report, indent=2, sort_keys=True)
    else:
        text = json.dumps(report)
    text += "\n"
    if out_path:
        try:
            with open(out_path, "w", encoding="utf-8") as fh:
                fh.write(text)
        except OSError as exc:
            sys.stderr.write("error: cannot write --out: %s\n" % exc)
            return 2
    else:
        sys.stdout.write(text)
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="corpus_batch_selector",
        description="Divide a fixture corpus into P0/P1/P2 priority batches "
                    "and emit a three-platform benchmark input list.",
    )
    parser.add_argument(
        "--manifest",
        default=None,
        help="path to a fixture-manifest/1 JSON file to consume",
    )
    parser.add_argument(
        "--root",
        default=".",
        help="corpus root to scan (default: current directory). "
             "Used when --manifest is not given; if <root>/fixture-manifest.json "
             "exists it is consumed, otherwise the root is scanned directly.",
    )
    parser.add_argument(
        "--pretty",
        action="store_true",
        help="pretty-print JSON (indent=2, sorted keys, trailing newline)",
    )
    parser.add_argument(
        "--out",
        default=None,
        help="write JSON report to this file instead of stdout",
    )
    args = parser.parse_args(argv)

    try:
        if args.manifest:
            manifest_path = Path(args.manifest)
            if not manifest_path.is_file():
                sys.stderr.write(
                    "error: manifest not found: %s\n" % manifest_path
                )
                return 2
            with open(manifest_path, "r", encoding="utf-8") as fh:
                manifest_obj = json.load(fh)
            report = select(manifest_obj)
        else:
            root = Path(args.root)
            if not root.exists():
                sys.stderr.write(
                    "error: root not found: %s\n" % root
                )
                return 2
            report = select_from_root(root)
    except FileNotFoundError as exc:
        sys.stderr.write("error: %s\n" % exc)
        return 2
    except ValueError as exc:
        sys.stderr.write("error: %s\n" % exc)
        return 2
    except OSError as exc:
        sys.stderr.write("error: %s\n" % exc)
        return 2

    return _emit(report, pretty=args.pretty, out_path=args.out)


if __name__ == "__main__":
    sys.exit(main())
