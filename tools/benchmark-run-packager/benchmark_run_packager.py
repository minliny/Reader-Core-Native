#!/usr/bin/env python3
"""Benchmark run packager.

Packages a single corpus benchmark *run directory* into an archivable bundle
directory (and optionally a zip) containing the run's inputs, outputs, logs and
environment information, plus a generated ``summary.json``.

A run directory is expected to look like::

    <run-dir>/
      manifest.json           # required — run metadata (runId, timestamp, ...)
      platform-result.json    # required — the platform's actual output
      canonical-result.json   # required — the canonical reference output
      diff-result.json        # required — diff between platform and canonical
      environment.json        # optional — captured environment info
      logs/                   # optional — log files
      *.log                   # optional — log files at the root
      ...                     # any other artifacts are copied through

This tool does **not** run any benchmark. It only validates and packages
already-produced results.

Output location rules (per project constraint "不写 Documents 新目录"):

  * The default bundle directory and default zip path live under
    ``/private/tmp``.
  * A user may override either with ``--out <dir>`` / ``--zip <path>``.
  * The tool refuses to write any output under ``~/Documents`` (which also
    covers the repository working tree), regardless of whether the path was
    defaulted or user-specified.

The module is intentionally stdlib-only (no third-party dependencies) and does
not touch any Core business logic.
"""

import argparse
import datetime
import hashlib
import json
import os
import platform
import shutil
import socket
import sys
import zipfile


TOOL_NAME = "benchmark-run-packager"
TOOL_VERSION = "1.1"
SCHEMA_VERSION = 1

DIFF_CLASS_KEYS = (
    "core-semantic-difference",
    "host-capability-difference",
    "platform-output-missing",
)

# (artifact key, expected filename). Order is stable for deterministic output.
REQUIRED_ARTIFACTS = [
    ("manifest", "manifest.json"),
    ("platform-result", "platform-result.json"),
    ("canonical-result", "canonical-result.json"),
    ("diff-result", "diff-result.json"),
]

PRIVATE_TMP = "/private/tmp"


class PackagingError(Exception):
    """Raised when a run directory cannot be validated or packaged."""


# --------------------------------------------------------------------------- #
# Path helpers
# --------------------------------------------------------------------------- #
def _documents_dir():
    return os.path.realpath(os.path.expanduser("~/Documents"))


def _is_under(path, base):
    """Return True if absolute ``path`` is equal to or nested under ``base``."""
    path = os.path.realpath(path)
    base = os.path.realpath(base)
    try:
        return os.path.commonpath([path, base]) == base
    except ValueError:
        # Different drives / roots on some platforms.
        return False


def assert_safe_output(path, user_specified):
    """Enforce the output-location policy.

    Default (non-user-specified) paths must resolve under ``/private/tmp``.
    No output — default or user-specified — may be written under
    ``~/Documents``.
    """
    if not user_specified:
        if not _is_under(path, PRIVATE_TMP):
            raise PackagingError(
                "default output must live under {tmp}; got {path}".format(
                    tmp=PRIVATE_TMP, path=path
                )
            )
    docs = _documents_dir()
    if _is_under(path, docs):
        raise PackagingError(
            "refusing to write output under ~/Documents ({path}); "
            "use /private/tmp or another user-specified path".format(path=path)
        )


def sanitize_for_path(name):
    """Make ``name`` safe to use as a single path component."""
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


# --------------------------------------------------------------------------- #
# File / JSON helpers
# --------------------------------------------------------------------------- #
def load_json_file(path):
    """Parse a JSON file. Returns the decoded value.

    Raises :class:`PackagingError` if the file cannot be read or parsed.
    """
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return json.load(handle)
    except FileNotFoundError:
        raise PackagingError("missing file: {0}".format(path))
    except (OSError, IOError) as err:
        raise PackagingError("cannot read {0}: {1}".format(path, err))
    except json.JSONDecodeError as err:
        raise PackagingError("invalid JSON in {0}: {1}".format(path, err))


def _load_optional_json(run_dir, filename):
    """Load an optional JSON file from ``run_dir``.

    Returns the decoded value, or ``None`` if the file is absent or not valid
    JSON. Optional artifacts (e.g. ``environment.json``) must never block
    packaging.
    """
    path = os.path.join(run_dir, filename)
    if not os.path.isfile(path):
        return None
    try:
        return load_json_file(path)
    except PackagingError:
        return None


