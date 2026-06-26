#!/usr/bin/env python3
"""Build environment doctor.

READ-ONLY dev-time probe of the local toolchain. Discovers and versions the
build tools used by Reader-Core-Native (Xcode, Swift, NDK, JDK, DevEco, hdc,
cmake, cargo, rustc) and emits a machine-readable JSON manifest.

This tool NEVER executes a real build. It only invokes ``--version``-style
probes or locates binaries via ``shutil.which`` / environment variables.

Python 3.9+ standard library only.

CLI:
    python3 tools/build-env-doctor/build_env_doctor.py [--indent N] [--pretty]

Exit codes:
    0  - report emitted (even if tools are missing; this is a report, not a gate)
    2  - usage error
"""

import argparse
import json
import os
import platform
import re
import shutil
import subprocess
import sys
from datetime import datetime, timezone


# ---------------------------------------------------------------------------
# Version parse helpers
# ---------------------------------------------------------------------------
def _first_line(out):
    """Return the first non-empty stripped line of ``out``, or ''."""
    for line in (out or "").splitlines():
        line = line.strip()
        if line:
            return line
    return ""


def _parse_xcode(out):
    # "Xcode 15.2\nBuild version 15C500b" -> "Xcode 15.2"
    return _first_line(out)


def _parse_swift(out):
    # "Apple Swift version 5.9.2 (swiftlang-...)\n..." -> first line
    return _first_line(out)


def _parse_jdk(out):
    # java -version prints to stderr, e.g.:
    #   openjdk version "17.0.8" 2023-07-18
    #   OpenJDK Runtime Environment ...
    m = re.search(r'version "([^"]+)"', out or "")
    if m:
        return m.group(1)
    return _first_line(out)


def _parse_cmake(out):
    # "cmake version 3.28.3\n..." -> "3.28.3"
    m = re.search(r"cmake version\s+(\S+)", out or "")
    if m:
        return m.group(1)
    return _first_line(out)


def _parse_cargo(out):
    # "cargo 1.75.0 (1d8b05cdd 2023-11-20)\n" -> first line
    return _first_line(out)


def _parse_rustc(out):
    # "rustc 1.75.0 (82e160fb7 2023-12-21)\n" -> first line
    return _first_line(out)


def _parse_hdc(out):
    # "Ver: 1.2.3\n..." -> "1.2.3" if present, else first line
    m = re.search(r"Ver:\s*(\S+)", out or "")
    if m:
        return m.group(1)
    return _first_line(out)


def _parse_ndk(out):
    # ndk-build --version prints GNU Make info; best-effort first line.
    return _first_line(out)


def _parse_deveco(out):
    return _first_line(out)


# ---------------------------------------------------------------------------
# Probe specifications
# ---------------------------------------------------------------------------
# Each spec:
#   name          - tool name
#   discover      - candidate binary names / paths tried via shutil.which
#   version_args  - argv appended to the binary to probe the version
#   parse_version - (stdout:str) -> version:str
#   kind          - "build" | "language" | "device-tool" | "sdk"
#   env_vars      - (optional) env var names consulted when the binary is
#                   missing; if any is set, the tool is reported found.
PROBES = [
    {
        "name": "xcode",
        "discover": ["xcodebuild"],
        "version_args": ["-version"],
        "parse_version": _parse_xcode,
        "kind": "build",
    },
    {
        "name": "swift",
        "discover": ["swift"],
        "version_args": ["--version"],
        "parse_version": _parse_swift,
        "kind": "language",
    },
    {
        "name": "ndk",
        "discover": ["ndk-build"],
        "version_args": ["--version"],
        "parse_version": _parse_ndk,
        "kind": "sdk",
        "env_vars": ["ANDROID_NDK_HOME", "ANDROID_NDK"],
    },
    {
        "name": "jdk",
        "discover": ["java"],
        "version_args": ["-version"],
        "parse_version": _parse_jdk,
        "kind": "language",
    },
    {
        "name": "deveco",
        "discover": ["devecostudio"],
        "version_args": ["--version"],
        "parse_version": _parse_deveco,
        "kind": "sdk",
        "env_vars": ["DEVECO_SDK_HOME"],
    },
    {
        "name": "hdc",
        "discover": ["hdc"],
        "version_args": ["version"],
        "parse_version": _parse_hdc,
        "kind": "device-tool",
    },
    {
        "name": "cmake",
        "discover": ["cmake"],
        "version_args": ["--version"],
        "parse_version": _parse_cmake,
        "kind": "build",
    },
    {
        "name": "cargo",
        "discover": ["cargo"],
        "version_args": ["--version"],
        "parse_version": _parse_cargo,
        "kind": "build",
    },
    {
        "name": "rustc",
        "discover": ["rustc"],
        "version_args": ["--version"],
        "parse_version": _parse_rustc,
        "kind": "language",
    },
]


