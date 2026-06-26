"""Tests for the corpus batch selector (tools/corpus-batch-selector).

Strict TDD: this file was written BEFORE the implementation. It uses synthetic
inputs only (tempfile.TemporaryDirectory + in-memory manifest objects) and does
NOT depend on the real repo corpus (which may be absent at baseline).

Run with:

    python3 -m unittest tests.tooling.test_corpus_batch_selector -v
"""

import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

_HERE = Path(__file__).resolve().parent
_REPO_ROOT = _HERE.parent.parent
_MODULE_PATH = _REPO_ROOT / "tools" / "corpus-batch-selector" / "corpus_batch_selector.py"
_CLI = str(_MODULE_PATH)


def _load_module():
    """Load corpus_batch_selector.py by file path (dir name has a hyphen)."""
    if not _MODULE_PATH.exists():
        raise ImportError(
            "corpus_batch_selector implementation not found at %s. "
            "TDD: write the implementation after the tests." % _MODULE_PATH
        )
    spec = importlib.util.spec_from_file_location(
        "corpus_batch_selector", _MODULE_PATH
    )
    module = importlib.util.module_from_spec(spec)
    sys.modules["corpus_batch_selector"] = module
    spec.loader.exec_module(module)
    return module


def _fixture(
    fid,
    source_type,
    sanitization="synthetic",
    platforms=None,
    capability_tags=None,
    path=None,
):
    """Build a minimal fixture-manifest/1 fixture dict for tests."""
    if platforms is None:
        platforms = ["ios", "android", "harmony"]
    if capability_tags is None:
        capability_tags = []
    if path is None:
        path = "samples/%s.json" % fid
    return {
        "id": fid,
        "path": path,
        "source_type": source_type,
        "format": "json",
        "platforms": list(platforms),
        "capability_tags": list(capability_tags),
        "sanitization": sanitization,
        "bytes": 1,
        "sha256": "0" * 64,
    }


def _manifest(fixtures, version="fixture-manifest/1"):
    return {
        "version": version,
        "generated_at": "2026-01-01T00:00:00+00:00",
        "tool": "fixture-manifest-generator",
        "root": "/tmp/test",
        "fixtures": list(fixtures),
        "summary": {"total": len(fixtures), "by_source_type": {}, "by_platform": {}},
    }


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


# ---------------------------------------------------------------------------
# Module loading: the import is deferred so the test collection itself does not
# blow up before the first test runs. If the implementation file is missing,
# every test will fail with ImportError (the expected RED state).
# ---------------------------------------------------------------------------

class _ModuleTestCase(unittest.TestCase):
    """Base class that loads the implementation once for all subtests."""

    @classmethod
    def setUpClass(cls):
        super().setUpClass()
        cls.mod = _load_module()


# ---------------------------------------------------------------------------
# 1. assign_batch
# ---------------------------------------------------------------------------

class AssignBatchTests(_ModuleTestCase):
    def test_book_source_synthetic_is_p0(self):
        self.assertEqual(
            self.mod.assign_batch(_fixture("bs-1", "book-source", "synthetic")),
            "P0",
        )

    def test_web_page_synthetic_is_p0(self):
        self.assertEqual(
            self.mod.assign_batch(_fixture("wp-1", "web-page", "synthetic")),
            "P0",
        )

    def test_json_api_synthetic_is_p1(self):
        self.assertEqual(
            self.mod.assign_batch(_fixture("ja-1", "json-api", "synthetic")),
            "P1",
        )

    def test_rss_feed_synthetic_is_p1(self):
        self.assertEqual(
            self.mod.assign_batch(_fixture("rf-1", "rss-feed", "synthetic")),
            "P1",
        )

    def test_xml_feed_synthetic_is_p1(self):
        self.assertEqual(
            self.mod.assign_batch(_fixture("xf-1", "xml-feed", "synthetic")),
            "P1",
        )

    def test_local_book_is_p2(self):
        self.assertEqual(
            self.mod.assign_batch(_fixture("lb-1", "local-book", "synthetic")),
            "P2",
        )

    def test_book_source_non_synthetic_is_p2(self):
        self.assertEqual(
            self.mod.assign_batch(
                _fixture("bs-2", "book-source", sanitization="unknown")
            ),
            "P2",
        )

    def test_unknown_source_type_is_p2(self):
        self.assertEqual(
            self.mod.assign_batch(_fixture("un-1", "unknown", "synthetic")),
            "P2",
        )


