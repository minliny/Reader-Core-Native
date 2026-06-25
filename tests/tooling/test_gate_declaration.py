"""Tests for the gate declaration checker (tools/gate-declaration).

Verifies that each platform (iOS/Android/HarmonyOS) declares its release
gates: simulator, device, corpus. Uses synthetic tempdirs and canned
declarations so it does not depend on real scripts/. Run with:

    python3 -m unittest tests.tooling.test_gate_declaration -v
"""

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime
from pathlib import Path

_HERE = Path(__file__).resolve().parent
_REPO_ROOT = _HERE.parent.parent
_MODULE_PATH = (
    _REPO_ROOT / "tools" / "gate-declaration" / "gate_declaration.py"
)
_CLI = str(_MODULE_PATH)
_WRAPPER = _REPO_ROOT / "scripts" / "gate-declaration.sh"


def _load_module():
    """Load gate_declaration.py by file path (dir name has a hyphen)."""
    if not _MODULE_PATH.exists():
        raise ImportError(
            "gate_declaration implementation not found at %s. "
            "TDD: write the implementation after the tests." % _MODULE_PATH
        )
    spec = importlib.util.spec_from_file_location(
        "gate_declaration", _MODULE_PATH
    )
    module = importlib.util.module_from_spec(spec)
    sys.modules["gate_declaration"] = module
    spec.loader.exec_module(module)
    return module


gd = _load_module()


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


def _gate(declared=True, source="src", notes=""):
    return {"declared": declared, "source": source, "notes": notes}


def _full_explicit_all_declared():
    """A gates.json with all 3 platforms x 3 gates declared true."""
    return {
        "version": "gate-declaration/1",
        "platforms": {
            plat: {
                gate: _gate(True, "scripts/build-%s.sh" % plat, "explicit")
                for gate in ("simulator", "device", "corpus")
            }
            for plat in ("ios", "android", "harmony")
        },
    }


# --------------------------------------------------------------------------- #
# 1-3. parse_explicit_decl
# --------------------------------------------------------------------------- #
class ParseExplicitDecl(unittest.TestCase):
    def test_valid_gates_json_returns_correct_structure(self):
        text = json.dumps({
            "version": "gate-declaration/1",
            "platforms": {
                "ios": {
                    "simulator": {
                        "declared": True, "source": "a.sh", "notes": "xcframework host sim"},
                    "device": {
                        "declared": False, "source": "", "notes": "missing"},
                    "corpus": {
                        "declared": True, "source": "b.sh", "notes": ""},
                },
            },
        })
        decl = gd.parse_explicit_decl(text)
        self.assertIsNotNone(decl)
        self.assertIn("ios", decl)
        self.assertTrue(decl["ios"]["simulator"]["declared"])
        self.assertEqual(decl["ios"]["simulator"]["source"], "a.sh")
        self.assertFalse(decl["ios"]["device"]["declared"])
        self.assertTrue(decl["ios"]["corpus"]["declared"])

    def test_malformed_json_returns_none(self):
        self.assertIsNone(gd.parse_explicit_decl("{not valid json"))
        self.assertIsNone(gd.parse_explicit_decl(""))

    def test_wrong_version_returns_none(self):
        text = json.dumps({"version": "gate-declaration/2", "platforms": {}})
        self.assertIsNone(gd.parse_explicit_decl(text))
        # Missing version entirely is also rejected.
        text2 = json.dumps({"platforms": {}})
        self.assertIsNone(gd.parse_explicit_decl(text2))


