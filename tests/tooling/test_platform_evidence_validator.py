"""Tests for the platform evidence validator (tools/platform-evidence-validator).

These tests build synthetic records and files with tempfile so they do not
depend on real reports. Run with:

    python3 -m unittest tests.tooling.test_platform_evidence_validator -v
"""

import importlib.util
import json
import sys
import tempfile
import unittest
from datetime import datetime
from pathlib import Path

_HERE = Path(__file__).resolve().parent
_REPO_ROOT = _HERE.parent.parent
_MODULE_PATH = (
    _REPO_ROOT / "tools" / "platform-evidence-validator" / "platform_evidence_validator.py"
)
_CLI = str(_MODULE_PATH)


def _load_module():
    """Load platform_evidence_validator.py by file path (dir name has a hyphen)."""
    if not _MODULE_PATH.exists():
        raise ImportError(
            "platform_evidence_validator implementation not found at %s. "
            "TDD: write the implementation after the tests." % _MODULE_PATH
        )
    spec = importlib.util.spec_from_file_location(
        "platform_evidence_validator", _MODULE_PATH
    )
    module = importlib.util.module_from_spec(spec)
    sys.modules["platform_evidence_validator"] = module
    spec.loader.exec_module(module)
    return module


pev = _load_module()


def _write(path, content):
    p = Path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    if isinstance(content, bytes):
        p.write_bytes(content)
    else:
        p.write_text(content, encoding="utf-8")
    return p


def _run_cli(args):
    return subprocess_run([sys.executable, _CLI] + [str(a) for a in args])


def _valid_record(**overrides):
    rec = {
        "version": "platform-evidence/1",
        "platform": "ios",
        "kind": "smoke",
        "capability": "reader.search",
        "status": "pass",
        "timestamp": "2026-06-25T08:00:00Z",
        "environment": {"os": "Darwin", "arch": "arm64", "toolchain": "xcode15"},
    }
    rec.update(overrides)
    return rec


# Late import so the helper reads cleanly above; subprocess is std-lib only.
import subprocess  # noqa: E402


def subprocess_run(argv):
    return subprocess.run(argv, capture_output=True, text=True)


class ValidateRecord(unittest.TestCase):
    def test_valid_single_record_has_no_errors(self):
        self.assertEqual(pev.validate_record(_valid_record()), [])

    def test_bad_version_reports_version_error(self):
        rec = _valid_record(version="platform-evidence/2")
        errs = pev.validate_record(rec)
        self.assertTrue(any("version" in e for e in errs), errs)

    def test_missing_version_reports_version_error(self):
        rec = _valid_record()
        rec.pop("version")
        errs = pev.validate_record(rec)
        self.assertTrue(any("version" in e for e in errs), errs)

    def test_bad_platform_enum_reports_error(self):
        rec = _valid_record(platform="windows")
        errs = pev.validate_record(rec)
        self.assertTrue(any("platform" in e for e in errs), errs)

    def test_bad_kind_enum_reports_error(self):
        rec = _valid_record(kind="integration")
        errs = pev.validate_record(rec)
        self.assertTrue(any("kind" in e for e in errs), errs)

    def test_empty_capability_reports_error(self):
        rec = _valid_record(capability="")
        errs = pev.validate_record(rec)
        self.assertTrue(any("capability" in e for e in errs), errs)

    def test_bad_status_enum_reports_error(self):
        rec = _valid_record(status="ok")
        errs = pev.validate_record(rec)
        self.assertTrue(any("status" in e for e in errs), errs)

    def test_malformed_timestamp_reports_error_and_valid_z_ok(self):
        bad = _valid_record(timestamp="not-a-timestamp")
        errs = pev.validate_record(bad)
        self.assertTrue(any("timestamp" in e for e in errs), errs)
        # Empty timestamp is also rejected.
        empty = _valid_record(timestamp="")
        self.assertTrue(any("timestamp" in e for e in pev.validate_record(empty)))
        # Trailing-Z ISO8601 is accepted.
        good = _valid_record(timestamp="2026-06-25T08:00:00Z")
        self.assertEqual(pev.validate_record(good), [])

    def test_corpus_without_fixture_id_errors_and_with_ok(self):
        bad = _valid_record(kind="corpus")
        bad.pop("fixture_id", None)
        errs = pev.validate_record(bad)
        self.assertTrue(any("fixture_id" in e for e in errs), errs)
        good = _valid_record(kind="corpus", fixture_id="corpus-rss-001")
        self.assertEqual(pev.validate_record(good), [])

    def test_unknown_top_level_field_reports_error(self):
        rec = _valid_record(extra_field="boom")
        errs = pev.validate_record(rec)
        self.assertTrue(
            any("unknown field" in e and "extra_field" in e for e in errs), errs
        )

    def test_environment_missing_arch_errors_and_with_os_arch_ok(self):
        bad = _valid_record(environment={"os": "Darwin"})
        errs = pev.validate_record(bad)
        self.assertTrue(
            any("environment" in e and "arch" in e for e in errs), errs
        )
        good = _valid_record(environment={"os": "Darwin", "arch": "arm64"})
        self.assertEqual(pev.validate_record(good), [])

    def test_environment_not_object_reports_error(self):
        rec = _valid_record(environment="Darwin/arm64")
        errs = pev.validate_record(rec)
        self.assertTrue(any("environment" in e for e in errs), errs)

    def test_artifact_and_notes_when_present_must_be_strings(self):
        rec = _valid_record(artifact=123)
        errs = pev.validate_record(rec)
        self.assertTrue(any("artifact" in e for e in errs), errs)
        rec2 = _valid_record(notes=["x"])
        self.assertTrue(any("notes" in e for e in pev.validate_record(rec2)))
        # Strings are fine.
        ok = _valid_record(artifact="logs/smoke.log", notes="all good")
        self.assertEqual(pev.validate_record(ok), [])


