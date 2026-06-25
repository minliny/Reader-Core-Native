#!/usr/bin/env python3
"""Corpus real-run collector.

Collects already-produced CLI / iOS / Android / HarmonyOS JSON result files
into one candidate package, then wires that package through the existing
corpus canonicalizer, cross-platform diff, and release blocker register.

This tool does not run Core business logic, does not run platform adapters,
and does not certify a release. It only copies local run artifacts, emits a
packager-compatible run directory, and records diff-derived blockers.

Package layout::

    <out>/
      manifest.json
      platform-result.json
      canonical-result.json
      diff-result.json
      environment.json
      corpus-blocker-register.json
      input.json
      raw/
        cli-result.json
        ios-result.json
        android-result.json
        harmony-result.json
        canonical-result.json
      candidates/
        cli-result.json
        ios-result.json
        android-result.json
        harmony-result.json

Usage::

    python3 tools/corpus-real-run-collector/corpus_real_run_collector.py \\
      --run-id 2026-06-25-real-001 \\
      --scenario authorized-corpus-search \\
      --input input.json \\
      --canonical canonical-result.json \\
      --candidate cli:cli-result.json \\
      --candidate ios:ios-result.json \\
      --candidate android:android-result.json \\
      --candidate harmony:harmony-result.json
"""

import argparse
import datetime
import hashlib
import json
import os
import platform
import re
import shutil
import socket
import sys


_HERE = os.path.dirname(os.path.abspath(__file__))
_ROOT = os.path.abspath(os.path.join(_HERE, "..", ".."))
sys.path.insert(0, os.path.join(_ROOT, "scripts"))
sys.path.insert(0, os.path.join(_ROOT, "tools", "cross-platform-diff"))
sys.path.insert(0, os.path.join(_ROOT, "tools", "release-blocker-register"))

import corpus_canonicalize as cc  # noqa: E402
import cross_platform_diff as cpd  # noqa: E402
import release_blocker_register as rbr  # noqa: E402


TOOL_NAME = "corpus-real-run-collector"
TOOL_VERSION = "1.0"
SCHEMA_VERSION = 1

PRIVATE_TMP = "/private/tmp"
REQUIRED_PLATFORMS = ("cli", "ios", "android", "harmony")
_PLATFORM_RE = re.compile(r"^[a-z][a-z0-9_.-]*$")


class CollectorError(Exception):
    """Raised when the collector cannot validate inputs or write outputs."""


def _now_iso():
    return datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds")


def _documents_dir():
    return os.path.realpath(os.path.expanduser("~/Documents"))


def _is_under(path, base):
    path = os.path.realpath(path)
    base = os.path.realpath(base)
    try:
        return os.path.commonpath([path, base]) == base
    except ValueError:
        return False


def sanitize_for_path(name):
    if not name:
        return "run"
    cleaned = []
    for ch in name:
        if ch.isalnum() or ch in "-_.":
            cleaned.append(ch)
        else:
            cleaned.append("-")
    cleaned = "".join(cleaned).strip("-.")
    return cleaned or "run"


def assert_safe_output(path, user_specified):
    if not user_specified and not _is_under(path, PRIVATE_TMP):
        raise CollectorError(
            "default output must live under {0}; got {1}".format(PRIVATE_TMP, path)
        )
    if _is_under(path, _documents_dir()):
        raise CollectorError(
            "refusing to write output under ~/Documents ({0}); "
            "use /private/tmp or another disposable path".format(path)
        )


def resolve_output_path(run_id, out_dir):
    if out_dir is None:
        path = os.path.join(PRIVATE_TMP, "{0}-candidate".format(sanitize_for_path(run_id)))
        user_specified = False
    else:
        path = os.path.abspath(out_dir)
        user_specified = True
    assert_safe_output(path, user_specified)
    return path


def sha256_of_file(path, chunk_size=65536):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        while True:
            chunk = handle.read(chunk_size)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def _write_json(path, value):
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(value, handle, indent=2, ensure_ascii=False, sort_keys=True)
        handle.write("\n")


def _load_json(path, label):
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return json.load(handle)
    except FileNotFoundError:
        raise CollectorError("{0} file not found: {1}".format(label, path))
    except (OSError, IOError) as err:
        raise CollectorError("cannot read {0} {1}: {2}".format(label, path, err))
    except json.JSONDecodeError as err:
        raise CollectorError("invalid JSON in {0} {1}: {2}".format(label, path, err))