# ---------------------------------------------------------------------------
# 2. build_entry
# ---------------------------------------------------------------------------

class BuildEntryTests(_ModuleTestCase):
    def test_includes_rationale_capability_tags_and_platforms(self):
        fx = _fixture(
            "bs-1",
            "book-source",
            "synthetic",
            platforms=["ios", "android", "harmony"],
            capability_tags=["search", "toc", "chapter-content"],
        )
        entry = self.mod.build_entry(fx, "P0")
        self.assertEqual(entry["fixture_id"], "bs-1")
        self.assertEqual(entry["path"], fx["path"])
        self.assertEqual(entry["source_type"], "book-source")
        self.assertEqual(
            entry["capability_tags"], ["search", "toc", "chapter-content"]
        )
        self.assertEqual(entry["platforms"], ["ios", "android", "harmony"])
        self.assertIn("rationale", entry)
        self.assertIsInstance(entry["rationale"], str)
        self.assertTrue(entry["rationale"], "rationale must be non-empty")

    def test_rationale_differs_per_batch(self):
        fx = _fixture("bs-1", "book-source", "synthetic")
        r0 = self.mod.build_entry(fx, "P0")["rationale"]
        r1 = self.mod.build_entry(fx, "P1")["rationale"]
        r2 = self.mod.build_entry(fx, "P2")["rationale"]
        self.assertEqual(len({r0, r1, r2}), 3)


# ---------------------------------------------------------------------------
# 3 & 4. platform_targets
# ---------------------------------------------------------------------------

class PlatformTargetsTests(_ModuleTestCase):
    def test_p0_runs_on_all_three_platforms(self):
        fx = _fixture("bs-1", "book-source", "synthetic",
                      platforms=["ios", "android", "harmony"])
        self.assertEqual(
            self.mod.platform_targets(fx, "P0"),
            ["ios", "android", "harmony"],
        )

    def test_p1_runs_on_all_fixture_platforms(self):
        fx = _fixture("ja-1", "json-api", "synthetic",
                      platforms=["ios", "android", "harmony"])
        self.assertEqual(
            self.mod.platform_targets(fx, "P1"),
            ["ios", "android", "harmony"],
        )

    def test_p2_runs_on_first_platform_only(self):
        fx = _fixture("lb-1", "local-book", "synthetic",
                      platforms=["ios", "android", "harmony"])
        self.assertEqual(self.mod.platform_targets(fx, "P2"), ["ios"])

    def test_empty_platforms_defaults_to_all_three(self):
        fx = _fixture("bs-1", "book-source", "synthetic", platforms=[])
        # P0 with empty platforms should default to all 3
        self.assertEqual(
            self.mod.platform_targets(fx, "P0"),
            ["ios", "android", "harmony"],
        )

    def test_empty_platforms_p2_defaults_then_first(self):
        fx = _fixture("lb-1", "local-book", "synthetic", platforms=[])
        # Empty → default to all 3, then P2 takes the first → ["ios"]
        self.assertEqual(self.mod.platform_targets(fx, "P2"), ["ios"])


# ---------------------------------------------------------------------------
# 5. select — batch counts
# ---------------------------------------------------------------------------

