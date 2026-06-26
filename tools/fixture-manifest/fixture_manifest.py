#!/usr/bin/env python3
"""Fixture manifest generator.

Scans corpus / samples / booksources directories and emits a manifest JSON
annotating each fixture's source, type, platform applicability, and expected
capability domain. Pure Python 3.9+ standard library.

CLI:
    python3 tools/fixture-manifest/fixture_manifest.py <root> \
        [--include NAME ...] [--indent N] [--pretty]
"""

import argparse
import hashlib
import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path

VERSION = "fixture-manifest/1"
TOOL = "fixture-manifest-generator"

SHORT_CODES = {
    "book-source": "bs",
    "web-page": "wp",
    "json-api": "ja",
    "xml-feed": "xf",
    "rss-feed": "rf",
    "local-book": "lb",
    "unknown": "un",
}

CAPABILITY_TAGS = {
    "book-source": ["search", "toc", "chapter-content"],
    "web-page": ["chapter-content"],
    "json-api": ["search"],
    "xml-feed": ["toc"],
    "rss-feed": ["toc", "search"],
    "local-book": ["chapter-content"],
    "unknown": [],
}

DEFAULT_PLATFORMS = ["ios", "android", "harmony"]


def _read_text(path):
    try:
        with open(path, "r", encoding="utf-8") as fh:
            return fh.read()
    except (UnicodeDecodeError, OSError):
        return None


def _slug(rel_path):
    base = str(rel_path.with_suffix("")).replace(os.sep, "/").replace("/", "-")
    return base.lstrip("-")


def _build_fixture(file_path, rel_path):
    rel_str = str(rel_path).replace(os.sep, "/")
    ext = file_path.suffix.lower()
    source_type = "unknown"
    fmt = "unknown"
    content_id = None
    text = None

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
        code = SHORT_CODES.get(source_type, "un")
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
        "capability_tags": list(CAPABILITY_TAGS.get(source_type, [])),
        "description": "%s fixture at %s" % (source_type, rel_str),
        "sanitization": sanitization,
        "bytes": size,
        "sha256": sha,
    }


def manifest_for_path(path):
    """Classify a single file into a fixture dict (or None if unrecognized)."""
    path = Path(path)
    if not path.is_file():
        return None
    name = path.name
    if name.startswith(".") or name.endswith(".manifest.json"):
        return None
    return _build_fixture(path, path)


def scan(root, include=None):
    """Recursively scan `root` and return a manifest dict.

    `include` optionally restricts scanning to the named top-level
    subdirectories (e.g. ["corpus", "samples", "booksources"]).
    Raises FileNotFoundError if `root` does not exist.
    """
    root = Path(root)
    if not root.is_dir():
        raise FileNotFoundError("scan root not found: %s" % root)
    root_abs = root.resolve()

    include_set = set(include) if include else None
    fixtures = []
    for dirpath, dirnames, filenames in os.walk(str(root)):
        rel_dir = os.path.relpath(dirpath, str(root))
        if include_set is not None and rel_dir == ".":
            dirnames[:] = [d for d in dirnames if d in include_set]
            continue
        for fn in filenames:
            if fn.startswith(".") or fn.endswith(".manifest.json"):
                continue
            fp = Path(dirpath) / fn
            rel = fp.relative_to(root)
            fixture = _build_fixture(fp, rel)
            if fixture is not None:
                fixtures.append(fixture)

    fixtures.sort(key=lambda f: f["id"])
    return _manifest_dict(root_abs, fixtures)


def _manifest_dict(root_abs, fixtures):
    by_source_type = {}
    by_platform = {"ios": 0, "android": 0, "harmony": 0}
    for f in fixtures:
        st = f["source_type"]
        by_source_type[st] = by_source_type.get(st, 0) + 1
        for p in f["platforms"]:
            if p in by_platform:
                by_platform[p] += 1
    return {
        "version": VERSION,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "tool": TOOL,
        "root": str(root_abs),
        "fixtures": fixtures,
        "summary": {
            "total": len(fixtures),
            "by_source_type": by_source_type,
            "by_platform": by_platform,
        },
    }


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="fixture_manifest",
        description="Generate a fixture manifest JSON for a scanned root.",
    )
    parser.add_argument("root", help="directory to scan")
    parser.add_argument(
        "--include", nargs="+", default=None, metavar="NAME",
        help="restrict scanning to these top-level subdirectory names",
    )
    parser.add_argument(
        "--indent", type=int, default=None,
        help="pretty-print JSON with this many spaces of indentation",
    )
    parser.add_argument(
        "--pretty", action="store_true",
        help="indent=2, sorted keys, trailing newline",
    )
    args = parser.parse_args(argv)

    root = Path(args.root)
    if not root.is_dir():
        sys.stderr.write(
            "error: scan root not found or not a directory: %s\n" % root
        )
        return 2
    try:
        manifest = scan(root, include=args.include)
    except FileNotFoundError as exc:
        sys.stderr.write("error: %s\n" % exc)
        return 2

    if args.pretty:
        out = json.dumps(manifest, indent=2, sort_keys=True)
    elif args.indent is not None:
        out = json.dumps(manifest, indent=args.indent)
    else:
        out = json.dumps(manifest)
    sys.stdout.write(out + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
