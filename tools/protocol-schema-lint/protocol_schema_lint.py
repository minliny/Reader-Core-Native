#!/usr/bin/env python3
"""Protocol schema fixture linter.

Batch-validates existing protocol fixtures under
``protocol/fixtures/conformance/`` against the command / event / runtime-config
JSON schemas. LINT ONLY — never modifies schemas or fixtures.

A minimal hand-rolled JSON Schema validator covers only the subset actually
used by the real schemas: ``type``, ``required``, ``properties``,
``additionalProperties: false``, ``enum``, ``const``, ``oneOf``/``anyOf``/
``allOf``, ``if``/``then``/``else``, ``items``, ``minimum``/``maximum``,
``minLength``, ``pattern`` and ``$ref`` (internal ``#/...`` and cross-file
``sibling.schema.json#/...``). Pure Python 3.9+ standard library.

CLI:
    python3 tools/protocol-schema-lint/protocol_schema_lint.py [root] [--pretty]
"""

import argparse
import json
import os
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

VERSION = "protocol-schema-lint/1"
TOOL = "protocol-schema-fixture-linter"

SCHEMA_FILES = {
    "command": "reader-command.schema.json",
    "event": "reader-event.schema.json",
    "runtime-config": "reader-runtime-config.schema.json",
}
CONFORMANCE_DIR = os.path.join("protocol", "fixtures", "conformance")

_schema_cache = {}


# --------------------------------------------------------------------------- #
# Schema loading
# --------------------------------------------------------------------------- #
def load_schema(path):
    """Parse a schema file into a dict (cached by absolute path)."""
    key = str(Path(path).resolve())
    if key not in _schema_cache:
        with open(key, "r", encoding="utf-8") as fh:
            _schema_cache[key] = json.load(fh)
    return _schema_cache[key]


def _find_protocol_dir(fixture_path):
    """Walk up from ``fixture_path`` to locate the ``protocol/`` directory
    that contains ``fixtures/conformance/``. Returns an absolute Path or None.
    """
    p = Path(fixture_path).resolve()
    for parent in [p] + list(p.parents):
        if (parent.name == "conformance"
                and parent.parent.name == "fixtures"
                and parent.parent.parent.name == "protocol"):
            return parent.parent.parent
    return None


# --------------------------------------------------------------------------- #
# Schema selection
# --------------------------------------------------------------------------- #
def select_schema(fixture_path, fixture_obj):
    """Pick one of "command" | "event" | "runtime-config" | None.

    Content markers win (method → command; host.complete/host.error method or
    an ``event``/``type`` field → event; dataDirectory/cacheDirectory/
    directories → runtime-config), then path-based fallback.
    """
    path_str = str(fixture_path).replace(os.sep, "/")
    if isinstance(fixture_obj, dict):
        method = fixture_obj.get("method")
        if isinstance(method, str):
            if method.startswith("host.complete") or method.startswith("host.error"):
                return "event"
            return "command"
        if "event" in fixture_obj or "type" in fixture_obj:
            return "event"
        if any(k in fixture_obj
               for k in ("dataDirectory", "cacheDirectory", "directories")):
            return "runtime-config"
    # path-based fallback (also covers malformed-JSON fixtures where obj is None)
    if "conformance/configs/" in path_str:
        return "runtime-config"
    if "conformance/cancel/" in path_str:
        return "event"
    if "conformance/commands/" in path_str:
        return "command"
    if "conformance/host/" in path_str:
        return "command"
    return None


# --------------------------------------------------------------------------- #
# Minimal JSON Schema validator
# --------------------------------------------------------------------------- #
def _type_name(obj):
    if obj is None:
        return "null"
    if isinstance(obj, bool):
        return "boolean"
    if isinstance(obj, int):
        return "integer"
    if isinstance(obj, float):
        return "number"
    if isinstance(obj, str):
        return "string"
    if isinstance(obj, list):
        return "array"
    if isinstance(obj, dict):
        return "object"
    return type(obj).__name__


def _check_type(obj, t):
    if isinstance(t, list):
        return any(_check_type(obj, item) for item in t)
    if t == "object":
        return isinstance(obj, dict)
    if t == "array":
        return isinstance(obj, list)
    if t == "string":
        return isinstance(obj, str)
    if t == "integer":
        return isinstance(obj, int) and not isinstance(obj, bool)
    if t == "number":
        return isinstance(obj, (int, float)) and not isinstance(obj, bool)
    if t == "boolean":
        return isinstance(obj, bool)
    if t == "null":
        return obj is None
    return True  # unknown type keyword — accept


def _const_eq(a, b):
    """Strict equality that distinguishes bool from int (JSON Schema const)."""
    if isinstance(a, bool) or isinstance(b, bool):
        return isinstance(a, bool) and isinstance(b, bool) and a == b
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return a == b
    if type(a) is not type(b):
        return False
    return a == b