def _copy_file(src, dst):
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    shutil.copyfile(src, dst)


def parse_candidate_spec(spec):
    if ":" not in spec:
        raise CollectorError(
            "candidate must use NAME:PATH form; got {0}".format(spec)
        )
    name, path = spec.split(":", 1)
    name = name.strip()
    path = path.strip()
    if not name:
        raise CollectorError("empty candidate platform in spec: {0}".format(spec))
    if not _PLATFORM_RE.match(name):
        raise CollectorError("invalid candidate platform name: {0}".format(name))
    if not path:
        raise CollectorError("empty candidate path for platform: {0}".format(name))
    return name, path


def parse_candidate_specs(specs, required_platforms=REQUIRED_PLATFORMS):
    candidates = {}
    for spec in specs:
        name, path = parse_candidate_spec(spec)
        if name in candidates:
            raise CollectorError("duplicate candidate platform: {0}".format(name))
        candidates[name] = os.path.abspath(path)

    required = set(required_platforms)
    seen = set(candidates.keys())
    missing = sorted(required - seen)
    extra = sorted(seen - required)
    if missing:
        raise CollectorError("missing required candidate platform(s): " + ", ".join(missing))
    if extra:
        raise CollectorError("unknown candidate platform(s): " + ", ".join(extra))
    return {name: candidates[name] for name in required_platforms}


def canonicalize_file(src, dst, label):
    data = _load_json(src, label)
    _write_json(dst, cc.canonicalize(data))
    return {
        "sourcePath": os.path.abspath(src),
        "packagePath": os.path.relpath(dst, os.path.dirname(os.path.dirname(dst))).replace(os.sep, "/"),
        "sourceSha256": sha256_of_file(src),
        "packageSha256": sha256_of_file(dst),
    }


def _artifact_rel(out_dir, path):
    return os.path.relpath(path, out_dir).replace(os.sep, "/")


def build_platform_result(run_id, scenario, input_rel, candidate_records):
    return {
        "schemaVersion": SCHEMA_VERSION,
        "tool": TOOL_NAME,
        "version": TOOL_VERSION,
        "type": "four-platform-real-run-candidate-package",
        "runId": run_id,
        "scenario": scenario,
        "input": input_rel,
        "platforms": {
            platform: {
                "raw": record["rawPath"],
                "canonicalized": record["canonicalizedPath"],
                "sourcePath": record["sourcePath"],
                "sourceSha256": record["sourceSha256"],
                "canonicalizedSha256": record["canonicalizedSha256"],
            }
            for platform, record in candidate_records.items()
        },
    }


def build_environment(cwd):
    return {
        "collector": {
            "tool": TOOL_NAME,
            "version": TOOL_VERSION,
            "pythonVersion": platform.python_version(),
            "platform": platform.platform(),
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "processor": platform.processor() or None,
            "host": socket.gethostname(),
            "cpuCount": os.cpu_count(),
            "cwd": cwd,
        }
    }


def _load_optional_source_manifest(path):
    if path is None:
        return None
    return _load_json(path, "source manifest")


