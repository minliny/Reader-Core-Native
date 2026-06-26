"""Tests for tools/evidence-indexer/evidence_indexer.py.

Strict TDD: these tests are written BEFORE the implementation. They feed
canned (path, obj) tuples to the pure classifiers and use tempdirs for the
disk-walking `collect` orchestrator, so they are fully deterministic and
never touch the real worktree.

Run:
    python3 -m unittest tests.tooling.test_evidence_indexer -v
"""

import hashlib
import importlib.util
import io
import json
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

_HERE = Path(__file__).resolve().parent
_REPO_ROOT = _HERE.parent.parent
_MODULE_PATH = _REPO_ROOT / "tools" / "evidence-indexer" / "evidence_indexer.py"
_CLI = str(_MODULE_PATH)


def _load_module():
    """Load evidence_indexer.py by file path (dir name has a hyphen)."""
    if not _MODULE_PATH.exists():
        raise ImportError(
            "evidence_indexer implementation not found at %s. "
            "TDD: write the implementation after the tests." % _MODULE_PATH
        )
    spec = importlib.util.spec_from_file_location("evidence_indexer", _MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    sys.modules["evidence_indexer"] = module
    spec.loader.exec_module(module)
    return module


ei = _load_module()


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
def _pe_record(**overrides):
    """A valid platform-evidence/1 single record."""
    base = {
        "version": "platform-evidence/1",
        "platform": "ios",
        "kind": "smoke",
        "capability": "reader.search",
        "status": "pass",
        "timestamp": "2026-06-25T08:00:00Z",
        "environment": {"os": "Darwin", "arch": "arm64"},
    }
    base.update(overrides)
    return base


def _write(path, content):
    p = Path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(content, encoding="utf-8")
    return p


def _write_json(path, obj):
    _write(path, json.dumps(obj))
    return path


def _sha1_12(rel_path):
    return hashlib.sha1(rel_path.encode("utf-8")).hexdigest()[:12]


# ---------------------------------------------------------------------------
# 1. classify_json on platform-evidence/1 single record
# ---------------------------------------------------------------------------
class PlatformEvidenceSingleTests(unittest.TestCase):
    def test_single_record_produces_one_entry_with_fields(self):
        path = "samples/tooling/evidence/ios_smoke.json"
        entries = ei.classify_json(path, _pe_record())
        self.assertEqual(len(entries), 1)
        e = entries[0]
        self.assertEqual(e["tier"], "smoke")
        self.assertEqual(e["platform"], "ios")
        self.assertEqual(e["capability"], "reader.search")
        self.assertEqual(e["status"], "pass")
        self.assertEqual(e["timestamp"], "2026-06-25T08:00:00Z")
        self.assertEqual(e["source"], path)
        self.assertEqual(e["path"], path)
        self.assertEqual(e["id"], _sha1_12(path))


# ---------------------------------------------------------------------------
# 2. classify_json on platform-evidence/1 batch with 3 records
# ---------------------------------------------------------------------------
class PlatformEvidenceBatchTests(unittest.TestCase):
    def test_batch_with_three_records_produces_three_entries(self):
        path = "samples/tooling/evidence/batch.json"
        obj = {
            "version": "platform-evidence/1",
            "records": [
                _pe_record(platform="ios", kind="unit", capability="a",
                           status="pass", timestamp="2026-06-25T08:00:00Z"),
                _pe_record(platform="android", kind="build", capability="b",
                           status="skipped", timestamp="2026-06-25T08:01:00Z"),
                _pe_record(platform="harmony", kind="device", capability="c",
                           status="fail", timestamp="2026-06-25T08:02:00Z"),
            ],
        }
        entries = ei.classify_json(path, obj)
        self.assertEqual(len(entries), 3)
        kinds = [e["tier"] for e in entries]
        self.assertEqual(kinds, ["unit", "build", "device"])
        plats = [e["platform"] for e in entries]
        self.assertEqual(plats, ["ios", "android", "harmony"])
        # All entries share the same source path but must have unique ids.
        for e in entries:
            self.assertEqual(e["source"], path)
        ids = [e["id"] for e in entries]
        self.assertEqual(len(set(ids)), 3, "batch entry ids must be unique")


# ---------------------------------------------------------------------------
# 3. classify_json on capability-catalog/1
# ---------------------------------------------------------------------------
class CapabilityCatalogTests(unittest.TestCase):
    def test_one_entry_tier_unknown_platform_host_status_pass(self):
        path = "reports/tooling/capability-catalog.json"
        obj = {
            "version": "capability-catalog/1",
            "generated_at": "2026-06-25T06:28:35Z",
            "tool": "capability-catalog-generator",
            "capabilities": [],
            "summary": {"total": 0},
        }
        entries = ei.classify_json(path, obj)
        self.assertEqual(len(entries), 1)
        e = entries[0]
        self.assertEqual(e["tier"], "unknown")
        self.assertEqual(e["platform"], "host")
        self.assertIsNone(e["capability"])
        self.assertEqual(e["status"], "pass")
        self.assertEqual(e["timestamp"], "2026-06-25T06:28:35Z")


# ---------------------------------------------------------------------------
# 4. classify_json on build-env-doctor/1
# ---------------------------------------------------------------------------
class BuildEnvDoctorTests(unittest.TestCase):
    def test_one_entry_tier_build_platform_host(self):
        path = "reports/tooling/build-env-doctor.json"
        obj = {
            "version": "build-env-doctor/1",
            "generated_at": "2026-06-25T06:00:00Z",
            "tool": "build-environment-doctor",
            "platform": {"os": "Darwin", "arch": "arm64"},
            "tools": [],
            "summary": {"total": 0, "found": 0, "missing": 0, "unknown": 0},
        }
        entries = ei.classify_json(path, obj)
        self.assertEqual(len(entries), 1)
        e = entries[0]
        self.assertEqual(e["tier"], "build")
        self.assertEqual(e["platform"], "host")
        self.assertIsNone(e["capability"])
        self.assertEqual(e["timestamp"], "2026-06-25T06:00:00Z")


# ---------------------------------------------------------------------------
# 5. classify_json on fixture-manifest/1
# ---------------------------------------------------------------------------
class FixtureManifestTests(unittest.TestCase):
    def test_one_entry_tier_corpus(self):
        path = "reports/tooling/fixture-manifest.json"
        obj = {
            "version": "fixture-manifest/1",
            "generated_at": "2026-06-25T06:00:00Z",
            "tool": "fixture-manifest-generator",
            "fixtures": [],
            "summary": {"total": 0, "by_platform": {}},
        }
        entries = ei.classify_json(path, obj)
        self.assertEqual(len(entries), 1)
        e = entries[0]
        self.assertEqual(e["tier"], "corpus")
        self.assertEqual(e["platform"], "host")


# ---------------------------------------------------------------------------
# 6. classify_json on protocol-schema-lint/1 (unexpected_invalid status)
# ---------------------------------------------------------------------------
class ProtocolSchemaLintTests(unittest.TestCase):
    def _obj(self, unexpected_invalid):
        return {
            "version": "protocol-schema-lint/1",
            "generated_at": "2026-06-25T06:00:00Z",
            "tool": "protocol-schema-lint",
            "results": [],
            "summary": {
                "total": 0,
                "valid": 0,
                "invalid": 0,
                "expected_invalid": 0,
                "unexpected_invalid": unexpected_invalid,
                "unexpected_valid": 0,
            },
        }

    def test_status_pass_when_unexpected_invalid_zero(self):
        entries = ei.classify_json("reports/x.json", self._obj(0))
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]["status"], "pass")
        self.assertEqual(entries[0]["tier"], "smoke")
        self.assertEqual(entries[0]["platform"], "host")

    def test_status_fail_when_unexpected_invalid_nonzero(self):
        entries = ei.classify_json("reports/x.json", self._obj(2))
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]["status"], "fail")