class SelectBatchCountsTests(_ModuleTestCase):
    def test_three_book_source_two_json_api_one_local_book(self):
        fixtures = [
            _fixture("bs-1", "book-source", "synthetic"),
            _fixture("bs-2", "book-source", "synthetic"),
            _fixture("bs-3", "book-source", "synthetic"),
            _fixture("ja-1", "json-api", "synthetic"),
            _fixture("ja-2", "json-api", "synthetic"),
            _fixture("lb-1", "local-book", "synthetic"),
        ]
        report = self.mod.select(_manifest(fixtures))
        self.assertEqual(report["version"], "corpus-batch-selector/1")
        self.assertEqual(report["tool"], "corpus-batch-selector")
        self.assertEqual(len(report["batches"]["P0"]), 3)
        self.assertEqual(len(report["batches"]["P1"]), 2)
        self.assertEqual(len(report["batches"]["P2"]), 1)
        self.assertEqual(
            report["summary"],
            {
                "p0": 3,
                "p1": 2,
                "p2": 1,
                "total": 6,
                "platform_counts": {
                    "ios": 6,    # P0(3) + P1(2) + P2(1)
                    "android": 5,  # P0(3) + P1(2)
                    "harmony": 5,  # P0(3) + P1(2)
                },
            },
        )


# ---------------------------------------------------------------------------
# 6. select — version validation
# ---------------------------------------------------------------------------

class SelectVersionValidationTests(_ModuleTestCase):
    def test_wrong_version_raises_value_error(self):
        fixtures = [_fixture("bs-1", "book-source", "synthetic")]
        with self.assertRaises(ValueError):
            self.mod.select(_manifest(fixtures, version="fixture-manifest/999"))


# ---------------------------------------------------------------------------
# 7. select — empty fixtures
# ---------------------------------------------------------------------------

class SelectEmptyTests(_ModuleTestCase):
    def test_empty_manifest_yields_empty_batches_and_zero_summary(self):
        report = self.mod.select(_manifest([]))
        self.assertEqual(report["batches"]["P0"], [])
        self.assertEqual(report["batches"]["P1"], [])
        self.assertEqual(report["batches"]["P2"], [])
        self.assertEqual(
            report["summary"],
            {
                "p0": 0,
                "p1": 0,
                "p2": 0,
                "total": 0,
                "platform_counts": {
                    "ios": 0,
                    "android": 0,
                    "harmony": 0,
                },
            },
        )
        self.assertEqual(
            report["platform_inputs"],
            {"ios": [], "android": [], "harmony": []},
        )


# ---------------------------------------------------------------------------
# 8. select — platform_inputs membership
# ---------------------------------------------------------------------------

class SelectPlatformInputsTests(_ModuleTestCase):
    def test_p0_appears_in_all_three_platform_inputs(self):
        fixtures = [
            _fixture("bs-1", "book-source", "synthetic",
                     platforms=["ios", "android", "harmony"]),
        ]
        report = self.mod.select(_manifest(fixtures))
        self.assertIn("bs-1", report["platform_inputs"]["ios"])
        self.assertIn("bs-1", report["platform_inputs"]["android"])
        self.assertIn("bs-1", report["platform_inputs"]["harmony"])

    def test_p2_appears_only_in_first_platform_input(self):
        fixtures = [
            _fixture("lb-1", "local-book", "synthetic",
                     platforms=["ios", "android", "harmony"]),
        ]
        report = self.mod.select(_manifest(fixtures))
        self.assertIn("lb-1", report["platform_inputs"]["ios"])
        self.assertNotIn("lb-1", report["platform_inputs"]["android"])
        self.assertNotIn("lb-1", report["platform_inputs"]["harmony"])

    def test_platform_inputs_lists_are_sorted(self):
        fixtures = [
            _fixture("bs-3", "book-source", "synthetic"),
            _fixture("bs-1", "book-source", "synthetic"),
            _fixture("bs-2", "book-source", "synthetic"),
        ]
        report = self.mod.select(_manifest(fixtures))
        self.assertEqual(
            report["platform_inputs"]["ios"], ["bs-1", "bs-2", "bs-3"]
        )


# ---------------------------------------------------------------------------
# 9 & 10. select_from_root
# ---------------------------------------------------------------------------