def _resolve_ref(ref, root, base_dir):
    """Resolve a $ref. Returns (sub_schema, resolved_root) or (None, None).

    Supports internal ``#/path`` refs (against ``root``) and cross-file
    ``sibling.schema.json#/path`` refs (loaded from ``base_dir``).
    """
    if ref.startswith("#/"):
        node = root
        for part in ref[2:].split("/"):
            if isinstance(node, dict) and part in node:
                node = node[part]
            else:
                return None, None
        return node, root
    if "#" in ref:
        file_part, frag = ref.split("#", 1)
        if file_part and base_dir is not None:
            sibling = os.path.join(str(base_dir), file_part)
            try:
                sib = load_schema(sibling)
            except (OSError, json.JSONDecodeError):
                return None, None
            node = sib
            for part in frag.lstrip("/").split("/"):
                if part == "":
                    break
                if isinstance(node, dict) and part in node:
                    node = node[part]
                else:
                    return None, None
            return node, sib
    return None, None


def validate(obj, schema, base_dir=None):
    """Validate ``obj`` against ``schema``. Returns a list of path-qualified
    error strings (empty list means valid). ``base_dir`` enables cross-file
    ``$ref`` resolution; internal ``#/...`` refs resolve against ``schema``.
    """
    return _validate(obj, schema, "$", schema, base_dir)


def _validate(obj, schema, path, root, base_dir):
    if not isinstance(schema, dict):
        return []

    errors = []

    if "$ref" in schema:
        resolved, resolved_root = _resolve_ref(schema["$ref"], root, base_dir)
        if resolved is None:
            errors.append("%s: could not resolve $ref %s" % (path, schema["$ref"]))
            return errors
        return _validate(obj, resolved, path, resolved_root, base_dir)

    if "type" in schema and not _check_type(obj, schema["type"]):
        errors.append("%s: expected type %s, got %s"
                      % (path, schema["type"], _type_name(obj)))

    if "const" in schema and not _const_eq(obj, schema["const"]):
        errors.append("%s: expected const %s"
                      % (path, json.dumps(schema["const"])))

    if "enum" in schema and not any(_const_eq(obj, v) for v in schema["enum"]):
        errors.append("%s: %s not in enum"
                      % (path, json.dumps(obj)))

    if "minLength" in schema and isinstance(obj, str) and len(obj) < schema["minLength"]:
        errors.append("%s: string shorter than minLength %d"
                      % (path, schema["minLength"]))

    if "pattern" in schema and isinstance(obj, str):
        try:
            matched = re.search(schema["pattern"], obj) is not None
        except re.error as exc:
            errors.append("%s: invalid regex pattern %s: %s"
                          % (path, json.dumps(schema["pattern"]), exc))
            matched = True
        if not matched:
            errors.append("%s: string does not match pattern %s"
                          % (path, json.dumps(schema["pattern"])))

    if (not isinstance(obj, bool)) and isinstance(obj, (int, float)):
        if "minimum" in schema and obj < schema["minimum"]:
            errors.append("%s: number below minimum %s"
                          % (path, json.dumps(schema["minimum"])))
        if "maximum" in schema and obj > schema["maximum"]:
            errors.append("%s: number above maximum %s"
                          % (path, json.dumps(schema["maximum"])))

    if isinstance(obj, dict):
        for req in schema.get("required", []):
            if req not in obj:
                errors.append("%s: missing required property '%s'" % (path, req))
        props = schema.get("properties", {})
        for key, val in obj.items():
            if key in props:
                errors.extend(_validate(val, props[key], "%s.%s" % (path, key),
                                        root, base_dir))
            elif schema.get("additionalProperties") is False:
                errors.append("%s: additional property '%s' not allowed"
                              % (path, key))

    if isinstance(obj, list):
        items = schema.get("items")
        if isinstance(items, dict):
            for i, item in enumerate(obj):
                errors.extend(_validate(item, items, "%s[%d]" % (path, i),
                                        root, base_dir))

    if "oneOf" in schema:
        matches = 0
        for sub in schema["oneOf"]:
            if not _validate(obj, sub, path, root, base_dir):
                matches += 1
        if matches != 1:
            errors.append("%s: oneOf matched %d branches (expected 1)"
                          % (path, matches))

    if "anyOf" in schema:
        if not any(not _validate(obj, sub, path, root, base_dir)
                   for sub in schema["anyOf"]):
            errors.append("%s: anyOf matched no branches" % path)

    if "if" in schema:
        condition_errors = _validate(obj, schema["if"], path, root, base_dir)
        if not condition_errors:
            then_schema = schema.get("then")
            if isinstance(then_schema, dict):
                errors.extend(_validate(obj, then_schema, path, root, base_dir))
        else:
            else_schema = schema.get("else")
            if isinstance(else_schema, dict):
                errors.extend(_validate(obj, else_schema, path, root, base_dir))

    if "allOf" in schema:
        for sub in schema["allOf"]:
            errors.extend(_validate(obj, sub, path, root, base_dir))

    return errors