def collect_real_run(run_id, scenario, input_path, canonical_path, candidates,
                     out_dir=None, register_path=None, severity="high",
                     source_manifest_path=None, cwd=None):
    """Collect artifacts and return a summary dict.

    ``candidates`` must be an ordered mapping for ``cli``, ``ios``, ``android``,
    and ``harmony``. Output is a packager-compatible run directory.
    """
    if not run_id or not run_id.strip():
        raise CollectorError("run id is required")
    run_id = run_id.strip()
    scenario = (scenario or "").strip() or "corpus-real-run"
    cwd = cwd or os.getcwd()

    input_path = os.path.abspath(input_path)
    canonical_path = os.path.abspath(canonical_path)
    candidates = {name: os.path.abspath(path) for name, path in candidates.items()}
    source_manifest_path = (
        os.path.abspath(source_manifest_path) if source_manifest_path else None
    )

    # Validate JSON before cleaning the output directory.
    input_doc = _load_json(input_path, "input")
    source_manifest = _load_optional_source_manifest(source_manifest_path)
    _load_json(canonical_path, "canonical")
    for platform_name, path in candidates.items():
        _load_json(path, "{0} candidate".format(platform_name))

    out_user_specified = out_dir is not None
    out_dir = resolve_output_path(run_id, out_dir)
    register_user_specified = register_path is not None or out_user_specified
    if register_path is None:
        register_path = os.path.join(out_dir, rbr.DEFAULT_REGISTER_NAME)
    register_path = os.path.abspath(register_path)
    rbr.assert_safe_register(register_path, user_specified=register_user_specified)

    if os.path.exists(out_dir):
        if os.path.isdir(out_dir):
            shutil.rmtree(out_dir)
        else:
            os.remove(out_dir)
    os.makedirs(os.path.join(out_dir, "raw"), exist_ok=True)
    os.makedirs(os.path.join(out_dir, "candidates"), exist_ok=True)

    input_out = os.path.join(out_dir, "input.json")
    _write_json(input_out, input_doc)
    input_rel = _artifact_rel(out_dir, input_out)

    raw_canonical = os.path.join(out_dir, "raw", "canonical-result.json")
    _copy_file(canonical_path, raw_canonical)
    canonical_out = os.path.join(out_dir, "canonical-result.json")
    canonical_record = canonicalize_file(canonical_path, canonical_out, "canonical")
    canonical_record["rawPath"] = _artifact_rel(out_dir, raw_canonical)
    canonical_record["packagePath"] = _artifact_rel(out_dir, canonical_out)

    candidate_records = {}
    for platform_name in REQUIRED_PLATFORMS:
        src = candidates[platform_name]
        raw_dst = os.path.join(out_dir, "raw", "{0}-result.json".format(platform_name))
        canon_dst = os.path.join(
            out_dir, "candidates", "{0}-result.json".format(platform_name)
        )
        _copy_file(src, raw_dst)
        canonicalize_file(src, canon_dst, "{0} candidate".format(platform_name))
        candidate_records[platform_name] = {
            "sourcePath": src,
            "rawPath": _artifact_rel(out_dir, raw_dst),
            "canonicalizedPath": _artifact_rel(out_dir, canon_dst),
            "sourceSha256": sha256_of_file(src),
            "rawSha256": sha256_of_file(raw_dst),
            "canonicalizedSha256": sha256_of_file(canon_dst),
        }

    diff_candidates = [
        (
            platform_name,
            os.path.join(out_dir, "candidates", "{0}-result.json".format(platform_name)),
        )
        for platform_name in REQUIRED_PLATFORMS
    ]
    diff_result = cpd.build_diff_result(canonical_out, diff_candidates)
    diff_out = os.path.join(out_dir, "diff-result.json")
    _write_json(diff_out, diff_result)

    platform_result = build_platform_result(
        run_id,
        scenario,
        input_rel,
        candidate_records,
    )
    platform_out = os.path.join(out_dir, "platform-result.json")
    _write_json(platform_out, platform_result)

    environment = build_environment(cwd)
    environment_out = os.path.join(out_dir, "environment.json")
    _write_json(environment_out, environment)

    register = rbr.load_register(register_path)
    added_blockers = rbr.add_blockers_from_diff(
        register,
        diff_result,
        run_id=run_id,
        severity=severity,
    )
    rbr.save_register(register_path, register)
    open_count, open_breakdown = rbr.gate_evaluate(register, run_id=run_id)

    manifest = {
        "schemaVersion": SCHEMA_VERSION,
        "tool": TOOL_NAME,
        "version": TOOL_VERSION,
        "runId": run_id,
        "scenario": scenario,
        "createdAt": _now_iso(),
        "sourceManifest": source_manifest,
        "input": {
            "sourcePath": input_path,
            "packagePath": input_rel,
            "sourceSha256": sha256_of_file(input_path),
            "packageSha256": sha256_of_file(input_out),
        },
        "canonical": canonical_record,
        "candidates": candidate_records,
        "artifacts": {
            "platformResult": _artifact_rel(out_dir, platform_out),
            "canonicalResult": _artifact_rel(out_dir, canonical_out),
            "diffResult": _artifact_rel(out_dir, diff_out),
            "environment": _artifact_rel(out_dir, environment_out),
            "blockerRegister": _artifact_rel(out_dir, register_path),
        },
        "diffSummary": {
            "match": diff_result["match"],
            "total": diff_result["total"],
            "byPlatform": diff_result["summary"],
        },
        "blockers": {
            "registerPath": register_path,
            "added": len(added_blockers),
            "open": open_count,
            "openByPlatform": open_breakdown,
        },
        "releaseCertification": "not-assessed",
        "notes": [
            "Collector only packages already-produced local JSON run artifacts.",
            "Diff-derived blockers must be resolved or waived by separate review.",
        ],
    }
    manifest_out = os.path.join(out_dir, "manifest.json")
    _write_json(manifest_out, manifest)

    return {
        "runId": run_id,
        "scenario": scenario,
        "outDir": out_dir,
        "manifest": manifest_out,
        "diffResult": diff_out,
        "register": register_path,
        "match": diff_result["match"],
        "total": diff_result["total"],
        "blockersAdded": len(added_blockers),
        "openBlockers": open_count,
        "openByPlatform": open_breakdown,
    }


