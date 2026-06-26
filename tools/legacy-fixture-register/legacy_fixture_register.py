#!/usr/bin/env python3
"""Legacy fixture register and proof-matrix checks.

This tool validates ``samples/legacy-fixtures/register.json`` and checks the
current integration proof matrix. It is intentionally registry-only: it does
not execute Core logic, does not implement rules, and does not run platform
adapters.
"""

import argparse
import json
import os
import subprocess
import sys


TOOL_NAME = "legacy-fixture-register"
SCHEMA_VERSION = 1
ALLOWED_STATUSES = (
    "covered",
    "pending-rule-kernel",
    "needs-platform-corpus-proof",
)
REQUIRED_ENTRY_IDS = (
    "booksource",
    "runtime-host",
    "corpus-oracle",
    "rule-dsl",
    "ios-platform",
    "android-platform",
    "harmonyos-platform",
)
REQUIRED_PLATFORMS = ("cli", "ios", "android", "harmony")


class RegisterError(Exception):
    """Raised when the register or proof matrix is invalid."""


def load_json_file(path):
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return json.load(handle)
    except FileNotFoundError:
        raise RegisterError("missing JSON file: {0}".format(path))
    except (OSError, IOError) as err:
        raise RegisterError("cannot read {0}: {1}".format(path, err))
    except json.JSONDecodeError as err:
        raise RegisterError("invalid JSON in {0}: {1}".format(path, err))


def validate_register(data):
    if not isinstance(data, dict):
        raise RegisterError("register must be a JSON object")
    if data.get("schemaVersion") != SCHEMA_VERSION:
        raise RegisterError("register schemaVersion must be {0}".format(SCHEMA_VERSION))
    if data.get("tool") != TOOL_NAME:
        raise RegisterError("register tool must be {0}".format(TOOL_NAME))

    status_values = data.get("statusValues")
    if status_values != list(ALLOWED_STATUSES):
        raise RegisterError("statusValues must exactly match {0}".format(ALLOWED_STATUSES))

    entries = data.get("entries")
    if not isinstance(entries, list) or not entries:
        raise RegisterError("register entries must be a non-empty list")

    seen = set()
    by_id = {}
    for entry in entries:
        if not isinstance(entry, dict):
            raise RegisterError("register entry must be an object")
        entry_id = entry.get("id")
        if not isinstance(entry_id, str) or not entry_id:
            raise RegisterError("register entry id must be non-empty")
        if entry_id in seen:
            raise RegisterError("duplicate register entry id: {0}".format(entry_id))
        seen.add(entry_id)
        by_id[entry_id] = entry

        status = entry.get("status")
        if status not in ALLOWED_STATUSES:
            raise RegisterError(
                "entry {0} has invalid status {1!r}".format(entry_id, status)
            )
        if status == "needs-platform-corpus-proof":
            if not entry.get("platformOutputPath"):
                raise RegisterError(
                    "entry {0} needs platformOutputPath".format(entry_id)
                )
            if not entry.get("fixturePaths"):
                raise RegisterError("entry {0} needs fixturePaths".format(entry_id))

    missing = [entry_id for entry_id in REQUIRED_ENTRY_IDS if entry_id not in by_id]
    if missing:
        raise RegisterError("register missing entries: {0}".format(", ".join(missing)))

    proof = data.get("proofMatrix")
    if not isinstance(proof, dict):
        raise RegisterError("register needs proofMatrix object")
    if not isinstance(proof.get("requiredRefs"), dict):
        raise RegisterError("proofMatrix.requiredRefs must be an object")
    if not proof.get("releaseGateManifest"):
        raise RegisterError("proofMatrix.releaseGateManifest is required")
    if not isinstance(proof.get("platformEvidence"), dict):
        raise RegisterError("proofMatrix.platformEvidence must be an object")

    return by_id


def _repo_path(repo_root, path):
    return path if os.path.isabs(path) else os.path.join(repo_root, path)