class ValidateBatch(unittest.TestCase):
    def test_valid_batch_with_two_records_has_no_errors(self):
        batch = {
            "version": "platform-evidence/1",
            "records": [
                _valid_record(platform="ios"),
                _valid_record(platform="android"),
            ],
        }
        self.assertEqual(pev.validate(batch), [])

    def test_empty_batch_records_reports_error(self):
        batch = {"version": "platform-evidence/1", "records": []}
        errs = pev.validate(batch)
        self.assertTrue(any("records" in e for e in errs), errs)

    def test_batch_record_errors_keyed_by_index(self):
        batch = {
            "version": "platform-evidence/1",
            "records": [
                _valid_record(),
                _valid_record(platform="windows"),
            ],
        }
        errs = pev.validate(batch)
        self.assertTrue(
            any("[records[1]]" in e and "platform" in e for e in errs), errs
        )

    def test_batch_record_version_optional_but_if_present_must_match(self):
        # Record without version is OK inside a batch.
        rec = _valid_record()
        rec.pop("version")
        batch = {"version": "platform-evidence/1", "records": [rec]}
        self.assertEqual(pev.validate(batch), [])
        # Record with a mismatched version is an error.
        rec2 = _valid_record(version="platform-evidence/2")
        batch2 = {"version": "platform-evidence/1", "records": [rec2]}
        errs = pev.validate(batch2)
        self.assertTrue(
            any("[records[0]]" in e and "version" in e for e in errs), errs
        )

    def test_batch_unknown_top_level_field_reports_error(self):
        batch = {
            "version": "platform-evidence/1",
            "records": [_valid_record()],
            "unexpected": True,
        }
        errs = pev.validate(batch)
        self.assertTrue(
            any("unknown field" in e and "unexpected" in e for e in errs), errs
        )

    def test_batch_records_not_a_list_reports_error(self):
        errs = pev.validate(
            {"version": "platform-evidence/1", "records": "nope"}
        )
        self.assertTrue(any("records" in e for e in errs), errs)


class ValidateRoot(unittest.TestCase):
    def test_non_object_root_reports_error(self):
        self.assertEqual(pev.validate([1, 2, 3]), ["root must be a JSON object"])
        self.assertEqual(pev.validate("hello"), ["root must be a JSON object"])
        self.assertEqual(pev.validate(42), ["root must be a JSON object"])