def sha256_of_file(path, chunk_size=65536):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        while True:
            chunk = handle.read(chunk_size)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def list_run_files(run_dir):
    """Return a sorted list of file descriptors for every file under ``run_dir``.

    Each descriptor is ``{"path": <posix relative path>, "size": int,
    "sha256": str}``.
    """
    run_dir = os.path.abspath(run_dir)
    entries = []
    for root, _dirs, files in os.walk(run_dir):
        for name in files:
            full = os.path.join(root, name)
            rel = os.path.relpath(full, run_dir)
            entries.append(
                {
                    "path": rel.replace(os.sep, "/"),
                    "size": os.path.getsize(full),
                    "sha256": sha256_of_file(full),
                }
            )
    entries.sort(key=lambda item: item["path"])
    return entries


# --------------------------------------------------------------------------- #
# Validation
# --------------------------------------------------------------------------- #
def validate_run_dir(run_dir):
    """Validate that ``run_dir`` contains the four required artifacts as JSON.

    Returns a tuple ``(validation, loaded)`` where ``validation`` is a dict
    describing presence/validity per artifact and ``loaded`` maps the artifact
    key to its decoded JSON value (only for artifacts that parsed successfully).
    """
    if not os.path.isdir(run_dir):
        raise PackagingError("run directory does not exist: {0}".format(run_dir))

    run_dir = os.path.abspath(run_dir)
    required = {}
    missing = []
    invalid_json = []
    loaded = {}

    for key, filename in REQUIRED_ARTIFACTS:
        path = os.path.join(run_dir, filename)
        present = os.path.isfile(path)
        valid_json = False
        if present:
            try:
                loaded[key] = load_json_file(path)
                valid_json = True
            except PackagingError:
                invalid_json.append(key)
        else:
            missing.append(key)
        required[key] = {
            "filename": filename,
            "present": present,
            "validJson": valid_json,
        }

    ok = not missing and not invalid_json
    validation = {
        "ok": ok,
        "required": required,
        "missing": missing,
        "invalidJson": invalid_json,
    }
    return validation, loaded


# --------------------------------------------------------------------------- #
# Summary derivation
# --------------------------------------------------------------------------- #
def derive_run_id(run_dir, manifest):
    """Determine the run id from the manifest, falling back to the dir name."""
    if isinstance(manifest, dict):
        for key in ("runId", "run_id", "id"):
            value = manifest.get(key)
            if isinstance(value, str) and value.strip():
                return value.strip()
    return os.path.basename(os.path.abspath(run_dir.rstrip(os.sep))) or "run"


def derive_diff_summary(diff_result):
    """Derive a compact summary from a diff-result document.

    Understands the shape produced by the sibling ``cross-platform-diff`` tool
    (a per-candidate ``summary`` with ``total`` counts) as well as plain
    ``{"match": bool}`` / ``{"total": int}`` documents.
    """
    match = None
    total = None
    classes = {key: 0 for key in DIFF_CLASS_KEYS}
    release_gate = None

    if isinstance(diff_result, dict):
        if isinstance(diff_result.get("releaseGate"), dict):
            release_gate = diff_result["releaseGate"]

        summary = diff_result.get("summary")
        if isinstance(summary, dict) and summary:
            counted = 0
            seen = False
            for value in summary.values():
                if isinstance(value, dict) and "total" in value:
                    try:
                        counted += int(value["total"])
                        seen = True
                    except (TypeError, ValueError):
                        pass
                class_counts = value.get("differenceClasses") if isinstance(value, dict) else None
                if isinstance(class_counts, dict):
                    for key, amount in class_counts.items():
                        try:
                            classes[key] = classes.get(key, 0) + int(amount)
                        except (TypeError, ValueError):
                            pass
            if seen:
                total = counted
                match = counted == 0
        elif "match" in diff_result and isinstance(diff_result["match"], bool):
            match = diff_result["match"]
        elif "total" in diff_result:
            try:
                total = int(diff_result["total"])
                match = total == 0
            except (TypeError, ValueError):
                pass

    return {
        "match": match,
        "total": total,
        "differenceClasses": classes,
        "releaseGate": release_gate,
    }


def collect_environment(run_dir, env_value):
    """Build the environment section of the summary.

    Always records the packager's own runtime environment and, if the run
    directory contains a valid ``environment.json`` object, nests it under
    ``run``.
    """
    section = {
        "packager": {
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
            "cwd": os.getcwd(),
        }
    }
    if isinstance(env_value, dict):
        section["run"] = env_value
    else:
        section["run"] = None
    return section


