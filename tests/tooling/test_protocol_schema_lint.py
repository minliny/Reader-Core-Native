"""Tests for the protocol schema fixture linter (tools/protocol-schema-lint).

These tests build synthetic tempdirs with fake schemas + fixtures so they do
not depend on the real repo corpus. One integration test points at the real
protocol/ directory (read-only). Run with:

    python3 -m unittest tests.tooling.test_protocol_schema_lint -v
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
_MODULE_PATH = _REPO_ROOT / "tools" / "protocol-schema-lint" / "protocol_schema_lint.py"
_CLI = str(_MODULE_PATH)


def _load_module():
    """Load protocol_schema_lint.py by file path (the dir name has a hyphen)."""
    if not _MODULE_PATH.exists():
        raise ImportError(
            "protocol_schema_lint implementation not found at %s. "
            "TDD: write the implementation after the tests." % _MODULE_PATH
        )
    spec = importlib.util.spec_from_file_location("protocol_schema_lint", _MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    sys.modules["protocol_schema_lint"] = module
    spec.loader.exec_module(module)
    return module


psl = _load_module()


def _write(path, content):
    p = Path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    if isinstance(content, bytes):
        p.write_bytes(content)
    else:
        p.write_text(content, encoding="utf-8")
    return p


FAKE_COMMAND_SCHEMA = {
    "type": "object",
    "additionalProperties": False,
    "required": ["protocolVersion", "requestId", "method"],
    "properties": {
        "protocolVersion": {"type": "integer", "const": 1},
        "requestId": {"type": "integer"},
        "method": {"type": "string"},
        "params": {"type": "object"},
    },
}

FAKE_EVENT_SCHEMA = {
    "oneOf": [
        {"$ref": "#/$defs/ResultEvent"},
        {"$ref": "#/$defs/ErrorEvent"},
    ],
    "$defs": {
        "ResultEvent": {
            "type": "object",
            "additionalProperties": False,
            "required": ["protocolVersion", "requestId", "type", "data"],
            "properties": {
                "protocolVersion": {"type": "integer", "const": 1},
                "requestId": {"type": "integer"},
                "type": {"type": "string", "const": "result"},
                "data": {"type": "object"},
            },
        },
        "ErrorEvent": {
            "type": "object",
            "additionalProperties": False,
            "required": ["protocolVersion", "requestId", "type", "error"],
            "properties": {
                "protocolVersion": {"type": "integer", "const": 1},
                "requestId": {"type": "integer"},
                "type": {"type": "string", "const": "error"},
                "error": {"$ref": "#/$defs/CoreError"},
            },
        },
        "CoreError": {
            "type": "object",
            "additionalProperties": False,
            "required": ["code", "message", "retryable"],
            "properties": {
                "code": {"type": "string", "enum": ["INTERNAL", "CANCELLED"]},
                "message": {"type": "string"},
                "retryable": {"type": "boolean"},
            },
        },
    },
}

FAKE_CONFIG_SCHEMA = {
    "type": "object",
    "additionalProperties": False,
    "properties": {
        "dataDirectory": {"type": "string", "minLength": 1},
        "cacheDirectory": {"type": "string", "minLength": 1},
    },
}


def _write_schemas(root):
    proto = Path(root) / "protocol"
    _write(proto / "reader-command.schema.json", json.dumps(FAKE_COMMAND_SCHEMA))
    _write(proto / "reader-event.schema.json", json.dumps(FAKE_EVENT_SCHEMA))
    _write(proto / "reader-runtime-config.schema.json", json.dumps(FAKE_CONFIG_SCHEMA))
    return proto


def _run_cli(args):
    return subprocess.run(
        [sys.executable, _CLI] + [str(a) for a in args],
        capture_output=True,
        text=True,
    )


class ValidateTests(unittest.TestCase):
    def test_validate_accepts_valid_object(self):
        schema = {
            "type": "object",
            "required": ["a"],
            "properties": {"a": {"type": "string"}},
            "additionalProperties": False,
        }
        self.assertEqual(psl.validate({"a": "x"}, schema), [])

    def test_validate_rejects_wrong_type(self):
        schema = {"type": "object"}
        errs = psl.validate("not an object", schema)
        self.assertTrue(errs)
        self.assertIn("expected type object", errs[0])

    def test_validate_rejects_missing_required_field(self):
        schema = {
            "type": "object",
            "required": ["a"],
            "properties": {"a": {"type": "string"}},
        }
        errs = psl.validate({}, schema)
        self.assertTrue(any("missing required" in e for e in errs))
        self.assertTrue(any("'a'" in e for e in errs))

    def test_validate_rejects_unknown_field_additional_properties_false(self):
        schema = {
            "type": "object",
            "properties": {"a": {"type": "string"}},
            "additionalProperties": False,
        }
        errs = psl.validate({"a": "x", "b": 1}, schema)
        self.assertTrue(any("additional property" in e for e in errs))
        self.assertTrue(any("b" in e for e in errs))

    def test_validate_rejects_bad_enum(self):
        schema = {"type": "string", "enum": ["A", "B"]}
        errs = psl.validate("C", schema)
        self.assertTrue(errs)
        self.assertTrue(any("enum" in e.lower() for e in errs))

    def test_validate_rejects_bad_const(self):
        schema = {"type": "integer", "const": 1}
        errs = psl.validate(2, schema)
        self.assertTrue(errs)
        self.assertTrue(any("const" in e.lower() for e in errs))

    def test_one_of_picks_right_branch_and_reports_when_none_match(self):
        schema = {
            "oneOf": [
                {"type": "object", "required": ["a"],
                 "properties": {"a": {"type": "string"}}},
                {"type": "object", "required": ["b"],
                 "properties": {"b": {"type": "integer"}}},
            ]
        }
        self.assertEqual(psl.validate({"a": "x"}, schema), [])
        self.assertEqual(psl.validate({"b": 2}, schema), [])
        none_errs = psl.validate({"c": 1}, schema)
        self.assertTrue(any("oneOf" in e for e in none_errs))
        both_errs = psl.validate({"a": "x", "b": 2}, schema)
        self.assertTrue(any("oneOf" in e for e in both_errs))

    def test_validate_rejects_short_string_min_length(self):
        schema = {"type": "string", "minLength": 1}
        errs = psl.validate("", schema)
        self.assertTrue(any("minLength" in e for e in errs))

    def test_validate_resolves_internal_ref(self):
        schema = {
            "type": "object",
            "required": ["err"],
            "properties": {"err": {"$ref": "#/$defs/CoreError"}},
            "$defs": {
                "CoreError": {
                    "type": "object",
                    "required": ["code"],
                    "properties": {
                        "code": {"type": "string", "enum": ["INTERNAL"]},
                    },
                },
            },
        }
        self.assertEqual(psl.validate({"err": {"code": "INTERNAL"}}, schema), [])
        errs = psl.validate({"err": {"code": "BOGUS"}}, schema)
        self.assertTrue(any("enum" in e.lower() for e in errs))

    def test_validate_integer_rejects_bool(self):
        # bool is a subclass of int in Python; must not satisfy type: integer.
        schema = {"type": "integer"}
        errs = psl.validate(True, schema)
        self.assertTrue(errs)


class SelectSchemaTests(unittest.TestCase):
    def test_select_schema_picks_by_method_and_fields(self):
        self.assertEqual(
            psl.select_schema("protocol/fixtures/conformance/commands/x.json",
                              {"method": "core.info"}),
            "command",
        )
        self.assertEqual(
            psl.select_schema("protocol/fixtures/conformance/host/x.json",
                              {"method": "host.complete"}),
            "event",
        )
        self.assertEqual(
            psl.select_schema("protocol/fixtures/conformance/host/x.json",
                              {"method": "host.error"}),
            "event",
        )
        self.assertEqual(
            psl.select_schema("protocol/fixtures/conformance/host/x.json",
                              {"method": "runtime.hostSmoke"}),
            "command",
        )
        self.assertEqual(
            psl.select_schema("protocol/fixtures/conformance/configs/x.json",
                              {"dataDirectory": "/x"}),
            "runtime-config",
        )
        self.assertEqual(
            psl.select_schema("protocol/fixtures/conformance/configs/x.json", {}),
            "runtime-config",
        )
        self.assertEqual(
            psl.select_schema("protocol/fixtures/conformance/cancel/x.json",
                              {"requestId": 1}),
            "event",
        )
        self.assertEqual(
            psl.select_schema("protocol/fixtures/conformance/commands/x.json", None),
            "command",
        )
        self.assertEqual(
            psl.select_schema("protocol/fixtures/conformance/elsewhere/x.json",
                              {"type": "result"}),
            "event",
        )
        self.assertIsNone(
            psl.select_schema("protocol/fixtures/conformance/elsewhere/x.json",
                              {"random": 1})
        )


class LintFixtureTests(unittest.TestCase):
    def test_lint_fixture_valid_passes(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write_schemas(root)
            p = _write(
                root / "protocol" / "fixtures" / "conformance" / "commands" /
                "valid-ping.json",
                json.dumps({"protocolVersion": 1, "requestId": 1,
                            "method": "runtime.ping", "params": {}}),
            )
            result = psl.lint_fixture(str(p), schema_dir=str(root / "protocol"),
                                      root=str(root))
        self.assertTrue(result["valid"], result)
        self.assertFalse(result["expected_invalid"])
        self.assertEqual(result["schema"], "command")
        self.assertEqual(result["errors"], [])

    def test_lint_fixture_invalid_fails(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write_schemas(root)
            p = _write(
                root / "protocol" / "fixtures" / "conformance" / "commands" /
                "invalid-no-id.json",
                json.dumps({"protocolVersion": 1, "method": "runtime.ping"}),
            )
            result = psl.lint_fixture(str(p), schema_dir=str(root / "protocol"),
                                      root=str(root))
        self.assertFalse(result["valid"])
        self.assertTrue(result["expected_invalid"])
        self.assertTrue(len(result["errors"]) > 0)

    def test_malformed_json_fixture_handled(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write_schemas(root)
            p = _write(
                root / "protocol" / "fixtures" / "conformance" / "commands" /
                "invalid-malformed.json",
                '{"protocolVersion":1,',
            )
            result = psl.lint_fixture(str(p), schema_dir=str(root / "protocol"),
                                      root=str(root))
        self.assertFalse(result["valid"])
        self.assertTrue(any("invalid JSON" in e for e in result["errors"]),
                        result["errors"])
        self.assertTrue(result["expected_invalid"])


class LintDirTests(unittest.TestCase):
    def test_lint_dir_mixed_fixtures(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write_schemas(root)
            _write(
                root / "protocol" / "fixtures" / "conformance" / "commands" /
                "valid-ping.json",
                json.dumps({"protocolVersion": 1, "requestId": 1,
                            "method": "runtime.ping", "params": {}}),
            )
            _write(
                root / "protocol" / "fixtures" / "conformance" / "commands" /
                "invalid-bad.json",
                json.dumps({"protocolVersion": 1, "method": "x"}),
            )
            _write(
                root / "protocol" / "fixtures" / "conformance" / "configs" /
                "valid-empty.json",
                "{}",
            )
            results = psl.lint_dir(str(root))
        paths = [r["path"] for r in results]
        self.assertIn("protocol/fixtures/conformance/commands/valid-ping.json",
                      paths)
        self.assertIn("protocol/fixtures/conformance/commands/invalid-bad.json",
                      paths)
        self.assertIn("protocol/fixtures/conformance/configs/valid-empty.json",
                      paths)
        self.assertEqual(len(results), 3)


class SummarizeTests(unittest.TestCase):
    def test_summarize_counts_unexpected(self):
        results = [
            {"path": "valid-bad.json", "schema": "command", "valid": False,
             "expected_invalid": False, "errors": ["x"]},
            {"path": "invalid-good.json", "schema": "command", "valid": True,
             "expected_invalid": True, "errors": []},
            {"path": "valid-ok.json", "schema": "command", "valid": True,
             "expected_invalid": False, "errors": []},
            {"path": "invalid-bad.json", "schema": "command", "valid": False,
             "expected_invalid": True, "errors": ["y"]},
        ]
        s = psl.summarize(results)
        self.assertEqual(s["total"], 4)
        self.assertEqual(s["valid"], 2)
        self.assertEqual(s["invalid"], 2)
        self.assertEqual(s["expected_invalid"], 1)
        self.assertEqual(s["unexpected_invalid"], 1)
        self.assertEqual(s["unexpected_valid"], 1)


class CLITests(unittest.TestCase):
    def test_cli_pretty_exit_zero_when_all_behave(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write_schemas(root)
            _write(
                root / "protocol" / "fixtures" / "conformance" / "commands" /
                "valid-ping.json",
                json.dumps({"protocolVersion": 1, "requestId": 1,
                            "method": "runtime.ping", "params": {}}),
            )
            _write(
                root / "protocol" / "fixtures" / "conformance" / "commands" /
                "invalid-bad.json",
                json.dumps({"protocolVersion": 1, "method": "x"}),
            )
            result = _run_cli([str(root), "--pretty"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(result.stdout.endswith("\n"))
        self.assertIn("\n  ", result.stdout)
        data = json.loads(result.stdout)
        self.assertEqual(data["version"], "protocol-schema-lint/1")
        self.assertEqual(data["tool"], "protocol-schema-fixture-linter")
        self.assertEqual(data["summary"]["unexpected_invalid"], 0)
        self.assertEqual(data["summary"]["unexpected_valid"], 0)
        top_keys = list(data.keys())
        self.assertEqual(top_keys, sorted(top_keys))

    def test_cli_exit_one_when_valid_fixture_fails(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write_schemas(root)
            _write(
                root / "protocol" / "fixtures" / "conformance" / "commands" /
                "valid-bad.json",
                json.dumps({"protocolVersion": 1, "method": "x"}),
            )
            result = _run_cli([str(root), "--pretty"])
        self.assertEqual(result.returncode, 1, result.stderr)
        data = json.loads(result.stdout)
        self.assertEqual(data["summary"]["unexpected_invalid"], 1)

    def test_cli_missing_root_exits_two(self):
        result = _run_cli(["/nonexistent-path-xyz-abc-12345"])
        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stdout, "")


class IntegrationTests(unittest.TestCase):
    def test_real_protocol_conformance_fixtures_behave_as_named(self):
        results = psl.lint_dir(str(_REPO_ROOT))
        self.assertTrue(len(results) > 0,
                        "no conformance fixtures found in real protocol/")
        for r in results:
            base = os.path.basename(r["path"])
            if base.startswith("valid-"):
                self.assertTrue(
                    r["valid"],
                    "valid-* fixture %s should pass: %s"
                    % (r["path"], r["errors"]),
                )
            elif base.startswith("invalid-"):
                self.assertFalse(
                    r["valid"],
                    "invalid-* fixture %s should fail" % r["path"],
                )
        summary = psl.summarize(results)
        self.assertEqual(summary["unexpected_invalid"], 0,
                         "unexpected invalid: %s" % summary)
        self.assertEqual(summary["unexpected_valid"], 0,
                         "unexpected valid: %s" % summary)


if __name__ == "__main__":
    unittest.main()