# --------------------------------------------------------------------------- #
# Linting
# --------------------------------------------------------------------------- #
def lint_fixture(path, schema_dir=None, root=None):
    """Lint a single fixture file.

    Returns ``{"path", "schema", "valid", "expected_invalid", "errors"}``.
    ``schema_dir`` is the directory holding the schema files (inferred from
    ``path`` if omitted). ``root`` is used to compute the relative ``path``
    field (otherwise the raw path is used).
    """
    p = Path(path)
    name = p.name
    expected_invalid = name.startswith("invalid-")

    if root is not None:
        try:
            rel = str(p.relative_to(Path(root))).replace(os.sep, "/")
        except ValueError:
            rel = str(p)
    else:
        rel = str(p)

    if schema_dir is None:
        proto = _find_protocol_dir(p)
        schema_dir = proto if proto is not None else None

    raw = None
    try:
        with open(p, "r", encoding="utf-8") as fh:
            raw = fh.read()
    except OSError as exc:
        return {
            "path": rel,
            "schema": "none",
            "valid": False,
            "expected_invalid": expected_invalid,
            "errors": ["could not read file: %s" % exc],
        }

    try:
        obj = json.loads(raw)
    except (json.JSONDecodeError, ValueError) as exc:
        schema_name = select_schema(rel, None) or "none"
        return {
            "path": rel,
            "schema": schema_name,
            "valid": False,
            "expected_invalid": expected_invalid,
            "errors": ["invalid JSON: %s" % exc],
        }

    schema_name = select_schema(rel, obj)
    if schema_name is None:
        return {
            "path": rel,
            "schema": "none",
            "valid": True,
            "expected_invalid": expected_invalid,
            "errors": [],
        }

    try:
        schema = load_schema(os.path.join(str(schema_dir), SCHEMA_FILES[schema_name]))
    except (OSError, json.JSONDecodeError) as exc:
        return {
            "path": rel,
            "schema": schema_name,
            "valid": False,
            "expected_invalid": expected_invalid,
            "errors": ["could not load schema %s: %s" % (schema_name, exc)],
        }

    errors = validate(obj, schema, base_dir=str(schema_dir) if schema_dir else None)
    return {
        "path": rel,
        "schema": schema_name,
        "valid": len(errors) == 0,
        "expected_invalid": expected_invalid,
        "errors": errors,
    }


def lint_dir(root):
    """Lint every ``*.json`` under ``<root>/protocol/fixtures/conformance/``."""
    root = Path(root)
    conformance = root / CONFORMANCE_DIR
    schema_dir = root / "protocol"
    results = []
    if not conformance.is_dir():
        return results
    for dirpath, _dirnames, filenames in os.walk(str(conformance)):
        for fn in sorted(filenames):
            if not fn.endswith(".json"):
                continue
            fp = Path(dirpath) / fn
            results.append(lint_fixture(fp, schema_dir=str(schema_dir),
                                        root=str(root)))
    results.sort(key=lambda r: r["path"])
    return results


def summarize(results):
    """Summarise lint results into conformance counts."""
    total = len(results)
    valid = sum(1 for r in results if r["valid"])
    invalid = total - valid
    expected_invalid = 0
    unexpected_invalid = 0
    unexpected_valid = 0
    for r in results:
        base = os.path.basename(r["path"])
        is_invalid_named = base.startswith("invalid-")
        is_valid_named = base.startswith("valid-")
        if is_invalid_named and not r["valid"]:
            expected_invalid += 1
        if is_valid_named and not r["valid"]:
            unexpected_invalid += 1
        if is_invalid_named and r["valid"]:
            unexpected_valid += 1
    return {
        "total": total,
        "valid": valid,
        "invalid": invalid,
        "expected_invalid": expected_invalid,
        "unexpected_invalid": unexpected_invalid,
        "unexpected_valid": unexpected_valid,
    }


def build_report(root):
    results = lint_dir(root)
    summary = summarize(results)
    return {
        "version": VERSION,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "tool": TOOL,
        "results": results,
        "summary": summary,
    }


# --------------------------------------------------------------------------- #
# CLI
# --------------------------------------------------------------------------- #
def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="protocol_schema_lint",
        description="Lint protocol conformance fixtures against JSON schemas.",
    )
    parser.add_argument("root", nargs="?", default=".",
                        help="repo root to scan (default: cwd)")
    parser.add_argument("--pretty", action="store_true",
                        help="indent=2, sorted keys, trailing newline")
    args = parser.parse_args(argv)

    root = Path(args.root)
    if not root.is_dir():
        sys.stderr.write(
            "error: root not found or not a directory: %s\n" % root
        )
        return 2
    try:
        report = build_report(str(root))
    except OSError as exc:
        sys.stderr.write("error: %s\n" % exc)
        return 2

    if args.pretty:
        out = json.dumps(report, indent=2, sort_keys=True)
    else:
        out = json.dumps(report)
    sys.stdout.write(out + "\n")

    summary = report["summary"]
    if summary["unexpected_invalid"] != 0 or summary["unexpected_valid"] != 0:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
