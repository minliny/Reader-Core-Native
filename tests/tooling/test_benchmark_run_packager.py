"""Tests for the benchmark run packager tool.

Run from the repository root:
    python3 -m unittest tests.tooling.test_benchmark_run_packager
or:
    python3 tests/tooling/test_benchmark_run_packager.py
"""

import json
import hashlib
import os
import shutil
import sys
import tempfile
import unittest
import zipfile

# Make the tool importable without a package install.
_TOOL_DIR = os.path.join(
    os.path.dirname(__file__), "..", "..", "tools", "benchmark-run-packager"
)
sys.path.insert(0, _TOOL_DIR)

import benchmark_run_packager as brp  # noqa: E402


PRIVATE_TMP = "/private/tmp"
PLATFORMS = ("android", "cli", "harmony", "ios")
CORE_IDENTITY = {
    "businessKernel": "reader-core-native-rust",
    "coreCommit": "090b96f",
    "abiVersion": 1,
    "protocolVersion": 1,
}


def _write_json(path, value):
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(value, handle)


def _sha256_text(value):
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def _sha256_content(value):
    if isinstance(value, (dict, list)):
        return _sha256_text(json.dumps(value))
    return _sha256_text(value)


def _make_run_dir(files):
    """Create a temp run directory under /private/tmp.

    ``files`` maps a relative path to either a string (raw contents) or a
    Python object (written as JSON). Returns the absolute path of the run dir.
    """
    run_dir = tempfile.mkdtemp(prefix="brp-run-", dir=PRIVATE_TMP)
    for rel, content in files.items():
        full = os.path.join(run_dir, rel)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        if isinstance(content, (dict, list)):
            _write_json(full, content)
        else:
            with open(full, "w", encoding="utf-8") as handle:
                handle.write(content)
    return run_dir


def _complete_files(diff_total=0):
    """Return a files dict for a complete, valid run directory."""
    return {
        "manifest.json": {"runId": "run-42", "timestamp": "2026-06-25T10:00:00Z"},
        "platform-result.json": {"book": {"title": "A"}},
        "canonical-result.json": {"book": {"title": "A"}},
        "diff-result.json": {
            "tool": "cross-platform-diff",
            "summary": {
                "android": {
                    "missing": 0, "extra": 0, "changed": diff_total,
                    "type_mismatch": 0, "total": diff_total,
                },
            },
        },
    }


def _collector_manifest(hashes=None):
    hashes = hashes or {}
    platform_runs = {
        platform: dict(CORE_IDENTITY)
        for platform in PLATFORMS
    }
    candidates = {
        platform: {
            "rawPath": "raw/{0}-result.json".format(platform),
            "canonicalizedPath": "candidates/{0}-result.json".format(platform),
            "sourceSha256": hashes.get(
                "raw/{0}-result.json".format(platform),
                platform[0] * 64,
            ),
            "rawSha256": hashes.get(
                "raw/{0}-result.json".format(platform),
                platform[0] * 64,
            ),
            "canonicalizedSha256": platform[-1] * 64,
            "canonicalizedFileSha256": hashes.get(
                "candidates/{0}-result.json".format(platform),
                platform[1] * 64,
            ),
        }
        for platform in PLATFORMS
    }
    return {
        "runId": "collector-run",
        "scenario": "four-platform-search",
        "sourceManifest": {
            "coreIdentity": dict(CORE_IDENTITY),
            "platformRuns": platform_runs,
        },
        "sourceManifestFile": {
            "packagePath": "raw/source-manifest.json",
            "sourceSha256": hashes.get("raw/source-manifest.json", "m" * 64),
            "packageSha256": hashes.get("raw/source-manifest.json", "n" * 64),
        },
        "input": {
            "packagePath": "input.json",
            "packageSha256": hashes.get("input.json", "i" * 64),
        },
        "canonical": {
            "rawPath": "raw/canonical-result.json",
            "packagePath": "canonical-result.json",
            "sourceSha256": hashes.get("raw/canonical-result.json", "r" * 64),
            "packageSha256": hashes.get("canonical-result.json", "c" * 64),
            "canonicalizedFileSha256": hashes.get("canonical-result.json", "c" * 64),
        },
        "hostParity": {
            "requiredPlatforms": ["ios", "android", "harmony"],
            "allPresent": True,
            "match": True,
            "total": 0,
            "byPlatform": {
                "ios": {"present": True, "match": True, "total": 0},
                "android": {"present": True, "match": True, "total": 0},
                "harmony": {"present": True, "match": True, "total": 0},
            },
        },
        "diffSummary": {
            "match": True,
            "total": 0,
            "byPlatform": {
                platform: {"match": True, "total": 0}
                for platform in PLATFORMS
            },
        },
        "corpusProof": {
            "type": "corpus-same-result-proof",
            "status": "pass",
            "reasons": [],
            "conditions": {
                "sourceManifestPresent": True,
                "schemaVersionBound": True,
                "coreIdentityBound": True,
                "artifactHashesBound": True,
                "runBindingDeclared": True,
                "runBindingMatches": True,
                "scenarioBindingDeclared": True,
                "scenarioBindingMatches": True,
                "expectedDeclared": True,
                "fullDiffExpectedDeclared": True,
                "byPlatformExpectedDeclared": True,
                "hostParityExpectedDeclared": True,
                "fourPlatformCandidatesPresent": True,
                "fourPlatformSummaryPresent": True,
                "fullDiffMatch": True,
                "hostParityMatch": True,
                "openBlockers": 0,
            },
            "missingCandidates": [],
            "missingSummary": [],
        },
        "candidates": candidates,
        "artifacts": {
            "platformResult": "platform-result.json",
            "canonicalResult": "canonical-result.json",
            "diffResult": "diff-result.json",
            "environment": "environment.json",
            "blockerRegister": "corpus-blocker-register.json",
        },
        "blockers": {
            "registerPath": "/private/tmp/collector-run/corpus-blocker-register.json",
            "added": 0,
            "open": 0,
            "openByPlatform": {},
        },
    }


