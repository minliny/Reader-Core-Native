"""Tests for the benchmark run packager tool.

Run from the repository root:
    python3 -m unittest tests.tooling.test_benchmark_run_packager
or:
    python3 tests/tooling/test_benchmark_run_packager.py
"""

import json
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


def _write_json(path, value):
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(value, handle)


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

    def test_release_gate_and_class_counts_preserved(self):
        diff = {
            "summary": {
                "ios": {
                    "total": 0,
                    "differenceClasses": {
                        "core-semantic-difference": 0,
                        "host-capability-difference": 0,
                        "platform-output-missing": 0,
                    },
                },
                "harmony": {
                    "total": 1,
                    "differenceClasses": {
                        "platform-output-missing": 1,
                    },
                },
            },
            "releaseGate": {
                "status": "blocked",
                "missingCandidates": ["harmony"],
                "blockedReasons": ["missing required platform output: harmony"],
            },
        }
        result = brp.derive_diff_summary(diff)
        self.assertFalse(result["match"])
        self.assertEqual(result["total"], 1)
        self.assertEqual(
            result["differenceClasses"]["platform-output-missing"],
            1,
        )
        self.assertEqual(result["releaseGate"]["status"], "blocked")


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

    def test_summary_has_required_fields(self):
        run_dir = _make_run_dir(_complete_files())
        self._track(run_dir)
        out_dir = self._track(tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP))
        os.rmdir(out_dir)

        summary = brp.package_run(run_dir, out_dir=out_dir)

        for field in ("schemaVersion", "tool", "version", "packagedAt", "runId",
                      "runDir", "validation", "manifest", "diffSummary",
                      "environment", "files", "bundle"):
            self.assertIn(field, summary)
        self.assertTrue(summary["validation"]["ok"])
        self.assertEqual(summary["diffSummary"]["total"], 0)
        self.assertTrue(summary["diffSummary"]["match"])
        self.assertIsNone(summary["bundle"]["zip"])
        paths = {entry["path"] for entry in summary["files"]}
        for _key, filename in brp.REQUIRED_ARTIFACTS:
            self.assertIn(filename, paths)
        for entry in summary["files"]:
            self.assertIn("size", entry)
            self.assertIn("sha256", entry)
            self.assertEqual(len(entry["sha256"]), 64)

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

    def test_render_mentions_release_gate_blocked(self):
        run_dir = _make_run_dir(_complete_files())
        out_dir = tempfile.mkdtemp(prefix="brp-out-", dir=PRIVATE_TMP)
        os.rmdir(out_dir)
        try:
            summary = brp.package_run(run_dir, out_dir=out_dir)
            summary["diffSummary"]["releaseGate"] = {
                "status": "blocked",
                "blockedReasons": ["missing required platform output: harmony"],
            }
            text = brp.render_summary(summary)
            self.assertIn("release gate: blocked", text)
            self.assertIn("missing required platform output: harmony", text)
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


if __name__ == "__main__":
    unittest.main()
