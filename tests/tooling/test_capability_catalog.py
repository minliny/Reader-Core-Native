"""Tests for tools/capability-catalog/capability_catalog.py.

Strict TDD: tests written BEFORE the implementation. Pure parsers are
exercised with synthetic content strings; the collect() orchestrator is
exercised with small tempdirs so tests do not depend on the real repo layout.

Run:
    python3 -m unittest tests.tooling.test_capability_catalog -v
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
_MODULE_PATH = _REPO_ROOT / "tools" / "capability-catalog" / "capability_catalog.py"
_CLI = str(_MODULE_PATH)


def _load_module():
    """Load capability_catalog.py by file path (the dir name has a hyphen)."""
    if not _MODULE_PATH.exists():
        raise ImportError(
            "capability_catalog implementation not found at %s. "
            "TDD: write the implementation after the tests." % _MODULE_PATH
        )
    spec = importlib.util.spec_from_file_location("capability_catalog", _MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    sys.modules["capability_catalog"] = module
    spec.loader.exec_module(module)
    return module


cc = _load_module()


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
# 1 & 2. parse_methods_from_schema: const + enum extracted; no methods -> []
# ---------------------------------------------------------------------------
class ParseMethodsFromSchemaTests(unittest.TestCase):
    def test_const_and_enum_methods_extracted_sorted_deduped(self):
        schema = json.dumps({
            "$defs": {
                "A": {"properties": {"method": {"const": "book.search"}}},
                "B": {"properties": {
                    "method": {"enum": ["book.detail", "book.toc", "book.search"]}
                }},
            }
        })
        methods = cc.parse_methods_from_schema(schema)
        self.assertEqual(methods, ["book.detail", "book.search", "book.toc"])

    def test_no_methods_returns_empty(self):
        self.assertEqual(cc.parse_methods_from_schema('{"type": "object"}'), [])
        self.assertEqual(cc.parse_methods_from_schema("not json at all"), [])

    def test_extracts_examples_capabilities_array_and_type_const(self):
        # Mirrors the real schema shapes: method.examples, the
        # x-reader-core-v1-capabilities array, and type.const event types
        # like host.request (result/error excluded: no dot).
        schema = json.dumps({
            "x-reader-core-v1-capabilities": ["core.info", "http.execute"],
            "properties": {
                "method": {"type": "string", "examples": ["book.search", "core.info"]}
            },
            "$defs": {
                "Evt": {"properties": {"type": {"const": "host.request"}}},
                "Other": {"properties": {"type": {"const": "result"}}},
            },
        })
        methods = cc.parse_methods_from_schema(schema)
        self.assertIn("book.search", methods)
        self.assertIn("core.info", methods)
        self.assertIn("http.execute", methods)
        self.assertIn("host.request", methods)
        self.assertNotIn("result", methods)
        self.assertEqual(methods, sorted(methods))


# ---------------------------------------------------------------------------
# 3. parse_feature_matrix: owner/status mapping + evidence
# ---------------------------------------------------------------------------
class ParseFeatureMatrixTests(unittest.TestCase):
    def test_owner_status_mapping_and_evidence(self):
        md = (
            "| Name | Owner | Status |\n"
            "|------|-------|--------|\n"
            "| book.search | Rust Core | 已完成 |\n"
            "| tls.socket | 平台负责 | Gap |\n"
            "| chapter.content | Rust Core | 部分完成 |\n"
        )
        rows = cc.parse_feature_matrix(md)
        self.assertEqual(len(rows), 3)
        by_name = {r["name"]: r for r in rows}
        self.assertEqual(by_name["book.search"]["owner"], "core")
        self.assertEqual(by_name["book.search"]["status"], "implemented")
        self.assertEqual(by_name["tls.socket"]["owner"], "host")
        self.assertEqual(by_name["tls.socket"]["status"], "missing")
        self.assertEqual(by_name["chapter.content"]["owner"], "core")
        self.assertEqual(by_name["chapter.content"]["status"], "partial")
        self.assertIn("已完成", by_name["book.search"]["evidence"])

    def test_checkmark_columns_map_to_owner(self):
        # Real FEATURE_MATRIX shape: header has Rust Core / Platform Adapter
        # columns; rows mark ownership with a checkmark in the column.
        md = (
            "| 能力 | Rust Core | Platform Adapter |\n"
            "|------|:---------:|:----------------:|\n"
            "| Search rules | ✅ | | |\n"
            "| TLS socket | | ✅ |\n"
        )
        rows = cc.parse_feature_matrix(md)
        by_name = {r["name"]: r for r in rows}
        self.assertEqual(by_name["Search rules"]["owner"], "core")
        self.assertEqual(by_name["TLS socket"]["owner"], "host")


# ---------------------------------------------------------------------------
# 4. parse_host_contracts: heading + code-block tokens extracted
# ---------------------------------------------------------------------------
class ParseHostContractsTests(unittest.TestCase):
    def test_heading_and_code_tokens_extracted(self):
        text = (
            "# Host HTTP Contract\n\n"
            "## http.execute\n\n"
            "```json\n"
            '{"method": "host.complete"}\n'
            "```\n"
        )
        names = cc.parse_host_contracts([("docs/host-app-contracts/http.md", text)])
        self.assertIn("host.complete", names)
        self.assertIn("http.execute", names)

    def test_empty_iterable_returns_empty(self):
        self.assertEqual(cc.parse_host_contracts([]), [])


# ---------------------------------------------------------------------------
# 5. collect: schema + matrix owner resolution
# ---------------------------------------------------------------------------
class CollectOwnerResolutionTests(unittest.TestCase):
    def test_schema_and_matrix_owner_resolution(self):
        with tempfile.TemporaryDirectory() as d:
            _write(Path(d) / "protocol" / "reader-command.schema.json", json.dumps({
                "$defs": {
                    "A": {"properties": {"method": {"const": "book.search"}}},
                    "B": {"properties": {"method": {"const": "source.import"}}},
                }
            }))
            _write(Path(d) / "FEATURE_MATRIX.md", (
                "| Name | Owner | Status |\n"
                "|------|-------|--------|\n"
                "| book.search | Rust Core | 部分完成 |\n"
                "| tls.socket | 平台负责 | Gap |\n"
            ))
            catalog = cc.collect(d)
        by_id = {c["id"]: c for c in catalog["capabilities"]}
        # book.search: schema + matrix "Rust Core" -> core, partial
        self.assertEqual(by_id["book.search"]["owner"], "core")
        self.assertEqual(by_id["book.search"]["status"], "partial")
        self.assertIn("FEATURE_MATRIX.md", by_id["book.search"]["evidence"])
        self.assertIn(
            "protocol/reader-command.schema.json",
            by_id["book.search"]["evidence"],
        )
        # source.import: schema-only -> core, implemented
        self.assertEqual(by_id["source.import"]["owner"], "core")
        self.assertEqual(by_id["source.import"]["status"], "implemented")
        # tls.socket: matrix-only "平台负责" -> host, missing
        self.assertEqual(by_id["tls.socket"]["owner"], "host")
        self.assertEqual(by_id["tls.socket"]["status"], "missing")


# ---------------------------------------------------------------------------
# 6. collect: missing docs/host-app-contracts -> no crash
# ---------------------------------------------------------------------------
class CollectMissingHostContractsTests(unittest.TestCase):
    def test_no_crash_when_host_contracts_absent(self):
        with tempfile.TemporaryDirectory() as d:
            _write(Path(d) / "protocol" / "reader-command.schema.json", json.dumps({
                "$defs": {"A": {"properties": {"method": {"const": "book.search"}}}}
            }))
            catalog = cc.collect(d)
        self.assertIsInstance(catalog["capabilities"], list)
        by_id = {c["id"]: c for c in catalog["capabilities"]}
        self.assertIn("book.search", by_id)


# ---------------------------------------------------------------------------
# 7. collect: empty tempdir -> empty catalog
# ---------------------------------------------------------------------------
class CollectEmptyTests(unittest.TestCase):
    def test_empty_tempdir_empty_catalog(self):
        with tempfile.TemporaryDirectory() as d:
            catalog = cc.collect(d)
        self.assertEqual(catalog["capabilities"], [])
        self.assertEqual(catalog["summary"]["total"], 0)
        self.assertEqual(
            catalog["summary"]["by_owner"],
            {"core": 0, "host": 0, "shared": 0},
        )
        self.assertEqual(
            catalog["summary"]["by_status"],
            {"implemented": 0, "partial": 0, "missing": 0, "unknown": 0},
        )


# ---------------------------------------------------------------------------
# 8. owner = shared when in BOTH schema (core) and host-contracts
# ---------------------------------------------------------------------------
class CollectSharedOwnerTests(unittest.TestCase):
    def test_shared_when_in_schema_and_host_contracts(self):
        with tempfile.TemporaryDirectory() as d:
            _write(Path(d) / "protocol" / "reader-command.schema.json", json.dumps({
                "$defs": {"A": {"properties": {"method": {"const": "http.execute"}}}}
            }))
            _write(Path(d) / "docs" / "host-app-contracts" / "http.md", (
                "# HTTP\n\n"
                "```json\n"
                '{"capability": "http.execute"}\n'
                "```\n"
            ))
            catalog = cc.collect(d)
        by_id = {c["id"]: c for c in catalog["capabilities"]}
        self.assertEqual(by_id["http.execute"]["owner"], "shared")
        self.assertIn(
            "docs/host-app-contracts/http.md",
            by_id["http.execute"]["evidence"],
        )


# ---------------------------------------------------------------------------
# 9. status fallback: schema-derived no matrix match -> implemented
# ---------------------------------------------------------------------------
class CollectStatusFallbackTests(unittest.TestCase):
    def test_schema_derived_no_matrix_match_implemented(self):
        with tempfile.TemporaryDirectory() as d:
            _write(Path(d) / "protocol" / "reader-command.schema.json", json.dumps({
                "$defs": {"A": {"properties": {"method": {"const": "core.info"}}}}
            }))
            catalog = cc.collect(d)
        by_id = {c["id"]: c for c in catalog["capabilities"]}
        self.assertEqual(by_id["core.info"]["status"], "implemented")


# ---------------------------------------------------------------------------
# 10. summary by_owner + by_status counts correct
# ---------------------------------------------------------------------------
class CollectSummaryTests(unittest.TestCase):
    def test_summary_counts(self):
        with tempfile.TemporaryDirectory() as d:
            _write(Path(d) / "protocol" / "reader-command.schema.json", json.dumps({
                "$defs": {
                    "A": {"properties": {"method": {"const": "core.info"}}},
                    "B": {"properties": {"method": {"const": "book.search"}}},
                }
            }))
            _write(Path(d) / "FEATURE_MATRIX.md", (
                "| Name | Owner | Status |\n"
                "|------|-------|--------|\n"
                "| book.search | Rust Core | 部分完成 |\n"
                "| tls.socket | 平台负责 | Gap |\n"
            ))
            catalog = cc.collect(d)
        s = catalog["summary"]
        self.assertEqual(s["total"], 3)
        self.assertEqual(s["by_owner"], {"core": 2, "host": 1, "shared": 0})
        self.assertEqual(
            s["by_status"],
            {"implemented": 1, "partial": 1, "missing": 1, "unknown": 0},
        )


# ---------------------------------------------------------------------------
# 11. CLI --pretty valid JSON exit 0; --out writes file
# ---------------------------------------------------------------------------
class CliTests(unittest.TestCase):
    def test_pretty_and_out(self):
        with tempfile.TemporaryDirectory() as d:
            _write(Path(d) / "protocol" / "reader-command.schema.json", json.dumps({
                "$defs": {"A": {"properties": {"method": {"const": "core.info"}}}}
            }))
            out_path = Path(d) / "report.json"
            result = _run_cli([d, "--pretty", "--out", str(out_path)])
            self.assertEqual(result.returncode, 0)
            parsed = json.loads(result.stdout)
            self.assertIn("capabilities", parsed)
            self.assertEqual(parsed["version"], "capability-catalog/1")
            self.assertTrue(out_path.exists())
            file_parsed = json.loads(out_path.read_text(encoding="utf-8"))
            self.assertIn("capabilities", file_parsed)

    def test_usage_error_exit2(self):
        result = _run_cli(["--no-such-flag"])
        self.assertEqual(result.returncode, 2)


# ---------------------------------------------------------------------------
# 12. capabilities sorted by id
# ---------------------------------------------------------------------------
class CollectSortedTests(unittest.TestCase):
    def test_capabilities_sorted_by_id(self):
        with tempfile.TemporaryDirectory() as d:
            _write(Path(d) / "protocol" / "reader-command.schema.json", json.dumps({
                "$defs": {
                    "A": {"properties": {"method": {"const": "zzz.last"}}},
                    "B": {"properties": {"method": {"const": "aaa.first"}}},
                    "C": {"properties": {"method": {"const": "mmm.middle"}}},
                }
            }))
            catalog = cc.collect(d)
        ids = [c["id"] for c in catalog["capabilities"]]
        self.assertEqual(ids, sorted(ids))


# ---------------------------------------------------------------------------
# Extra: catalog shape / version / generated_at / entry fields
# ---------------------------------------------------------------------------
class CatalogShapeTests(unittest.TestCase):
    def test_catalog_top_level_fields(self):
        with tempfile.TemporaryDirectory() as d:
            catalog = cc.collect(d)
        self.assertEqual(catalog["version"], "capability-catalog/1")
        self.assertEqual(catalog["tool"], "capability-catalog-generator")
        self.assertIn("generated_at", catalog)
        self.assertTrue(catalog["generated_at"].endswith("Z"))

    def test_capability_entry_shape(self):
        with tempfile.TemporaryDirectory() as d:
            _write(Path(d) / "protocol" / "reader-command.schema.json", json.dumps({
                "$defs": {"A": {"properties": {"method": {"const": "core.info"}}}}
            }))
            catalog = cc.collect(d)
        entry = catalog["capabilities"][0]
        for key in ("id", "name", "owner", "status", "evidence", "source",
                    "platforms", "notes"):
            self.assertIn(key, entry, "missing key %s in %s" % (key, entry))
        self.assertEqual(entry["id"], "core.info")
        self.assertEqual(entry["name"], "Core Info")
        self.assertEqual(entry["platforms"], ["ios", "android", "harmony"])
        self.assertEqual(entry["notes"], "")


if __name__ == "__main__":
    unittest.main()
