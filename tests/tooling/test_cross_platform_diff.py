#!/usr/bin/env python3
"""Tests for the cross-platform corpus diff.

Run with:
    python3 -m unittest tests.tooling.test_cross_platform_diff -v
or:
    python3 tests/tooling/test_cross_platform_diff.py
"""

import json
import os
import sys
import tempfile
import unittest

_HERE = os.path.dirname(os.path.abspath(__file__))
_ROOT = os.path.abspath(os.path.join(_HERE, "..", ".."))
sys.path.insert(0, os.path.join(_ROOT, "tools", "cross-platform-diff"))

import cross_platform_diff as cpd  # noqa: E402


def _write(tmpdir, name, obj):
    path = os.path.join(tmpdir, name)
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(obj, handle)
    return path


FOUR_PLATFORM_CANDIDATES = ("cli", "ios", "android", "harmony")


def _fixture_dir(name):
    return os.path.join(_ROOT, "samples", "corpus-release-gate", name)


def _fixture_diff_inputs(name):
    fixture = _fixture_dir(name)
    canonical = os.path.join(fixture, "canonical-result.json")
    candidates = [
        (
            platform,
            os.path.join(fixture, "candidates", "{0}-result.json".format(platform)),
        )
        for platform in FOUR_PLATFORM_CANDIDATES
    ]
    return canonical, candidates


class TestParsing(unittest.TestCase):
    def test_name_colon_path(self):
        name, path = cpd._parse_candidate_spec("ios:results/ios.json")
        self.assertEqual(name, "ios")
        self.assertEqual(path, "results/ios.json")

    def test_bare_path_uses_stem(self):
        name, path = cpd._parse_candidate_spec("results/android.json")
        self.assertEqual(name, "android")
        self.assertEqual(path, "results/android.json")

    def test_empty_name_rejected(self):
        with self.assertRaises(cpd.DiffError):
            cpd._parse_candidate_spec(":results/x.json")


class TestCollectDifferences(unittest.TestCase):
    def test_identical_canonical_no_diffs(self):
        a = {"title": "Hello", "items": [1, 2, 3]}
        self.assertEqual(cpd.collect_differences(a, a), [])

    def test_missing_key_in_candidate(self):
        canonical = {"title": "x", "url": "y"}
        candidate = {"title": "x"}
        diffs = cpd.collect_differences(canonical, candidate)
        self.assertEqual(len(diffs), 1)
        self.assertEqual(diffs[0]["kind"], "missing-in-candidate")
        self.assertEqual(diffs[0]["path"], "url")

    def test_unexpected_key_in_candidate(self):
        canonical = {"title": "x"}
        candidate = {"title": "x", "extra": 1}
        diffs = cpd.collect_differences(canonical, candidate)
        self.assertEqual(len(diffs), 1)
        self.assertEqual(diffs[0]["kind"], "unexpected-in-candidate")
        self.assertEqual(diffs[0]["path"], "extra")

    def test_value_mismatch(self):
        diffs = cpd.collect_differences({"n": 1}, {"n": 2})
        self.assertEqual(len(diffs), 1)
        self.assertEqual(diffs[0]["kind"], "value-mismatch")
        self.assertEqual(diffs[0]["path"], "n")

    def test_list_length_mismatch(self):
        diffs = cpd.collect_differences({"a": [1, 2]}, {"a": [1, 2, 3]})
        self.assertEqual(len(diffs), 1)
        self.assertEqual(diffs[0]["kind"], "unexpected-in-candidate")
        self.assertTrue(diffs[0]["path"].endswith("[2]"))

    def test_nested_path(self):
        canonical = {"a": {"b": {"c": 1}}}
        candidate = {"a": {"b": {"c": 2}}}
        diffs = cpd.collect_differences(canonical, candidate)
        self.assertEqual(diffs[0]["path"], "a.b.c")