def build_summary(run_dir, validation, loaded, files, diff_summary, environment,
                  bundle_out, zip_path):
    """Assemble the ``summary.json`` document."""
    manifest = loaded.get("manifest") if isinstance(loaded, dict) else None
    run_id = derive_run_id(run_dir, manifest)
    now = datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds")
    return {
        "schemaVersion": SCHEMA_VERSION,
        "tool": TOOL_NAME,
        "version": TOOL_VERSION,
        "packagedAt": now,
        "runId": run_id,
        "runDir": os.path.abspath(run_dir),
        "runDirName": os.path.basename(os.path.abspath(run_dir.rstrip(os.sep))),
        "validation": validation,
        "manifest": manifest,
        "diffSummary": diff_summary,
        "environment": environment,
        "files": files,
        "bundle": {
            "outDir": os.path.abspath(bundle_out),
            "zip": os.path.abspath(zip_path) if zip_path else None,
        },
    }


# --------------------------------------------------------------------------- #
# Packaging
# --------------------------------------------------------------------------- #
def _write_json(path, value):
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(value, handle, indent=2, ensure_ascii=False)
        handle.write("\n")


def _zip_directory(bundle_dir, zip_path):
    """Zip ``bundle_dir`` into ``zip_path`` with a single top-level folder."""
    bundle_dir = os.path.abspath(bundle_dir)
    top = os.path.basename(bundle_dir.rstrip(os.sep))
    with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as archive:
        for root, _dirs, files in os.walk(bundle_dir):
            for name in files:
                full = os.path.join(root, name)
                rel = os.path.relpath(full, bundle_dir).replace(os.sep, "/")
                arcname = "{0}/{1}".format(top, rel)
                archive.write(full, arcname)


def package_run(run_dir, out_dir=None, make_zip=False, zip_path=None):
    """Validate and package a benchmark run directory.

    ``run_dir`` is the source run directory. ``out_dir`` overrides the bundle
    directory (default ``/private/tmp/<run-id>-bundle``). When ``make_zip`` is
    true, a zip is also produced; ``zip_path`` overrides its location (default
    ``/private/tmp/<run-id>-bundle.zip``).

    Returns the generated summary dict. Raises :class:`PackagingError` on any
    validation or IO failure.
    """
    if not os.path.isdir(run_dir):
        raise PackagingError("run directory does not exist: {0}".format(run_dir))
    run_dir = os.path.abspath(run_dir)

    validation, loaded = validate_run_dir(run_dir)
    if not validation["ok"]:
        problems = []
        problems.extend("{0} missing".format(k) for k in validation["missing"])
        problems.extend("{0} not valid JSON".format(k) for k in validation["invalidJson"])
        raise PackagingError(
            "run directory failed validation ({0}): {1}".format(
                run_dir, "; ".join(problems)
            )
        )

    manifest = loaded.get("manifest")
    run_id = derive_run_id(run_dir, manifest)
    safe_id = sanitize_for_path(run_id)

    # Resolve + guard the bundle directory.
    if out_dir is None:
        bundle_out = os.path.join(PRIVATE_TMP, "{0}-bundle".format(safe_id))
        bundle_user_specified = False
    else:
        bundle_out = os.path.abspath(out_dir)
        bundle_user_specified = True
    assert_safe_output(bundle_out, bundle_user_specified)

    # Resolve + guard the zip path.
    resolved_zip_path = None
    if make_zip:
        if zip_path is None:
            resolved_zip_path = os.path.join(
                PRIVATE_TMP, "{0}-bundle.zip".format(safe_id)
            )
            zip_user_specified = False
        else:
            resolved_zip_path = os.path.abspath(zip_path)
            zip_user_specified = True
        assert_safe_output(resolved_zip_path, zip_user_specified)

    # Fresh bundle directory (output lives in /private/tmp or a user-chosen
    # disposable location, so overwriting a previous bundle is safe).
    if os.path.exists(bundle_out):
        if os.path.isdir(bundle_out):
            shutil.rmtree(bundle_out)
        else:
            os.remove(bundle_out)
    shutil.copytree(run_dir, bundle_out)

    # Build the summary from the *source* run directory inventory.
    files = list_run_files(run_dir)
    diff_summary = derive_diff_summary(loaded.get("diff-result"))
    environment = collect_environment(run_dir, _load_optional_json(run_dir, "environment.json"))
    summary = build_summary(
        run_dir,
        validation,
        loaded,
        files,
        diff_summary,
        environment,
        bundle_out,
        resolved_zip_path,
    )

    # Write summary.json into the bundle (so the zip includes it).
    _write_json(os.path.join(bundle_out, "summary.json"), summary)

    if resolved_zip_path is not None:
        if os.path.exists(resolved_zip_path):
            os.remove(resolved_zip_path)
        _zip_directory(bundle_out, resolved_zip_path)

    return summary


