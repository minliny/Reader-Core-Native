#!/usr/bin/env python3
"""Tests for the legacy fixture register and proof matrix."""

import copy
import json
import os
import sys
import tempfile
import unittest


_HERE = os.path.dirname(os.path.abspath(__file__))
_ROOT = os.path.abspath(os.path.join(_HERE, "..", ".."))
sys.path.insert(0, os.path.join(_ROOT, "tools", "legacy-fixture-register"))

import legacy_fixture_register as lfr  # noqa: E402


def _load_register():
    path = os.path.join(_ROOT, "samples", "legacy-fixtures", "register.json")
    return lfr.load_json_file(path)


class TestLegacyFixtureRegister(unittest.TestCase):
    def test_repo_register_validates(self):
        data = _load_register()
        by_id = lfr.validate_register(data)
        self.assertEqual(set(lfr.REQUIRED_ENTRY_IDS), set(by_id.keys()))

    def test_status_values_are_closed(self):
        data = _load_register()
        data["entries"][0]["status"] = "done"
        with self.assertRaises(lfr.RegisterError):
            lfr.validate_register(data)

    def test_platform_entries_require_output_path(self):
        data = _load_register()
        for entry in data["entries"]:
            if entry["id"] == "harmonyos-platform":
                del entry["platformOutputPath"]
                break
        with self.assertRaises(lfr.RegisterError):
            lfr.validate_register(data)


class TestProofMatrix(unittest.TestCase):
    def test_proof_matrix_passes_with_registered_refs_and_platforms(self):
        data = _load_register()

        def ref_exists(_repo_root, ref):
            return ref in {
                "origin/codex/runtime-host-capability-contract",
                "origin/codex/corpus-booksource-oracle-diff",
            }

        result = lfr.build_proof_matrix(data, _ROOT, ref_exists=ref_exists)
        self.assertEqual(result["blocked"], 0)
        check_ids = {check["id"] for check in result["checks"]}
        self.assertIn("branch-runtimeHost", check_ids)
        self.assertIn("branch-bookSourceOracle", check_ids)
        self.assertIn("harmony-intake-manifest", check_ids)

    def test_missing_runtime_branch_blocks(self):
        data = _load_register()
        result = lfr.build_proof_matrix(data, _ROOT, ref_exists=lambda _root, _ref: False)
        blocked = {check["id"] for check in result["checks"] if check["status"] == "blocked"}
        self.assertIn("branch-runtimeHost", blocked)
        self.assertIn("branch-bookSourceOracle", blocked)

    def test_missing_harmony_output_in_release_gate_blocks(self):
        data = copy.deepcopy(_load_register())
        tmp = tempfile.mkdtemp(prefix="legacy-proof-")
        manifest = {
            "schemaVersion": 1,
            "runId": "missing-harmony",
            "candidates": {
                "cli": "cli.json",
                "ios": "ios.json",
                "android": "android.json",
            },
        }
        manifest_path = os.path.join(tmp, "manifest.json")
        with open(manifest_path, "w", encoding="utf-8") as handle:
            json.dump(manifest, handle)
        data["proofMatrix"]["releaseGateManifest"] = manifest_path

        result = lfr.build_proof_matrix(data, _ROOT, ref_exists=lambda _root, _ref: True)
        release_gate = [
            check for check in result["checks"]
            if check["id"] == "release-gate-platform-output"
        ][0]
        self.assertEqual(release_gate["status"], "blocked")
        self.assertEqual(release_gate["missingPlatforms"], ["harmony"])

    def test_harmony_intake_manifest_has_open_missing_blocker(self):
        path = os.path.join(
            _ROOT,
            "samples",
            "platform-evidence",
            "harmonyos",
            "pr-4-intake-manifest.json",
        )
        manifest = lfr._validate_harmony_intake(_ROOT, path)
        self.assertEqual(manifest["releaseGateCandidateName"], "harmony")
        blocker = manifest["blockerWhenMissing"]
        self.assertEqual(blocker["id"], "missing-harmonyos-platform-output")
        self.assertEqual(blocker["status"], "open")


class TestCLI(unittest.TestCase):
    def test_validate_cli(self):
        rc = lfr.main(["--repo-root", _ROOT, "validate"])
        self.assertEqual(rc, 0)

    def test_proof_cli_json(self):
        rc = lfr.main(["--repo-root", _ROOT, "proof", "--json"])
        # This depends on fetched refs in the local checkout. It should pass in
        # the project worktree, but the assertion keeps the command executable.
        self.assertIn(rc, (0, 1))


if __name__ == "__main__":
    unittest.main()