# --------------------------------------------------------------------------- #
# 4-5. heuristic_scan
# --------------------------------------------------------------------------- #
class HeuristicScan(unittest.TestCase):
    def test_ios_simulator_device_corpus_keywords_detected(self):
        # simulator via "simulator"
        decl = gd.heuristic_scan([("build-ios.sh", "build for simulator")])
        self.assertTrue(decl["ios"]["simulator"]["declared"])
        # simulator via "xcframework"
        decl = gd.heuristic_scan([("build-ios.sh", "create xcframework")])
        self.assertTrue(decl["ios"]["simulator"]["declared"])
        # device via "device"
        decl = gd.heuristic_scan([("build-ios.sh", "run on device")])
        self.assertTrue(decl["ios"]["device"]["declared"])
        # corpus via "corpus"
        decl = gd.heuristic_scan([("build-ios.sh", "test corpus")])
        self.assertTrue(decl["ios"]["corpus"]["declared"])
        # source + notes marked heuristic
        decl = gd.heuristic_scan(
            [("build-ios.sh", "simulator device corpus")]
        )
        self.assertEqual(decl["ios"]["simulator"]["source"], "build-ios.sh")
        self.assertEqual(decl["ios"]["simulator"]["notes"], "heuristic")

    def test_ohos_script_maps_to_harmony(self):
        decl = gd.heuristic_scan(
            [("build-ohos.sh", "simulator device corpus")]
        )
        self.assertIn("harmony", decl)
        self.assertTrue(decl["harmony"]["simulator"]["declared"])
        self.assertTrue(decl["harmony"]["device"]["declared"])
        self.assertTrue(decl["harmony"]["corpus"]["declared"])
        self.assertNotIn("ohos", decl)

    def test_non_platform_script_is_ignored(self):
        decl = gd.heuristic_scan(
            [("build-env-doctor.sh", "simulator device corpus")]
        )
        for plat in ("ios", "android", "harmony"):
            self.assertNotIn(plat, decl)


# --------------------------------------------------------------------------- #
# 6. merge
# --------------------------------------------------------------------------- #
class Merge(unittest.TestCase):
    def test_explicit_overrides_heuristic_and_fills_gaps(self):
        explicit = {
            "ios": {
                "simulator": _gate(True, "explicit-src", "explicit")}
        }
        heuristic = {
            "ios": {
                "simulator": _gate(True, "heuristic-src", "heuristic"),
                "device": _gate(True, "heuristic-dev", "heuristic"),
            }
        }
        merged = gd.merge(explicit, heuristic)
        # explicit wins for ios.simulator
        self.assertEqual(merged["ios"]["simulator"]["source"], "explicit-src")
        # heuristic fills ios.device
        self.assertTrue(merged["ios"]["device"]["declared"])
        self.assertEqual(merged["ios"]["device"]["source"], "heuristic-dev")
        # ios.corpus missing from both -> default
        self.assertFalse(merged["ios"]["corpus"]["declared"])
        self.assertEqual(merged["ios"]["corpus"]["source"], "")
        self.assertEqual(merged["ios"]["corpus"]["notes"], "missing")

    def test_merge_with_none_explicit_uses_heuristic_and_defaults(self):
        heuristic = {"ios": {"simulator": _gate(True, "h", "heuristic")}}
        merged = gd.merge(None, heuristic)
        self.assertTrue(merged["ios"]["simulator"]["declared"])
        self.assertFalse(merged["ios"]["device"]["declared"])
        for plat in ("ios", "android", "harmony"):
            self.assertIn(plat, merged)

    def test_merge_all_three_platforms_always_present(self):
        merged = gd.merge(None, None)
        for plat in ("ios", "android", "harmony"):
            self.assertIn(plat, merged)
            for gate in ("simulator", "device", "corpus"):
                self.assertFalse(merged[plat][gate]["declared"])
                self.assertEqual(merged[plat][gate]["notes"], "missing")


# --------------------------------------------------------------------------- #
# 7-8. evaluate
# --------------------------------------------------------------------------- #
class Evaluate(unittest.TestCase):
    def _all_declared(self):
        return {
            plat: {
                gate: _gate(True, "s", "")
                for gate in ("simulator", "device", "corpus")
            }
            for plat in ("ios", "android", "harmony")
        }

    def test_platform_all_three_gates_fail_closed_true_no_issues(self):
        ev = gd.evaluate(self._all_declared())
        ios = next(p for p in ev["platforms"] if p["platform"] == "ios")
        self.assertTrue(ios["fail_closed"])
        self.assertEqual(ios["issues"], [])
        self.assertTrue(ev["fail_closed"])

    def test_platform_missing_device_fail_closed_false_with_issue(self):
        decl = self._all_declared()
        decl["ios"]["device"] = _gate(False, "", "missing")
        ev = gd.evaluate(decl)
        ios = next(p for p in ev["platforms"] if p["platform"] == "ios")
        self.assertFalse(ios["fail_closed"])
        self.assertIn("missing device gate", ios["issues"])
        # overall false because one platform fails
        self.assertFalse(ev["fail_closed"])

    def test_overall_fail_closed_true_only_if_all_platforms_pass(self):
        decl = self._all_declared()
        decl["harmony"]["corpus"] = _gate(False, "", "missing")
        ev = gd.evaluate(decl)
        harmony = next(
            p for p in ev["platforms"] if p["platform"] == "harmony"
        )
        self.assertFalse(harmony["fail_closed"])
        self.assertIn("missing corpus gate", harmony["issues"])
        self.assertFalse(ev["fail_closed"])

    def test_evaluate_includes_all_three_platforms_ordered(self):
        ev = gd.evaluate({})
        plats = [p["platform"] for p in ev["platforms"]]
        self.assertEqual(plats, ["ios", "android", "harmony"])