# --------------------------------------------------------------------------- #
# Rendering + CLI
# --------------------------------------------------------------------------- #
def render_summary(summary):
    """Render a human-readable text summary of a packaging result."""
    lines = [
        "Benchmark run packager summary",
        "  tool: {0} v{1}".format(summary["tool"], summary["version"]),
        "  runId: {0}".format(summary["runId"]),
        "  runDir: {0}".format(summary["runDir"]),
        "  packagedAt: {0}".format(summary["packagedAt"]),
        "  validation: {0}".format(
            "OK" if summary["validation"]["ok"] else "FAILED"
        ),
    ]
    for key, info in summary["validation"]["required"].items():
        state = "missing"
        if info["present"] and info["validJson"]:
            state = "ok"
        elif info["present"]:
            state = "invalid-json"
        lines.append("    {0}: {1}".format(key, state))

    diff = summary["diffSummary"]
    if diff["match"] is None:
        diff_text = "unknown"
    else:
        diff_text = "match" if diff["match"] else "mismatch"
    if diff["total"] is not None:
        diff_text += " (total differences: {0})".format(diff["total"])
    lines.append("  diff: {0}".format(diff_text))
    classes = diff.get("differenceClasses")
    if isinstance(classes, dict) and any(classes.values()):
        class_bits = []
        for key in sorted(classes.keys()):
            value = classes[key]
            if value:
                class_bits.append("{0}: {1}".format(key, value))
        if class_bits:
            lines.append("  diff classes: {0}".format(", ".join(class_bits)))
    release_gate = diff.get("releaseGate")
    if isinstance(release_gate, dict):
        status = release_gate.get("status", "not-evaluated")
        lines.append("  release gate: {0}".format(status))
        reasons = release_gate.get("blockedReasons")
        if isinstance(reasons, list):
            for reason in reasons:
                lines.append("    - {0}".format(reason))

    lines.append("  files packaged: {0}".format(len(summary["files"])))
    lines.append("  bundle dir: {0}".format(summary["bundle"]["outDir"]))
    if summary["bundle"]["zip"]:
        lines.append("  zip: {0}".format(summary["bundle"]["zip"]))
    else:
        lines.append("  zip: (not created)")
    return "\n".join(lines) + "\n"


def parse_args(argv):
    parser = argparse.ArgumentParser(
        prog=TOOL_NAME,
        description=(
            "Package a benchmark run directory (manifest + platform/canonical/"
            "diff results + logs + environment) into an archivable bundle and "
            "optional zip. Does not run any benchmark."
        ),
    )
    parser.add_argument(
        "run_dir",
        help="Path to the run directory to package.",
    )
    parser.add_argument(
        "--out",
        dest="out_dir",
        default=None,
        help=(
            "Bundle output directory. Default: /private/tmp/<run-id>-bundle. "
            "Must not be under ~/Documents."
        ),
    )
    parser.add_argument(
        "--zip",
        dest="zip_path",
        nargs="?",
        const="",
        default=None,
        help=(
            "Also produce a zip archive. Pass an explicit path to override the "
            "default (/private/tmp/<run-id>-bundle.zip). Must not be under "
            "~/Documents."
        ),
    )
    return parser.parse_args(argv)


def main(argv=None):
    if argv is None:
        argv = sys.argv[1:]
    args = parse_args(argv)

    make_zip = args.zip_path is not None
    zip_path = args.zip_path if args.zip_path else None

    try:
        summary = package_run(
            args.run_dir,
            out_dir=args.out_dir,
            make_zip=make_zip,
            zip_path=zip_path,
        )
    except PackagingError as err:
        sys.stderr.write("error: {0}\n".format(err))
        return 2

    sys.stdout.write(render_summary(summary))
    return 0


if __name__ == "__main__":
    sys.exit(main())
