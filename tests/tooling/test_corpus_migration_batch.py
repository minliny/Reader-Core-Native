#!/usr/bin/env python3
"""Structural and privacy validation for the Reader-Core migration batch
(round 2) of the sanitized corpus.

Covers the 7 fixtures migrated from Documents/Reader-Core:
  bs-002, bs-005, bs-009, bs-010 (book-source)
  ja-002, ja-003 (json-api)
  wp-002 (web-page)

Run with:
    python3 -m unittest tests.tooling.test_corpus_migration_batch -v
"""

import json
import os
import unittest
from pathlib import Path

_HERE = Path(__file__).resolve().parent
_REPO_ROOT = _HERE.parent.parent
_CORPUS_ROOT = _REPO_ROOT / "fixtures" / "sanitized-corpus"

# (id, source_type_dir, fixture_filename, manifest_filename)
_NEW_FIXTURES = [
    ("bs-002", "book-source", "bs-002-fixture.json", "bs-002.manifest.json"),
    ("bs-005", "book-source", "bs-005-fixture.json", "bs-005.manifest.json"),
    ("bs-009", "book-source", "bs-009-fixture.json", "bs-009.manifest.json"),
    ("bs-010", "book-source", "bs-010-fixture.json", "bs-010.manifest.json"),
    ("ja-002", "json-api", "ja-002-fixture.json", "ja-002.manifest.json"),
    ("ja-003", "json-api", "ja-003-fixture.json", "ja-003.manifest.json"),
    ("wp-002", "web-page", "wp-002-fixture.html", "wp-002.manifest.json"),
]

_REQUIRED_MANIFEST_FIELDS = [
    "id", "source_type", "format", "source_description", "sanitization",
    "capability_tags", "privacy_check", "consumer_branch", "fixture_file",
    "added_in_round",
]

# Real strings that must NOT appear anywhere in the sanitized corpus.
_FORBIDDEN_REAL_STRINGS = [
    "sudugu.org",
    "manmeng168.com",
    "kuwo.cn",
    "974685bdc9957e8c",      # real api_secret from 书源示例.json
    "b0fae45e-1b4f-4057-930c-fbea2b7789f6",  # real device_id
    "60562717",              # real uid
    "捞尸人",                # real copyrighted novel title (bs-009 source)
    "纯洁滴小龙",            # real author (bs-009 source)
    "速读谷",                # real site name (bs-009 source)
    "$%@*!^#!@(@",           # real md5 sign salt
    "LND-AL40",              # real device fingerprint in User-Agent
]


def _load_json(path):
    with open(path, "r", encoding="utf-8") as fh:
        return json.load(fh)


def _read_text(path):
    with open(path, "r", encoding="utf-8") as fh:
        return fh.read()


class ManifestStructureTests(unittest.TestCase):
    """Every new manifest must carry the full required field set."""

    def test_all_seven_manifests_exist_and_parse(self):
        for fid, sdir, _fext, mname in _NEW_FIXTURES:
            path = _CORPUS_ROOT / sdir / mname
            self.assertTrue(path.exists(), f"manifest missing: {path}")
            data = _load_json(path)
            self.assertEqual(data["id"], fid)

    def test_each_manifest_has_all_required_fields(self):
        for fid, sdir, _fext, mname in _NEW_FIXTURES:
            data = _load_json(_CORPUS_ROOT / sdir / mname)
            for field in _REQUIRED_MANIFEST_FIELDS:
                self.assertIn(
                    field, data,
                    f"{fid}: manifest missing required field {field!r}")

    def test_all_fixtures_added_in_round_two(self):
        for fid, sdir, _fext, mname in _NEW_FIXTURES:
            data = _load_json(_CORPUS_ROOT / sdir / mname)
            self.assertEqual(
                data["added_in_round"], 2,
                f"{fid}: expected added_in_round=2, got {data['added_in_round']}")

    def test_manifest_fixture_file_matches_actual(self):
        for fid, sdir, fext, mname in _NEW_FIXTURES:
            data = _load_json(_CORPUS_ROOT / sdir / mname)
            self.assertEqual(
                data["fixture_file"], fext,
                f"{fid}: fixture_file {data['fixture_file']!r} != {fext!r}")
            actual = _CORPUS_ROOT / sdir / fext
            self.assertTrue(actual.exists(), f"{fid}: fixture file missing: {actual}")

    def test_manifest_source_type_matches_directory(self):
        type_by_dir = {
            "book-source": "book-source",
            "json-api": "json-api",
            "web-page": "web-page",
        }
        for fid, sdir, _fext, mname in _NEW_FIXTURES:
            data = _load_json(_CORPUS_ROOT / sdir / mname)
            self.assertEqual(
                data["source_type"], type_by_dir[sdir],
                f"{fid}: source_type {data['source_type']!r} != {type_by_dir[sdir]!r}")

    def test_every_manifest_carries_at_least_one_capability_tag(self):
        for fid, sdir, _fext, mname in _NEW_FIXTURES:
            data = _load_json(_CORPUS_ROOT / sdir / mname)
            self.assertIsInstance(data["capability_tags"], list)
            self.assertGreater(
                len(data["capability_tags"]), 0,
                f"{fid}: capability_tags must not be empty")

    def test_every_manifest_has_migrated_from_field(self):
        for fid, sdir, _fext, mname in _NEW_FIXTURES:
            data = _load_json(_CORPUS_ROOT / sdir / mname)
            self.assertIn(
                "migrated_from", data,
                f"{fid}: migrated_from field missing (round-2 fixtures must record origin)")
            self.assertTrue(
                data["migrated_from"].startswith("Documents/Reader-Core"),
                f"{fid}: migrated_from should point at Reader-Core source")