# ---------------------------------------------------------------------------
# 7. classify_json on evidence-index/1 -> skip (empty)
# ---------------------------------------------------------------------------
class EvidenceIndexSkipTests(unittest.TestCase):
    def test_evidence_index_version_returns_empty(self):
        obj = {
            "version": "evidence-index/1",
            "generated_at": "2026-06-25T06:00:00Z",
            "tool": "evidence-indexer",
            "entries": [{"id": "abc"}],
            "summary": {"total": 1},
        }
        self.assertEqual(ei.classify_json("reports/idx.json", obj), [])


# ---------------------------------------------------------------------------
# 8. classify_json on unknown version
# ---------------------------------------------------------------------------
class UnknownVersionTests(unittest.TestCase):
    def test_unknown_version_one_entry_tier_unknown(self):
        path = "reports/tooling/something.json"
        obj = {
            "version": "some-other-tool/7",
            "generated_at": "2026-06-25T06:00:00Z",
            "tool": "something",
        }
        entries = ei.classify_json(path, obj)
        self.assertEqual(len(entries), 1)
        e = entries[0]
        self.assertEqual(e["tier"], "unknown")
        self.assertIsNone(e["capability"])
        self.assertEqual(e["timestamp"], "2026-06-25T06:00:00Z")

    def test_unknown_version_without_generated_at_timestamp_null(self):
        obj = {"version": "weird/1", "tool": "weird"}
        entries = ei.classify_json("reports/w.json", obj)
        self.assertEqual(len(entries), 1)
        self.assertIsNone(entries[0]["timestamp"])