class SelectFromRootTests(_ModuleTestCase):
    def test_tempdir_with_synthetic_fixtures_no_manifest(self):
        with tempfile.TemporaryDirectory() as d:
            # Write synthetic fixture files under samples/ so the manifest
            # generator classifies them as synthetic.
            _write(
                os.path.join(d, "samples", "bs-1.json"),
                json.dumps({
                    "id": "bs-1",
                    "source_type": "book-source",
                    "url": "https://example.test/",
                }),
            )
            _write(
                os.path.join(d, "samples", "ja-1.json"),
                json.dumps({"id": "ja-1", "data": [], "url": "https://example.test/"}),
            )
            _write(
                os.path.join(d, "samples", "lb-1.txt"),
                "hello world example.test placeholder",
            )
            report = self.mod.select_from_root(Path(d))
        self.assertEqual(report["version"], "corpus-batch-selector/1")
        # bs-1 → P0, ja-1 → P1, lb-1 → P2
        self.assertEqual(len(report["batches"]["P0"]), 1)
        self.assertEqual(len(report["batches"]["P1"]), 1)
        self.assertEqual(len(report["batches"]["P2"]), 1)
        self.assertEqual(
            report["batches"]["P0"][0]["source_type"], "book-source"
        )
        self.assertEqual(
            report["batches"]["P1"][0]["source_type"], "json-api"
        )
        self.assertEqual(
            report["batches"]["P2"][0]["source_type"], "local-book"
        )

    def test_missing_dir_raises_file_not_found(self):
        with self.assertRaises(FileNotFoundError):
            self.mod.select_from_root(
                Path("/nonexistent-path-xyz-abc-12345")
            )

    def test_uses_sibling_manifest_when_present(self):
        with tempfile.TemporaryDirectory() as d:
            manifest = _manifest([
                _fixture("bs-1", "book-source", "synthetic"),
                _fixture("ja-1", "json-api", "synthetic"),
            ])
            _write(
                os.path.join(d, "fixture-manifest.json"),
                json.dumps(manifest),
            )
            report = self.mod.select_from_root(Path(d))
        self.assertEqual(len(report["batches"]["P0"]), 1)
        self.assertEqual(len(report["batches"]["P1"]), 1)
        self.assertEqual(len(report["batches"]["P2"]), 0)


# ---------------------------------------------------------------------------
# 11, 12, 13. CLI
# ---------------------------------------------------------------------------