def _consumable_payload(path):
    """Return only the consumable fixture payload, stripping audit metadata.

    The `sanitization_notes` / `source_description` / `sanitization` fields
    legitimately reference original real values (domain names, secret values,
    copyrighted titles) to document what was redacted. The privacy guard must
    only check the data a rule engine would actually parse.
    """
    text = _read_text(path)
    if path.suffix == ".json":
        data = json.loads(text)
        data.pop("sanitization_notes", None)
        data.pop("description", None)
        data.pop("source_description", None)
        data.pop("sanitization", None)
        # bs-009 / bs-002 etc. carry rule+samples; strip nothing else.
        return json.dumps(data, ensure_ascii=False)
    return text


class PrivacyGuardTests(unittest.TestCase):
    """No real credentials, domains, or copyrighted text may leak into the
    consumable payload. The sanitization_notes audit metadata is allowed to
    reference original values (that is its purpose)."""

    def test_no_forbidden_real_strings_in_consumable_payload(self):
        for fid, sdir, fext, _mname in _NEW_FIXTURES:
            payload = _consumable_payload(_CORPUS_ROOT / sdir / fext)
            for bad in _FORBIDDEN_REAL_STRINGS:
                self.assertNotIn(
                    bad, payload,
                    f"{fid}: forbidden real string {bad!r} found in consumable payload of {fext}")

    def test_sanitization_notes_may_reference_originals_for_audit(self):
        # Sanity: bs-009's notes DO mention sudugu.org (that's the audit point).
        data = _load_json(_CORPUS_ROOT / "book-source" / "bs-009-fixture.json")
        notes_text = json.dumps(data.get("sanitization_notes", {}), ensure_ascii=False)
        self.assertIn("sudugu.org", notes_text)
        self.assertIn("捞尸人", notes_text)

    def test_all_hosts_use_example_test(self):
        # Every http(s) URL in every fixture must be *.example.test.
        import re
        url_re = re.compile(r"https?://([a-zA-Z0-9._-]+)")
        for fid, sdir, fext, _mname in _NEW_FIXTURES:
            text = _read_text(_CORPUS_ROOT / sdir / fext)
            for match in url_re.finditer(text):
                host = match.group(1)
                self.assertTrue(
                    host.endswith("example.test"),
                    f"{fid}: non-example.test host {host!r} in fixture {fext}")

    def test_every_manifest_privacy_check_passed(self):
        for fid, sdir, _fext, mname in _NEW_FIXTURES:
            data = _load_json(_CORPUS_ROOT / sdir / mname)
            pc = data["privacy_check"]
            self.assertTrue(
                pc["passed"], f"{fid}: privacy_check.passed is not true")
            self.assertIsInstance(pc["checked_for"], list)
            self.assertGreater(
                len(pc["checked_for"]), 0,
                f"{fid}: privacy_check.checked_for is empty")