class ValidateFile(unittest.TestCase):
    def test_malformed_json_returns_none_and_invalid_json_error(self):
        with tempfile.TemporaryDirectory() as d:
            p = _write(Path(d) / "broken.json", "{not valid json")
            obj, errs = pev.validate_file(p)
        self.assertIsNone(obj)
        self.assertEqual(len(errs), 1)
        self.assertTrue(errs[0].startswith("invalid JSON:"), errs)

    def test_valid_file_returns_obj_and_no_errors(self):
        with tempfile.TemporaryDirectory() as d:
            p = _write(Path(d) / "ok.json", json.dumps(_valid_record()))
            obj, errs = pev.validate_file(p)
        self.assertIsNotNone(obj)
        self.assertEqual(errs, [])


class ValidateDirAndSummarize(unittest.TestCase):
    def test_validate_dir_recurses_and_summarize_counts(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "a.json", json.dumps(_valid_record()))
            _write(
                root / "sub" / "b.json",
                json.dumps(_valid_record(platform="windows")),
            )
            _write(root / "c.txt", "not json")
            results = pev.validate_dir(root)
        # Only *.json files, recursively.
        paths = sorted(r["path"] for r in results)
        self.assertEqual(paths, ["a.json", "sub/b.json"])
        by_path = {r["path"]: r for r in results}
        self.assertTrue(by_path["a.json"]["valid"])
        self.assertFalse(by_path["sub/b.json"]["valid"])
        summary = pev.summarize(results)
        self.assertEqual(summary, {"total": 2, "valid": 1, "invalid": 1})

    def test_summarize_on_empty_results(self):
        self.assertEqual(
            pev.summarize([]), {"total": 0, "valid": 0, "invalid": 0}
        )


class CLI(unittest.TestCase):
    def test_cli_on_dir_with_valid_and_invalid_exits_one(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "good.json", json.dumps(_valid_record()))
            _write(
                root / "bad.json",
                json.dumps(_valid_record(platform="windows")),
            )
            result = _run_cli([root, "--pretty"])
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertTrue(result.stdout.endswith("\n"))
        data = json.loads(result.stdout)
        self.assertEqual(data["version"], "platform-evidence-validator/1")
        self.assertEqual(data["tool"], "platform-evidence-validator")
        self.assertEqual(
            data["summary"], {"total": 2, "valid": 1, "invalid": 1}
        )
        # --pretty means indent=2 + sorted keys.
        self.assertIn("\n  ", result.stdout)
        top_keys = list(data.keys())
        self.assertEqual(top_keys, sorted(top_keys))

    def test_cli_on_valid_dir_exits_zero(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "good.json", json.dumps(_valid_record()))
            result = _run_cli([root])
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads(result.stdout)
        self.assertEqual(data["summary"], {"total": 1, "valid": 1, "invalid": 0})

    def test_cli_on_single_file_exits_zero_when_valid(self):
        with tempfile.TemporaryDirectory() as d:
            p = _write(Path(d) / "rec.json", json.dumps(_valid_record()))
            result = _run_cli([p])
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads(result.stdout)
        self.assertEqual(data["summary"]["valid"], 1)
        self.assertEqual(data["summary"]["invalid"], 0)

    def test_cli_on_single_invalid_file_exits_one(self):
        with tempfile.TemporaryDirectory() as d:
            p = _write(
                Path(d) / "rec.json",
                json.dumps(_valid_record(platform="windows")),
            )
            result = _run_cli([p])
        self.assertEqual(result.returncode, 1, result.stderr)
        data = json.loads(result.stdout)
        self.assertEqual(data["summary"]["invalid"], 1)

    def test_cli_on_missing_path_exits_two(self):
        result = _run_cli(["/nonexistent-path-xyz-abc-12345"])
        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stdout, "")

    def test_cli_generated_at_is_iso8601(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write(root / "good.json", json.dumps(_valid_record()))
            result = _run_cli([root])
        data = json.loads(result.stdout)
        datetime.fromisoformat(data["generated_at"])


if __name__ == "__main__":
    unittest.main()
