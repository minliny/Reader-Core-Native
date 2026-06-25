"""Tests for tools/release-blockers/release_blockers.py.

Strict TDD: tests written BEFORE the implementation. The blocker-derivation
logic is exercised through pure functions with canned parsed inputs; the
disk-reading ``collect()`` orchestrator is exercised with small tempdirs so
tests never touch the real worktree.

Run:
    python3 -m unittest tests.tooling.test_release_blockers -v
"""

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

_HERE = Path(__file__).resolve().parent
_REPO_ROOT = _HERE.parent.parent
_MODULE_PATH = _REPO_ROOT / "tools" / "release-blockers" / "release_blockers.py"
_CLI = str(_MODULE_PATH)


def _load_module():
    """Load release_blockers.py by file path (the dir name has a hyphen)."""
    if not _MODULE_PATH.exists():
        raise ImportError(
            "release_blockers implementation not found at %s. "
            "TDD: write the implementation after the tests." % _MODULE_PATH
        )
    spec = importlib.util.spec_from_file_location("release_blockers", _MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    sys.modules["release_blockers"] = module
    spec.loader.exec_module(module)
    return module


rb = _load_module()


# ---------------------------------------------------------------------------
# Canned input factories
# ---------------------------------------------------------------------------
_DEFAULT_PLATFORMS = ["ios", "android", "harmony"]


def _cap(**overrides):
    """A canned capability entry."""
    base = {
        "id": "test.cap",
        "owner": "core",
        "status": "implemented",
        "platforms": list(_DEFAULT_PLATFORMS),
        "evidence": [],
        "source": "FEATURE_MATRIX.md",
    }
    base.update(overrides)
    return base


def _ev(**overrides):
    """A canned evidence entry."""
    base = {
        "tier": "smoke",
        "platform": "ios",
        "capability": "test.cap",
        "status": "pass",
        "source": "evidence/test.json",
        "path": "evidence/test.json",
    }
    base.update(overrides)
    return base


def _write(path, content):
    p = Path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(content, encoding="utf-8")
    return p


def _run_cli(args):
    return subprocess.run(
        [sys.executable, _CLI] + [str(a) for a in args],
        capture_output=True,
        text=True,
    )


# ---------------------------------------------------------------------------
# 1. derive_blockers: missing core + zero evidence -> blocker, missing status
# ---------------------------------------------------------------------------
class DeriveMissingCoreZeroEvidenceTests(unittest.TestCase):
    def test_missing_core_zero_evidence_is_blocker(self):
        caps = [_cap(id="book.search", owner="core", status="missing")]
        blockers = rb.derive_blockers(caps, [], [])
        self.assertEqual(len(blockers), 1)
        b = blockers[0]
        self.assertEqual(b["severity"], "blocker")
        self.assertEqual(b["evidence_status"], "missing")
        self.assertEqual(b["capability"], "book.search")
        self.assertEqual(b["platform"], "all")
        self.assertTrue(b["id"].startswith("rb-"))


# ---------------------------------------------------------------------------
# 2. derive_blockers: missing capability WITH a passing evidence -> NOT blocker
# ---------------------------------------------------------------------------
class DeriveMissingWithPassingTests(unittest.TestCase):
    def test_missing_with_passing_evidence_not_blocker(self):
        caps = [_cap(id="book.search", owner="core", status="missing")]
        ev = [_ev(capability="book.search", status="pass", platform="ios")]
        blockers = rb.derive_blockers(caps, ev, [])
        self.assertEqual(blockers, [])


# ---------------------------------------------------------------------------
# 3. derive_blockers: failing_evidence -> blocker, evidence_status fail
# ---------------------------------------------------------------------------
class DeriveFailingEvidenceTests(unittest.TestCase):
    def test_failing_evidence_is_blocker(self):
        caps = [_cap(id="book.search", owner="core", status="implemented")]
        ev = [_ev(capability="book.search", status="fail", platform="ios")]
        blockers = rb.derive_blockers(caps, ev, [])
        self.assertEqual(len(blockers), 1)
        b = blockers[0]
        self.assertEqual(b["severity"], "blocker")
        self.assertEqual(b["evidence_status"], "fail")


# ---------------------------------------------------------------------------
# 4. derive_blockers: partial + zero passing -> high
# ---------------------------------------------------------------------------
class DerivePartialZeroPassingTests(unittest.TestCase):
    def test_partial_zero_passing_is_high(self):
        # Owner host so the partial+core+zero-evidence blocker rule does not fire.
        caps = [_cap(id="book.search", owner="host", status="partial")]
        blockers = rb.derive_blockers(caps, [], [])
        self.assertEqual(len(blockers), 1)
        self.assertEqual(blockers[0]["severity"], "high")


# ---------------------------------------------------------------------------
# 5. derive_blockers: partial + ios-only-passing, required [ios,android,harmony]
#    -> medium, evidence_status partial, platform "all"
# ---------------------------------------------------------------------------
class DerivePartialSomePassingTests(unittest.TestCase):
    def test_partial_some_platforms_passing_is_medium(self):
        caps = [_cap(id="book.search", owner="core", status="partial",
                     platforms=["ios", "android", "harmony"])]
        ev = [_ev(capability="book.search", status="pass", platform="ios")]
        blockers = rb.derive_blockers(caps, ev, [])
        self.assertEqual(len(blockers), 1)
        b = blockers[0]
        self.assertEqual(b["severity"], "medium")
        self.assertEqual(b["evidence_status"], "partial")
        self.assertEqual(b["platform"], "all")


# ---------------------------------------------------------------------------
# 6. derive_blockers: host-owned + zero evidence -> medium
# ---------------------------------------------------------------------------
class DeriveHostOwnedZeroEvidenceTests(unittest.TestCase):
    def test_host_owned_zero_evidence_is_medium(self):
        caps = [_cap(id="ui.nav", owner="host", status="implemented")]
        blockers = rb.derive_blockers(caps, [], [])
        self.assertEqual(len(blockers), 1)
        b = blockers[0]
        self.assertEqual(b["severity"], "medium")
        self.assertEqual(b["platform"], "all")


# ---------------------------------------------------------------------------
# 7. derive_blockers: unknown + zero evidence -> low
# ---------------------------------------------------------------------------
class DeriveUnknownZeroEvidenceTests(unittest.TestCase):
    def test_unknown_zero_evidence_is_low(self):
        caps = [_cap(id="mystery.cap", owner="core", status="unknown")]
        blockers = rb.derive_blockers(caps, [], [])
        self.assertEqual(len(blockers), 1)
        b = blockers[0]
        self.assertEqual(b["severity"], "low")
        self.assertEqual(b["evidence_status"], "unknown")


# ---------------------------------------------------------------------------
# 8. derive_blockers: implemented + at least one passing -> NOT a blocker
# ---------------------------------------------------------------------------
class DeriveImplementedPassingTests(unittest.TestCase):
    def test_implemented_with_passing_not_blocker(self):
        caps = [_cap(id="book.search", owner="core", status="implemented")]
        ev = [_ev(capability="book.search", status="pass", platform="ios")]
        blockers = rb.derive_blockers(caps, ev, [])
        self.assertEqual(blockers, [])


# ---------------------------------------------------------------------------
# 9. derive_blockers: implemented + zero evidence -> medium (unverified)
# ---------------------------------------------------------------------------
class DeriveImplementedZeroEvidenceTests(unittest.TestCase):
    def test_implemented_zero_evidence_is_medium(self):
        caps = [_cap(id="book.search", owner="core", status="implemented")]
        blockers = rb.derive_blockers(caps, [], [])
        self.assertEqual(len(blockers), 1)
        self.assertEqual(blockers[0]["severity"], "medium")


# ---------------------------------------------------------------------------
# 10. derive_blockers: empty capabilities -> empty blockers
# ---------------------------------------------------------------------------
class DeriveEmptyTests(unittest.TestCase):
    def test_empty_capabilities_empty_blockers(self):
        self.assertEqual(rb.derive_blockers([], [], []), [])


# ---------------------------------------------------------------------------
# 11. severity sort: blocker > high > medium > low, then by capability id
# ---------------------------------------------------------------------------
class DeriveSortTests(unittest.TestCase):
    def test_severity_sort_order_then_id(self):
        caps = [
            _cap(id="zzz.low", owner="core", status="unknown"),
            _cap(id="aaa.low", owner="core", status="unknown"),
            _cap(id="mmm.blocker", owner="core", status="missing"),
            _cap(id="bbb.blocker", owner="core", status="missing"),
            _cap(id="nnn.high", owner="host", status="partial"),
            _cap(id="qqq.medium", owner="host", status="implemented"),
        ]
        blockers = rb.derive_blockers(caps, [], [])
        severities = [b["severity"] for b in blockers]
        ids = [b["capability"] for b in blockers]
        self.assertEqual(
            severities,
            ["blocker", "blocker", "high", "medium", "low", "low"],
        )
        # Within each severity tier, ids are sorted ascending.
        self.assertEqual(
            ids,
            ["bbb.blocker", "mmm.blocker", "nnn.high", "qqq.medium",
             "aaa.low", "zzz.low"],
        )


# ---------------------------------------------------------------------------
# 12. platform: "all" when multiple platforms missing; specific when one
# ---------------------------------------------------------------------------
class DerivePlatformFieldTests(unittest.TestCase):
    def test_platform_specific_when_exactly_one_missing(self):
        caps = [_cap(id="book.search", owner="core", status="partial",
                     platforms=["ios", "android", "harmony"])]
        ev = [
            _ev(capability="book.search", status="pass", platform="ios"),
            _ev(capability="book.search", status="pass", platform="android"),
        ]
        blockers = rb.derive_blockers(caps, ev, [])
        self.assertEqual(len(blockers), 1)
        self.assertEqual(blockers[0]["platform"], "harmony")

    def test_platform_all_when_multiple_missing(self):
        caps = [_cap(id="book.search", owner="core", status="partial",
                     platforms=["ios", "android", "harmony"])]
        ev = [_ev(capability="book.search", status="pass", platform="ios")]
        blockers = rb.derive_blockers(caps, ev, [])
        self.assertEqual(blockers[0]["platform"], "all")


# ---------------------------------------------------------------------------
# 13. parse_feature_matrix: status/owner mapping on a sample markdown table
# ---------------------------------------------------------------------------
class ParseFeatureMatrixTests(unittest.TestCase):
    def test_owner_status_mapping(self):
        md = (
            "| Name | Owner | Status |\n"
            "|------|-------|--------|\n"
            "| book.search | Rust Core | 已完成 |\n"
            "| tls.socket | 平台负责 | Gap |\n"
            "| chapter.content | Rust Core | 部分完成 |\n"
        )
        rows = rb.parse_feature_matrix(md)
        by_id = {r["id"]: r for r in rows}
        self.assertEqual(by_id["book.search"]["owner"], "core")
        self.assertEqual(by_id["book.search"]["status"], "implemented")
        self.assertEqual(by_id["tls.socket"]["owner"], "host")
        self.assertEqual(by_id["tls.socket"]["status"], "missing")
        self.assertEqual(by_id["chapter.content"]["owner"], "core")
        self.assertEqual(by_id["chapter.content"]["status"], "partial")
        self.assertEqual(
            by_id["book.search"]["platforms"], ["ios", "android", "harmony"]
        )


# ---------------------------------------------------------------------------
# 14. parse_evidence_index: None -> [], obj with entries -> list
# ---------------------------------------------------------------------------
class ParseEvidenceIndexTests(unittest.TestCase):
    def test_none_returns_empty(self):
        self.assertEqual(rb.parse_evidence_index(None), [])

    def test_obj_with_entries_returns_list(self):
        obj = {"entries": [
            {"capability": "x", "status": "pass", "platform": "ios"},
            {"capability": "y", "status": "fail", "platform": "android"},
        ]}
        ev = rb.parse_evidence_index(obj)
        self.assertEqual(len(ev), 2)
        self.assertEqual(ev[0]["capability"], "x")

    def test_obj_without_entries_returns_empty(self):
        self.assertEqual(rb.parse_evidence_index({}), [])
        self.assertEqual(rb.parse_evidence_index({"entries": []}), [])


# ---------------------------------------------------------------------------
# 15. parse_migration_ledger: None -> [], sample md -> rows
# ---------------------------------------------------------------------------
class ParseMigrationLedgerTests(unittest.TestCase):
    def test_none_returns_empty(self):
        self.assertEqual(rb.parse_migration_ledger(None), [])

    def test_sample_md_rows(self):
        md = (
            "| Capability | Status | Notes |\n"
            "|------------|--------|-------|\n"
            "| book.search | done | ok |\n"
            "| tls.socket | pending | needs work |\n"
        )
        rows = rb.parse_migration_ledger(md)
        self.assertEqual(len(rows), 2)
        self.assertEqual(rows[0]["capability"], "book.search")
        self.assertEqual(rows[0]["status"], "done")
        self.assertEqual(rows[1]["capability"], "tls.socket")
        self.assertEqual(rows[1]["notes"], "needs work")


# ---------------------------------------------------------------------------
# 16. collect: tempdir with catalog + evidence index + FEATURE_MATRIX.md
# ---------------------------------------------------------------------------
class CollectWithCatalogAndEvidenceTests(unittest.TestCase):
    def test_collect_with_catalog_and_evidence(self):
        with tempfile.TemporaryDirectory() as d:
            _write(Path(d) / "reports" / "tooling" / "capability-catalog.json",
                   json.dumps({
                       "version": "capability-catalog/1",
                       "capabilities": [
                           {"id": "missing.cap", "owner": "core",
                            "status": "missing",
                            "platforms": ["ios", "android", "harmony"],
                            "evidence": ["FEATURE_MATRIX.md"]},
                           {"id": "implemented.cap", "owner": "core",
                            "status": "implemented",
                            "platforms": ["ios", "android", "harmony"],
                            "evidence": ["FEATURE_MATRIX.md"]},
                       ],
                   }))
            ev_path = Path(d) / "evidence-index.json"
            _write(ev_path, json.dumps({
                "version": "evidence-index/1",
                "entries": [
                    {"capability": "implemented.cap", "status": "pass",
                     "platform": "ios", "path": "ev1.json",
                     "source": "ev1.json", "tier": "smoke"},
                ],
            }))
            report = rb.collect(d, evidence_index_path=str(ev_path))
        blockers = report["blockers"]
        # missing.cap with zero evidence -> blocker.
        # implemented.cap with passing evidence -> not a blocker.
        self.assertEqual(len(blockers), 1)
        self.assertEqual(blockers[0]["capability"], "missing.cap")
        self.assertEqual(blockers[0]["severity"], "blocker")
        s = report["summary"]
        self.assertEqual(s["total"], 1)
        self.assertEqual(s["by_severity"]["blocker"], 1)


# ---------------------------------------------------------------------------
# 17. collect: tempdir with NO evidence index -> zero-evidence mode
# ---------------------------------------------------------------------------
class CollectNoEvidenceIndexTests(unittest.TestCase):
    def test_collect_no_evidence_index_zero_evidence_mode(self):
        with tempfile.TemporaryDirectory() as d:
            _write(Path(d) / "reports" / "tooling" / "capability-catalog.json",
                   json.dumps({
                       "capabilities": [
                           {"id": "missing.cap", "owner": "core",
                            "status": "missing",
                            "platforms": ["ios", "android", "harmony"],
                            "evidence": []},
                           {"id": "partial.cap", "owner": "host",
                            "status": "partial",
                            "platforms": ["ios", "android", "harmony"],
                            "evidence": []},
                       ],
                   }))
            report = rb.collect(d)
        blockers = report["blockers"]
        by_cap = {b["capability"]: b for b in blockers}
        self.assertIn("missing.cap", by_cap)
        self.assertEqual(by_cap["missing.cap"]["severity"], "blocker")
        self.assertIn("partial.cap", by_cap)
        self.assertEqual(by_cap["partial.cap"]["severity"], "high")


# ---------------------------------------------------------------------------
# 18. collect: tempdir with NO capability-catalog.json -> falls back to matrix
# ---------------------------------------------------------------------------
class CollectFallbackToMatrixTests(unittest.TestCase):
    def test_collect_no_catalog_falls_back_to_matrix(self):
        with tempfile.TemporaryDirectory() as d:
            _write(Path(d) / "FEATURE_MATRIX.md", (
                "| Name | Owner | Status |\n"
                "|------|-------|--------|\n"
                "| tls.socket | 平台负责 | Gap |\n"
            ))
            report = rb.collect(d)
        by_cap = {b["capability"]: b for b in report["blockers"]}
        self.assertIn("tls.socket", by_cap)
        self.assertEqual(by_cap["tls.socket"]["severity"], "blocker")


# ---------------------------------------------------------------------------
# 19. collect: empty tempdir (no inputs) -> empty blockers, summary zeros
# ---------------------------------------------------------------------------
class CollectEmptyTests(unittest.TestCase):
    def test_collect_empty_tempdir(self):
        with tempfile.TemporaryDirectory() as d:
            report = rb.collect(d)
        self.assertEqual(report["blockers"], [])
        self.assertEqual(report["summary"]["total"], 0)
        self.assertEqual(
            report["summary"]["by_severity"],
            {"blocker": 0, "high": 0, "medium": 0, "low": 0},
        )


# ---------------------------------------------------------------------------
# 20. CLI --pretty on a tempdir -> valid JSON exit 0; --out writes file
# ---------------------------------------------------------------------------
class CliPrettyOutTests(unittest.TestCase):
    def test_pretty_and_out(self):
        with tempfile.TemporaryDirectory() as d:
            _write(Path(d) / "reports" / "tooling" / "capability-catalog.json",
                   json.dumps({
                       "capabilities": [
                           {"id": "missing.cap", "owner": "core",
                            "status": "missing",
                            "platforms": ["ios", "android", "harmony"],
                            "evidence": []},
                       ],
                   }))
            out = Path(d) / "out.json"
            result = _run_cli([d, "--pretty", "--out", str(out)])
            self.assertEqual(result.returncode, 0)
            parsed = json.loads(result.stdout)
            self.assertEqual(parsed["version"], "release-blockers/1")
            self.assertIn("blockers", parsed)
            self.assertTrue(out.exists())
            file_parsed = json.loads(out.read_text(encoding="utf-8"))
            self.assertEqual(file_parsed["version"], "release-blockers/1")


# ---------------------------------------------------------------------------
# 21. CLI on missing root -> exit 2
# ---------------------------------------------------------------------------
class CliMissingRootTests(unittest.TestCase):
    def test_missing_root_exit2(self):
        result = _run_cli(["/no/such/path/exists/here/zzz"])
        self.assertEqual(result.returncode, 2)


# ---------------------------------------------------------------------------
# 22. summary by_severity + by_platform counts correct
# ---------------------------------------------------------------------------
class SummaryCountsTests(unittest.TestCase):
    def test_summary_by_severity_and_platform(self):
        with tempfile.TemporaryDirectory() as d:
            _write(Path(d) / "reports" / "tooling" / "capability-catalog.json",
                   json.dumps({
                       "capabilities": [
                           {"id": "a.missing", "owner": "core",
                            "status": "missing",
                            "platforms": ["ios", "android", "harmony"],
                            "evidence": []},
                           {"id": "b.unknown", "owner": "core",
                            "status": "unknown",
                            "platforms": ["ios", "android", "harmony"],
                            "evidence": []},
                           {"id": "c.host", "owner": "host",
                            "status": "implemented",
                            "platforms": ["ios", "android", "harmony"],
                            "evidence": []},
                       ],
                   }))
            report = rb.collect(d)
        s = report["summary"]
        self.assertEqual(s["total"], 3)
        self.assertEqual(s["by_severity"]["blocker"], 1)
        self.assertEqual(s["by_severity"]["high"], 0)
        self.assertEqual(s["by_severity"]["medium"], 1)
        self.assertEqual(s["by_severity"]["low"], 1)
        # All three blockers have zero evidence -> platform "all" for each.
        self.assertEqual(s["by_platform"]["all"], 3)


# ---------------------------------------------------------------------------
# Extra: blocker entry shape + top-level report shape
# ---------------------------------------------------------------------------
class BlockerEntryShapeTests(unittest.TestCase):
    def test_entry_shape(self):
        caps = [_cap(id="book.search", owner="core", status="missing")]
        blockers = rb.derive_blockers(caps, [], [])
        b = blockers[0]
        for key in ("id", "capability", "platform", "severity", "reason",
                    "sources", "evidence_status", "mitigation"):
            self.assertIn(key, b, "missing key %s in %s" % (key, b))
        self.assertEqual(b["id"], "rb-book.search")
        self.assertIsInstance(b["sources"], list)
        self.assertIsInstance(b["reason"], str)
        self.assertIsInstance(b["mitigation"], str)

    def test_sources_include_evidence_paths(self):
        caps = [_cap(id="book.search", owner="core", status="implemented",
                     evidence=["FEATURE_MATRIX.md"])]
        ev = [_ev(capability="book.search", status="fail", platform="ios",
                  path="reports/ios/smoke.json")]
        blockers = rb.derive_blockers(caps, ev, [])
        self.assertEqual(len(blockers), 1)
        sources = blockers[0]["sources"]
        self.assertIn("FEATURE_MATRIX.md", sources)
        self.assertIn("reports/ios/smoke.json", sources)


class ReportShapeTests(unittest.TestCase):
    def test_top_level_fields(self):
        with tempfile.TemporaryDirectory() as d:
            report = rb.collect(d)
        self.assertEqual(report["version"], "release-blockers/1")
        self.assertEqual(report["tool"], "release-blocker-register-generator")
        self.assertIn("generated_at", report)
        self.assertTrue(report["generated_at"].endswith("Z"))
        self.assertEqual(report["blockers"], [])
        self.assertEqual(
            report["summary"]["by_severity"],
            {"blocker": 0, "high": 0, "medium": 0, "low": 0},
        )


if __name__ == "__main__":
    unittest.main()