class Bs005GoldenExpectedTests(unittest.TestCase):
    """bs-005 must carry a golden expected array with the 3 ToC entries."""

    def test_expected_has_three_entries_with_correct_vip_flags(self):
        data = _load_json(_CORPUS_ROOT / "book-source" / "bs-005-fixture.json")
        self.assertIn("expected", data)
        expected = data["expected"]
        self.assertEqual(len(expected), 3)
        self.assertFalse(expected[0]["isVip"])
        self.assertFalse(expected[1]["isVip"])
        self.assertTrue(expected[2]["isVip"])
        titles = [e["chapterTitle"] for e in expected]
        self.assertEqual(titles[0], "第一章 测试标题")
        self.assertEqual(titles[2], "VIP章节 第三章 测试标题")

    def test_expected_urls_use_example_test(self):
        data = _load_json(_CORPUS_ROOT / "book-source" / "bs-005-fixture.json")
        for entry in data["expected"]:
            self.assertTrue(
                entry["chapterURL"].startswith("https://books.example.test/"))


class Bs009SanitizationTests(unittest.TestCase):
    """bs-009 (case_022) must preserve DOM structure but strip all real content."""

    def test_preserves_key_dom_classes(self):
        data = _load_json(_CORPUS_ROOT / "book-source" / "bs-009-fixture.json")
        detail = data["samples"]["detailResponse"]
        for cls in ["itemtxt", "des", "dir", "list"]:
            self.assertIn(cls, detail, f"detailResponse must preserve .{cls} class")
        toc = data["samples"]["tocResponse"]
        self.assertIn("id=\"list\"", toc)
        chapter = data["samples"]["chapterResponse"]
        self.assertIn("class=\"con\"", chapter)
        self.assertIn("class=\"prenext\"", chapter)
        self.assertIn("class=\"submenu\"", chapter)

    def test_chapter_content_is_synthetic_placeholder(self):
        data = _load_json(_CORPUS_ROOT / "book-source" / "bs-009-fixture.json")
        chapter = data["samples"]["chapterResponse"]
        self.assertIn("占位文本", chapter)
        # Must NOT contain any of the real novel's prose.
        self.assertNotIn("李追远", chapter)
        self.assertNotIn("李三江", chapter)

    def test_has_sanitization_notes_section(self):
        data = _load_json(_CORPUS_ROOT / "book-source" / "bs-009-fixture.json")
        self.assertIn("sanitization_notes", data)
        notes = data["sanitization_notes"]
        self.assertEqual(notes["original_site"], "sudugu.org (速读谷)")
        self.assertIn("捞尸人", notes["original_work"])
        self.assertIsInstance(notes["redactions"], list)
        self.assertGreaterEqual(len(notes["redactions"]), 8)


class Bs010RedactionTests(unittest.TestCase):
    """bs-010 (书源示例.json) must redact all credentials and neutralize domains."""

    def _payload(self):
        # Strip sanitization_notes (audit metadata) before checking the payload.
        path = _CORPUS_ROOT / "book-source" / "bs-010-fixture.json"
        return _consumable_payload(path)

    def test_has_two_sanitized_sources(self):
        data = _load_json(_CORPUS_ROOT / "book-source" / "bs-010-fixture.json")
        self.assertEqual(len(data["sources"]), 2)

    def test_all_api_secret_values_are_redacted(self):
        text = self._payload()
        # The real secret must not appear; REDACTED must appear in its place.
        self.assertNotIn("974685bdc9957e8c", text)
        self.assertIn("REDACTED", text)

    def test_all_device_ids_are_zero_uuids(self):
        text = self._payload()
        self.assertNotIn("b0fae45e-1b4f-4057-930c-fbea2b7789f6", text)
        self.assertIn("00000000-0000-0000-0000-000000000000", text)

    def test_all_uids_are_zero(self):
        text = self._payload()
        self.assertNotIn("60562717", text)

    def test_source_names_are_sanitized(self):
        data = _load_json(_CORPUS_ROOT / "book-source" / "bs-010-fixture.json")
        names = [s["bookSourceName"] for s in data["sources"]]
        for n in names:
            self.assertIn("Sanitized API Source", n)
        # Real source names must not appear in the consumable payload.
        text = self._payload()
        self.assertNotIn("番薯小说", text)
        self.assertNotIn("酷我小说", text)

    def test_preserves_rule_dsl_patterns_for_parity(self):
        text = self._payload()
        # These are the structurally valuable patterns that the rule-engine-parity
        # branch must support — they must survive sanitization verbatim.
        for pattern in ["@put:{", "@js:result.replace", "java.md5Encode",
                        "java.timeFormat", "##", "||$.data"]:
            self.assertIn(pattern, text, f"rule DSL pattern {pattern!r} lost in sanitization")

    def test_has_sanitization_notes_section(self):
        data = _load_json(_CORPUS_ROOT / "book-source" / "bs-010-fixture.json")
        notes = data["sanitization_notes"]
        self.assertEqual(notes["original_source"],
                         "Documents/Reader-Core/docs/design/书源示例.json (393KB, 3 real book sources)")
        self.assertEqual(notes["sources_retained"], 2)
        self.assertGreaterEqual(len(notes["redactions"]), 8)


