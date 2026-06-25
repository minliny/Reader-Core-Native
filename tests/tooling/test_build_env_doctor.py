"""Tests for tools/build-env-doctor/build_env_doctor.py.

Strict TDD: these tests are written BEFORE the implementation. They use a
fake runner so they NEVER touch the real host environment and are fully
deterministic (they do not depend on whether Xcode/Swift/etc. are installed).

Run:
    python3 -m unittest tests.tooling.test_build_env_doctor -v
"""

import io
import json
import os
import sys
import unittest
from contextlib import redirect_stderr, redirect_stdout
from datetime import datetime
from unittest import mock

# The tool lives at tools/build-env-doctor/build_env_doctor.py. The directory
# name contains a hyphen so it is not an importable package; add it to sys.path.
_TOOL_DIR = os.path.join(
    os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
    "tools",
    "build-env-doctor",
)
if _TOOL_DIR not in sys.path:
    sys.path.insert(0, _TOOL_DIR)

import build_env_doctor as bed  # noqa: E402  (import after sys.path tweak)


def _which_returns(path):
    """Build a shutil.which replacement that returns `path` for any query."""
    return lambda cmd, *a, **kw: path


class FakeRunner:
    """Canned subprocess runner: (args_list) -> (returncode, stdout, stderr).

    Matches outputs by (1) the full first arg, (2) the basename of the first
    arg (since probe_tool passes the absolute path returned by shutil.which),
    (3) the joined arg string. Falls back to (127, "", "command not found").
    """

    def __init__(self, outputs):
        # outputs maps a binary name / full path / joined args -> result tuple.
        self.outputs = outputs
        self.calls = []

    def __call__(self, args):
        self.calls.append(list(args))
        key = args[0] if args else ""
        if key in self.outputs:
            return self.outputs[key]
        # Try the basename (probe_tool passes absolute paths from shutil.which).
        base = os.path.basename(key) if key else ""
        if base in self.outputs:
            return self.outputs[base]
        joined = " ".join(args)
        if joined in self.outputs:
            return self.outputs[joined]
        return (127, "", "command not found")


# ---------------------------------------------------------------------------
# 1. probe_tool found + version parsed -> status "ok"
# ---------------------------------------------------------------------------
class ProbeToolFoundTests(unittest.TestCase):
    def test_found_with_version_parsed_status_ok(self):
        spec = {
            "name": "swift",
            "discover": ["swift"],
            "version_args": ["--version"],
            "parse_version": lambda out: out.splitlines()[0].strip()
            if out.splitlines()
            else "",
            "kind": "language",
        }
        runner = FakeRunner({"swift": (0, "swift-5.9.2\nApple Swift version 5.9.2\n", "")})
        with mock.patch("shutil.which", new=_which_returns("/usr/bin/swift")):
            result = bed.probe_tool(spec, runner=runner)
        self.assertEqual(result["name"], "swift")
        self.assertEqual(result["kind"], "language")
        self.assertTrue(result["found"])
        self.assertEqual(result["path"], "/usr/bin/swift")
        self.assertEqual(result["version"], "swift-5.9.2")
        self.assertEqual(result["status"], "ok")
        self.assertEqual(result["notes"], "")


# ---------------------------------------------------------------------------
# 2. probe_tool binary missing -> status "missing", found false
# ---------------------------------------------------------------------------
class ProbeToolMissingTests(unittest.TestCase):
    def test_binary_missing_status_missing(self):
        spec = {
            "name": "xcode",
            "discover": ["xcodebuild"],
            "version_args": ["-version"],
            "parse_version": lambda out: out.strip(),
            "kind": "build",
        }
        runner = FakeRunner({"xcodebuild": (127, "", "not found")})
        with mock.patch("shutil.which", new=_which_returns(None)):
            result = bed.probe_tool(spec, runner=runner)
        self.assertFalse(result["found"])
        self.assertEqual(result["path"], "")
        self.assertEqual(result["version"], "")
        self.assertEqual(result["status"], "missing")


# ---------------------------------------------------------------------------
# 3. probe_tool found but version parse empty -> status "unknown"
# ---------------------------------------------------------------------------
class ProbeToolUnknownVersionTests(unittest.TestCase):
    def test_found_but_version_empty_status_unknown(self):
        spec = {
            "name": "hdc",
            "discover": ["hdc"],
            "version": ["version"],
            "version_args": ["version"],
            "parse_version": lambda out: "",  # parser yields nothing
            "kind": "device-tool",
        }
        runner = FakeRunner({"hdc": (0, "some unverifiable output\n", "")})
        with mock.patch("shutil.which", new=_which_returns("/usr/local/bin/hdc")):
            result = bed.probe_tool(spec, runner=runner)
        self.assertTrue(result["found"])
        self.assertEqual(result["path"], "/usr/local/bin/hdc")
        self.assertEqual(result["version"], "")
        self.assertEqual(result["status"], "unknown")