# ---------------------------------------------------------------------------
# Runner
# ---------------------------------------------------------------------------
def default_runner(args):
    """Execute a probe command, returning (returncode, stdout, stderr).

    NEVER raises. On timeout, missing binary, or any exception, returns
    (127, "", str(e)).
    """
    try:
        proc = subprocess.run(
            args, capture_output=True, text=True, timeout=10
        )
        return (proc.returncode, proc.stdout, proc.stderr)
    except FileNotFoundError as e:
        return (127, "", str(e))
    except subprocess.TimeoutExpired as e:
        return (127, "", str(e))
    except Exception as e:  # pragma: no cover - defensive catch-all
        return (127, "", str(e))


# ---------------------------------------------------------------------------
# Probe logic
# ---------------------------------------------------------------------------
def _discover_binary(spec):
    """Return the absolute path of the first discovered candidate, or ''."""
    for candidate in spec.get("discover", []):
        if not candidate:
            continue
        found = shutil.which(candidate)
        if found:
            return found
    return ""


def _env_var_lookup(spec):
    """Return (env_var_name, env_value) for the first set env var, or ('', '')."""
    for var in spec.get("env_vars", []):
        val = os.environ.get(var)
        if val:
            return (var, val)
    return ("", "")


def probe_tool(spec, runner=None):
    """Probe a single tool.

    Returns a dict:
        name, kind, found, path, version, status, notes
    where status is "ok" (found + version), "missing" (not found), or
    "unknown" (found but version empty).
    """
    if runner is None:
        runner = default_runner

    name = spec["name"]
    kind = spec["kind"]
    notes = ""

    binary_path = _discover_binary(spec)

    if binary_path:
        args = [binary_path] + list(spec.get("version_args", []))
        rc, out, err = runner(args)
        version = spec["parse_version"](out or "")
        # Some tools (e.g. java -version) print the version to stderr.
        if not version and err:
            version = spec["parse_version"](err)
        found = True
        status = "ok" if version else "unknown"
        return {
            "name": name,
            "kind": kind,
            "found": found,
            "path": binary_path,
            "version": version,
            "status": status,
            "notes": notes,
        }

    # Binary not on PATH; try env-var fallback (ndk, deveco).
    env_var, env_val = _env_var_lookup(spec)
    if env_var:
        notes = "found via env var %s" % env_var
        return {
            "name": name,
            "kind": kind,
            "found": True,
            "path": env_val,
            "version": "",
            "status": "unknown",
            "notes": notes,
        }

    return {
        "name": name,
        "kind": kind,
        "found": False,
        "path": "",
        "version": "",
        "status": "missing",
        "notes": notes,
    }


# ---------------------------------------------------------------------------
# Manifest
# ---------------------------------------------------------------------------
def run_doctor(runner=None):
    """Probe every tool and return the full manifest dict."""
    if runner is None:
        runner = default_runner

    tools = [probe_tool(spec, runner=runner) for spec in PROBES]
    found = sum(1 for t in tools if t["found"])
    missing = sum(1 for t in tools if t["status"] == "missing")
    unknown = sum(1 for t in tools if t["status"] == "unknown")

    return {
        "version": "build-env-doctor/1",
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "tool": "build-environment-doctor",
        "platform": {
            "os": platform.system(),
            "arch": platform.machine(),
            "kernel": platform.release(),
        },
        "tools": tools,
        "summary": {
            "total": len(tools),
            "found": found,
            "missing": missing,
            "unknown": unknown,
        },
    }


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------
def main(argv=None):
    """CLI entry point. Returns exit code (0 success, 2 usage error).

    On usage error argparse raises SystemExit(2) which propagates.
    """
    parser = argparse.ArgumentParser(
        prog="build_env_doctor",
        description="READ-ONLY build environment doctor. Emits a JSON manifest.",
    )
    parser.add_argument(
        "--indent",
        type=int,
        default=None,
        help="Indent JSON by N spaces.",
    )
    parser.add_argument(
        "--pretty",
        action="store_true",
        help="Pretty-print JSON (indent=2, sorted keys, trailing newline).",
    )
    args = parser.parse_args(argv)

    manifest = run_doctor()

    if args.pretty:
        text = json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    elif args.indent is not None:
        text = json.dumps(manifest, indent=args.indent)
    else:
        text = json.dumps(manifest)

    sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