def render_summary(summary):
    lines = [
        "Corpus real-run collector summary",
        "  tool: {0} v{1}".format(TOOL_NAME, TOOL_VERSION),
        "  runId: {0}".format(summary["runId"]),
        "  scenario: {0}".format(summary["scenario"]),
        "  outDir: {0}".format(summary["outDir"]),
        "  diff: {0} (total differences: {1})".format(
            "match" if summary["match"] else "mismatch",
            summary["total"],
        ),
        "  blockers added: {0}".format(summary["blockersAdded"]),
        "  open blockers: {0}".format(summary["openBlockers"]),
        "  register: {0}".format(summary["register"]),
    ]
    if summary["openByPlatform"]:
        lines.append("  open by platform:")
        for platform_name in sorted(summary["openByPlatform"].keys()):
            lines.append(
                "    {0}: {1}".format(platform_name, summary["openByPlatform"][platform_name])
            )
    return "\n".join(lines) + "\n"


def parse_args(argv):
    parser = argparse.ArgumentParser(
        prog=TOOL_NAME,
        description=(
            "Collect already-produced CLI / iOS / Android / HarmonyOS corpus "
            "JSON results into one candidate package, then run canonicalizer, "
            "cross-platform diff, and blocker registration."
        ),
    )
    parser.add_argument("--run-id", required=True, help="Stable id for this corpus run.")
    parser.add_argument("--scenario", default="corpus-real-run")
    parser.add_argument("--input", required=True, help="Path to the shared input JSON.")
    parser.add_argument(
        "--source-manifest",
        default=None,
        help="Optional source run manifest to embed in the collector manifest.",
    )
    parser.add_argument(
        "--canonical",
        required=True,
        help="Path to the canonical reference JSON result.",
    )
    parser.add_argument(
        "--candidate",
        action="append",
        default=[],
        metavar="PLATFORM:PATH",
        help=(
            "Candidate result JSON. Repeat for cli, ios, android, and harmony. "
            "All four are required."
        ),
    )
    parser.add_argument(
        "--out",
        dest="out_dir",
        default=None,
        help="Output package directory. Default: /private/tmp/<run-id>-candidate.",
    )
    parser.add_argument(
        "--register",
        default=None,
        help=(
            "Blocker register path. Default: <out>/corpus-blocker-register.json. "
            "Must not be under ~/Documents."
        ),
    )
    parser.add_argument(
        "--severity",
        choices=(rbr.SEVERITY_HIGH, rbr.SEVERITY_MEDIUM, rbr.SEVERITY_LOW),
        default=rbr.SEVERITY_HIGH,
        help="Severity for blockers derived from this run's diff.",
    )
    return parser.parse_args(argv)


def main(argv=None):
    if argv is None:
        argv = sys.argv[1:]
    args = parse_args(argv)
    try:
        candidates = parse_candidate_specs(args.candidate)
        summary = collect_real_run(
            run_id=args.run_id,
            scenario=args.scenario,
            input_path=args.input,
            canonical_path=args.canonical,
            candidates=candidates,
            out_dir=args.out_dir,
            register_path=args.register,
            severity=args.severity,
            source_manifest_path=args.source_manifest,
        )
    except (CollectorError, rbr.RegisterError, cpd.DiffError) as err:
        sys.stderr.write("error: {0}\n".format(err))
        return 2
    sys.stdout.write(render_summary(summary))
    return 0


if __name__ == "__main__":
    sys.exit(main())
