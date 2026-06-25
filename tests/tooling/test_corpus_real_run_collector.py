#!/usr/bin/env python3
"""Tests for the corpus real-run collector.

Run with:
    python3 -m unittest tests.tooling.test_corpus_real_run_collector -v
or:
    python3 tests/tooling/test_corpus_real_run_collector.py
"""

import json
import os
import shutil
import sys
import tempfile
import unittest


_HERE = os.path.dirname(os.path.abspath(__file__))
_ROOT = os.path.abspath(os.path.join(_HERE, "..", ".."))
sys.path.insert(0, os.path.join(_ROOT, "tools", "corpus-real-run-collector"))
sys.path.insert(0, os.path.join(_ROOT, "tools", "benchmark-run-packager"))
sys.path.insert(0, os.path.join(_ROOT, "tools", "release-blocker-register"))

import corpus_real_run_collector as collector  # noqa: E402
import benchmark_run_packager as brp  # noqa: E402
import release_blocker_register as rbr  # noqa: E402


PRIVATE_TMP = "/private/tmp"
PLATFORMS = ("cli", "ios", "android", "harmony")


def _fixture_dir(name):
    return os.path.join(_ROOT, "samples", "corpus-release-gate", name)


def _fixture_inputs(name):
    fixture = _fixture_dir(name)
    candidates = {
        platform: os.path.join(fixture, "candidates", "{0}-result.json".format(platform))
        for platform in PLATFORMS
    }
    return {
        "input": os.path.join(fixture, "input.json"),
        "canonical": os.path.join(fixture, "canonical-result.json"),
        "manifest": os.path.join(fixture, "manifest.json"),
        "candidates": candidates,
    }


class CandidateParsingTests(unittest.TestCase):
    def test_requires_name_colon_path(self):
        with self.assertRaises(collector.CollectorError):
            collector.parse_candidate_spec("cli-result.json")

    def test_rejects_duplicate_platform(self):
        specs = ["cli:a.json", "cli:b.json", "ios:i.json",
                 "android:a.json", "harmony:h.json"]
        with self.assertRaises(collector.CollectorError):
            collector.parse_candidate_specs(specs)

    def test_requires_all_four_platforms(self):
        specs = ["cli:c.json", "ios:i.json", "android:a.json"]
        with self.assertRaises(collector.CollectorError):
            collector.parse_candidate_specs(specs)

    def test_rejects_unknown_platform(self):
        specs = ["cli:c.json", "ios:i.json", "android:a.json",
                 "harmony:h.json", "web:w.json"]
        with self.assertRaises(collector.CollectorError):
            collector.parse_candidate_specs(specs)