# ---------------------------------------------------------------------------
# 4. run_doctor with fake runner covering all tools -> summary counts
# ---------------------------------------------------------------------------
# Env var names that probes consult as a fallback when the binary is missing.
# Tests clear these so "all missing" scenarios are deterministic regardless of
# the host developer machine's real environment.
_PROBE_ENV_VARS = (
    "ANDROID_NDK_HOME",
    "ANDROID_NDK",
    "DEVECO_SDK_HOME",
)


def _clear_probe_env():
    """Return a dict that masks all probe env vars as unset.

    os._Environ rejects None values, so we use empty strings. probe_tool
    treats empty-string env vars as "not set" (truthiness check), achieving
    the desired "missing" semantics. mock.patch.dict restores originals after.
    """
    return {k: "" for k in _PROBE_ENV_VARS}


class RunDoctorSummaryTests(unittest.TestCase):
    def _fake_runner_for_all(self):
        """A fake runner that reports every tool as found with a version."""
        outputs = {}
        for spec in bed.PROBES:
            # Use the first discover candidate as the lookup key.
            primary = spec["discover"][0] if spec["discover"] else spec["name"]
            outputs[primary] = (0, "1.2.3\n", "")
            outputs[spec["name"]] = (0, "1.2.3\n", "")
        return FakeRunner(outputs)

    def test_summary_counts_match_len_probes(self):
        # Every tool found via which + runner returns version.
        with mock.patch("shutil.which", new=_which_returns("/fake/bin/tool")):
            manifest = bed.run_doctor(runner=self._fake_runner_for_all())
        self.assertEqual(manifest["summary"]["total"], len(bed.PROBES))
        self.assertEqual(manifest["summary"]["found"], len(bed.PROBES))
        self.assertEqual(manifest["summary"]["missing"], 0)
        self.assertEqual(manifest["summary"]["unknown"], 0)
        # Each entry is well-formed.
        for entry in manifest["tools"]:
            for key in ("name", "kind", "found", "path", "version", "status", "notes"):
                self.assertIn(key, entry, f"missing key {key} in {entry}")

    def test_summary_counts_when_all_missing(self):
        with mock.patch("shutil.which", new=_which_returns(None)), \
                mock.patch.dict(os.environ, _clear_probe_env(), clear=False):
            manifest = bed.run_doctor(runner=FakeRunner({}))
        self.assertEqual(manifest["summary"]["total"], len(bed.PROBES))
        self.assertEqual(manifest["summary"]["found"], 0)
        self.assertEqual(manifest["summary"]["missing"], len(bed.PROBES))
        self.assertEqual(manifest["summary"]["unknown"], 0)

    def test_summary_found_plus_missing_equals_total(self):
        # Invariant: found + missing == total (unknown is a subset of found).
        with mock.patch("shutil.which", new=_which_returns(None)), \
                mock.patch.dict(os.environ, _clear_probe_env(), clear=False):
            manifest = bed.run_doctor(runner=FakeRunner({}))
        s = manifest["summary"]
        self.assertEqual(s["found"] + s["missing"], s["total"])


# ---------------------------------------------------------------------------
# 5. CLI --pretty outputs valid JSON, exit 0
# ---------------------------------------------------------------------------
class CliPrettyTests(unittest.TestCase):
    def test_pretty_outputs_valid_json_exit0(self):
        buf = io.StringIO()
        with redirect_stdout(buf):
            with mock.patch("shutil.which", new=_which_returns(None)):
                rc = bed.main(["--pretty"])
        out = buf.getvalue()
        self.assertEqual(rc, 0)
        # --pretty should end with a trailing newline.
        self.assertTrue(out.endswith("\n"))
        parsed = json.loads(out)
        self.assertIn("version", parsed)
        self.assertIn("tools", parsed)
        self.assertIn("summary", parsed)

    def test_indent_argument_outputs_valid_json(self):
        buf = io.StringIO()
        with redirect_stdout(buf):
            with mock.patch("shutil.which", new=_which_returns(None)):
                rc = bed.main(["--indent", "4"])
        self.assertEqual(rc, 0)
        parsed = json.loads(buf.getvalue())
        self.assertIn("tools", parsed)