def _collector_files():
    files = {
        "platform-result.json": '{"platform":"cli"}\n',
        "canonical-result.json": '{"results":[]}\n',
        "diff-result.json": {
            "tool": "cross-platform-diff",
            "match": True,
            "total": 0,
            "summary": {
                platform: {"match": True, "total": 0}
                for platform in PLATFORMS
            },
            "candidates": {
                platform: {
                    "match": True,
                    "total": 0,
                    "differences": [],
                    "sha256": platform[0] * 64,
                    "canonicalizedSha256": platform[-1] * 64,
                }
                for platform in PLATFORMS
            },
        },
        "environment.json": '{"tool":"collector"}\n',
        "corpus-blocker-register.json": {
            "schemaVersion": 1,
            "tool": "release-blocker-register",
            "version": "1.0",
            "nextId": 1,
            "blockers": [],
        },
        "input.json": '{"query":"river"}\n',
        "raw/source-manifest.json": '{"schemaVersion":1}\n',
        "raw/canonical-result.json": '{"results":[]}\n',
    }
    for platform in PLATFORMS:
        files["raw/{0}-result.json".format(platform)] = (
            '{{"platform":"{0}","raw":true}}\n'.format(platform)
        )
        files["candidates/{0}-result.json".format(platform)] = (
            '{{"platform":"{0}","canonical":true}}\n'.format(platform)
        )
    hashes = {
        rel: _sha256_content(content)
        for rel, content in files.items()
    }
    files["manifest.json"] = _collector_manifest(hashes)
    return files


def _collector_blocked_files():
    files = _collector_files()
    diff_result = files["diff-result.json"]
    diff_result["match"] = False
    diff_result["total"] = 1
    diff_result["summary"]["android"] = {"match": False, "total": 1}
    diff_result["candidates"]["android"]["match"] = False
    diff_result["candidates"]["android"]["total"] = 1
    diff_result["candidates"]["android"]["differences"] = [{
        "path": "results[0].title",
        "expected": "canonical",
        "actual": "android",
    }]

    register = files["corpus-blocker-register.json"]
    register["nextId"] = 2
    register["blockers"].append({
        "id": "BLK-0001",
        "runId": "collector-run",
        "scenario": "four-platform-search",
        "platform": "android",
        "fieldPath": "results[0].title",
        "status": "open",
    })

    manifest = files["manifest.json"]
    manifest["diffSummary"] = {
        "match": False,
        "total": 1,
        "byPlatform": diff_result["summary"],
    }
    manifest["hostParity"] = {
        "requiredPlatforms": ["ios", "android", "harmony"],
        "allPresent": True,
        "match": False,
        "total": 1,
        "byPlatform": {
            "ios": {"present": True, "match": True, "total": 0},
            "android": {"present": True, "match": False, "total": 1},
            "harmony": {"present": True, "match": True, "total": 0},
        },
    }
    manifest["blockers"] = {
        "registerPath": "/private/tmp/collector-run/corpus-blocker-register.json",
        "added": 1,
        "open": 1,
        "openByPlatform": {"android": 1},
    }
    manifest["corpusProof"]["status"] = "blocked"
    manifest["corpusProof"]["reasons"] = [
        "full-diff-mismatch",
        "host-parity-mismatch",
        "open-blockers",
    ]
    manifest["corpusProof"]["conditions"]["fullDiffMatch"] = False
    manifest["corpusProof"]["conditions"]["hostParityMatch"] = False
    manifest["corpusProof"]["conditions"]["openBlockers"] = 1
    return files


def _tamper_packaged_diff_result_to_android_mismatch(out_dir):
    diff_path = os.path.join(out_dir, "diff-result.json")
    with open(diff_path, "r", encoding="utf-8") as handle:
        diff_result = json.load(handle)
    diff_result["match"] = False
    diff_result["total"] = 1
    diff_result["summary"]["android"] = {"match": False, "total": 1}
    diff_result["candidates"]["android"]["match"] = False
    diff_result["candidates"]["android"]["total"] = 1
    diff_result["candidates"]["android"]["differences"] = [{
        "path": "results[0].title",
        "expected": "canonical",
        "actual": "android",
    }]
    _write_json(diff_path, diff_result)