class CLITests(_ModuleTestCase):
    def test_manifest_flag_emits_valid_json_exit_0(self):
        with tempfile.TemporaryDirectory() as d:
            manifest_path = _write(
                os.path.join(d, "manifest.json"),
                json.dumps(_manifest([
                    _fixture("bs-1", "book-source", "synthetic"),
                    _fixture("ja-1", "json-api", "synthetic"),
                ])),
            )
            result = _run_cli(["--manifest", manifest_path])
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(report["version"], "corpus-batch-selector/1")
        self.assertEqual(len(report["batches"]["P0"]), 1)
        self.assertEqual(len(report["batches"]["P1"]), 1)

    def test_manifest_flag_with_out_writes_file(self):
        with tempfile.TemporaryDirectory() as d:
            manifest_path = _write(
                os.path.join(d, "manifest.json"),
                json.dumps(_manifest([
                    _fixture("bs-1", "book-source", "synthetic"),
                ])),
            )
            out_path = os.path.join(d, "out.json")
            result = _run_cli(
                ["--manifest", manifest_path, "--out", out_path]
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(os.path.exists(out_path))
            with open(out_path, "r", encoding="utf-8") as fh:
                report = json.load(fh)
        self.assertEqual(len(report["batches"]["P0"]), 1)

    def test_root_flag_on_tempdir_emits_valid_json(self):
        with tempfile.TemporaryDirectory() as d:
            _write(
                os.path.join(d, "samples", "bs-1.json"),
                json.dumps({
                    "id": "bs-1",
                    "source_type": "book-source",
                    "url": "https://example.test/",
                }),
            )
            result = _run_cli(["--root", d])
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(report["version"], "corpus-batch-selector/1")
        self.assertTrue(
            any(e["fixture_id"] == "bs-1"
                for e in report["batches"]["P0"])
        )

    def test_missing_manifest_path_exits_2(self):
        result = _run_cli([
            "--manifest", "/nonexistent/manifest.json",
        ])
        self.assertEqual(result.returncode, 2)

    def test_pretty_flag_indents_and_sorts(self):
        with tempfile.TemporaryDirectory() as d:
            manifest_path = _write(
                os.path.join(d, "manifest.json"),
                json.dumps(_manifest([
                    _fixture("bs-1", "book-source", "synthetic"),
                ])),
            )
            result = _run_cli(["--manifest", manifest_path, "--pretty"])
        self.assertEqual(result.returncode, 0, result.stderr)
        # Pretty output starts with a 2-space indent on the first key.
        self.assertTrue(result.stdout.startswith("{\n  "))
        # Round-trips as JSON.
        report = json.loads(result.stdout)
        self.assertEqual(len(report["batches"]["P0"]), 1)


# ---------------------------------------------------------------------------
# 14. entries sorted by fixture_id within each batch
# ---------------------------------------------------------------------------

class SortedEntriesTests(_ModuleTestCase):
    def test_entries_sorted_by_fixture_id_within_each_batch(self):
        fixtures = [
            _fixture("bs-3", "book-source", "synthetic"),
            _fixture("bs-1", "book-source", "synthetic"),
            _fixture("bs-2", "book-source", "synthetic"),
            _fixture("ja-2", "json-api", "synthetic"),
            _fixture("ja-1", "json-api", "synthetic"),
            _fixture("lb-2", "local-book", "synthetic"),
            _fixture("lb-1", "local-book", "synthetic"),
        ]
        report = self.mod.select(_manifest(fixtures))
        for batch_name in ("P0", "P1", "P2"):
            ids = [e["fixture_id"] for e in report["batches"][batch_name]]
            self.assertEqual(ids, sorted(ids), msg=batch_name)
        self.assertEqual(
            [e["fixture_id"] for e in report["batches"]["P0"]],
            ["bs-1", "bs-2", "bs-3"],
        )
        self.assertEqual(
            [e["fixture_id"] for e in report["batches"]["P1"]],
            ["ja-1", "ja-2"],
        )
        self.assertEqual(
            [e["fixture_id"] for e in report["batches"]["P2"]],
            ["lb-1", "lb-2"],
        )


# ---------------------------------------------------------------------------
# 15. summary platform_counts correct
# ---------------------------------------------------------------------------

class SummaryPlatformCountsTests(_ModuleTestCase):
    def test_platform_counts_match_platform_inputs_lengths(self):
        fixtures = [
            _fixture("bs-1", "book-source", "synthetic",
                     platforms=["ios", "android", "harmony"]),
            _fixture("bs-2", "book-source", "synthetic",
                     platforms=["ios", "android", "harmony"]),
            _fixture("ja-1", "json-api", "synthetic",
                     platforms=["ios", "android", "harmony"]),
            _fixture("lb-1", "local-book", "synthetic",
                     platforms=["ios", "android", "harmony"]),
        ]
        report = self.mod.select(_manifest(fixtures))
        counts = report["summary"]["platform_counts"]
        self.assertEqual(
            counts["ios"], len(report["platform_inputs"]["ios"])
        )
        self.assertEqual(
            counts["android"], len(report["platform_inputs"]["android"])
        )
        self.assertEqual(
            counts["harmony"], len(report["platform_inputs"]["harmony"])
        )
        # P0(bs-1,bs-2) → all 3 platforms each
        # P1(ja-1) → all 3 platforms
        # P2(lb-1) → ios only
        # ios: 2 + 1 + 1 = 4; android: 2 + 1 = 3; harmony: 2 + 1 = 3
        self.assertEqual(counts, {"ios": 4, "android": 3, "harmony": 3})


if __name__ == "__main__":
    unittest.main()
