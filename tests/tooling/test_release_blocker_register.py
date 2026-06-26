#!/usr/bin/env python3
"""Tests for the release blocker register.

Run with:
    python3 -m unittest tests.tooling.test_release_blocker_register -v
or:
    python3 tests/tooling/test_release_blocker_register.py
"""

import json
import os
import sys
import tempfile
import unittest

_HERE = os.path.dirname(os.path.abspath(__file__))
_ROOT = os.path.abspath(os.path.join(_HERE, "..", ".."))
sys.path.insert(0, os.path.join(_ROOT, "tools", "release-blocker-register"))
sys.path.insert(0, os.path.join(_ROOT, "tools", "cross-platform-diff"))

import release_blocker_register as rbr  # noqa: E402
import cross_platform_diff as cpd  # noqa: E402


FOUR_PLATFORM_CANDIDATES = ("cli", "ios", "android", "harmony")


def _fixture_diff_inputs(name):
    fixture = os.path.join(_ROOT, "samples", "corpus-release-gate", name)
    canonical = os.path.join(fixture, "canonical-result.json")
    candidates = [
        (
            platform,
            os.path.join(fixture, "candidates", "{0}-result.json".format(platform)),
        )
        for platform in FOUR_PLATFORM_CANDIDATES
    ]
    return canonical, candidates


def _diff_result(matching=None, mismatching=("android",)):
    """Build a minimal diff-result with the requested candidate outcomes."""
    if matching is None:
        matching = tuple(
            platform for platform in FOUR_PLATFORM_CANDIDATES
            if platform not in mismatching
        )
    candidates = {}
    for name in matching:
        candidates[name] = {
            "match": True,
            "total": 0,
            "sha256": "c" * 64,
            "canonicalizedSha256": "d" * 64,
            "differences": [],
        }
    diffs = []
    for name in mismatching:
        candidates[name] = {
            "match": False,
            "total": 1,
            "sha256": "a" * 64,
            "canonicalizedSha256": "b" * 64,
            "differences": [
                {"path": "title", "kind": "value-mismatch",
                 "canonical": "x", "candidate": "y"},
            ],
        }
        diffs.append(name)
    return {
        "schemaVersion": 1,
        "tool": "cross-platform-diff",
        "version": "1.0",
        "canonical": {
            "path": "/tmp/canon.json",
            "sha256": "c" * 64,
            "canonicalizedSha256": "d" * 64,
        },
        "candidates": candidates,
        "summary": {n: {"match": c["match"], "total": c["total"]}
                    for n, c in candidates.items()},
        "match": not diffs,
        "total": len(diffs),
    }


class TestPathPolicy(unittest.TestCase):
    def test_default_under_private_tmp(self):
        path = rbr.resolve_register_path(None)
        self.assertTrue(rbr._is_under(path, rbr.PRIVATE_TMP))

    def test_user_specified_under_documents_rejected(self):
        docs = rbr._documents_dir()
        target = os.path.join(docs, "blocker.json")
        with self.assertRaises(rbr.RegisterError):
            rbr.resolve_register_path(target)

    def test_user_specified_tmp_allowed(self):
        target = os.path.join(rbr.PRIVATE_TMP, "my-register.json")
        self.assertEqual(rbr.resolve_register_path(target), target)