class ValidateRunDirTests(unittest.TestCase):
    def test_complete_run_dir_validates(self):
        run_dir = _make_run_dir(_complete_files())
        try:
            validation, loaded = brp.validate_run_dir(run_dir)
            self.assertTrue(validation["ok"])
            self.assertEqual(validation["missing"], [])
            self.assertEqual(validation["invalidJson"], [])
            for key, _filename in brp.REQUIRED_ARTIFACTS:
                self.assertTrue(validation["required"][key]["present"])
                self.assertTrue(validation["required"][key]["validJson"])
            for key in ("manifest", "platform-result", "canonical-result",
                        "diff-result"):
                self.assertIn(key, loaded)
        finally:
            shutil.rmtree(run_dir, ignore_errors=True)

    def test_collector_artifact_integrity_validates(self):
        run_dir = _make_run_dir(_collector_files())
        try:
            validation, _loaded = brp.validate_run_dir(run_dir)
            self.assertTrue(validation["ok"])
            collector_artifacts = validation["collectorArtifacts"]
            self.assertTrue(collector_artifacts["checked"])
            self.assertTrue(collector_artifacts["ok"])
            self.assertTrue(validation["collectorConsistency"]["checked"])
            self.assertTrue(validation["collectorConsistency"]["ok"])
            self.assertIn("sourceManifestFile", collector_artifacts["artifacts"])
            self.assertIn("candidate.ios.raw", collector_artifacts["artifacts"])
        finally:
            shutil.rmtree(run_dir, ignore_errors=True)

    def test_collector_missing_declared_artifact_fails_validation(self):
        run_dir = _make_run_dir(_collector_files())
        try:
            os.remove(os.path.join(run_dir, "raw", "source-manifest.json"))

            validation, _loaded = brp.validate_run_dir(run_dir)

            self.assertFalse(validation["ok"])
            self.assertFalse(validation["collectorArtifacts"]["ok"])
            self.assertTrue(any(
                "sourceManifestFile" in error
                for error in validation["collectorArtifacts"]["errors"]
            ))
        finally:
            shutil.rmtree(run_dir, ignore_errors=True)

    def test_collector_missing_artifact_declarations_fail_validation(self):
        files = _complete_files()
        files["manifest.json"] = {
            "sourceManifest": {
                "coreIdentity": dict(CORE_IDENTITY),
                "platformRuns": {"ios": dict(CORE_IDENTITY)},
            }
        }
        run_dir = _make_run_dir(files)
        try:
            validation, _loaded = brp.validate_run_dir(run_dir)

            self.assertFalse(validation["ok"])
            self.assertIn(
                "collector artifact declaration missing: sourceManifestFile",
                validation["collectorArtifacts"]["errors"],
            )
        finally:
            shutil.rmtree(run_dir, ignore_errors=True)

    def test_collector_diff_summary_mismatch_fails_validation(self):
        files = _collector_files()
        files["manifest.json"]["diffSummary"]["total"] = 1
        run_dir = _make_run_dir(files)
        try:
            validation, _loaded = brp.validate_run_dir(run_dir)

            self.assertFalse(validation["ok"])
            self.assertIn(
                "collector manifest diffSummary does not match diff-result.json",
                validation["collectorConsistency"]["errors"],
            )
        finally:
            shutil.rmtree(run_dir, ignore_errors=True)

    def test_collector_host_parity_mismatch_fails_validation(self):
        files = _collector_files()
        files["manifest.json"]["hostParity"]["total"] = 1
        run_dir = _make_run_dir(files)
        try:
            validation, _loaded = brp.validate_run_dir(run_dir)

            self.assertFalse(validation["ok"])
            self.assertIn(
                "collector manifest hostParity does not match diff-result.json",
                validation["collectorConsistency"]["errors"],
            )
        finally:
            shutil.rmtree(run_dir, ignore_errors=True)

    def test_collector_register_open_mismatch_fails_packaging(self):
        files = _collector_files()
        files["corpus-blocker-register.json"]["blockers"].append({
            "id": "BLK-0001",
            "runId": "collector-run",
            "platform": "android",
            "fieldPath": "results[1].name",
            "status": "open",
        })
        run_dir = _make_run_dir(files)
        try:
            with self.assertRaises(brp.PackagingError) as raised:
                brp.package_run(run_dir)
            self.assertIn("blockers.open", str(raised.exception))
            self.assertIn("blocker register", str(raised.exception))
        finally:
            shutil.rmtree(run_dir, ignore_errors=True)

    def test_collector_hash_mismatch_fails_packaging(self):
        files = _collector_files()
        files["raw/ios-result.json"] = '{"platform":"ios","raw":"tampered"}\n'
        run_dir = _make_run_dir(files)
        try:
            with self.assertRaises(brp.PackagingError) as raised:
                brp.package_run(run_dir)
            self.assertIn("candidate.ios.raw", str(raised.exception))
            self.assertIn("mismatch", str(raised.exception))
        finally:
            shutil.rmtree(run_dir, ignore_errors=True)

    def test_missing_required_file_reported(self):
        files = _complete_files()
        del files["canonical-result.json"]
        run_dir = _make_run_dir(files)
        try:
            validation, loaded = brp.validate_run_dir(run_dir)
            self.assertFalse(validation["ok"])
            self.assertIn("canonical-result", validation["missing"])
            self.assertNotIn("canonical-result", loaded)
        finally:
            shutil.rmtree(run_dir, ignore_errors=True)

    def test_invalid_json_reported(self):
        files = _complete_files()
        files["manifest.json"] = "{ not valid json"
        run_dir = _make_run_dir(files)
        try:
            validation, loaded = brp.validate_run_dir(run_dir)
            self.assertFalse(validation["ok"])
            self.assertIn("manifest", validation["invalidJson"])
            self.assertNotIn("manifest", loaded)
            self.assertTrue(validation["required"]["manifest"]["present"])
            self.assertFalse(validation["required"]["manifest"]["validJson"])
        finally:
            shutil.rmtree(run_dir, ignore_errors=True)

    def test_nonexistent_run_dir_raises(self):
        with self.assertRaises(brp.PackagingError):
            brp.validate_run_dir("/private/tmp/does-not-exist-brp-xyz")


class DeriveRunIdTests(unittest.TestCase):
    def test_from_manifest_run_id(self):
        run_id = brp.derive_run_id(
            "/private/tmp/some-run", {"runId": "run-42", "timestamp": "x"}
        )
        self.assertEqual(run_id, "run-42")

    def test_falls_back_to_dirname(self):
        run_id = brp.derive_run_id("/private/tmp/my-run-dir/", {"no": "id"})
        self.assertEqual(run_id, "my-run-dir")

    def test_falls_back_when_manifest_not_dict(self):
        run_id = brp.derive_run_id("/private/tmp/my-run-dir", ["not", "a", "dict"])
        self.assertEqual(run_id, "my-run-dir")


class DeriveDiffSummaryTests(unittest.TestCase):
    def test_cross_platform_diff_shape_mismatch(self):
        diff = {
            "summary": {
                "android": {"total": 1},
                "harmony": {"total": 2},
            }
        }
        result = brp.derive_diff_summary(diff)
        self.assertEqual(result["total"], 3)
        self.assertFalse(result["match"])

    def test_cross_platform_diff_shape_match(self):
        diff = {
            "summary": {
                "android": {"total": 0},
                "harmony": {"total": 0},
            }
        }
        result = brp.derive_diff_summary(diff)
        self.assertEqual(result["total"], 0)
        self.assertTrue(result["match"])

    def test_match_bool_shape(self):
        result = brp.derive_diff_summary({"match": True})
        self.assertTrue(result["match"])
        self.assertIsNone(result["total"])

    def test_total_int_shape(self):
        result = brp.derive_diff_summary({"total": 5})
        self.assertEqual(result["total"], 5)
        self.assertFalse(result["match"])

    def test_empty_shape_returns_none(self):
        result = brp.derive_diff_summary({})
        self.assertIsNone(result["match"])
        self.assertIsNone(result["total"])


class DeriveEvidenceSummaryTests(unittest.TestCase):
    def test_missing_source_manifest_returns_none(self):
        self.assertIsNone(brp.derive_evidence_summary({"runId": "run-42"}))

    def test_extracts_core_identity_and_platform_hashes(self):
        result = brp.derive_evidence_summary(_collector_manifest())

        self.assertEqual(result["runId"], "collector-run")
        self.assertEqual(result["scenario"], "four-platform-search")
        self.assertEqual(result["coreIdentity"], CORE_IDENTITY)
        self.assertTrue(result["hostParity"]["match"])
        self.assertEqual(result["corpusProof"]["status"], "pass")
        self.assertEqual(result["hostParity"]["requiredPlatforms"], ["ios", "android", "harmony"])
        self.assertEqual(sorted(result["platforms"].keys()), list(PLATFORMS))
        android = result["platforms"]["android"]
        self.assertEqual(android["businessKernel"], "reader-core-native-rust")
        self.assertEqual(android["coreCommit"], "090b96f")
        self.assertEqual(android["raw"], "raw/android-result.json")
        self.assertEqual(android["canonicalized"], "candidates/android-result.json")
        self.assertEqual(android["sourceSha256"], "a" * 64)
        self.assertEqual(android["canonicalizedSha256"], "d" * 64)
        self.assertEqual(android["canonicalizedFileSha256"], "n" * 64)


