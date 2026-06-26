#!/usr/bin/env python3
"""Tests for the corpus canonicalizer.

Run with:
    python3 -m unittest tests.tooling.test_canonicalize -v
or:
    python3 tests/tooling/test_canonicalize.py

These tests prove that synonymous outputs from different platforms
(differing only in field order, whitespace, line endings, HTML entity
encoding, URL trailing slash, or run-variable timestamps) collapse to
the same canonical JSON.
"""

import json
import os
import sys
import unittest

# Make scripts/ importable regardless of cwd.
_HERE = os.path.dirname(os.path.abspath(__file__))
_ROOT = os.path.abspath(os.path.join(_HERE, "..", ".."))
sys.path.insert(0, os.path.join(_ROOT, "scripts"))

import corpus_canonicalize as cc  # noqa: E402


def canon(obj):
    """Canonicalize a python object and return the serialized string."""
    return cc.serialize(cc.canonicalize(obj))


class TestFieldOrder(unittest.TestCase):
    def test_object_keys_sorted(self):
        a = {"b": 1, "a": 2, "c": 3}
        b = {"c": 3, "a": 2, "b": 1}
        self.assertEqual(canon(a), canon(b))

    def test_nested_keys_sorted(self):
        a = {"z": {"y": 1, "x": 2}}
        b = {"z": {"x": 2, "y": 1}}
        self.assertEqual(canon(a), canon(b))

    def test_keys_sorted_in_list_items(self):
        a = [{"b": 1, "a": 2}, {"d": 4, "c": 3}]
        b = [{"a": 2, "b": 1}, {"c": 3, "d": 4}]
        self.assertEqual(canon(a), canon(b))


class TestWhitespace(unittest.TestCase):
    def test_leading_trailing_stripped(self):
        a = {"t": "  hello world  "}
        b = {"t": "hello world"}
        self.assertEqual(canon(a), canon(b))

    def test_internal_space_runs_collapsed(self):
        a = {"t": "hello     world\t\tagain"}
        b = {"t": "hello world again"}
        self.assertEqual(canon(a), canon(b))

    def test_newlines_preserved(self):
        # Newlines are paragraph separators and must be preserved.
        a = {"t": "para1\npara2"}
        b = {"t": "para1\npara2"}
        self.assertEqual(canon(a), canon(b))


class TestLineEndings(unittest.TestCase):
    def test_crlf_normalized_to_lf(self):
        a = {"t": "line1\r\nline2"}
        b = {"t": "line1\nline2"}
        self.assertEqual(canon(a), canon(b))

    def test_cr_normalized_to_lf(self):
        a = {"t": "line1\rline2"}
        b = {"t": "line1\nline2"}
        self.assertEqual(canon(a), canon(b))

    def test_trailing_newline_stripped(self):
        a = {"t": "content\n\n"}
        b = {"t": "content"}
        self.assertEqual(canon(a), canon(b))


class TestHtmlEntities(unittest.TestCase):
    def test_common_named_entities_decoded(self):
        a = {"t": "a &amp; b &lt;c&gt; &quot;q&quot; &#39;s&#39; &nbsp;x"}
        b = {"t": "a & b <c> \"q\" 's'  x"}
        self.assertEqual(canon(a), canon(b))

    def test_numeric_entities_decoded(self):
        a = {"t": "&#65;&#x42;&#60;"}
        b = {"t": "AB<"}
        self.assertEqual(canon(a), canon(b))

    def test_entity_then_whitespace_collapse(self):
        a = {"t": "x &amp; &amp; y"}
        b = {"t": "x & & y"}
        self.assertEqual(canon(a), canon(b))


class TestUrlTrailingSlash(unittest.TestCase):
    def test_root_slash_stripped(self):
        a = {"u": "https://example.com/"}
        b = {"u": "https://example.com"}
        self.assertEqual(canon(a), canon(b))

    def test_path_slash_stripped(self):
        a = {"u": "https://example.com/path/"}
        b = {"u": "https://example.com/path"}
        self.assertEqual(canon(a), canon(b))

    def test_query_preserved_slash_before_query_stripped(self):
        a = {"u": "https://example.com/path/?q=1"}
        b = {"u": "https://example.com/path?q=1"}
        self.assertEqual(canon(a), canon(b))

    def test_non_url_not_touched(self):
        # A plain string that is not a URL keeps its slash.
        a = {"u": "not/a/url/"}
        self.assertIn("/", canon(a))


class TestVariableTimestampFields(unittest.TestCase):
    def test_timestamp_normalized(self):
        a = {"name": "x", "timestamp": 1700000000}
        b = {"name": "x", "timestamp": 1700000099}
        self.assertEqual(canon(a), canon(b))

    def test_request_id_normalized(self):
        a = {"name": "x", "request_id": "abc-123"}
        b = {"name": "x", "request_id": "def-456"}
        self.assertEqual(canon(a), canon(b))

    def test_camel_case_variable_field(self):
        a = {"name": "x", "traceId": "t1"}
        b = {"name": "x", "traceId": "t2"}
        self.assertEqual(canon(a), canon(b))

    def test_host_operation_and_run_ids_normalized(self):
        a = {"operationId": 1, "runId": "run-a", "name": "x"}
        b = {"operationId": 2, "runId": "run-b", "name": "x"}
        self.assertEqual(canon(a), canon(b))

    def test_non_variable_field_kept(self):
        a = {"name": "x", "title": "different"}
        b = {"name": "x", "title": "values"}
        self.assertNotEqual(canon(a), canon(b))