# ---------------------------------------------------------------------------
# 9. classify_md
# ---------------------------------------------------------------------------
class ClassifyMdTests(unittest.TestCase):
    def test_reports_smoke_md_tier_smoke(self):
        e = ei.classify_md("reports/foo-smoke.md", "# smoke\n")
        self.assertIsNotNone(e)
        self.assertEqual(e["tier"], "smoke")
        self.assertEqual(e["status"], "unknown")
        self.assertIsNone(e["timestamp"])
        self.assertIsNone(e["capability"])

    def test_evidence_ios_device_md_tier_device_platform_ios(self):
        e = ei.classify_md("evidence/ios-device.md", "# ios device\n")
        self.assertIsNotNone(e)
        self.assertEqual(e["tier"], "device")
        self.assertEqual(e["platform"], "ios")

    def test_file_not_under_reports_or_evidence_returns_none(self):
        self.assertIsNone(
            ei.classify_md("samples/tooling/evidence/README.md", "# readme\n")
        )
        self.assertIsNone(ei.classify_md("docs/build.md", "# build\n"))

    def test_md_round_keyword_tier_unknown(self):
        e = ei.classify_md("reports/round-1.md", "# round\n")
        self.assertIsNotNone(e)
        self.assertEqual(e["tier"], "unknown")

    def test_md_android_corpus_keywords(self):
        e = ei.classify_md("reports/android-corpus-run.md", "# x\n")
        self.assertIsNotNone(e)
        self.assertEqual(e["tier"], "corpus")
        self.assertEqual(e["platform"], "android")