class Ja002ErrorMarkerTests(unittest.TestCase):
    """ja-002 must carry the two 404 marker payloads."""

    def test_has_two_cases_with_correct_markers(self):
        data = _load_json(_CORPUS_ROOT / "json-api" / "ja-002-fixture.json")
        self.assertEqual(len(data["cases"]), 2)
        markers = {c["payload"]["marker"] for c in data["cases"]}
        self.assertEqual(markers, {"POLICY_CONTENT_FAILED", "HTTP_404_CONTENT_FAILED"})

    def test_each_case_has_expected_behavior(self):
        data = _load_json(_CORPUS_ROOT / "json-api" / "ja-002-fixture.json")
        for case in data["cases"]:
            self.assertIn("expected_behavior", case)
            self.assertTrue(len(case["expected_behavior"]) > 0)


class Ja003GroupConsistencyTests(unittest.TestCase):
    """ja-003 must carry pass/fail/reject cases for single_key_unique."""

    def test_has_three_cases_covering_pass_fail_reject(self):
        data = _load_json(_CORPUS_ROOT / "json-api" / "ja-003-fixture.json")
        self.assertEqual(len(data["cases"]), 3)
        expectations = {c["expect"] for c in data["cases"]}
        self.assertEqual(expectations, {"pass", "fail", "reject"})

    def test_pass_case_has_unique_keys(self):
        data = _load_json(_CORPUS_ROOT / "json-api" / "ja-003-fixture.json")
        pass_case = next(c for c in data["cases"] if c["expect"] == "pass")
        names = [r["device_name"] for r in pass_case["records"]]
        self.assertEqual(len(names), len(set(names)))

    def test_fail_case_has_duplicate(self):
        data = _load_json(_CORPUS_ROOT / "json-api" / "ja-003-fixture.json")
        fail_case = next(c for c in data["cases"] if c["expect"] == "fail")
        names = [r["device_name"] for r in fail_case["records"]]
        self.assertNotEqual(len(names), len(set(names)))

    def test_reject_case_has_unknown_rule_type(self):
        data = _load_json(_CORPUS_ROOT / "json-api" / "ja-003-fixture.json")
        reject_case = next(c for c in data["cases"] if c["expect"] == "reject")
        self.assertNotEqual(reject_case["rule"]["type"], "single_key_unique")


class Wp002HtmlTests(unittest.TestCase):
    """wp-002 must be valid HTML with the JS-runtime fallback contract."""

    def test_html_has_article_with_data_sample(self):
        text = _read_text(_CORPUS_ROOT / "web-page" / "wp-002-fixture.html")
        self.assertIn("<article", text)
        self.assertIn("data-sample=\"sample_js_runtime_001\"", text)
        self.assertIn("timeout fallback contract", text)

    def test_html_is_minimal_no_external_resources(self):
        text = _read_text(_CORPUS_ROOT / "web-page" / "wp-002-fixture.html")
        self.assertNotIn("<script", text)
        self.assertNotIn("<link", text)
        self.assertNotIn("http://", text)
        self.assertNotIn("https://", text)


if __name__ == "__main__":
    unittest.main()
