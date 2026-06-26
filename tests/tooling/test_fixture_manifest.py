"""Tests for the fixture manifest generator (tools/fixture-manifest).

These tests build synthetic inputs with tempfile.TemporaryDirectory so they
do not depend on the real repo corpus (which may be absent). Run with:

    python3 -m unittest tests.tooling.test_fixture_manifest -v
"""

import hashlib
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime
from pathlib import Path

_HERE = Path(__file__).resolve().parent
_REPO_ROOT = _HERE.parent.parent
_MODULE_PATH = _REPO_ROOT / "tools" / "fixture-manifest" / "fixture_manifest.py"
_CLI = str(_MODULE_PATH)


def _load_module():
    """Load fixture_manifest.py by file path (the dir name has a hyphen)."""
    if not _MODULE_PATH.exists():
        raise ImportError(
            "fixture_manifest implementation not found at %s. "
            "TDD: write the implementation after the tests." % _MODULE_PATH
        )
    spec = importlib.util.spec_from_file_location("fixture_manifest", _MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    sys.modules["fixture_manifest"] = module
    spec.loader.exec_module(module)
    return module


fm = _load_module()


def _write(path, content):
    p = Path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    if isinstance(content, bytes):
        p.write_bytes(content)
    else:
        p.write_text(content, encoding="utf-8")
    return p


def _run_cli(args):
    return subprocess.run(
        [sys.executable, _CLI] + [str(a) for a in args],
        capture_output=True,
        text=True,
    )


class ScanEmptyAndMissing(unittest.TestCase):
    def test_scan_empty_dir_total_zero(self):
        with tempfile.TemporaryDirectory() as d:
            manifest = fm.scan(Path(d))
        self.assertEqual(manifest["version"], "fixture-manifest/1")
        self.assertEqual(manifest["tool"], "fixture-manifest-generator")
        self.assertEqual(manifest["summary"]["total"], 0)
        self.assertEqual(manifest["fixtures"], [])
        self.assertEqual(manifest["summary"]["by_source_type"], {})
        self.assertEqual(
            manifest["summary"]["by_platform"],
            {"ios": 0, "android": 0, "harmony": 0},
        )
        self.assertEqual(manifest["root"], str(Path(d).resolve()))

    def test_scan_missing_dir_raises_file_not_found(self):
        with self.assertRaises(FileNotFoundError):
            fm.scan(Path("/nonexistent-path-xyz-abc-12345"))

    def test_manifest_has_generated_at_iso8601(self):
        with tempfile.TemporaryDirectory() as d:
            manifest = fm.scan(Path(d))
        ts = manifest["generated_at"]
        self.assertIsInstance(ts, str)
        self.assertTrue(len(ts) > 0)
        # iso8601 with timezone should round-trip via fromisoformat.
        datetime.fromisoformat(ts)


class ScanClassification(unittest.TestCase):
    def test_scan_classifies_mixed_formats(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "book.json",
                   '{"source_type": "book-source", "url": "https://demo.example/book"}')
            _write(root / "page.html", "<html><body>hello</body></html>")
            _write(root / "catalog.xml", "<catalog><item>data</item></catalog>")
            _write(root / "notes.txt", "hello world")
            _write(root / "feed.xml",
                   "<rss version=\"2.0\"><channel><title>x</title></channel></rss>")
            manifest = fm.scan(root)
        by_path = {f["path"]: f for f in manifest["fixtures"]}

        book = by_path["book.json"]
        self.assertEqual(book["source_type"], "book-source")
        self.assertEqual(book["format"], "json")
        self.assertEqual(book["id"], "bs-book")
        self.assertEqual(book["capability_tags"],
                         ["search", "toc", "chapter-content"])

        page = by_path["page.html"]
        self.assertEqual(page["source_type"], "web-page")
        self.assertEqual(page["format"], "html")
        self.assertEqual(page["id"], "wp-page")
        self.assertEqual(page["capability_tags"], ["chapter-content"])

        catalog = by_path["catalog.xml"]
        self.assertEqual(catalog["source_type"], "xml-feed")
        self.assertEqual(catalog["format"], "xml")
        self.assertEqual(catalog["id"], "xf-catalog")
        self.assertEqual(catalog["capability_tags"], ["toc"])

        notes = by_path["notes.txt"]
        self.assertEqual(notes["source_type"], "local-book")
        self.assertEqual(notes["format"], "text")
        self.assertEqual(notes["id"], "lb-notes")
        self.assertEqual(notes["capability_tags"], ["chapter-content"])

        feed = by_path["feed.xml"]
        self.assertEqual(feed["source_type"], "rss-feed")
        self.assertEqual(feed["format"], "xml")
        self.assertEqual(feed["id"], "rf-feed")
        self.assertEqual(feed["capability_tags"], ["toc", "search"])

    def test_scan_json_api_inference(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "api.json", '{"data": [1, 2, 3]}')
            _write(root / "plain.json", '{"name": "thing"}')
            manifest = fm.scan(root)
        by_path = {f["path"]: f for f in manifest["fixtures"]}
        self.assertEqual(by_path["api.json"]["source_type"], "json-api")
        self.assertEqual(by_path["api.json"]["capability_tags"], ["search"])
        self.assertEqual(by_path["plain.json"]["source_type"], "unknown")
        self.assertEqual(by_path["plain.json"]["capability_tags"], [])

    def test_scan_json_with_explicit_id_field_preferred(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "book.json",
                   '{"id": "my-custom-id", "source_type": "book-source"}')
            manifest = fm.scan(root)
        self.assertEqual(len(manifest["fixtures"]), 1)
        self.assertEqual(manifest["fixtures"][0]["id"], "my-custom-id")

    def test_scan_manifest_json_files_are_skipped(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "thing.json", '{"source_type": "book-source"}')
            _write(root / "thing.manifest.json",
                   '{"version": "fixture-manifest/1"}')
            manifest = fm.scan(root)
        paths = [f["path"] for f in manifest["fixtures"]]
        self.assertIn("thing.json", paths)
        self.assertNotIn("thing.manifest.json", paths)
        self.assertEqual(manifest["summary"]["total"], 1)

    def test_scan_binary_and_unknown_extensions_skipped(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "img.png", b"\x89PNG\r\n\x1a\n\x00\x00")
            _write(root / "archive.zip", b"PK\x03\x04")
            _write(root / "data.lock", "{}")
            _write(root / "keep.json", '{"source_type": "book-source"}')
            manifest = fm.scan(root)
        paths = [f["path"] for f in manifest["fixtures"]]
        self.assertEqual(paths, ["keep.json"])

    def test_scan_dotfiles_skipped(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / ".hidden.json", '{"source_type": "book-source"}')
            _write(root / "keep.json", '{"source_type": "book-source"}')
            manifest = fm.scan(root)
        paths = [f["path"] for f in manifest["fixtures"]]
        self.assertEqual(paths, ["keep.json"])


class ScanHashingAndSanitization(unittest.TestCase):
    def test_sha256_and_bytes_correct(self):
        content = b"hello world\n"
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "notes.txt", content)
            manifest = fm.scan(root)
        f = manifest["fixtures"][0]
        self.assertEqual(f["bytes"], len(content))
        self.assertEqual(f["sha256"], hashlib.sha256(content).hexdigest())

    def test_sanitization_synthetic_under_samples_subdir(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "samples" / "book.json",
                   '{"source_type": "book-source", "url": "https://demo.example/book"}')
            _write(root / "plain" / "book.json",
                   '{"source_type": "book-source", "url": "https://demo.example/book"}')
            manifest = fm.scan(root)
        by_path = {f["path"]: f for f in manifest["fixtures"]}
        self.assertEqual(by_path["samples/book.json"]["sanitization"], "synthetic")
        self.assertEqual(by_path["plain/book.json"]["sanitization"], "unknown")

    def test_sanitization_synthetic_when_content_has_example_test_host(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "plain" / "book.json",
                   '{"source_type": "book-source", "url": "https://api.example.test/x"}')
            manifest = fm.scan(root)
        by_path = {f["path"]: f for f in manifest["fixtures"]}
        self.assertEqual(
            by_path["plain/book.json"]["sanitization"], "synthetic")

    def test_platforms_default_to_all_three(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "book.json", '{"source_type": "book-source"}')
            _write(root / "page.html", "<html></html>")
            manifest = fm.scan(root)
        for f in manifest["fixtures"]:
            self.assertEqual(f["platforms"], ["ios", "android", "harmony"])