class TestCanonicalizedEquality(unittest.TestCase):
    """Synonymous outputs (per the canonicalizer) must compare equal."""

    def test_field_order_and_whitespace_collapse(self):
        canonical = {"title": "hello world", "n": 1}
        candidate = {"n": 1, "title": "  hello   world  "}
        a = cpd.cc.canonicalize(canonical)
        b = cpd.cc.canonicalize(candidate)
        self.assertEqual(cpd.collect_differences(a, b), [])

    def test_html_entity_and_url_trailing_slash(self):
        canonical = {"name": "A &amp; B", "url": "https://example.com/path/"}
        candidate = {"name": "A & B", "url": "https://example.com/path"}
        a = cpd.cc.canonicalize(canonical)
        b = cpd.cc.canonicalize(candidate)
        self.assertEqual(cpd.collect_differences(a, b), [])

    def test_run_variable_fields_ignored(self):
        canonical = {"title": "x", "timestamp": "2026-01-01T00:00:00Z"}
        candidate = {"title": "x", "timestamp": "2026-06-25T12:00:00Z"}
        a = cpd.cc.canonicalize(canonical)
        b = cpd.cc.canonicalize(candidate)
        self.assertEqual(cpd.collect_differences(a, b), [])


class TestBuildDiffResult(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="cpd-test-")

    def test_one_match_one_mismatch(self):
        canonical = _write(self.tmp, "canonical.json", {"title": "x", "n": 1})
        good = _write(self.tmp, "ios.json", {"title": "x", "n": 1})
        bad = _write(self.tmp, "android.json", {"title": "x", "n": 2})

        result = cpd.build_diff_result(canonical, [("ios", good), ("android", bad)])

        self.assertEqual(result["match"], False)
        self.assertEqual(result["total"], 1)
        self.assertTrue(result["candidates"]["ios"]["match"])
        self.assertEqual(result["candidates"]["ios"]["total"], 0)
        self.assertFalse(result["candidates"]["android"]["match"])
        self.assertEqual(result["candidates"]["android"]["total"], 1)

        # summary shape consumed by the run packager includes per-candidate
        # match/total plus classified difference counts.
        self.assertTrue(result["summary"]["ios"]["match"])
        self.assertEqual(result["summary"]["ios"]["total"], 0)
        self.assertFalse(result["summary"]["android"]["match"])
        self.assertEqual(result["summary"]["android"]["total"], 1)
        self.assertEqual(
            result["summary"]["android"]["differenceClasses"][
                cpd.CLASS_CORE_SEMANTIC
            ],
            1,
        )
        self.assertEqual(result["releaseGate"]["status"], "not-evaluated")

    def test_all_match(self):
        canonical = _write(self.tmp, "canonical.json", {"title": "x"})
        a = _write(self.tmp, "a.json", {"title": "x"})
        b = _write(self.tmp, "b.json", {"title": "x"})
        result = cpd.build_diff_result(canonical, [("a", a), ("b", b)])
        self.assertTrue(result["match"])
        self.assertEqual(result["total"], 0)

    def test_canonical_sha256_recorded(self):
        canonical = _write(self.tmp, "canonical.json", {"title": "x"})
        cand = _write(self.tmp, "c.json", {"title": "x"})
        result = cpd.build_diff_result(canonical, [("c", cand)])
        self.assertEqual(len(result["canonical"]["sha256"]), 64)
        self.assertEqual(len(result["candidates"]["c"]["sha256"]), 64)

    def test_four_platform_fixture_all_candidates_match(self):
        canonical, candidates = _fixture_diff_inputs("four-platform-match")

        result = cpd.build_diff_result(canonical, candidates)

        self.assertTrue(result["match"])
        self.assertEqual(result["total"], 0)
        self.assertEqual(set(result["candidates"].keys()), set(FOUR_PLATFORM_CANDIDATES))
        for platform in FOUR_PLATFORM_CANDIDATES:
            self.assertTrue(result["summary"][platform]["match"])
            self.assertEqual(result["summary"][platform]["total"], 0)
            self.assertEqual(
                result["summary"][platform]["differenceClasses"][
                    cpd.CLASS_CORE_SEMANTIC
                ],
                0,
            )

    def test_four_platform_fixture_minimal_mismatch(self):
        canonical, candidates = _fixture_diff_inputs("four-platform-mismatch")

        result = cpd.build_diff_result(canonical, candidates)

        self.assertFalse(result["match"])
        self.assertEqual(result["total"], 1)
        for platform in ("cli", "ios", "harmony"):
            self.assertTrue(result["candidates"][platform]["match"])
            self.assertEqual(result["candidates"][platform]["total"], 0)
        android = result["candidates"]["android"]
        self.assertFalse(android["match"])
        self.assertEqual(android["total"], 1)
        self.assertEqual(android["differences"][0]["path"], "results[1].name")
        self.assertEqual(android["differences"][0]["kind"], "value-mismatch")
        self.assertEqual(
            android["differences"][0]["classification"],
            cpd.CLASS_CORE_SEMANTIC,
        )

    def test_classifies_host_capability_differences(self):
        canonical = _write(
            self.tmp,
            "canonical.json",
            {"host": {"requests": [{"params": {"url": "https://a.test"}}]}},
        )
        cand = _write(
            self.tmp,
            "ios.json",
            {"host": {"requests": [{"params": {"url": "https://b.test"}}]}},
        )
        result = cpd.build_diff_result(canonical, [("ios", cand)])
        diff = result["candidates"]["ios"]["differences"][0]
        self.assertEqual(diff["classification"], cpd.CLASS_HOST_CAPABILITY)
        self.assertEqual(
            result["summary"]["ios"]["differenceClasses"][
                cpd.CLASS_HOST_CAPABILITY
            ],
            1,
        )

    def test_release_gate_passes_when_required_three_candidates_match(self):
        canonical = _write(self.tmp, "canonical.json", {"title": "x"})
        ios = _write(self.tmp, "ios.json", {"title": "x"})
        android = _write(self.tmp, "android.json", {"title": "x"})
        harmony = _write(self.tmp, "harmony.json", {"title": "x"})
        result = cpd.build_diff_result(
            canonical,
            [("ios", ios), ("android", android), ("harmony", harmony)],
            required_candidates=["ios", "android", "harmony"],
        )
        self.assertTrue(result["match"])
        self.assertEqual(result["releaseGate"]["status"], "passed")
        self.assertEqual(
            result["releaseGate"]["matchingCandidates"],
            ["ios", "android", "harmony"],
        )

    def test_release_gate_blocks_missing_platform_output(self):
        canonical = _write(self.tmp, "canonical.json", {"title": "x"})
        ios = _write(self.tmp, "ios.json", {"title": "x"})
        android = _write(self.tmp, "android.json", {"title": "x"})
        result = cpd.build_diff_result(
            canonical,
            [("ios", ios), ("android", android)],
            required_candidates=["ios", "android", "harmony"],
        )
        self.assertFalse(result["match"])
        self.assertEqual(result["releaseGate"]["status"], "blocked")
        self.assertEqual(result["releaseGate"]["missingCandidates"], ["harmony"])
        self.assertEqual(
            result["candidates"]["harmony"]["differences"][0]["classification"],
            cpd.CLASS_PLATFORM_MISSING,
        )


class TestCLI(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="cpd-cli-")

    def _run(self, argv):
        return cpd.main(argv)

    def test_requires_candidate(self):
        canonical = _write(self.tmp, "canonical.json", {"title": "x"})
        # No --candidate -> exit 2.
        rc = self._run([canonical])
        self.assertEqual(rc, 2)

    def test_duplicate_name_rejected(self):
        canonical = _write(self.tmp, "canonical.json", {"title": "x"})
        a = _write(self.tmp, "a.json", {"title": "x"})
        rc = self._run([canonical, "--candidate", "x:" + a, "--candidate", "x:" + a])
        self.assertEqual(rc, 2)

    def test_writes_output_file(self):
        canonical = _write(self.tmp, "canonical.json", {"title": "x"})
        a = _write(self.tmp, "a.json", {"title": "x"})
        out = os.path.join(self.tmp, "diff-result.json")
        rc = self._run([canonical, "--candidate", "a:" + a, "-o", out])
        self.assertEqual(rc, 0)
        with open(out, "r", encoding="utf-8") as handle:
            doc = json.load(handle)
        self.assertEqual(doc["tool"], "cross-platform-diff")
        self.assertTrue(doc["match"])


if __name__ == "__main__":
    unittest.main()