# ---------------------------------------------------------------------------
# 6. platform field contains os/arch/kernel keys
# ---------------------------------------------------------------------------
class PlatformFieldTests(unittest.TestCase):
    def test_platform_has_os_arch_kernel(self):
        with mock.patch("shutil.which", new=_which_returns(None)):
            manifest = bed.run_doctor(runner=FakeRunner({}))
        plat = manifest["platform"]
        self.assertIn("os", plat)
        self.assertIn("arch", plat)
        self.assertIn("kernel", plat)
        self.assertIsInstance(plat["os"], str)
        self.assertIsInstance(plat["arch"], str)
        self.assertIsInstance(plat["kernel"], str)


# ---------------------------------------------------------------------------
# 7. default_runner on a definitely-missing binary returns (127, "", ...)
#    without raising.
# ---------------------------------------------------------------------------
class DefaultRunnerTests(unittest.TestCase):
    def test_default_runner_missing_binary_no_raise(self):
        # A command name that cannot exist on the host.
        bogus = "zzz_definitely_not_a_real_binary_xyz_12345"
        rc, out, err = bed.default_runner([bogus, "--version"])
        self.assertEqual(rc, 127)
        self.assertEqual(out, "")
        self.assertIsInstance(err, str)
        self.assertTrue(len(err) > 0)

    def test_default_runner_success_shape(self):
        # `true` exists on macOS and exits 0 with no output.
        rc, out, err = bed.default_runner(["true"])
        self.assertEqual(rc, 0)
        self.assertEqual(out, "")
        self.assertEqual(err, "")


# ---------------------------------------------------------------------------
# 8. env-var probe (ndk via ANDROID_NDK_HOME) found when env set + binary
#    missing -> found true, status ok or unknown.
# ---------------------------------------------------------------------------
class EnvVarProbeTests(unittest.TestCase):
    def test_ndk_found_via_env_var_when_binary_missing(self):
        # Find the ndk spec.
        ndk_spec = None
        for spec in bed.PROBES:
            if spec["name"] == "ndk":
                ndk_spec = spec
                break
        self.assertIsNotNone(ndk_spec, "PROBES must include an 'ndk' entry")

        env = {
            "ANDROID_NDK_HOME": "/fake/Android/Sdk/ndk/25.1.8937393",
            "ANDROID_NDK": "/fake/Android/Sdk/ndk/25.1.8937393",
        }
        runner = FakeRunner({"ndk-build": (127, "", "not found")})
        with mock.patch("shutil.which", new=_which_returns(None)), \
                mock.patch.dict(os.environ, env, clear=False):
            result = bed.probe_tool(ndk_spec, runner=runner)
        self.assertTrue(result["found"])
        self.assertIn(result["status"], ("ok", "unknown"))
        # Path should reference the env-var value (non-empty).
        self.assertTrue(result["path"])

    def test_deveco_found_via_env_var_when_binary_missing(self):
        deveco_spec = None
        for spec in bed.PROBES:
            if spec["name"] == "deveco":
                deveco_spec = spec
                break
        self.assertIsNotNone(deveco_spec, "PROBES must include a 'deveco' entry")
        env = {"DEVECO_SDK_HOME": "/fake/DeEcoStudio/sdk"}
        runner = FakeRunner({})
        with mock.patch("shutil.which", new=_which_returns(None)), \
                mock.patch.dict(os.environ, env, clear=False):
            result = bed.probe_tool(deveco_spec, runner=runner)
        self.assertTrue(result["found"])
        self.assertIn(result["status"], ("ok", "unknown"))