class CollectRealRunTests(unittest.TestCase):
    def setUp(self):
        self.paths = []

    def tearDown(self):
        for path in self.paths:
            shutil.rmtree(path, ignore_errors=True)
            if os.path.exists(path):
                try:
                    os.remove(path)
                except OSError:
                    pass

    def _tmp_out(self, prefix):
        path = tempfile.mkdtemp(prefix=prefix, dir=PRIVATE_TMP)
        os.rmdir(path)
        self.paths.append(path)
        return path

    def test_collects_four_platform_match_package(self):
        fixtures = _fixture_inputs("four-platform-match")
        out = self._tmp_out("collector-match-")

        summary = collector.collect_real_run(
            run_id="collector-four-platform-match",
            scenario="fixture-search",
            input_path=fixtures["input"],
            canonical_path=fixtures["canonical"],
            candidates=fixtures["candidates"],
            out_dir=out,
            source_manifest_path=fixtures["manifest"],
        )

        self.assertTrue(summary["match"])
        self.assertEqual(summary["total"], 0)
        self.assertEqual(summary["blockersAdded"], 0)
        for rel in ("manifest.json", "platform-result.json",
                    "canonical-result.json", "diff-result.json",
                    "environment.json", "corpus-blocker-register.json",
                    "input.json"):
            self.assertTrue(os.path.isfile(os.path.join(out, rel)), rel)
        for platform in PLATFORMS:
            self.assertTrue(
                os.path.isfile(os.path.join(out, "raw", platform + "-result.json"))
            )
            self.assertTrue(
                os.path.isfile(os.path.join(out, "candidates", platform + "-result.json"))
            )

        validation, loaded = brp.validate_run_dir(out)
        self.assertTrue(validation["ok"])
        self.assertTrue(loaded["diff-result"]["match"])
        self.assertEqual(set(loaded["diff-result"]["candidates"].keys()), set(PLATFORMS))

    def test_collects_mismatch_into_android_blocker(self):
        fixtures = _fixture_inputs("four-platform-mismatch")
        out = self._tmp_out("collector-mismatch-")

        summary = collector.collect_real_run(
            run_id="collector-four-platform-mismatch",
            scenario="fixture-search",
            input_path=fixtures["input"],
            canonical_path=fixtures["canonical"],
            candidates=fixtures["candidates"],
            out_dir=out,
        )

        self.assertFalse(summary["match"])
        self.assertEqual(summary["total"], 1)
        self.assertEqual(summary["blockersAdded"], 1)
        self.assertEqual(summary["openByPlatform"], {"android": 1})

        register = rbr.load_register(os.path.join(out, "corpus-blocker-register.json"))
        blockers = rbr.filter_blockers(
            register["blockers"],
            status=rbr.STATUS_OPEN,
            run_id="collector-four-platform-mismatch",
        )
        self.assertEqual(len(blockers), 1)
        self.assertEqual(blockers[0]["platform"], "android")
        self.assertEqual(blockers[0]["fieldPath"], "results[1].name")

    def test_refuses_documents_output(self):
        fixtures = _fixture_inputs("four-platform-match")
        docs_out = os.path.join(os.path.expanduser("~/Documents"), "collector-refused")

        with self.assertRaises(collector.CollectorError):
            collector.collect_real_run(
                run_id="collector-refused",
                scenario="fixture-search",
                input_path=fixtures["input"],
                canonical_path=fixtures["canonical"],
                candidates=fixtures["candidates"],
                out_dir=docs_out,
            )

        self.assertFalse(os.path.exists(docs_out))

    @unittest.skipUnless(os.path.isdir("/var/tmp"), "/var/tmp is not available")
    def test_user_specified_output_outside_private_tmp_keeps_local_register(self):
        fixtures = _fixture_inputs("four-platform-match")
        out = tempfile.mkdtemp(prefix="collector-var-tmp-", dir="/var/tmp")
        shutil.rmtree(out)
        self.paths.append(out)

        summary = collector.collect_real_run(
            run_id="collector-var-tmp",
            scenario="fixture-search",
            input_path=fixtures["input"],
            canonical_path=fixtures["canonical"],
            candidates=fixtures["candidates"],
            out_dir=out,
        )

        self.assertEqual(summary["outDir"], os.path.abspath(out))
        self.assertEqual(
            summary["register"],
            os.path.join(os.path.abspath(out), "corpus-blocker-register.json"),
        )
        self.assertTrue(os.path.isfile(summary["register"]))


class MainCliTests(unittest.TestCase):
    def setUp(self):
        self.paths = []

    def tearDown(self):
        for path in self.paths:
            shutil.rmtree(path, ignore_errors=True)
            if os.path.exists(path):
                try:
                    os.remove(path)
                except OSError:
                    pass

    def test_cli_writes_candidate_package(self):
        fixtures = _fixture_inputs("four-platform-match")
        out = tempfile.mkdtemp(prefix="collector-cli-", dir=PRIVATE_TMP)
        os.rmdir(out)
        self.paths.append(out)

        argv = [
            "--run-id", "collector-cli-four-platform-match",
            "--scenario", "fixture-search",
            "--input", fixtures["input"],
            "--canonical", fixtures["canonical"],
            "--out", out,
        ]
        for platform in PLATFORMS:
            argv.extend(["--candidate", "{0}:{1}".format(
                platform,
                fixtures["candidates"][platform],
            )])

        rc = collector.main(argv)

        self.assertEqual(rc, 0)
        with open(os.path.join(out, "diff-result.json"), "r",
                  encoding="utf-8") as handle:
            diff = json.load(handle)
        self.assertTrue(diff["match"])


if __name__ == "__main__":
    unittest.main()