class TestBlockersFromDiff(unittest.TestCase):
    def test_only_mismatching_candidates_produce_blockers(self):
        diff = _diff_result(
            matching=("cli", "ios"),
            mismatching=("android", "harmony"),
        )
        entries = rbr.blockers_from_diff(diff, run_id="run-1", severity="high")
        platforms = sorted(e["platform"] for e in entries)
        self.assertEqual(platforms, ["android", "harmony"])
        for e in entries:
            self.assertEqual(e["status"], rbr.STATUS_OPEN)
            self.assertEqual(e["severity"], "high")
            self.assertEqual(e["runId"], "run-1")
            self.assertEqual(e["fieldPath"], "title")
            self.assertEqual(e["canonicalSha256"], "c" * 64)
            self.assertEqual(e["candidateSha256"], "a" * 64)
            self.assertEqual(e["canonicalizedSha256"], "d" * 64)
            self.assertEqual(e["candidateCanonicalizedSha256"], "b" * 64)

    def test_all_matching_yields_no_blockers(self):
        diff = _diff_result(matching=FOUR_PLATFORM_CANDIDATES, mismatching=())
        self.assertEqual(rbr.blockers_from_diff(diff, "run-1", "medium"), [])

    def test_missing_required_candidate_yields_platform_blocker(self):
        diff = _diff_result(matching=("ios",), mismatching=())

        entries = rbr.blockers_from_diff(diff, run_id="run-1", severity="high")

        self.assertEqual(
            [(e["platform"], e["kind"], e["fieldPath"]) for e in entries],
            [
                ("android", "missing-platform-candidate", "<candidate>"),
                ("cli", "missing-platform-candidate", "<candidate>"),
                ("harmony", "missing-platform-candidate", "<candidate>"),
            ],
        )
        for entry in entries:
            self.assertEqual(
                entry["reason"],
                "required four-platform candidate missing from diff-result",
            )
            self.assertEqual(entry["canonicalSha256"], "c" * 64)
            self.assertEqual(entry["candidateSha256"], "")
            self.assertEqual(entry["canonicalizedSha256"], "d" * 64)
            self.assertEqual(entry["candidateCanonicalizedSha256"], "")

    def test_rejects_non_object_diff(self):
        with self.assertRaises(rbr.RegisterError):
            rbr.blockers_from_diff([], "run-1", "medium")
        with self.assertRaises(rbr.RegisterError):
            rbr.blockers_from_diff({}, "run-1", "medium")

    def test_four_platform_mismatch_fixture_yields_android_blocker(self):
        canonical, candidates = _fixture_diff_inputs("four-platform-mismatch")
        diff = cpd.build_diff_result(canonical, candidates)

        entries = rbr.blockers_from_diff(
            diff,
            run_id="fixture-four-platform-mismatch",
            severity="high",
        )

        self.assertEqual(len(entries), 1)
        blocker = entries[0]
        self.assertEqual(blocker["platform"], "android")
        self.assertEqual(blocker["fieldPath"], "results[1].name")
        self.assertEqual(blocker["kind"], "value-mismatch")
        self.assertEqual(blocker["severity"], "high")
        self.assertEqual(blocker["status"], rbr.STATUS_OPEN)
        self.assertEqual(len(blocker["canonicalizedSha256"]), 64)
        self.assertEqual(len(blocker["candidateCanonicalizedSha256"]), 64)


class TestRegisterLifecycle(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="rbr-test-")
        self.path = os.path.join(self.tmp, "register.json")

    def _load(self):
        return rbr.load_register(self.path)

    def test_add_from_diff_assigns_sequential_ids(self):
        reg = self._load()
        diff = _diff_result(mismatching=("android", "harmony"))
        added = rbr.add_blockers_from_diff(reg, diff, "run-1", "medium")
        self.assertEqual([e["id"] for e in added], ["BLK-0001", "BLK-0002"])
        self.assertEqual(reg["nextId"], 3)

    def test_add_from_diff_is_idempotent(self):
        reg = self._load()
        diff = _diff_result(mismatching=("android",))
        rbr.add_blockers_from_diff(reg, diff, "run-1", "medium")
        second = rbr.add_blockers_from_diff(reg, diff, "run-1", "medium")
        self.assertEqual(second, [])
        self.assertEqual(len(reg["blockers"]), 1)

    def test_different_run_id_not_deduped(self):
        reg = self._load()
        diff = _diff_result(mismatching=("android",))
        rbr.add_blockers_from_diff(reg, diff, "run-1", "medium")
        added = rbr.add_blockers_from_diff(reg, diff, "run-2", "medium")
        self.assertEqual(len(added), 1)
        self.assertEqual(added[0]["runId"], "run-2")

    def test_waive_requires_rationale(self):
        reg = self._load()
        diff = _diff_result(mismatching=("android",))
        added = rbr.add_blockers_from_diff(reg, diff, "run-1", "medium")
        entry = added[0]
        with self.assertRaises(rbr.RegisterError):
            rbr.waive_blocker(entry, "   ", "tester")
        rbr.waive_blocker(entry, "accepted drift", "tester")
        self.assertEqual(entry["status"], rbr.STATUS_WAIVED)
        self.assertEqual(entry["waiver"]["rationale"], "accepted drift")
        self.assertIsNotNone(entry["resolvedAt"])

    def test_cannot_waive_closed(self):
        reg = self._load()
        diff = _diff_result(mismatching=("android",))
        entry = rbr.add_blockers_from_diff(reg, diff, "run-1", "medium")[0]
        rbr.close_blocker(entry)
        with self.assertRaises(rbr.RegisterError):
            rbr.waive_blocker(entry, "late waiver", "tester")

    def test_close_then_reopen(self):
        reg = self._load()
        diff = _diff_result(mismatching=("android",))
        entry = rbr.add_blockers_from_diff(reg, diff, "run-1", "medium")[0]
        rbr.close_blocker(entry)
        self.assertEqual(entry["status"], rbr.STATUS_CLOSED)
        rbr.reopen_blocker(entry)
        self.assertEqual(entry["status"], rbr.STATUS_OPEN)
        self.assertIsNone(entry["resolvedAt"])
        self.assertIsNone(entry["waiver"])

    def test_persistence_roundtrip(self):
        reg = self._load()
        diff = _diff_result(mismatching=("android",))
        rbr.add_blockers_from_diff(reg, diff, "run-1", "medium")
        rbr.save_register(self.path, reg)
        reloaded = self._load()
        self.assertEqual(len(reloaded["blockers"]), 1)
        self.assertEqual(reloaded["blockers"][0]["id"], "BLK-0001")
        self.assertEqual(reloaded["nextId"], 2)

    def test_compute_next_id_from_unmanaged_register(self):
        # A register file written without nextId still recovers id sequencing.
        raw = {
            "schemaVersion": 1, "tool": rbr.TOOL_NAME,
            "blockers": [{"id": "BLK-0005"}, {"id": "BLK-0007"}],
        }
        with open(self.path, "w", encoding="utf-8") as handle:
            json.dump(raw, handle)
        reg = self._load()
        self.assertEqual(reg["nextId"], 8)