class ScanSummaryAndOrdering(unittest.TestCase):
    def test_summary_counts_correct(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "book.json", '{"source_type": "book-source"}')
            _write(root / "page.html", "<html></html>")
            _write(root / "notes.txt", "hello")
            manifest = fm.scan(root)
        self.assertEqual(manifest["summary"]["total"], 3)
        self.assertEqual(
            manifest["summary"]["by_source_type"],
            {"book-source": 1, "web-page": 1, "local-book": 1},
        )
        self.assertEqual(
            manifest["summary"]["by_platform"],
            {"ios": 3, "android": 3, "harmony": 3},
        )

    def test_fixtures_sorted_by_id(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "notes.txt", "hello")
            _write(root / "book.json", '{"source_type": "book-source"}')
            _write(root / "page.html", "<html></html>")
            manifest = fm.scan(root)
        ids = [f["id"] for f in manifest["fixtures"]]
        self.assertEqual(ids, sorted(ids))

    def test_description_format(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "book.json", '{"source_type": "book-source"}')
            manifest = fm.scan(root)
        f = manifest["fixtures"][0]
        self.assertEqual(f["description"], "book-source fixture at book.json")


class ScanIncludeFilter(unittest.TestCase):
    def test_include_restricts_to_named_subdirs(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "corpus" / "a.json", '{"source_type": "book-source"}')
            _write(root / "other" / "b.json", '{"source_type": "book-source"}')
            manifest = fm.scan(root, include=["corpus"])
        paths = [f["path"] for f in manifest["fixtures"]]
        self.assertEqual(paths, ["corpus/a.json"])

    def test_include_multiple_subdirs(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "corpus" / "a.json", '{"source_type": "book-source"}')
            _write(root / "samples" / "b.json", '{"source_type": "book-source"}')
            _write(root / "other" / "c.json", '{"source_type": "book-source"}')
            manifest = fm.scan(root, include=["corpus", "samples"])
        paths = sorted(f["path"] for f in manifest["fixtures"])
        self.assertEqual(paths, ["corpus/a.json", "samples/b.json"])