# --------------------------------------------------------------------------- #
# 9-12, 16. collect
# --------------------------------------------------------------------------- #
class Collect(unittest.TestCase):
    def test_explicit_all_declared_fail_closed_true(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(
                root / "docs" / "ci-gates" / "gates.json",
                json.dumps(_full_explicit_all_declared()),
            )
            report = gd.collect(root)
        self.assertTrue(report["fail_closed"])
        self.assertEqual(report["version"], "gate-declaration-report/1")
        self.assertEqual(report["tool"], "gate-declaration-checker")
        self.assertEqual(report["summary"]["missing_gates"], 0)
        self.assertEqual(report["summary"]["fail_closed_count"], 3)
        self.assertEqual(report["summary"]["platforms"], 3)

    def test_explicit_missing_ios_device_fail_closed_false(self):
        data = _full_explicit_all_declared()
        data["platforms"]["ios"]["device"] = _gate(False, "", "not yet wired")
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(
                root / "docs" / "ci-gates" / "gates.json", json.dumps(data)
            )
            report = gd.collect(root)
        self.assertFalse(report["fail_closed"])
        ios = next(p for p in report["platforms"] if p["platform"] == "ios")
        self.assertFalse(ios["fail_closed"])
        self.assertIn("missing device gate", ios["issues"])
        self.assertEqual(report["summary"]["missing_gates"], 1)

    def test_no_gates_json_heuristic_covers_all_platforms_and_gates(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            content = "simulator device corpus"
            _write(root / "scripts" / "build-ios.sh", content)
            _write(root / "scripts" / "build-android.sh", content)
            _write(root / "scripts" / "build-harmony.sh", content)
            report = gd.collect(root)
        self.assertTrue(report["fail_closed"], report)
        self.assertEqual(report["summary"]["missing_gates"], 0)
        self.assertEqual(report["summary"]["fail_closed_count"], 3)
        # source is the script path, notes heuristic
        ios = next(p for p in report["platforms"] if p["platform"] == "ios")
        self.assertEqual(ios["gates"]["simulator"]["notes"], "heuristic")

    def test_empty_tempdir_all_missing_fail_closed_false_nine(self):
        with tempfile.TemporaryDirectory() as d:
            report = gd.collect(Path(d))
        self.assertFalse(report["fail_closed"])
        plats = [p["platform"] for p in report["platforms"]]
        self.assertEqual(plats, ["ios", "android", "harmony"])
        self.assertEqual(report["summary"]["missing_gates"], 9)
        self.assertEqual(report["summary"]["fail_closed_count"], 0)
        for p in report["platforms"]:
            self.assertEqual(len(p["issues"]), 3)

    def test_summary_missing_gates_and_fail_closed_count_correct(self):
        # all declared -> 0 missing, 3 fail_closed
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(
                root / "docs" / "ci-gates" / "gates.json",
                json.dumps(_full_explicit_all_declared()),
            )
            report = gd.collect(root)
        self.assertEqual(
            report["summary"],
            {"platforms": 3, "missing_gates": 0, "fail_closed_count": 3},
        )
        # empty -> 9 missing, 0 fail_closed
        with tempfile.TemporaryDirectory() as d:
            report = gd.collect(Path(d))
        self.assertEqual(
            report["summary"],
            {"platforms": 3, "missing_gates": 9, "fail_closed_count": 0},
        )

    def test_generated_at_is_iso8601(self):
        with tempfile.TemporaryDirectory() as d:
            report = gd.collect(Path(d))
        datetime.fromisoformat(report["generated_at"])


# --------------------------------------------------------------------------- #
# 13-15. CLI
# --------------------------------------------------------------------------- #
class CLI(unittest.TestCase):
    def test_pretty_on_all_declared_tempdir_exits_zero_valid_json(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(
                root / "docs" / "ci-gates" / "gates.json",
                json.dumps(_full_explicit_all_declared()),
            )
            result = _run_cli([root, "--pretty"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(result.stdout.endswith("\n"))
        data = json.loads(result.stdout)
        self.assertTrue(data["fail_closed"])
        # --pretty -> indent=2 + sorted keys
        self.assertIn("\n  ", result.stdout)
        top_keys = list(data.keys())
        self.assertEqual(top_keys, sorted(top_keys))

    def test_missing_gate_tempdir_exits_one(self):
        data = _full_explicit_all_declared()
        data["platforms"]["ios"]["device"] = _gate(False, "", "not yet wired")
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(
                root / "docs" / "ci-gates" / "gates.json", json.dumps(data)
            )
            result = _run_cli([root])
        self.assertEqual(result.returncode, 1, result.stderr)
        data = json.loads(result.stdout)
        self.assertFalse(data["fail_closed"])

    def test_missing_root_exits_two(self):
        result = _run_cli(["/nonexistent-path-xyz-abc-12345"])
        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stdout, "")


# --------------------------------------------------------------------------- #
# Wrapper smoke test (scripts/gate-declaration.sh)
# --------------------------------------------------------------------------- #
class Wrapper(unittest.TestCase):
    def test_wrapper_execs_python_tool_and_emits_json(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(
                root / "docs" / "ci-gates" / "gates.json",
                json.dumps(_full_explicit_all_declared()),
            )
            result = subprocess.run(
                ["bash", str(_WRAPPER), root],
                capture_output=True,
                text=True,
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads(result.stdout)
        self.assertEqual(data["tool"], "gate-declaration-checker")


# --------------------------------------------------------------------------- #
# Edge cases: fail-closed + fallback semantics
# --------------------------------------------------------------------------- #
class EdgeCases(unittest.TestCase):
    def test_parse_non_object_json_returns_none(self):
        self.assertIsNone(gd.parse_explicit_decl("[1, 2, 3]"))
        self.assertIsNone(gd.parse_explicit_decl('"hello"'))
        self.assertIsNone(gd.parse_explicit_decl("42"))

    def test_parse_valid_version_missing_platforms_returns_empty(self):
        text = json.dumps({"version": "gate-declaration/1"})
        decl = gd.parse_explicit_decl(text)
        self.assertEqual(decl, {})

    def test_heuristic_scan_empty_scripts_returns_empty(self):
        self.assertEqual(gd.heuristic_scan([]), {})

    def test_explicit_declared_false_not_overridden_by_heuristic(self):
        # Explicit says ios.device declared=false; heuristic says true.
        # Explicit must win -> stays false (FAIL CLOSED, not rescued).
        explicit = {"ios": {"device": _gate(False, "", "not yet wired")}}
        heuristic = {
            "ios": {"device": _gate(True, "heuristic.sh", "heuristic")}
        }
        merged = gd.merge(explicit, heuristic)
        self.assertFalse(merged["ios"]["device"]["declared"])
        self.assertEqual(merged["ios"]["device"]["notes"], "not yet wired")

    def test_collect_malformed_gates_json_falls_back_to_heuristic(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(
                root / "docs" / "ci-gates" / "gates.json", "{not valid json"
            )
            content = "simulator device corpus"
            _write(root / "scripts" / "build-ios.sh", content)
            _write(root / "scripts" / "build-android.sh", content)
            _write(root / "scripts" / "build-harmony.sh", content)
            report = gd.collect(root)
        self.assertTrue(report["fail_closed"], report)
        self.assertEqual(report["summary"]["missing_gates"], 0)

    def test_collect_wrong_version_gates_json_falls_back_to_heuristic(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(
                root / "docs" / "ci-gates" / "gates.json",
                json.dumps(
                    {"version": "gate-declaration/2", "platforms": {}}
                ),
            )
            content = "simulator device corpus"
            _write(root / "scripts" / "build-ios.sh", content)
            _write(root / "scripts" / "build-android.sh", content)
            _write(root / "scripts" / "build-harmony.sh", content)
            report = gd.collect(root)
        self.assertTrue(report["fail_closed"], report)

    def test_evaluate_sparse_decl_lists_all_three_issues_in_order(self):
        ev = gd.evaluate({})
        ios = next(p for p in ev["platforms"] if p["platform"] == "ios")
        self.assertFalse(ios["fail_closed"])
        self.assertEqual(
            ios["issues"],
            ["missing simulator gate", "missing device gate", "missing corpus gate"],
        )


if __name__ == "__main__":
    unittest.main()