class SanitizeForPathTests(unittest.TestCase):
    def test_strips_unsafe_chars(self):
        self.assertEqual(brp.sanitize_for_path("run 42/2026"), "run-42-2026")

    def test_keeps_safe_chars(self):
        self.assertEqual(brp.sanitize_for_path("run-42.0_beta"), "run-42.0_beta")

    def test_empty_falls_back(self):
        self.assertEqual(brp.sanitize_for_path(""), "run")


class AssertSafeOutputTests(unittest.TestCase):
    def test_default_under_private_tmp_ok(self):
        brp.assert_safe_output("/private/tmp/foo-bundle", user_specified=False)

    def test_default_outside_private_tmp_rejected(self):
        with self.assertRaises(brp.PackagingError):
            brp.assert_safe_output("/var/tmp/foo-bundle", user_specified=False)

    def test_user_specified_outside_documents_ok(self):
        brp.assert_safe_output("/private/tmp/custom-bundle", user_specified=True)
        brp.assert_safe_output("/var/tmp/custom-bundle", user_specified=True)

    def test_user_specified_under_documents_rejected(self):
        docs = os.path.expanduser("~/Documents")
        target = os.path.join(docs, "brp-bundle-test")
        with self.assertRaises(brp.PackagingError):
            brp.assert_safe_output(target, user_specified=True)