class ManifestForPath(unittest.TestCase):
    def test_manifest_for_path_json_returns_fixture(self):
        with tempfile.TemporaryDirectory() as d:
            p = _write(Path(d) / "book.json",
                       '{"source_type": "book-source", "url": "https://demo.example/b"}')
            expected_sha = hashlib.sha256(p.read_bytes()).hexdigest()
            fixture = fm.manifest_for_path(p)
        self.assertIsNotNone(fixture)
        self.assertEqual(fixture["source_type"], "book-source")
        self.assertEqual(fixture["format"], "json")
        self.assertEqual(fixture["capability_tags"],
                         ["search", "toc", "chapter-content"])
        self.assertEqual(fixture["sha256"], expected_sha)
        self.assertTrue(fixture["id"].startswith("bs-"))

    def test_manifest_for_path_png_returns_none(self):
        with tempfile.TemporaryDirectory() as d:
            p = _write(Path(d) / "img.png", b"\x89PNG\r\n")
            self.assertIsNone(fm.manifest_for_path(p))


class CLI(unittest.TestCase):
    def test_cli_pretty_outputs_sorted_indented_json_exit_zero(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "book.json", '{"source_type": "book-source"}')
            result = _run_cli([root, "--pretty"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(result.stdout.endswith("\n"))
        # Indented (multi-line) output.
        self.assertIn("\n  ", result.stdout)
        data = json.loads(result.stdout)
        self.assertEqual(data["version"], "fixture-manifest/1")
        # sort_keys=True means top-level keys are alphabetical: 'fixtures' first.
        top_keys = list(data.keys())
        self.assertEqual(top_keys, sorted(top_keys))

    def test_cli_default_outputs_valid_json(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "book.json", '{"source_type": "book-source"}')
            result = _run_cli([root])
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads(result.stdout)
        self.assertEqual(data["summary"]["total"], 1)

    def test_cli_missing_root_exits_two(self):
        result = _run_cli(["/nonexistent-path-xyz-abc-12345"])
        self.assertEqual(result.returncode, 2)
        # Should not emit a stack trace to stdout; stderr carries the message.
        self.assertEqual(result.stdout, "")

    def test_cli_include_flag_restricts_scanning(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "corpus" / "a.json", '{"source_type": "book-source"}')
            _write(root / "other" / "b.json", '{"source_type": "book-source"}')
            result = _run_cli([root, "--include", "corpus", "--pretty"])
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads(result.stdout)
        paths = [f["path"] for f in data["fixtures"]]
        self.assertEqual(paths, ["corpus/a.json"])

    def test_cli_indent_n_produces_indented_output(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "book.json", '{"source_type": "book-source"}')
            result = _run_cli([root, "--indent", "4"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("\n    ", result.stdout)
        json.loads(result.stdout)


class EdgeCases(unittest.TestCase):
    def test_nested_file_id_reflects_full_relative_path(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "corpus" / "nested" / "a.json",
                   '{"source_type": "book-source"}')
            manifest = fm.scan(root)
        self.assertEqual(len(manifest["fixtures"]), 1)
        f = manifest["fixtures"][0]
        self.assertEqual(f["path"], "corpus/nested/a.json")
        self.assertEqual(f["id"], "bs-corpus-nested-a")

    def test_malformed_json_classified_unknown(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "broken.json", "{not valid json")
            manifest = fm.scan(root)
        f = manifest["fixtures"][0]
        self.assertEqual(f["source_type"], "unknown")
        self.assertEqual(f["format"], "json")
        self.assertEqual(f["capability_tags"], [])
        self.assertEqual(f["id"], "un-broken")

    def test_atom_feed_classified_rss_feed(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "atom.xml",
                   '<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom">'
                   '<entry><title>x</title></entry></feed>')
            manifest = fm.scan(root)
        f = manifest["fixtures"][0]
        self.assertEqual(f["source_type"], "rss-feed")
        self.assertEqual(f["format"], "xml")
        self.assertEqual(f["capability_tags"], ["toc", "search"])

    def test_htm_extension_classified_html(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "page.htm", "<html><body>hi</body></html>")
            manifest = fm.scan(root)
        f = manifest["fixtures"][0]
        self.assertEqual(f["source_type"], "web-page")
        self.assertEqual(f["format"], "html")
        self.assertEqual(f["id"], "wp-page")

    def test_include_walks_nested_subdirs_under_named_top(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "corpus" / "nested" / "a.json",
                   '{"source_type": "book-source"}')
            _write(root / "other" / "b.json", '{"source_type": "book-source"}')
            manifest = fm.scan(root, include=["corpus"])
        paths = [f["path"] for f in manifest["fixtures"]]
        self.assertEqual(paths, ["corpus/nested/a.json"])

    def test_empty_txt_file_has_zero_bytes_and_valid_sha(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "empty.txt", "")
            manifest = fm.scan(root)
        f = manifest["fixtures"][0]
        self.assertEqual(f["bytes"], 0)
        self.assertEqual(f["sha256"], hashlib.sha256(b"").hexdigest())
        self.assertEqual(f["source_type"], "local-book")

    def test_json_array_classified_unknown(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "arr.json", '[{"source_type": "book-source"}]')
            manifest = fm.scan(root)
        f = manifest["fixtures"][0]
        self.assertEqual(f["source_type"], "unknown")
        self.assertEqual(f["capability_tags"], [])


if __name__ == "__main__":
    unittest.main()