# ---------------------------------------------------------------------------
# Edge cases
# ---------------------------------------------------------------------------
class EdgeCaseTests(unittest.TestCase):
    def test_probes_contains_required_tools(self):
        names = {s["name"] for s in bed.PROBES}
        for required in (
            "xcode",
            "swift",
            "ndk",
            "jdk",
            "deveco",
            "hdc",
            "cmake",
            "cargo",
            "rustc",
        ):
            self.assertIn(required, names, f"PROBES missing {required}")

    def test_each_spec_has_required_fields(self):
        for spec in bed.PROBES:
            self.assertIn("name", spec)
            self.assertIn("discover", spec)
            self.assertIn("version_args", spec)
            self.assertIn("parse_version", spec)
            self.assertIn("kind", spec)
            self.assertIn(spec["kind"], ("build", "language", "device-tool", "sdk"))

    def test_manifest_version_string(self):
        with mock.patch("shutil.which", new=_which_returns(None)):
            manifest = bed.run_doctor(runner=FakeRunner({}))
        self.assertEqual(manifest["version"], "build-env-doctor/1")
        self.assertEqual(manifest["tool"], "build-environment-doctor")
        self.assertIn("generated_at", manifest)

    def test_xcode_version_parse(self):
        # "Xcode 15.2\nBuild version 15C500b" -> "Xcode 15.2"
        spec = None
        for s in bed.PROBES:
            if s["name"] == "xcode":
                spec = s
                break
        self.assertIsNotNone(spec)
        out = "Xcode 15.2\nBuild version 15C500b\n"
        self.assertEqual(spec["parse_version"](out), "Xcode 15.2")

    def test_cli_usage_error_exit2(self):
        # Unknown argument -> exit 2. Suppress argparse's stderr usage message
        # so the test output stays pristine.
        buf = io.StringIO()
        with redirect_stdout(buf):
            with self.assertRaises(SystemExit) as ctx, \
                    open(os.devnull, "w") as devnull, \
                    redirect_stderr(devnull):
                bed.main(["--no-such-flag"])
        self.assertEqual(ctx.exception.code, 2)

    def test_cli_no_args_outputs_valid_json(self):
        buf = io.StringIO()
        with redirect_stdout(buf):
            with mock.patch("shutil.which", new=_which_returns(None)):
                rc = bed.main([])
        self.assertEqual(rc, 0)
        parsed = json.loads(buf.getvalue())
        self.assertIn("tools", parsed)

    def test_env_var_empty_string_treated_as_missing(self):
        # An env var set to "" should NOT count as "found".
        ndk_spec = next(s for s in bed.PROBES if s["name"] == "ndk")
        env = {"ANDROID_NDK_HOME": "", "ANDROID_NDK": ""}
        runner = FakeRunner({})
        with mock.patch("shutil.which", new=_which_returns(None)), \
                mock.patch.dict(os.environ, env, clear=False):
            result = bed.probe_tool(ndk_spec, runner=runner)
        self.assertFalse(result["found"])
        self.assertEqual(result["status"], "missing")

    def test_generated_at_is_iso8601(self):
        # The timestamp must be a parseable ISO8601 UTC string.
        with mock.patch("shutil.which", new=_which_returns(None)):
            manifest = bed.run_doctor(runner=FakeRunner({}))
        ts = manifest["generated_at"]
        # Should end with 'Z' (UTC) and parse via datetime.fromisoformat.
        self.assertTrue(ts.endswith("Z"))
        # Python 3.9 fromisoformat does not accept the trailing 'Z'; strip it.
        parsed = datetime.fromisoformat(ts[:-1])
        self.assertIsNotNone(parsed)

    def test_pretty_output_sorted_keys(self):
        # --pretty must emit sorted keys. Python dicts preserve insertion order,
        # so the parsed top-level key order reflects the JSON serialization order.
        buf = io.StringIO()
        with redirect_stdout(buf):
            with mock.patch("shutil.which", new=_which_returns(None)):
                bed.main(["--pretty"])
        parsed = json.loads(buf.getvalue())
        top_keys = list(parsed.keys())
        self.assertEqual(top_keys, sorted(top_keys))
        # Sanity: a few expected top-level keys are present and ordered.
        self.assertEqual(top_keys[0], "generated_at")
        self.assertEqual(top_keys[-1], "version")

    def test_jdk_version_parsed_from_stderr(self):
        # java -version writes the version to stderr; probe_tool must fall back
        # to parsing stderr when stdout yields no version.
        jdk_spec = next(s for s in bed.PROBES if s["name"] == "jdk")
        runner = FakeRunner({
            "java": (0, "", 'openjdk version "17.0.8" 2023-07-18\n'),
        })
        with mock.patch("shutil.which", new=_which_returns("/usr/bin/java")):
            result = bed.probe_tool(jdk_spec, runner=runner)
        self.assertTrue(result["found"])
        self.assertEqual(result["version"], "17.0.8")
        self.assertEqual(result["status"], "ok")

    def test_probe_found_runner_nonzero_but_version_present(self):
        # If the binary is found and a version is parseable, status is "ok"
        # even if the probe exits non-zero (some tools are quirky).
        spec = {
            "name": "cmake",
            "discover": ["cmake"],
            "version_args": ["--version"],
            "parse_version": lambda out: _first_line_of(out),
            "kind": "build",
        }
        runner = FakeRunner({"cmake": (1, "cmake version 3.28.3\n", "")})
        with mock.patch("shutil.which", new=_which_returns("/usr/bin/cmake")):
            result = bed.probe_tool(spec, runner=runner)
        self.assertTrue(result["found"])
        self.assertEqual(result["version"], "cmake version 3.28.3")
        self.assertEqual(result["status"], "ok")


def _first_line_of(out):
    for line in (out or "").splitlines():
        line = line.strip()
        if line:
            return line
    return ""


if __name__ == "__main__":
    unittest.main()