def git_ref_exists(repo_root, ref):
    result = subprocess.run(
        ["git", "rev-parse", "--verify", "--quiet", ref],
        cwd=repo_root,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return result.returncode == 0


def _release_gate_candidates(repo_root, manifest_path):
    manifest = load_json_file(_repo_path(repo_root, manifest_path))
    candidates = manifest.get("candidates")
    if not isinstance(candidates, dict):
        raise RegisterError("release gate manifest has no candidates object")
    return set(candidates.keys())


def _check_release_gate_manifest(repo_root, manifest_path):
    candidates = _release_gate_candidates(repo_root, manifest_path)
    missing = [platform for platform in REQUIRED_PLATFORMS if platform not in candidates]
    if missing:
        return {
            "id": "release-gate-platform-output",
            "status": "blocked",
            "message": "release gate manifest missing platform output: {0}".format(
                ", ".join(missing)
            ),
            "missingPlatforms": missing,
        }
    return {
        "id": "release-gate-platform-output",
        "status": "passed",
        "message": "release gate manifest registers cli/ios/android/harmony outputs",
    }


def _check_platform_evidence(by_id, platform, config):
    entry_id = config.get("entryId")
    entry = by_id.get(entry_id)
    if not entry:
        return {
            "id": "{0}-evidence-registered".format(platform),
            "status": "blocked",
            "message": "platform evidence entry {0!r} is missing".format(entry_id),
        }
    missing = []
    if entry.get("status") != "needs-platform-corpus-proof":
        missing.append("status=needs-platform-corpus-proof")
    if not entry.get("platformOutputPath"):
        missing.append("platformOutputPath")
    if not entry.get("fixturePaths"):
        missing.append("fixturePaths")
    if missing:
        return {
            "id": "{0}-evidence-registered".format(platform),
            "status": "blocked",
            "message": "{0} evidence entry missing {1}".format(
                platform, ", ".join(missing)
            ),
        }
    return {
        "id": "{0}-evidence-registered".format(platform),
        "status": "passed",
        "message": "{0} evidence path is registered".format(platform),
    }


def _validate_harmony_intake(repo_root, path):
    manifest = load_json_file(_repo_path(repo_root, path))
    required = (
        "schemaVersion",
        "platform",
        "sourceRepo",
        "sourceBranch",
        "sourcePr",
        "expectedCorpusOutputPath",
        "nativeIntakePath",
        "releaseGateCandidateName",
        "blockerWhenMissing",
    )
    missing = [key for key in required if key not in manifest]
    if missing:
        raise RegisterError(
            "HarmonyOS intake manifest missing {0}".format(", ".join(missing))
        )
    if manifest.get("platform") != "harmony":
        raise RegisterError("HarmonyOS intake platform must be harmony")
    blocker = manifest.get("blockerWhenMissing")
    if not isinstance(blocker, dict) or blocker.get("status") != "open":
        raise RegisterError("HarmonyOS intake blockerWhenMissing must be open")
    return manifest


def build_proof_matrix(data, repo_root, ref_exists=git_ref_exists):
    by_id = validate_register(data)
    proof = data["proofMatrix"]
    checks = []

    for name, ref in sorted(proof["requiredRefs"].items()):
        exists = ref_exists(repo_root, ref)
        checks.append({
            "id": "branch-{0}".format(name),
            "status": "passed" if exists else "blocked",
            "message": "{0} exists".format(ref) if exists else "{0} is missing".format(ref),
            "ref": ref,
        })

    for platform, config in sorted(proof["platformEvidence"].items()):
        checks.append(_check_platform_evidence(by_id, platform, config))
        if platform == "harmony" and config.get("intakeManifest"):
            try:
                _validate_harmony_intake(repo_root, config["intakeManifest"])
                checks.append({
                    "id": "harmony-intake-manifest",
                    "status": "passed",
                    "message": "HarmonyOS intake manifest is valid",
                })
            except RegisterError as err:
                checks.append({
                    "id": "harmony-intake-manifest",
                    "status": "blocked",
                    "message": str(err),
                })

    try:
        checks.append(_check_release_gate_manifest(repo_root, proof["releaseGateManifest"]))
    except RegisterError as err:
        checks.append({
            "id": "release-gate-platform-output",
            "status": "blocked",
            "message": str(err),
        })

    return {
        "schemaVersion": SCHEMA_VERSION,
        "tool": TOOL_NAME,
        "checks": checks,
        "passed": sum(1 for check in checks if check["status"] == "passed"),
        "blocked": sum(1 for check in checks if check["status"] != "passed"),
    }


def _default_register_path(repo_root):
    return os.path.join(repo_root, "samples", "legacy-fixtures", "register.json")


def _print_json(obj):
    print(json.dumps(obj, indent=2, ensure_ascii=False))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--repo-root",
        default=os.getcwd(),
        help="repository root, default: current working directory",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    validate = sub.add_parser("validate", help="validate the legacy fixture register")
    validate.add_argument("register", nargs="?", help="register JSON path")

    proof = sub.add_parser("proof", help="run proof matrix checks")
    proof.add_argument("register", nargs="?", help="register JSON path")
    proof.add_argument("--json", action="store_true", help="print JSON output")

    args = parser.parse_args(argv)
    repo_root = os.path.abspath(args.repo_root)
    register_path = args.register or _default_register_path(repo_root)

    try:
        data = load_json_file(register_path)
        if args.command == "validate":
            validate_register(data)
            print("legacy fixture register ok")
            return 0
        if args.command == "proof":
            result = build_proof_matrix(data, repo_root)
            if args.json:
                _print_json(result)
            else:
                for check in result["checks"]:
                    print("[{0}] {1}: {2}".format(
                        check["status"], check["id"], check["message"]
                    ))
                print("passed={0} blocked={1}".format(
                    result["passed"], result["blocked"]
                ))
            return 0 if result["blocked"] == 0 else 1
    except RegisterError as err:
        print("error: {0}".format(err), file=sys.stderr)
        return 2

    return 2


if __name__ == "__main__":
    sys.exit(main())
