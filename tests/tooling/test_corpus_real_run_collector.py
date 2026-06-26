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
HOST_PLATFORMS = ("ios", "android", "harmony")
CORE_IDENTITY = {
    "businessKernel": "reader-core-native-rust",
    "coreCommit": "090b96f",
    "abiVersion": 1,
    "protocolVersion": 1,
}


def _platform_runs(identity=None):
    identity = dict(identity or CORE_IDENTITY)
    return {
        platform: dict(identity)
        for platform in PLATFORMS
    }


def _expected_by_platform(mismatching=()):
    mismatching = set(mismatching)
    return {
        platform: {
            "match": platform not in mismatching,
            "total": 1 if platform in mismatching else 0,
        }
        for platform in PLATFORMS
    }


def _expected_host_parity(mismatching=()):
    return {
        "match": not bool(mismatching),
        "total": len(tuple(mismatching)),
    }


def _sha256(path):
    return collector.sha256_of_file(path)


def _artifact_hashes(fixtures):
    return {
        "input": _sha256(fixtures["input"]),
        "canonical": _sha256(fixtures["canonical"]),
        "candidates": {
            platform: _sha256(fixtures["candidates"][platform])
            for platform in PLATFORMS
        },
    }


def _manifest_doc(fixtures, expected):
    return {
        "schemaVersion": 1,
        "input": fixtures["input"],
        "canonical": fixtures["canonical"],
        "candidates": fixtures["candidates"],
        "artifacts": _artifact_hashes(fixtures),
        "coreIdentity": dict(CORE_IDENTITY),
        "platformRuns": _platform_runs(),
        "expected": expected,
    }


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

    def _tmp_manifest(self, doc):
        fd, path = tempfile.mkstemp(
            prefix="collector-manifest-",
            suffix=".json",
            dir=PRIVATE_TMP,
        )
        os.close(fd)
        with open(path, "w", encoding="utf-8") as handle:
            json.dump(doc, handle)
        self.paths.append(path)
        return path

    def test_collects_four_platform_match_package(self):
        fixtures = _fixture_inputs("four-platform-match")
        out = self._tmp_out("collector-match-")

        summary = collector.collect_real_run(
            run_id="fixture-four-platform-match",
            scenario="four-platform-search",
            input_path=fixtures["input"],
            canonical_path=fixtures["canonical"],
            candidates=fixtures["candidates"],
            out_dir=out,
            source_manifest_path=fixtures["manifest"],
        )

        self.assertTrue(summary["match"])
        self.assertEqual(summary["total"], 0)
        self.assertEqual(summary["hostParity"]["requiredPlatforms"], list(HOST_PLATFORMS))
        self.assertTrue(summary["hostParity"]["allPresent"])
        self.assertTrue(summary["hostParity"]["match"])
        self.assertEqual(summary["hostParity"]["total"], 0)
        self.assertEqual(summary["corpusProof"]["status"], "pass")
        self.assertEqual(summary["corpusProof"]["reasons"], [])
        self.assertTrue(summary["corpusProof"]["conditions"]["schemaVersionBound"])
        self.assertTrue(summary["corpusProof"]["conditions"]["coreIdentityBound"])
        self.assertTrue(summary["corpusProof"]["conditions"]["artifactHashesBound"])
        self.assertTrue(summary["corpusProof"]["conditions"]["runBindingDeclared"])
        self.assertTrue(summary["corpusProof"]["conditions"]["runBindingMatches"])
        self.assertTrue(summary["corpusProof"]["conditions"]["scenarioBindingDeclared"])
        self.assertTrue(summary["corpusProof"]["conditions"]["scenarioBindingMatches"])
        self.assertTrue(summary["corpusProof"]["conditions"]["expectedDeclared"])
        self.assertTrue(summary["corpusProof"]["conditions"]["fullDiffExpectedDeclared"])
        self.assertTrue(summary["corpusProof"]["conditions"]["byPlatformExpectedDeclared"])
        self.assertTrue(summary["corpusProof"]["conditions"]["hostParityExpectedDeclared"])
        self.assertTrue(summary["corpusProof"]["conditions"]["hostParityMatch"])
        self.assertEqual(summary["blockersAdded"], 0)
        for rel in ("manifest.json", "platform-result.json",
                    "canonical-result.json", "diff-result.json",
                    "environment.json", "corpus-blocker-register.json",
                    "input.json", "raw/source-manifest.json"):
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
        manifest = loaded["manifest"]
        diff_result = loaded["diff-result"]
        self.assertEqual(manifest["hostParity"], summary["hostParity"])
        self.assertEqual(manifest["corpusProof"], summary["corpusProof"])
        self.assertEqual(
            manifest["sourceManifestFile"]["packagePath"],
            "raw/source-manifest.json",
        )
        self.assertEqual(
            manifest["sourceManifestFile"]["sourceSha256"],
            _sha256(fixtures["manifest"]),
        )
        self.assertEqual(
            manifest["sourceManifestFile"]["packageSha256"],
            _sha256(os.path.join(out, "raw", "source-manifest.json")),
        )
        self.assertEqual(
            manifest["canonical"]["canonicalizedSha256"],
            diff_result["canonical"]["canonicalizedSha256"],
        )
        self.assertEqual(len(manifest["canonical"]["canonicalizedFileSha256"]), 64)
        for platform in PLATFORMS:
            self.assertEqual(
                manifest["candidates"][platform]["canonicalizedSha256"],
                diff_result["candidates"][platform]["canonicalizedSha256"],
            )
            self.assertEqual(
                len(manifest["candidates"][platform]["canonicalizedFileSha256"]),
                64,
            )

    def test_source_manifest_paths_must_match_inputs(self):
        fixtures = _fixture_inputs("four-platform-match")
        out = self._tmp_out("collector-manifest-mismatch-")
        manifest_doc = _manifest_doc(fixtures, {"match": True, "total": 0})
        manifest_doc["input"] = "wrong-input.json"
        manifest = self._tmp_manifest(manifest_doc)

        with self.assertRaises(collector.CollectorError):
            collector.collect_real_run(
                run_id="collector-manifest-mismatch",
                scenario="fixture-search",
                input_path=fixtures["input"],
                canonical_path=fixtures["canonical"],
                candidates=fixtures["candidates"],
                out_dir=out,
                source_manifest_path=manifest,
            )

    def test_source_manifest_run_id_must_match_output(self):
        fixtures = _fixture_inputs("four-platform-match")
        out = self._tmp_out("collector-runid-mismatch-")

        with self.assertRaises(collector.CollectorError):
            collector.collect_real_run(
                run_id="wrong-run-id",
                scenario="four-platform-search",
                input_path=fixtures["input"],
                canonical_path=fixtures["canonical"],
                candidates=fixtures["candidates"],
                out_dir=out,
                source_manifest_path=fixtures["manifest"],
            )

    def test_source_manifest_rejects_unsupported_schema_version(self):
        fixtures = _fixture_inputs("four-platform-match")
        out = self._tmp_out("collector-schema-version-mismatch-")
        manifest_doc = _manifest_doc(fixtures, {
            "match": True,
            "total": 0,
            "hostParity": _expected_host_parity(),
            "byPlatform": _expected_by_platform(),
        })
        manifest_doc["schemaVersion"] = 2
        manifest_doc["runId"] = "schema-version-mismatch"
        manifest_doc["scenario"] = "four-platform-search"
        manifest = self._tmp_manifest(manifest_doc)

        with self.assertRaises(collector.CollectorError):
            collector.collect_real_run(
                run_id="schema-version-mismatch",
                scenario="four-platform-search",
                input_path=fixtures["input"],
                canonical_path=fixtures["canonical"],
                candidates=fixtures["candidates"],
                out_dir=out,
                source_manifest_path=manifest,
            )

    def test_source_manifest_artifact_hash_must_match_inputs(self):
        fixtures = _fixture_inputs("four-platform-match")
        out = self._tmp_out("collector-artifact-hash-mismatch-")
        manifest_doc = _manifest_doc(fixtures, {
            "match": True,
            "total": 0,
            "hostParity": _expected_host_parity(),
            "byPlatform": _expected_by_platform(),
        })
        manifest_doc["runId"] = "artifact-hash-mismatch"
        manifest_doc["scenario"] = "four-platform-search"
        manifest_doc["artifacts"]["candidates"]["android"] = "0" * 64
        manifest = self._tmp_manifest(manifest_doc)

        with self.assertRaises(collector.CollectorError):
            collector.collect_real_run(
                run_id="artifact-hash-mismatch",
                scenario="four-platform-search",
                input_path=fixtures["input"],
                canonical_path=fixtures["canonical"],
                candidates=fixtures["candidates"],
                out_dir=out,
                source_manifest_path=manifest,
            )

    def test_source_manifest_scenario_must_match_output(self):
        fixtures = _fixture_inputs("four-platform-match")
        out = self._tmp_out("collector-scenario-mismatch-")

        with self.assertRaises(collector.CollectorError):
            collector.collect_real_run(
                run_id="fixture-four-platform-match",
                scenario="wrong-scenario",
                input_path=fixtures["input"],
                canonical_path=fixtures["canonical"],
                candidates=fixtures["candidates"],
                out_dir=out,
                source_manifest_path=fixtures["manifest"],
            )

    def test_source_manifest_expected_diff_must_match_actual(self):
        fixtures = _fixture_inputs("four-platform-mismatch")
        out = self._tmp_out("collector-expected-mismatch-")
        manifest = self._tmp_manifest(
            _manifest_doc(fixtures, {"match": True, "total": 0})
        )

        with self.assertRaises(collector.CollectorError):
            collector.collect_real_run(
                run_id="collector-expected-mismatch",
                scenario="fixture-search",
                input_path=fixtures["input"],
                canonical_path=fixtures["canonical"],
                candidates=fixtures["candidates"],
                out_dir=out,
                source_manifest_path=manifest,
            )

    def test_source_manifest_expected_by_platform_must_match_actual(self):
        fixtures = _fixture_inputs("four-platform-mismatch")
        out = self._tmp_out("collector-expected-platform-mismatch-")
        manifest = self._tmp_manifest(
            _manifest_doc(fixtures, {
                "match": False,
                "total": 1,
                "byPlatform": _expected_by_platform(mismatching=("ios",)),
            })
        )

        with self.assertRaises(collector.CollectorError):
            collector.collect_real_run(
                run_id="collector-expected-platform-mismatch",
                scenario="fixture-search",
                input_path=fixtures["input"],
                canonical_path=fixtures["canonical"],
                candidates=fixtures["candidates"],
                out_dir=out,
                source_manifest_path=manifest,
            )

    def test_source_manifest_expected_host_parity_must_match_actual(self):
        fixtures = _fixture_inputs("four-platform-mismatch")
        out = self._tmp_out("collector-expected-host-parity-mismatch-")
        manifest = self._tmp_manifest(
            _manifest_doc(fixtures, {
                "match": False,
                "total": 1,
                "hostParity": _expected_host_parity(),
                "byPlatform": _expected_by_platform(mismatching=("android",)),
            })
        )

        with self.assertRaises(collector.CollectorError):
            collector.collect_real_run(
                run_id="collector-expected-host-parity-mismatch",
                scenario="fixture-search",
                input_path=fixtures["input"],
                canonical_path=fixtures["canonical"],
                candidates=fixtures["candidates"],
                out_dir=out,
                source_manifest_path=manifest,
            )

    def test_corpus_proof_requires_explicit_platform_and_host_expectations(self):
        fixtures = _fixture_inputs("four-platform-match")
        out = self._tmp_out("collector-proof-missing-expectations-")
        manifest = self._tmp_manifest(
            _manifest_doc(fixtures, {"match": True, "total": 0})
        )

        summary = collector.collect_real_run(
            run_id="collector-proof-missing-expectations",
            scenario="fixture-search",
            input_path=fixtures["input"],
            canonical_path=fixtures["canonical"],
            candidates=fixtures["candidates"],
            out_dir=out,
            source_manifest_path=manifest,
        )

        self.assertTrue(summary["match"])
        self.assertTrue(summary["hostParity"]["match"])
        self.assertEqual(summary["corpusProof"]["status"], "blocked")
        self.assertTrue(summary["corpusProof"]["conditions"]["expectedDeclared"])
        self.assertTrue(summary["corpusProof"]["conditions"]["fullDiffExpectedDeclared"])
        self.assertFalse(summary["corpusProof"]["conditions"]["byPlatformExpectedDeclared"])
        self.assertFalse(summary["corpusProof"]["conditions"]["hostParityExpectedDeclared"])
        self.assertIn(
            "expected-by-platform-missing",
            summary["corpusProof"]["reasons"],
        )
        self.assertIn(
            "expected-host-parity-missing",
            summary["corpusProof"]["reasons"],
        )

    def test_corpus_proof_requires_source_manifest_schema_version(self):
        fixtures = _fixture_inputs("four-platform-match")
        out = self._tmp_out("collector-proof-missing-schema-version-")
        manifest_doc = _manifest_doc(fixtures, {
            "match": True,
            "total": 0,
            "hostParity": _expected_host_parity(),
            "byPlatform": _expected_by_platform(),
        })
        del manifest_doc["schemaVersion"]
        manifest_doc["runId"] = "collector-proof-missing-schema-version"
        manifest_doc["scenario"] = "four-platform-search"
        manifest = self._tmp_manifest(manifest_doc)

        summary = collector.collect_real_run(
            run_id="collector-proof-missing-schema-version",
            scenario="four-platform-search",
            input_path=fixtures["input"],
            canonical_path=fixtures["canonical"],
            candidates=fixtures["candidates"],
            out_dir=out,
            source_manifest_path=manifest,
        )

        self.assertTrue(summary["match"])
        self.assertTrue(summary["hostParity"]["match"])
        self.assertEqual(summary["corpusProof"]["status"], "blocked")
        self.assertFalse(summary["corpusProof"]["conditions"]["schemaVersionBound"])
        self.assertIn("schema-version-not-bound", summary["corpusProof"]["reasons"])

    def test_corpus_proof_requires_artifact_hashes(self):
        fixtures = _fixture_inputs("four-platform-match")
        out = self._tmp_out("collector-proof-missing-artifact-hashes-")
        manifest_doc = _manifest_doc(fixtures, {
            "match": True,
            "total": 0,
            "hostParity": _expected_host_parity(),
            "byPlatform": _expected_by_platform(),
        })
        manifest_doc["runId"] = "collector-proof-missing-artifact-hashes"
        manifest_doc["scenario"] = "four-platform-search"
        del manifest_doc["artifacts"]
        manifest = self._tmp_manifest(manifest_doc)

        summary = collector.collect_real_run(
            run_id="collector-proof-missing-artifact-hashes",
            scenario="four-platform-search",
            input_path=fixtures["input"],
            canonical_path=fixtures["canonical"],
            candidates=fixtures["candidates"],
            out_dir=out,
            source_manifest_path=manifest,
        )

        self.assertTrue(summary["match"])
        self.assertTrue(summary["hostParity"]["match"])
        self.assertEqual(summary["corpusProof"]["status"], "blocked")
        self.assertFalse(summary["corpusProof"]["conditions"]["artifactHashesBound"])
        self.assertIn("artifact-hashes-not-bound", summary["corpusProof"]["reasons"])

    def test_source_manifest_expected_blocker_path_must_match_actual(self):
        fixtures = _fixture_inputs("four-platform-mismatch")
        out = self._tmp_out("collector-expected-blocker-path-mismatch-")
        manifest = self._tmp_manifest(
            _manifest_doc(fixtures, {
                "match": False,
                "total": 1,
                "byPlatform": _expected_by_platform(mismatching=("android",)),
                "blockerPlatform": "android",
                "blockerPath": "results[0].name",
            })
        )

        with self.assertRaises(collector.CollectorError):
            collector.collect_real_run(
                run_id="collector-expected-blocker-path-mismatch",
                scenario="fixture-search",
                input_path=fixtures["input"],
                canonical_path=fixtures["canonical"],
                candidates=fixtures["candidates"],
                out_dir=out,
                source_manifest_path=manifest,
            )

    def test_source_manifest_platform_runs_must_use_same_core_commit(self):
        fixtures = _fixture_inputs("four-platform-match")
        out = self._tmp_out("collector-core-mismatch-")
        manifest_doc = _manifest_doc(fixtures, {"match": True, "total": 0})
        manifest_doc["platformRuns"]["android"]["coreCommit"] = "deadbee"
        manifest = self._tmp_manifest(manifest_doc)

        with self.assertRaises(collector.CollectorError):
            collector.collect_real_run(
                run_id="collector-core-mismatch",
                scenario="fixture-search",
                input_path=fixtures["input"],
                canonical_path=fixtures["canonical"],
                candidates=fixtures["candidates"],
                out_dir=out,
                source_manifest_path=manifest,
            )

    def test_source_manifest_requires_rust_business_kernel(self):
        fixtures = _fixture_inputs("four-platform-match")
        out = self._tmp_out("collector-kernel-mismatch-")
        manifest_doc = _manifest_doc(fixtures, {"match": True, "total": 0})
        manifest_doc["coreIdentity"]["businessKernel"] = "platform-wrapper"
        manifest = self._tmp_manifest(manifest_doc)

        with self.assertRaises(collector.CollectorError):
            collector.collect_real_run(
                run_id="collector-kernel-mismatch",
                scenario="fixture-search",
                input_path=fixtures["input"],
                canonical_path=fixtures["canonical"],
                candidates=fixtures["candidates"],
                out_dir=out,
                source_manifest_path=manifest,
            )

    def test_collects_mismatch_into_android_blocker(self):
        fixtures = _fixture_inputs("four-platform-mismatch")
        out = self._tmp_out("collector-mismatch-")

        summary = collector.collect_real_run(
            run_id="fixture-four-platform-mismatch",
            scenario="four-platform-search",
            input_path=fixtures["input"],
            canonical_path=fixtures["canonical"],
            candidates=fixtures["candidates"],
            out_dir=out,
            source_manifest_path=fixtures["manifest"],
        )

        self.assertFalse(summary["match"])
        self.assertEqual(summary["total"], 1)
        self.assertEqual(summary["hostParity"]["requiredPlatforms"], list(HOST_PLATFORMS))
        self.assertFalse(summary["hostParity"]["match"])
        self.assertEqual(summary["hostParity"]["total"], 1)
        self.assertEqual(
            summary["hostParity"]["byPlatform"]["android"],
            {"present": True, "match": False, "total": 1},
        )
        self.assertEqual(summary["corpusProof"]["status"], "blocked")
        self.assertNotIn("source-manifest-missing", summary["corpusProof"]["reasons"])
        self.assertTrue(summary["corpusProof"]["conditions"]["coreIdentityBound"])
        self.assertIn("full-diff-mismatch", summary["corpusProof"]["reasons"])
        self.assertIn("host-parity-mismatch", summary["corpusProof"]["reasons"])
        self.assertIn("open-blockers:1", summary["corpusProof"]["reasons"])
        self.assertEqual(summary["blockersAdded"], 1)
        self.assertEqual(summary["openByPlatform"], {"android": 1})

        register = rbr.load_register(os.path.join(out, "corpus-blocker-register.json"))
        blockers = rbr.filter_blockers(
            register["blockers"],
            status=rbr.STATUS_OPEN,
            run_id="fixture-four-platform-mismatch",
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