class TestFilteringAndGate(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="rbr-gate-")
        self.path = os.path.join(self.tmp, "register.json")

    def _seed(self):
        reg = rbr.load_register(self.path)
        diff = _diff_result(mismatching=("android", "harmony"))
        rbr.add_blockers_from_diff(reg, diff, "run-1", "medium")
        # waive the harmony blocker
        for e in reg["blockers"]:
            if e["platform"] == "harmony":
                rbr.waive_blocker(e, "platform-specific", "tester")
        rbr.save_register(self.path, reg)
        return reg

    def test_filter_by_status(self):
        reg = self._seed()
        open_only = rbr.filter_blockers(reg["blockers"], status=rbr.STATUS_OPEN)
        self.assertEqual([e["platform"] for e in open_only], ["android"])

    def test_gate_counts_open_only(self):
        reg = self._seed()
        count, breakdown = rbr.gate_evaluate(reg)
        self.assertEqual(count, 1)  # harmony waived, android open
        self.assertEqual(breakdown, {"android": 1})

    def test_gate_scoped_to_run_id(self):
        reg = self._seed()
        count, _ = rbr.gate_evaluate(reg, run_id="other-run")
        self.assertEqual(count, 0)

    def test_gate_zero_when_all_resolved(self):
        reg = self._seed()
        for e in reg["blockers"]:
            rbr.close_blocker(e)
        count, breakdown = rbr.gate_evaluate(reg)
        self.assertEqual(count, 0)
        self.assertEqual(breakdown, {})


class TestCLI(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="rbr-cli-")
        self.path = os.path.join(self.tmp, "register.json")
        self.diff_path = os.path.join(self.tmp, "diff-result.json")
        with open(self.diff_path, "w", encoding="utf-8") as handle:
            json.dump(_diff_result(mismatching=("android",)), handle)

    def _run(self, *cli_args):
        return rbr.main(["--register", self.path] + list(cli_args))

    def test_add_from_diff_then_list(self):
        rc = self._run("add-from-diff", self.diff_path, "--run-id", "run-1")
        self.assertEqual(rc, 0)
        rc = self._run("list", "--status", "open")
        self.assertEqual(rc, 0)

    def test_waive_via_cli(self):
        self._run("add-from-diff", self.diff_path, "--run-id", "run-1")
        rc = self._run("waive", "BLK-0001", "--rationale", "ok", "--by", "me")
        self.assertEqual(rc, 0)
        reg = rbr.load_register(self.path)
        self.assertEqual(reg["blockers"][0]["status"], rbr.STATUS_WAIVED)

    def test_gate_exit_code_blocked(self):
        self._run("add-from-diff", self.diff_path, "--run-id", "run-1")
        rc = self._run("gate")
        self.assertEqual(rc, 1)  # open blocker present

    def test_gate_blocks_partial_platform_diff(self):
        with open(self.diff_path, "w", encoding="utf-8") as handle:
            json.dump(_diff_result(matching=("ios",), mismatching=()), handle)

        self._run("add-from-diff", self.diff_path, "--run-id", "run-1")
        rc = self._run("gate")

        self.assertEqual(rc, 1)
        reg = rbr.load_register(self.path)
        kinds = sorted(entry["kind"] for entry in reg["blockers"])
        self.assertEqual(kinds, [
            "missing-platform-candidate",
            "missing-platform-candidate",
            "missing-platform-candidate",
        ])

    def test_gate_exit_code_clear_after_close(self):
        self._run("add-from-diff", self.diff_path, "--run-id", "run-1")
        self._run("close", "BLK-0001")
        rc = self._run("gate")
        self.assertEqual(rc, 0)  # no open blockers

    def test_register_under_documents_rejected(self):
        docs = rbr._documents_dir()
        bad = os.path.join(docs, "should-not-write.json")
        rc = rbr.main(["--register", bad, "list"])
        self.assertEqual(rc, 2)

    def test_show_missing_id(self):
        rc = self._run("show", "BLK-9999")
        self.assertEqual(rc, 2)


if __name__ == "__main__":
    unittest.main()