class PackageRunTests(unittest.TestCase):
    def setUp(self):
        self._paths = []

    def tearDown(self):
        for path in self._paths:
            shutil.rmtree(path, ignore_errors=True)
            if os.path.exists(path):
                try:
                    os.remove(path)
                except OSError:
                    pass

    def _track(self, path):
        self._paths.append(path)
        return path

    def test_copies_artifacts_and_writes_summary(self):
        run_dir = _make_run_dir(_complete_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)

        summary = brp.package_run(run_dir, out_dir=out_dir)

        for _key, filename in brp.REQUIRED_ARTIFACTS:
            self.assertTrue(
                os.path.isfile(os.path.join(out_dir, filename)), filename
            )
        summary_path = os.path.join(out_dir, "summary.json")
        self.assertTrue(os.path.isfile(summary_path))
        with open(summary_path, "r", encoding="utf-8") as handle:
            on_disk = json.load(handle)
        self.assertEqual(on_disk["runId"], "run-42")
        self.assertEqual(on_disk["tool"], brp.TOOL_NAME)
        self.assertEqual(on_disk["bundle"]["outDir"], os.path.abspath(out_dir))
        self.assertTrue(os.path.isfile(os.path.join(out_dir, "bundle-manifest.json")))
        self.assertTrue(os.path.isfile(os.path.join(out_dir, "bundle-manifest.sha256")))
        self.assertTrue(brp.verify_bundle_manifest(out_dir)["ok"])
        self.assertEqual(summary["bundle"]["manifest"]["path"], "bundle-manifest.json")
        self.assertEqual(len(summary["bundle"]["manifest"]["sha256"]), 64)

    def test_summary_has_required_fields(self):
        run_dir = _make_run_dir(_complete_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)

        summary = brp.package_run(run_dir, out_dir=out_dir)

        for field in ("schemaVersion", "tool", "version", "packagedAt", "runId",
                      "runDir", "validation", "manifest", "diffSummary",
                      "evidence", "environment", "files", "bundle"):
            self.assertIn(field, summary)
        self.assertTrue(summary["validation"]["ok"])
        self.assertEqual(summary["diffSummary"]["total"], 0)
        self.assertTrue(summary["diffSummary"]["match"])
        self.assertIsNone(summary["evidence"])
        self.assertIsNone(summary["bundle"]["zip"])
        paths = {entry["path"] for entry in summary["files"]}
        for _key, filename in brp.REQUIRED_ARTIFACTS:
            self.assertIn(filename, paths)
        for entry in summary["files"]:
            self.assertIn("size", entry)
            self.assertIn("sha256", entry)
            self.assertEqual(len(entry["sha256"]), 64)

    def test_bundle_manifest_detects_tampered_file(self):
        run_dir = _make_run_dir(_complete_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        with open(os.path.join(out_dir, "platform-result.json"), "w", encoding="utf-8") as handle:
            handle.write('{"book":{"title":"tampered"}}\n')

        validation = brp.verify_bundle_manifest(out_dir)
        self.assertFalse(validation["ok"])
        self.assertIn(
            "bundle manifest sha256 mismatch: platform-result.json",
            validation["errors"],
        )

    def test_bundle_manifest_rejects_self_consistent_failed_summary(self):
        run_dir = _make_run_dir(_complete_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)
        summary_path = os.path.join(out_dir, "summary.json")
        with open(summary_path, "r", encoding="utf-8") as handle:
            summary = json.load(handle)
        summary["validation"]["ok"] = False
        brp._write_json(summary_path, summary)
        brp._write_bundle_manifest(out_dir, summary)

        validation = brp.verify_bundle_manifest(out_dir)

        self.assertFalse(validation["ok"])
        self.assertIn(
            "bundle summary.json validation.ok is not true",
            validation["errors"],
        )

    def test_bundle_zip_rejects_self_consistent_failed_summary(self):
        run_dir = _make_run_dir(_complete_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        zip_path = self._track(os.path.join(PRIVATE_TMP, "brp-bad-summary.zip"))
        brp.package_run(run_dir, out_dir=out_dir, make_zip=True, zip_path=zip_path)
        summary_path = os.path.join(out_dir, "summary.json")
        with open(summary_path, "r", encoding="utf-8") as handle:
            summary = json.load(handle)
        summary["validation"]["ok"] = False
        brp._write_json(summary_path, summary)
        brp._write_bundle_manifest(out_dir, summary)
        brp._zip_directory(out_dir, zip_path)

        validation = brp.verify_bundle_zip(zip_path)

        self.assertFalse(validation["ok"])
        self.assertIn(
            "bundle summary.json validation.ok is not true",
            validation["errors"],
        )

    def test_bundle_manifest_recomputes_payload_validation(self):
        run_dir = _make_run_dir(_collector_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)
        summary_path = os.path.join(out_dir, "summary.json")
        with open(summary_path, "r", encoding="utf-8") as handle:
            summary = json.load(handle)
        _tamper_packaged_diff_result_to_android_mismatch(out_dir)
        brp._write_bundle_manifest(out_dir, summary)

        validation = brp.verify_bundle_manifest(out_dir)

        self.assertFalse(validation["ok"])
        self.assertIn("bundle payload validation.ok is not true", validation["errors"])
        self.assertIn(
            "bundle payload collectorConsistency: collector manifest diffSummary "
            "does not match diff-result.json",
            validation["errors"],
        )
        self.assertIn(
            "bundle summary.json validation does not match payload validation",
            validation["errors"],
        )
        self.assertIn(
            "bundle summary.json diffSummary does not match payload diff-result.json",
            validation["errors"],
        )

    def test_bundle_zip_recomputes_payload_validation(self):
        run_dir = _make_run_dir(_collector_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        zip_path = self._track(os.path.join(PRIVATE_TMP, "brp-payload-tamper.zip"))
        brp.package_run(run_dir, out_dir=out_dir, make_zip=True, zip_path=zip_path)
        summary_path = os.path.join(out_dir, "summary.json")
        with open(summary_path, "r", encoding="utf-8") as handle:
            summary = json.load(handle)
        _tamper_packaged_diff_result_to_android_mismatch(out_dir)
        brp._write_bundle_manifest(out_dir, summary)
        brp._zip_directory(out_dir, zip_path)

        validation = brp.verify_bundle_zip(zip_path)

        self.assertFalse(validation["ok"])
        self.assertIn("bundle payload validation.ok is not true", validation["errors"])
        self.assertIn(
            "bundle payload collectorConsistency: collector manifest diffSummary "
            "does not match diff-result.json",
            validation["errors"],
        )
        self.assertIn(
            "bundle summary.json validation does not match payload validation",
            validation["errors"],
        )
        self.assertIn(
            "bundle summary.json diffSummary does not match payload diff-result.json",
            validation["errors"],
        )

    def test_bundle_verification_can_require_corpus_proof_pass(self):
        run_dir = _make_run_dir(_collector_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        validation = brp.verify_bundle_manifest(
            out_dir,
            require_corpus_proof_pass=True,
        )

        self.assertTrue(validation["ok"])
        self.assertTrue(validation["requiredCorpusProofPass"])

    def test_bundle_verification_rejects_incomplete_pass_proof_when_required(self):
        files = _collector_files()
        files["manifest.json"]["corpusProof"]["conditions"][
            "schemaVersionBound"
        ] = False
        run_dir = _make_run_dir(files)
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        plain_validation = brp.verify_bundle_manifest(out_dir)
        strict_validation = brp.verify_bundle_manifest(
            out_dir,
            require_corpus_proof_pass=True,
        )

        self.assertTrue(plain_validation["ok"])
        self.assertFalse(strict_validation["ok"])
        self.assertIn(
            "bundle summary.json evidence.corpusProof.conditions.schemaVersionBound "
            "is not true",
            strict_validation["errors"],
        )

    def test_bundle_verification_rejects_pass_proof_with_reasons_when_required(self):
        files = _collector_files()
        files["manifest.json"]["corpusProof"]["reasons"] = [
            "schema-version-not-bound"
        ]
        run_dir = _make_run_dir(files)
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        strict_validation = brp.verify_bundle_manifest(
            out_dir,
            require_corpus_proof_pass=True,
        )

        self.assertFalse(strict_validation["ok"])
        self.assertIn(
            "bundle summary.json evidence.corpusProof.reasons must be empty",
            strict_validation["errors"],
        )

    def test_bundle_verification_rejects_blocked_corpus_proof_when_required(self):
        run_dir = _make_run_dir(_collector_blocked_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        plain_validation = brp.verify_bundle_manifest(out_dir)
        strict_validation = brp.verify_bundle_manifest(
            out_dir,
            require_corpus_proof_pass=True,
        )

        self.assertTrue(plain_validation["ok"])
        self.assertFalse(strict_validation["ok"])
        self.assertIn(
            "bundle summary.json evidence.corpusProof.status is not pass",
            strict_validation["errors"],
        )

    def test_bundle_verification_rejects_plain_bundle_when_proof_required(self):
        run_dir = _make_run_dir(_complete_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        plain_validation = brp.verify_bundle_manifest(out_dir)
        strict_validation = brp.verify_bundle_manifest(
            out_dir,
            require_corpus_proof_pass=True,
        )

        self.assertTrue(plain_validation["ok"])
        self.assertFalse(strict_validation["ok"])
        self.assertIn(
            "bundle summary.json evidence.corpusProof.status is not pass",
            strict_validation["errors"],
        )

    def test_bundle_zip_verification_can_require_corpus_proof_pass(self):
        run_dir = _make_run_dir(_collector_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        zip_path = self._track(os.path.join(PRIVATE_TMP, "brp-proof-pass.zip"))
        brp.package_run(run_dir, out_dir=out_dir, make_zip=True, zip_path=zip_path)

        validation = brp.verify_bundle_zip(
            zip_path,
            require_corpus_proof_pass=True,
        )

        self.assertTrue(validation["ok"])
        self.assertTrue(validation["requiredCorpusProofPass"])

    def test_bundle_verification_can_require_core_commit(self):
        run_dir = _make_run_dir(_collector_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        validation = brp.verify_bundle_manifest(
            out_dir,
            required_core_commit="090b96f",
        )

        self.assertTrue(validation["ok"])
        self.assertEqual(validation["requiredCoreCommit"], "090b96f")

    def test_bundle_verification_allows_full_commit_for_short_manifest_commit(self):
        run_dir = _make_run_dir(_collector_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        validation = brp.verify_bundle_manifest(
            out_dir,
            required_core_commit="090b96f1234567890abcdef1234567890abcdef1",
        )

        self.assertTrue(validation["ok"])

    def test_bundle_verification_rejects_wrong_core_commit(self):
        run_dir = _make_run_dir(_collector_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        plain_validation = brp.verify_bundle_manifest(out_dir)
        strict_validation = brp.verify_bundle_manifest(
            out_dir,
            required_core_commit="deadbee",
        )

        self.assertTrue(plain_validation["ok"])
        self.assertFalse(strict_validation["ok"])
        self.assertIn(
            "bundle summary.json evidence.coreIdentity.coreCommit does not match "
            "required core commit: expected deadbee, got 090b96f",
            strict_validation["errors"],
        )

    def test_bundle_verification_rejects_plain_bundle_when_core_commit_required(self):
        run_dir = _make_run_dir(_complete_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        validation = brp.verify_bundle_manifest(
            out_dir,
            required_core_commit="090b96f",
        )

        self.assertFalse(validation["ok"])
        self.assertIn(
            "bundle summary.json evidence.coreIdentity.coreCommit is missing",
            validation["errors"],
        )

    def test_bundle_zip_verification_can_require_core_commit(self):
        run_dir = _make_run_dir(_collector_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        zip_path = self._track(os.path.join(PRIVATE_TMP, "brp-core-commit.zip"))
        brp.package_run(run_dir, out_dir=out_dir, make_zip=True, zip_path=zip_path)

        validation = brp.verify_bundle_zip(
            zip_path,
            required_core_commit="090b96f",
        )

        self.assertTrue(validation["ok"])
        self.assertEqual(validation["requiredCoreCommit"], "090b96f")

    def test_bundle_verification_can_require_run_id_and_scenario(self):
        run_dir = _make_run_dir(_collector_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        validation = brp.verify_bundle_manifest(
            out_dir,
            required_run_id="collector-run",
            required_scenario="four-platform-search",
        )

        self.assertTrue(validation["ok"])
        self.assertEqual(validation["requiredRunId"], "collector-run")
        self.assertEqual(validation["requiredScenario"], "four-platform-search")

    def test_bundle_verification_rejects_wrong_run_id(self):
        run_dir = _make_run_dir(_collector_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        validation = brp.verify_bundle_manifest(
            out_dir,
            required_run_id="wrong-run",
        )

        self.assertFalse(validation["ok"])
        self.assertIn(
            "bundle summary.json runId does not match required runId: "
            "expected wrong-run, got collector-run",
            validation["errors"],
        )

    def test_bundle_verification_rejects_wrong_scenario(self):
        run_dir = _make_run_dir(_collector_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        validation = brp.verify_bundle_manifest(
            out_dir,
            required_scenario="wrong-scenario",
        )

        self.assertFalse(validation["ok"])
        self.assertIn(
            "bundle summary.json manifest.scenario does not match required "
            "scenario: expected wrong-scenario, got four-platform-search",
            validation["errors"],
        )
        self.assertIn(
            "bundle summary.json evidence.scenario does not match required "
            "scenario: expected wrong-scenario, got four-platform-search",
            validation["errors"],
        )

    def test_bundle_verification_rejects_plain_bundle_when_scenario_required(self):
        run_dir = _make_run_dir(_complete_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        validation = brp.verify_bundle_manifest(
            out_dir,
            required_scenario="four-platform-search",
        )

        self.assertFalse(validation["ok"])
        self.assertIn(
            "bundle summary.json manifest.scenario does not match required "
            "scenario: expected four-platform-search, got None",
            validation["errors"],
        )

    def test_bundle_zip_verification_can_require_run_id_and_scenario(self):
        run_dir = _make_run_dir(_collector_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        zip_path = self._track(os.path.join(PRIVATE_TMP, "brp-run-scenario.zip"))
        brp.package_run(run_dir, out_dir=out_dir, make_zip=True, zip_path=zip_path)

        validation = brp.verify_bundle_zip(
            zip_path,
            required_run_id="collector-run",
            required_scenario="four-platform-search",
        )

        self.assertTrue(validation["ok"])
        self.assertEqual(validation["requiredRunId"], "collector-run")
        self.assertEqual(validation["requiredScenario"], "four-platform-search")

    def test_summary_exposes_collector_core_identity_evidence(self):
        files = _collector_files()
        run_dir = _make_run_dir(files)
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)

        summary = brp.package_run(run_dir, out_dir=out_dir)

        self.assertEqual(summary["evidence"]["runId"], "collector-run")
        self.assertEqual(summary["evidence"]["scenario"], "four-platform-search")
        self.assertEqual(summary["evidence"]["coreIdentity"], CORE_IDENTITY)
        self.assertEqual(
            summary["evidence"]["sourceManifestFile"],
            {
                "raw": "raw/source-manifest.json",
                "sourceSha256": _sha256_text('{"schemaVersion":1}\n'),
                "packageSha256": _sha256_text('{"schemaVersion":1}\n'),
            },
        )
        self.assertEqual(
            sorted(summary["evidence"]["platforms"].keys()),
            list(PLATFORMS),
        )
        self.assertEqual(
            summary["evidence"]["platforms"]["ios"]["canonicalized"],
            "candidates/ios-result.json",
        )
        self.assertEqual(
            summary["evidence"]["platforms"]["ios"]["canonicalizedFileSha256"],
            _sha256_text('{"platform":"ios","canonical":true}\n'),
        )

    def test_bundle_includes_logs_and_environment(self):
        files = _complete_files()
        files["environment.json"] = {"os": "macOS", "arch": "arm64"}
        files["logs/run.log"] = "2026-06-25 INFO started\n"
        files["worker.log"] = "worker output\n"
        run_dir = _make_run_dir(files)
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)

        summary = brp.package_run(run_dir, out_dir=out_dir)

        self.assertTrue(os.path.isfile(os.path.join(out_dir, "environment.json")))
        self.assertTrue(os.path.isfile(os.path.join(out_dir, "logs", "run.log")))
        self.assertTrue(os.path.isfile(os.path.join(out_dir, "worker.log")))
        self.assertEqual(summary["environment"]["run"], {"os": "macOS", "arch": "arm64"})
        self.assertEqual(summary["environment"]["packager"]["tool"], brp.TOOL_NAME)

    def test_creates_zip_when_requested(self):
        run_dir = _make_run_dir(_complete_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)
        zip_path = self._track(os.path.join(PRIVATE_TMP, "brp-test-bundle.zip"))

        summary = brp.package_run(
            run_dir, out_dir=out_dir, make_zip=True, zip_path=zip_path
        )

        self.assertEqual(summary["bundle"]["zip"], os.path.abspath(zip_path))
        self.assertTrue(os.path.isfile(zip_path))
        with zipfile.ZipFile(zip_path, "r") as archive:
            names = archive.namelist()
        self.assertTrue(any(n.endswith("/summary.json") for n in names))
        self.assertTrue(any(n.endswith("/manifest.json") for n in names))
        self.assertTrue(any(n.endswith("/bundle-manifest.json") for n in names))
        self.assertTrue(any(n.endswith("/bundle-manifest.sha256") for n in names))
        self.assertTrue(all("/" in n for n in names))

    def test_no_zip_by_default(self):
        run_dir = _make_run_dir(_complete_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)

        summary = brp.package_run(run_dir, out_dir=out_dir)

        self.assertIsNone(summary["bundle"]["zip"])

    def test_default_bundle_under_private_tmp(self):
        run_dir = _make_run_dir(_complete_files())
        self._track(run_dir)

        summary = brp.package_run(run_dir)

        bundle = summary["bundle"]["outDir"]
        self._track(bundle)
        self.assertTrue(bundle.startswith(PRIVATE_TMP + "/"))
        self.assertTrue(bundle.endswith("-bundle"))
        self.assertTrue(os.path.isfile(os.path.join(bundle, "summary.json")))

    def test_default_zip_under_private_tmp(self):
        run_dir = _make_run_dir(_complete_files())
        self._track(run_dir)

        summary = brp.package_run(run_dir, make_zip=True)

        bundle = summary["bundle"]["outDir"]
        zip_path = summary["bundle"]["zip"]
        self._track(bundle)
        self._track(zip_path)
        self.assertTrue(zip_path.startswith(PRIVATE_TMP + "/"))
        self.assertTrue(zip_path.endswith("-bundle.zip"))
        self.assertTrue(os.path.isfile(zip_path))

    def test_refuses_documents_output(self):
        run_dir = _make_run_dir(_complete_files())
        self._track(run_dir)
        docs = os.path.expanduser("~/Documents")
        bad_out = os.path.join(docs, "brp-bundle-refused")

        with self.assertRaises(brp.PackagingError):
            brp.package_run(run_dir, out_dir=bad_out)
        self.assertFalse(os.path.exists(bad_out))

    def test_failed_validation_raises(self):
        files = _complete_files()
        del files["diff-result.json"]
        run_dir = _make_run_dir(files)
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)

        with self.assertRaises(brp.PackagingError):
            brp.package_run(run_dir, out_dir=out_dir)
        self.assertFalse(os.path.exists(out_dir))

    def test_rerun_overwrites_previous_bundle(self):
        run_dir = _make_run_dir(_complete_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)

        first = brp.package_run(run_dir, out_dir=out_dir)
        first_packaged_at = first["packagedAt"]
        summary_path = os.path.join(out_dir, "summary.json")
        self.assertTrue(os.path.isfile(summary_path))

        second = brp.package_run(run_dir, out_dir=out_dir)
        self.assertEqual(second["bundle"]["outDir"], os.path.abspath(out_dir))
        self.assertTrue(os.path.isfile(summary_path))


class RenderSummaryTests(unittest.TestCase):
    def test_render_mentions_runid_and_paths(self):
        run_dir = _make_run_dir(_complete_files())
        out_dir = tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP)
        os.rmdir(out_dir)
        try:
            summary = brp.package_run(run_dir, out_dir=out_dir)
            text = brp.render_summary(summary)
            self.assertIn("run-42", text)
            self.assertIn(out_dir, text)
            self.assertIn("validation: OK", text)
            self.assertIn("zip: (not created)", text)
        finally:
            shutil.rmtree(run_dir, ignore_errors=True)
            shutil.rmtree(out_dir, ignore_errors=True)

    def test_render_mentions_core_identity_when_present(self):
        run_dir = _make_run_dir(_collector_files())
        out_dir = tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP)
        os.rmdir(out_dir)
        try:
            summary = brp.package_run(run_dir, out_dir=out_dir)
            text = brp.render_summary(summary)
            self.assertIn("reader-core-native-rust 090b96f", text)
            self.assertIn("platforms: android, cli, harmony, ios", text)
            self.assertIn("host parity: match (total differences: 0)", text)
            self.assertIn("corpus proof: pass", text)
        finally:
            shutil.rmtree(run_dir, ignore_errors=True)
            shutil.rmtree(out_dir, ignore_errors=True)


class MainCliTests(unittest.TestCase):
    def setUp(self):
        self._paths = []

    def tearDown(self):
        for path in self._paths:
            shutil.rmtree(path, ignore_errors=True)
            if os.path.exists(path):
                try:
                    os.remove(path)
                except OSError:
                    pass

    def test_main_packages_and_returns_zero(self):
        run_dir = _make_run_dir(_complete_files())
        self._paths.append(run_dir)
        out_dir = tempfile.mkdtemp(prefix="brp-cli-", dir=PRIVATE_TMP)
        os.rmdir(out_dir)
        self._paths.append(out_dir)

        exit_code = brp.main([run_dir, "--out", out_dir])
        self.assertEqual(exit_code, 0)
        self.assertTrue(os.path.isfile(os.path.join(out_dir, "summary.json")))

    def test_main_returns_nonzero_on_validation_failure(self):
        files = _complete_files()
        del files["manifest.json"]
        run_dir = _make_run_dir(files)
        self._paths.append(run_dir)
        out_dir = tempfile.mkdtemp(prefix="brp-cli-", dir=PRIVATE_TMP)
        os.rmdir(out_dir)
        self._paths.append(out_dir)

        exit_code = brp.main([run_dir, "--out", out_dir])
        self.assertNotEqual(exit_code, 0)
        self.assertFalse(os.path.exists(out_dir))

    def test_main_zip_flag_without_path_uses_private_tmp(self):
        run_dir = _make_run_dir(_complete_files())
        self._paths.append(run_dir)
        out_dir = tempfile.mkdtemp(prefix="brp-cli-", dir=PRIVATE_TMP)
        os.rmdir(out_dir)
        self._paths.append(out_dir)

        try:
            exit_code = brp.main([run_dir, "--out", out_dir, "--zip"])
            self.assertEqual(exit_code, 0)
            # The default zip path is derived from the run id and lives under
            # /private/tmp. Locate it via the bundle's summary.json.
            with open(os.path.join(out_dir, "summary.json"), "r",
                      encoding="utf-8") as handle:
                summary = json.load(handle)
            zip_path = summary["bundle"]["zip"]
            self.assertIsNotNone(zip_path)
            self.assertTrue(zip_path.startswith(PRIVATE_TMP + "/"))
            self._paths.append(zip_path)
            self.assertTrue(os.path.isfile(zip_path))
        finally:
            pass

    def test_main_verifies_bundle_directory(self):
        run_dir = _make_run_dir(_complete_files())
        self._paths.append(run_dir)
        out_dir = tempfile.mkdtemp(prefix="brp-cli-", dir=PRIVATE_TMP)
        os.rmdir(out_dir)
        self._paths.append(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        exit_code = brp.main(["--verify-bundle", out_dir])

        self.assertEqual(exit_code, 0)

    def test_main_verifies_bundle_zip(self):
        run_dir = _make_run_dir(_complete_files())
        self._paths.append(run_dir)
        out_dir = tempfile.mkdtemp(prefix="brp-cli-", dir=PRIVATE_TMP)
        os.rmdir(out_dir)
        self._paths.append(out_dir)
        zip_path = os.path.join(PRIVATE_TMP, "brp-cli-verify-bundle.zip")
        self._paths.append(zip_path)
        brp.package_run(run_dir, out_dir=out_dir, make_zip=True, zip_path=zip_path)

        exit_code = brp.main(["--verify-bundle", zip_path])

        self.assertEqual(exit_code, 0)

    def test_main_verifies_bundle_directory_with_required_corpus_proof_pass(self):
        run_dir = _make_run_dir(_collector_files())
        self._paths.append(run_dir)
        out_dir = tempfile.mkdtemp(prefix="brp-cli-", dir=PRIVATE_TMP)
        os.rmdir(out_dir)
        self._paths.append(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        exit_code = brp.main([
            "--verify-bundle",
            out_dir,
            "--require-corpus-proof-pass",
        ])

        self.assertEqual(exit_code, 0)

    def test_main_verifies_bundle_directory_with_required_core_commit(self):
        run_dir = _make_run_dir(_collector_files())
        self._paths.append(run_dir)
        out_dir = tempfile.mkdtemp(prefix="brp-cli-", dir=PRIVATE_TMP)
        os.rmdir(out_dir)
        self._paths.append(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        exit_code = brp.main([
            "--verify-bundle",
            out_dir,
            "--require-corpus-proof-pass",
            "--require-core-commit",
            "090b96f",
            "--require-run-id",
            "collector-run",
            "--require-scenario",
            "four-platform-search",
        ])

        self.assertEqual(exit_code, 0)

    def test_main_required_core_commit_rejects_wrong_commit(self):
        run_dir = _make_run_dir(_collector_files())
        self._paths.append(run_dir)
        out_dir = tempfile.mkdtemp(prefix="brp-cli-", dir=PRIVATE_TMP)
        os.rmdir(out_dir)
        self._paths.append(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        exit_code = brp.main([
            "--verify-bundle",
            out_dir,
            "--require-core-commit",
            "deadbee",
        ])

        self.assertNotEqual(exit_code, 0)

    def test_main_required_scenario_rejects_wrong_scenario(self):
        run_dir = _make_run_dir(_collector_files())
        self._paths.append(run_dir)
        out_dir = tempfile.mkdtemp(prefix="brp-cli-", dir=PRIVATE_TMP)
        os.rmdir(out_dir)
        self._paths.append(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        exit_code = brp.main([
            "--verify-bundle",
            out_dir,
            "--require-scenario",
            "wrong-scenario",
        ])

        self.assertNotEqual(exit_code, 0)

    def test_main_required_corpus_proof_pass_rejects_blocked_bundle(self):
        run_dir = _make_run_dir(_collector_blocked_files())
        self._paths.append(run_dir)
        out_dir = tempfile.mkdtemp(prefix="brp-cli-", dir=PRIVATE_TMP)
        os.rmdir(out_dir)
        self._paths.append(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        plain_exit = brp.main(["--verify-bundle", out_dir])
        strict_exit = brp.main([
            "--verify-bundle",
            out_dir,
            "--require-corpus-proof-pass",
        ])

        self.assertEqual(plain_exit, 0)
        self.assertNotEqual(strict_exit, 0)

    def test_main_required_corpus_proof_pass_rejects_plain_bundle(self):
        run_dir = _make_run_dir(_complete_files())
        self._paths.append(run_dir)
        out_dir = tempfile.mkdtemp(prefix="brp-cli-", dir=PRIVATE_TMP)
        os.rmdir(out_dir)
        self._paths.append(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)

        exit_code = brp.main([
            "--verify-bundle",
            out_dir,
            "--require-corpus-proof-pass",
        ])

        self.assertNotEqual(exit_code, 0)

    def test_main_rejects_required_corpus_proof_pass_without_verify_mode(self):
        exit_code = brp.main(["--require-corpus-proof-pass"])

        self.assertNotEqual(exit_code, 0)

    def test_main_rejects_required_core_commit_without_verify_mode(self):
        exit_code = brp.main(["--require-core-commit", "090b96f"])

        self.assertNotEqual(exit_code, 0)

    def test_main_rejects_required_run_id_without_verify_mode(self):
        exit_code = brp.main(["--require-run-id", "collector-run"])

        self.assertNotEqual(exit_code, 0)

    def test_main_rejects_required_scenario_without_verify_mode(self):
        exit_code = brp.main(["--require-scenario", "four-platform-search"])

        self.assertNotEqual(exit_code, 0)

    def test_main_verify_bundle_returns_nonzero_on_tamper(self):
        run_dir = _make_run_dir(_complete_files())
        self._paths.append(run_dir)
        out_dir = tempfile.mkdtemp(prefix="brp-cli-", dir=PRIVATE_TMP)
        os.rmdir(out_dir)
        self._paths.append(out_dir)
        brp.package_run(run_dir, out_dir=out_dir)
        with open(os.path.join(out_dir, "platform-result.json"), "w",
                  encoding="utf-8") as handle:
            handle.write('{"book":{"title":"tampered"}}\n')

        exit_code = brp.main(["--verify-bundle", out_dir])

        self.assertNotEqual(exit_code, 0)

    def test_main_requires_run_dir_without_verify_mode(self):
        exit_code = brp.main([])

        self.assertNotEqual(exit_code, 0)


if __name__ == "__main__":
    unittest.main()