# ---------------------------------------------------------------------------
# 10. collect on a tempdir with a mix
# ---------------------------------------------------------------------------
class CollectMixTests(unittest.TestCase):
    def test_collect_mixed_entries_and_summary(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_json(root / "reports/tooling/capability-catalog.json", {
                "version": "capability-catalog/1",
                "generated_at": "2026-06-25T06:00:00Z",
                "tool": "capability-catalog-generator",
                "capabilities": [], "summary": {"total": 0},
            })
            _write_json(root / "samples/tooling/evidence/ios_smoke.json",
                        _pe_record())
            _write_json(root / "samples/tooling/evidence/batch.json", {
                "version": "platform-evidence/1",
                "records": [
                    _pe_record(kind="unit", capability="a"),
                    _pe_record(kind="build", capability="b",
                               platform="android"),
                ],
            })
            _write(root / "reports/smoke-round.md", "# smoke round\n")
            index = ei.collect(str(root))
        self.assertEqual(index["version"], "evidence-index/1")
        self.assertEqual(index["tool"], "evidence-indexer")
        self.assertIn("generated_at", index)
        entries = index["entries"]
        # 1 (catalog) + 1 (ios single) + 2 (batch) + 1 (md) = 5
        self.assertEqual(len(entries), 5)
        s = index["summary"]
        self.assertEqual(s["total"], 5)
        # catalog -> unknown; ios_smoke -> smoke; batch -> unit, build;
        # md "smoke-round.md" -> smoke (smoke keyword wins over round->unknown).
        self.assertEqual(s["by_tier"].get("unknown"), 1)  # catalog only
        self.assertEqual(s["by_tier"].get("smoke"), 2)  # ios_smoke + md
        self.assertEqual(s["by_tier"].get("unit"), 1)
        self.assertEqual(s["by_tier"].get("build"), 1)
        self.assertEqual(s["by_platform"].get("ios"), 2)  # ios_smoke + batch[0]
        self.assertEqual(s["by_platform"].get("android"), 1)
        self.assertEqual(s["by_platform"].get("host"), 1)  # catalog


# ---------------------------------------------------------------------------
# 11. collect skips .git/, target/, binary, malformed JSON
# ---------------------------------------------------------------------------
class CollectSkipTests(unittest.TestCase):
    def test_collect_skips_forbidden_and_malformed(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            # Forbidden dirs.
            _write_json(root / ".git/secret.json",
                        {"version": "platform-evidence/1"})
            _write_json(root / "target/build.json",
                        {"version": "build-env-doctor/1",
                         "generated_at": "2026-06-25T06:00:00Z"})
            _write_json(root / "node_modules/pkg.json",
                        {"version": "weird/1"})
            # Dotfile.
            _write(root / ".config.json", '{"version": "weird/1"}')
            # Malformed JSON.
            _write(root / "reports/broken.json", "{not valid json")
            # Binary file (null bytes) with .json name.
            (root / "reports/bin.json").write_bytes(b"\x00\x01\x02\xff")
            # A valid report to confirm we still index the good one.
            _write_json(root / "reports/good.json", {
                "version": "capability-catalog/1",
                "generated_at": "2026-06-25T06:00:00Z",
                "tool": "ccg", "capabilities": [], "summary": {"total": 0},
            })
            index = ei.collect(str(root))
        sources = [e["source"] for e in index["entries"]]
        self.assertIn("reports/good.json", sources)
        # None of the forbidden/malformed/binary/dotfile paths indexed.
        for bad in (".git/secret.json", "target/build.json",
                    "node_modules/pkg.json", ".config.json",
                    "reports/broken.json", "reports/bin.json"):
            self.assertNotIn(bad, sources, "indexed forbidden: %s" % bad)
        self.assertEqual(index["summary"]["total"], 1)


# ---------------------------------------------------------------------------
# 12. collect on empty tempdir
# ---------------------------------------------------------------------------
class CollectEmptyTests(unittest.TestCase):
    def test_collect_empty_tempdir(self):
        with tempfile.TemporaryDirectory() as td:
            index = ei.collect(td)
        self.assertEqual(index["entries"], [])
        self.assertEqual(index["summary"]["total"], 0)
        self.assertEqual(index["summary"]["by_tier"], {})
        self.assertEqual(index["summary"]["by_platform"], {})
        self.assertEqual(index["summary"]["by_status"], {})


# ---------------------------------------------------------------------------
# 13. collect on missing dir -> FileNotFoundError
# ---------------------------------------------------------------------------
class CollectMissingDirTests(unittest.TestCase):
    def test_collect_missing_dir_raises_filenotfound(self):
        with self.assertRaises(FileNotFoundError):
            ei.collect(str(Path(tempfile.gettempdir()) / "no_such_dir_xyz_123"))


# ---------------------------------------------------------------------------
# 14. id is first 12 hex of sha1 of rel path
# ---------------------------------------------------------------------------
class IdShapeTests(unittest.TestCase):
    def test_id_is_sha1_12_of_rel_path(self):
        path = "samples/tooling/evidence/ios_smoke.json"
        entries = ei.classify_json(path, _pe_record())
        self.assertEqual(entries[0]["id"], _sha1_12(path))
        self.assertEqual(len(entries[0]["id"]), 12)
        # Hex characters only.
        int(entries[0]["id"], 16)


# ---------------------------------------------------------------------------
# 15. entries sorted by source then id
# ---------------------------------------------------------------------------
class SortedEntriesTests(unittest.TestCase):
    def test_entries_sorted_by_source_then_id(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write_json(root / "reports/zzz.json", {
                "version": "capability-catalog/1",
                "generated_at": "2026-06-25T06:00:00Z",
                "tool": "c", "capabilities": [], "summary": {},
            })
            _write_json(root / "reports/aaa.json", {
                "version": "build-env-doctor/1",
                "generated_at": "2026-06-25T06:00:00Z",
                "tool": "b", "tools": [], "summary": {},
                "platform": {},
            })
            _write_json(root / "reports/aaa_batch.json", {
                "version": "platform-evidence/1",
                "records": [
                    _pe_record(kind="unit", capability="a"),
                    _pe_record(kind="build", capability="b"),
                ],
            })
            index = ei.collect(str(root))
        entries = index["entries"]
        keys = [(e["source"], e["id"]) for e in entries]
        self.assertEqual(keys, sorted(keys))


# ---------------------------------------------------------------------------
# 16. CLI --pretty valid JSON exit 0; --out writes file
# ---------------------------------------------------------------------------
class CliTests(unittest.TestCase):
    def test_pretty_outputs_valid_json_exit0(self):
        with tempfile.TemporaryDirectory() as td:
            _write_json(Path(td) / "reports/good.json", {
                "version": "capability-catalog/1",
                "generated_at": "2026-06-25T06:00:00Z",
                "tool": "c", "capabilities": [], "summary": {},
            })
            buf = io.StringIO()
            with redirect_stdout(buf):
                rc = ei.main([td, "--pretty"])
        self.assertEqual(rc, 0)
        out = buf.getvalue()
        self.assertTrue(out.endswith("\n"))
        parsed = json.loads(out)
        self.assertEqual(parsed["version"], "evidence-index/1")
        self.assertGreater(len(parsed["entries"]), 0)

    def test_out_writes_file(self):
        with tempfile.TemporaryDirectory() as td:
            _write_json(Path(td) / "reports/good.json", {
                "version": "capability-catalog/1",
                "generated_at": "2026-06-25T06:00:00Z",
                "tool": "c", "capabilities": [], "summary": {},
            })
            out_path = os.path.join(td, "index.json")
            rc = ei.main([td, "--out", out_path])
            self.assertEqual(rc, 0)
            with open(out_path, "r", encoding="utf-8") as f:
                parsed = json.load(f)
            self.assertEqual(parsed["version"], "evidence-index/1")
            self.assertGreater(len(parsed["entries"]), 0)


# ---------------------------------------------------------------------------
# 17. CLI on missing root -> exit 2
# ---------------------------------------------------------------------------
class CliMissingRootTests(unittest.TestCase):
    def test_cli_missing_root_exit2(self):
        missing = str(Path(tempfile.gettempdir()) / "no_such_root_xyz_123")
        with redirect_stderr(io.StringIO()):
            rc = ei.main([missing])
        self.assertEqual(rc, 2)


# ---------------------------------------------------------------------------
# 18. by_tier/by_platform/by_status only include present keys
# ---------------------------------------------------------------------------
class SummaryKeysTests(unittest.TestCase):
    def test_summary_only_includes_present_keys(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            # Only ios smoke pass entries.
            _write_json(root / "reports/ios.json",
                        _pe_record(platform="ios", kind="smoke",
                                   status="pass"))
            index = ei.collect(str(root))
        s = index["summary"]
        self.assertEqual(set(s["by_tier"].keys()), {"smoke"})
        self.assertEqual(set(s["by_platform"].keys()), {"ios"})
        self.assertEqual(set(s["by_status"].keys()), {"pass"})


# ---------------------------------------------------------------------------
# Extra contract coverage: platform-evidence-validator, worktree-conflict,
# corpus-batch-selector, gate-declaration-report.
# ---------------------------------------------------------------------------
class ExtraVersionTests(unittest.TestCase):
    def test_platform_evidence_validator_summary_entry(self):
        obj = {
            "version": "platform-evidence-validator/1",
            "generated_at": "2026-06-25T06:00:00Z",
            "tool": "platform-evidence-validator",
            "results": [
                {"path": "a.json", "valid": True, "errors": []},
                {"path": "b.json", "valid": False, "errors": ["x"]},
            ],
            "summary": {"total": 2, "valid": 1, "invalid": 1},
        }
        entries = ei.classify_json("reports/val.json", obj)
        self.assertEqual(len(entries), 1)
        e = entries[0]
        self.assertEqual(e["status"], "fail")
        self.assertEqual(e["timestamp"], "2026-06-25T06:00:00Z")

    def test_platform_evidence_validator_all_valid_status_pass(self):
        obj = {
            "version": "platform-evidence-validator/1",
            "generated_at": "2026-06-25T06:00:00Z",
            "tool": "pev",
            "results": [{"path": "a.json", "valid": True, "errors": []}],
            "summary": {"total": 1, "valid": 1, "invalid": 0},
        }
        entries = ei.classify_json("reports/val.json", obj)
        self.assertEqual(entries[0]["status"], "pass")

    def test_worktree_conflict_report_entry(self):
        obj = {
            "version": "worktree-conflict-report/1",
            "generated_at": "2026-06-25T06:00:00Z",
            "tool": "worktree-conflict",
            "conflicts": [],
        }
        entries = ei.classify_json("reports/wc.json", obj)
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]["tier"], "unknown")
        self.assertEqual(entries[0]["platform"], "host")

    def test_corpus_batch_selector_entry(self):
        obj = {
            "version": "corpus-batch-selector/1",
            "generated_at": "2026-06-25T06:00:00Z",
            "tool": "corpus-batch-selector",
            "selected": [],
        }
        entries = ei.classify_json("reports/cbs.json", obj)
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]["tier"], "corpus")

    def test_gate_declaration_report_per_platform_gate(self):
        obj = {
            "version": "gate-declaration-report/1",
            "generated_at": "2026-06-25T06:00:00Z",
            "tool": "gate-declaration",
            "gates": [
                {"platform": "ios", "fail_closed": True},
                {"platform": "android", "fail_closed": False},
            ],
        }
        entries = ei.classify_json("reports/gate.json", obj)
        self.assertEqual(len(entries), 2)
        by_plat = {e["platform"]: e["status"] for e in entries}
        self.assertEqual(by_plat["ios"], "pass")
        self.assertEqual(by_plat["android"], "fail")
        for e in entries:
            self.assertEqual(e["tier"], "smoke")


# ---------------------------------------------------------------------------
# Edge: invalid platform/kind/status normalized to unknown
# ---------------------------------------------------------------------------
class NormalizeTests(unittest.TestCase):
    def test_bad_platform_normalized_to_unknown(self):
        entries = ei.classify_json(
            "reports/x.json",
            _pe_record(platform="windows"),
        )
        self.assertEqual(entries[0]["platform"], "unknown")

    def test_bad_kind_normalized_to_unknown_tier(self):
        entries = ei.classify_json(
            "reports/x.json",
            _pe_record(kind="weird"),
        )
        self.assertEqual(entries[0]["tier"], "unknown")


if __name__ == "__main__":
    unittest.main()