class TestNestedAndComposite(unittest.TestCase):
    def test_mixed_nested_normalization(self):
        a = {
            "items": [
                {"Url": "https://h.test/a/b/  ", "Title": "A &amp; B"},
            ],
            "timestamp": 1,
        }
        b = {
            "items": [
                {"title": "A & B", "url": "https://h.test/a/b"},
            ],
            "timestamp": 2,
        }
        # NOTE: keys are case-sensitive; "Url" != "url". This pair is
        # intentionally NOT expected to be equal — it documents that the
        # canonicalizer is case-sensitive on keys.
        self.assertNotEqual(canon(a), canon(b))


class TestResultTypePairs(unittest.TestCase):
    """Synonymous outputs for each result type collapse to one canonical form."""

    def test_chapter_content_pair(self):
        a = {
            "type": "chapter",
            "content": "Hello &amp; welcome\r\n\r\n  to chapter 1  ",
            "url": "https://h.test/book/ch1/",
            "timestamp": 1700000000,
        }
        b = {
            "url": "https://h.test/book/ch1",
            "type": "chapter",
            "timestamp": 1700000099,
            "content": "Hello & welcome\n\nto chapter 1",
        }
        self.assertEqual(canon(a), canon(b))

    def test_toc_pair(self):
        a = {
            "type": "toc",
            "chapters": [
                {"title": "Ch&nbsp;1", "url": "https://h.test/c1/"},
                {"title": "Ch&nbsp;2", "url": "https://h.test/c2/"},
            ],
            "request_id": "req-A",
        }
        b = {
            "request_id": "req-B",
            "type": "toc",
            "chapters": [
                {"url": "https://h.test/c1", "title": "Ch 1"},
                {"url": "https://h.test/c2", "title": "Ch 2"},
            ],
        }
        self.assertEqual(canon(a), canon(b))

    def test_search_pair(self):
        a = {
            "type": "search",
            "results": [
                {"name": "Book   One", "author": "Author A", "bookUrl": "https://h.test/b1/"},
            ],
            "timestamp": 1,
        }
        b = {
            "type": "search",
            "timestamp": 2,
            "results": [
                {"bookUrl": "https://h.test/b1", "author": "Author A", "name": "Book One"},
            ],
        }
        self.assertEqual(canon(a), canon(b))

    def test_book_detail_pair(self):
        a = {
            "type": "detail",
            "name": "Demo &amp; Co",
            "author": "A&nbsp;B",
            "coverUrl": "https://h.test/cover.jpg",
            "intro": "Line1\r\nLine2",
            "updated_at": "2024-01-01T00:00:00Z",
        }
        b = {
            "intro": "Line1\nLine2",
            "updated_at": "2024-06-01T12:00:00Z",
            "coverUrl": "https://h.test/cover.jpg",
            "author": "A B",
            "name": "Demo & Co",
            "type": "detail",
        }
        self.assertEqual(canon(a), canon(b))


class TestSerialize(unittest.TestCase):
    def test_serialize_is_sorted_and_stable(self):
        out = cc.serialize(cc.canonicalize({"b": 1, "a": 2}))
        # Keys appear in sorted order.
        self.assertLess(out.index('"a"'), out.index('"b"'))

    def test_serialize_uses_two_space_indent(self):
        out = cc.serialize(cc.canonicalize({"a": 1}))
        self.assertIn('\n  "a"', out)


class TestSamplePairs(unittest.TestCase):
    """Each sample pair in samples/canonical/ must canonicalize to one form."""

    PAIRS = [
        ("chapter-a.json", "chapter-b.json"),
        ("toc-a.json", "toc-b.json"),
        ("search-a.json", "search-b.json"),
        ("detail-a.json", "detail-b.json"),
    ]

    def _load(self, name):
        path = os.path.join(_ROOT, "samples", "canonical", name)
        with open(path, "r", encoding="utf-8") as f:
            return cc.canonicalize(json.load(f))

    def test_pairs_canonicalize_equal(self):
        for a_name, b_name in self.PAIRS:
            with self.subTest(pair=(a_name, b_name)):
                self.assertEqual(
                    cc.serialize(self._load(a_name)),
                    cc.serialize(self._load(b_name)),
                    msg=f"{a_name} and {b_name} should canonicalize equal",
                )

    def test_canonical_form_is_sorted(self):
        out = cc.serialize(self._load("detail-a.json"))
        # author precedes category precedes coverUrl (alphabetical).
        self.assertLess(out.index('"author"'), out.index('"category"'))
        self.assertLess(out.index('"category"'), out.index('"coverUrl"'))

    def test_variable_fields_are_sentinels(self):
        obj = self._load("chapter-a.json")
        self.assertEqual(obj["timestamp"], cc.VARIABLE_SENTINEL)
        self.assertEqual(obj["request_id"], cc.VARIABLE_SENTINEL)


class TestCli(unittest.TestCase):
    def test_cli_reads_file_and_outputs_canonical(self):
        import tempfile
        import subprocess

        # NOTE: avoid field name "t" — it is a variable (timestamp) field.
        inp = {"b": 2, "a": 1, "text": "x &amp; y", "url": "https://h.test/p/"}
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump(inp, f)
            in_path = f.name
        try:
            script = os.path.join(_ROOT, "scripts", "corpus_canonicalize.py")
            result = subprocess.run(
                [sys.executable, script, in_path],
                capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 0, msg=result.stderr)
            parsed = json.loads(result.stdout)
            # Keys sorted.
            self.assertEqual(list(parsed.keys()), ["a", "b", "text", "url"])
            # Entity decoded.
            self.assertEqual(parsed["text"], "x & y")
            # URL slash stripped.
            self.assertEqual(parsed["url"], "https://h.test/p")
        finally:
            os.unlink(in_path)


if __name__ == "__main__":
    unittest.main(verbosity=2)
