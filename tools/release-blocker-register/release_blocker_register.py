#!/usr/bin/env python3
"""Release blocker register.

A persistent register of cross-platform corpus divergences that block a
release, plus the waiver workflow around them. Blockers are derived from
``diff-result.json`` documents produced by the sibling ``cross-platform-diff``
tool: every difference of a non-matching platform candidate becomes a
blocker entry. A blocker can then be **waived** (with a mandatory rationale)
or **closed**.

The register is a single JSON file. By default it lives under
``/private/tmp`` and the tool refuses to write under ``~/Documents`` (which
covers the repository working tree); override with ``--register PATH``.

This tool deliberately does **not** certify a release. The ``gate``
subcommand reports how many blockers are still open and sets a non-zero exit
code when any remain — it never declares a release "ready". Reporting
four-endpoint consistency requires diff-result documents that carry named
``cli``, ``ios``, ``android``, and ``harmony`` candidates; a single-platform
CLI result does not satisfy that and the register will not pretend otherwise.

No network access. No remote data. No Core business logic.

Register schema (``schemaVersion`` 1)::

    {
      "schemaVersion": 1,
      "tool": "release-blocker-register",
      "version": "1.0",
      "updatedAt": "<iso8601 utc>",
      "nextId": <int>,
      "blockers": [ <blocker>, ... ]
    }

Blocker entry::

    {
      "id": "BLK-0007",
      "runId": "...",
      "platform": "<candidate name>",
      "fieldPath": "<difference path>",
      "kind": "value-mismatch | missing-in-candidate | unexpected-in-candidate |
              missing-platform-candidate",
      "canonicalSha256": "...",
      "candidateSha256": "...",
      "canonicalizedSha256": "...",
      "candidateCanonicalizedSha256": "...",
      "canonicalSnippet": "...",
      "candidateSnippet": "...",
      "severity": "high | medium | low",
      "status": "open | waived | closed",
      "reason": "...",
      "waiver": null | {"rationale": "...", "waivedBy": "...", "waivedAt": "..."},
      "createdAt": "<iso8601 utc>",
      "resolvedAt": null | "<iso8601 utc>"
    }

Usage::

    python3 tools/release-blocker-register/release_blocker_register.py \
        add-from-diff diff-result.json --run-id 2026-06-25-001

    python3 tools/release-blocker-register/release_blocker_register.py \
        waive BLK-0007 --rationale "accepted formatting drift on iOS"

    python3 tools/release-blocker-register/release_blocker_register.py list --status open

    python3 tools/release-blocker-register/release_blocker_register.py gate --run-id 2026-06-25-001
"""

import argparse
import datetime
import json
import os
import socket
import sys


TOOL_NAME = "release-blocker-register"
TOOL_VERSION = "1.0"
SCHEMA_VERSION = 1

PRIVATE_TMP = "/private/tmp"
DEFAULT_REGISTER_NAME = "corpus-blocker-register.json"
REQUIRED_PLATFORMS = ("cli", "ios", "android", "harmony")

STATUS_OPEN = "open"
STATUS_WAIVED = "waived"
STATUS_CLOSED = "closed"
_STATUSES = (STATUS_OPEN, STATUS_WAIVED, STATUS_CLOSED)

SEVERITY_HIGH = "high"
SEVERITY_MEDIUM = "medium"
SEVERITY_LOW = "low"
_SEVERITIES = (SEVERITY_HIGH, SEVERITY_MEDIUM, SEVERITY_LOW)


class RegisterError(Exception):
    """Raised on register access / validation failures."""


# --------------------------------------------------------------------------- #
# Path policy
# --------------------------------------------------------------------------- #
def _documents_dir():
    return os.path.realpath(os.path.expanduser("~/Documents"))


def _is_under(path, base):
    path = os.path.realpath(path)
    base = os.path.realpath(base)
    try:
        return os.path.commonpath([path, base]) == base
    except ValueError:
        return False


def assert_safe_register(path, user_specified):
    """Enforce the register-location policy.

    Default (non-user-specified) paths must resolve under ``/private/tmp``.
    No register — default or user-specified — may live under ``~/Documents``.
    """
    if not user_specified:
        if not _is_under(path, PRIVATE_TMP):
            raise RegisterError(
                "default register must live under {0}; got {1}".format(PRIVATE_TMP, path)
            )
    if _is_under(path, _documents_dir()):
        raise RegisterError(
            "refusing to write register under ~/Documents ({0}); "
            "use /private/tmp or another user-specified path".format(path)
        )


def resolve_register_path(user_path):
    if user_path is None:
        path = os.path.join(PRIVATE_TMP, DEFAULT_REGISTER_NAME)
        user_specified = False
    else:
        path = os.path.abspath(user_path)
        user_specified = True
    assert_safe_register(path, user_specified)
    return path


# --------------------------------------------------------------------------- #
# Time / id helpers
# --------------------------------------------------------------------------- #
def _now_iso():
    return datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds")


def _format_id(n):
    return "BLK-{0:04d}".format(n)


def _new_id(register):
    n = register.get("nextId", 1)
    register["nextId"] = n + 1
    return _format_id(n)


# --------------------------------------------------------------------------- #
# Persistence
# --------------------------------------------------------------------------- #
def _empty_register():
    return {
        "schemaVersion": SCHEMA_VERSION,
        "tool": TOOL_NAME,
        "version": TOOL_VERSION,
        "updatedAt": _now_iso(),
        "nextId": 1,
        "blockers": [],
    }


def load_register(path):
    """Load the register at ``path``, or return a fresh empty register if it
    does not yet exist."""
    if not os.path.isfile(path):
        return _empty_register()
    try:
        with open(path, "r", encoding="utf-8") as handle:
            data = json.load(handle)
    except (OSError, IOError) as err:
        raise RegisterError("cannot read register {0}: {1}".format(path, err))
    except json.JSONDecodeError as err:
        raise RegisterError("invalid JSON in register {0}: {1}".format(path, err))
    if not isinstance(data, dict) or "blockers" not in data:
        raise RegisterError("register {0} is not a valid register document".format(path))
    data.setdefault("schemaVersion", SCHEMA_VERSION)
    data.setdefault("tool", TOOL_NAME)
    data.setdefault("version", TOOL_VERSION)
    data.setdefault("nextId", _compute_next_id(data["blockers"]))
    return data


def _compute_next_id(blockers):
    highest = 0
    for entry in blockers:
        bid = entry.get("id", "")
        if isinstance(bid, str) and bid.startswith("BLK-"):
            try:
                highest = max(highest, int(bid[4:]))
            except ValueError:
                pass
    return highest + 1


def save_register(path, register):
    register["updatedAt"] = _now_iso()
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as handle:
        json.dump(register, handle, indent=2, ensure_ascii=False)
        handle.write("\n")
    os.replace(tmp, path)


# --------------------------------------------------------------------------- #
# Blocker construction
# --------------------------------------------------------------------------- #
def _blocker_key(run_id, platform, field_path):
    return (run_id or "", platform or "", field_path or "")


def _existing_keys(register):
    keys = set()
    for entry in register["blockers"]:
        keys.add(_blocker_key(
            entry.get("runId"),
            entry.get("platform"),
            entry.get("fieldPath"),
        ))
    return keys


def blockers_from_diff(diff_result, run_id, severity):
    """Build blocker entries (without ids / timestamps) from a diff-result.

    Missing required platform candidates and non-matching candidates contribute
    blockers. This prevents a single-platform or partial-platform diff-result
    from passing the register gate just because every present candidate matched.
    """
    if not isinstance(diff_result, dict):
        raise RegisterError("diff-result is not an object")
    candidates = diff_result.get("candidates")
    if not isinstance(candidates, dict):
        raise RegisterError("diff-result has no 'candidates' object")

    canonical_sha = ""
    canonicalized_sha = ""
    canonical = diff_result.get("canonical")
    if isinstance(canonical, dict):
        canonical_sha = canonical.get("sha256", "") or ""
        canonicalized_sha = canonical.get("canonicalizedSha256", "") or ""

    entries = []
    missing_platforms = sorted(set(REQUIRED_PLATFORMS) - set(candidates.keys()))
    for name in missing_platforms:
        entries.append({
            "runId": run_id,
            "platform": name,
            "fieldPath": "<candidate>",
            "kind": "missing-platform-candidate",
            "canonicalSha256": canonical_sha,
            "candidateSha256": "",
            "canonicalizedSha256": canonicalized_sha,
            "candidateCanonicalizedSha256": "",
            "canonicalSnippet": None,
            "candidateSnippet": None,
            "severity": severity,
            "status": STATUS_OPEN,
            "reason": "required four-platform candidate missing from diff-result",
            "waiver": None,
        })

    for name in sorted(candidates.keys()):
        info = candidates[name]
        if not isinstance(info, dict) or info.get("match"):
            continue
        candidate_sha = info.get("sha256", "") or ""
        candidate_canonicalized_sha = info.get("canonicalizedSha256", "") or ""
        for diff in info.get("differences", []):
            field_path = diff.get("path", "") if isinstance(diff, dict) else ""
            entries.append({
                "runId": run_id,
                "platform": name,
                "fieldPath": field_path,
                "kind": diff.get("kind", "value-mismatch") if isinstance(diff, dict) else "value-mismatch",
                "canonicalSha256": canonical_sha,
                "candidateSha256": candidate_sha,
                "canonicalizedSha256": canonicalized_sha,
                "candidateCanonicalizedSha256": candidate_canonicalized_sha,
                "canonicalSnippet": (diff.get("canonical") if isinstance(diff, dict) else None),
                "candidateSnippet": (diff.get("candidate") if isinstance(diff, dict) else None),
                "severity": severity,
                "status": STATUS_OPEN,
                "reason": "cross-platform divergence from canonical reference",
                "waiver": None,
            })
    return entries


def add_blockers_from_diff(register, diff_result, run_id, severity):
    """Add blockers derived from a diff-result, skipping duplicates.

    Returns the list of newly added blocker entries (with ids assigned).
    """
    existing = _existing_keys(register)
    added = []
    for entry in blockers_from_diff(diff_result, run_id, severity):
        if _blocker_key(entry["runId"], entry["platform"], entry["fieldPath"]) in existing:
            continue
        entry["id"] = _new_id(register)
        entry["createdAt"] = _now_iso()
        entry["resolvedAt"] = None
        register["blockers"].append(entry)
        existing.add(_blocker_key(entry["runId"], entry["platform"], entry["fieldPath"]))
        added.append(entry)
    return added


def add_manual_blocker(register, run_id, platform, field_path, severity, reason,
                        canonical_sha="", candidate_sha="",
                        canonicalized_sha="", candidate_canonicalized_sha=""):
    entry = {
        "id": _new_id(register),
        "runId": run_id,
        "platform": platform,
        "fieldPath": field_path,
        "kind": "value-mismatch",
        "canonicalSha256": canonical_sha,
        "candidateSha256": candidate_sha,
        "canonicalizedSha256": canonicalized_sha,
        "candidateCanonicalizedSha256": candidate_canonicalized_sha,
        "canonicalSnippet": None,
        "candidateSnippet": None,
        "severity": severity,
        "status": STATUS_OPEN,
        "reason": reason or "manually registered blocker",
        "waiver": None,
        "createdAt": _now_iso(),
        "resolvedAt": None,
    }
    register["blockers"].append(entry)
    return entry


# --------------------------------------------------------------------------- #
# Lookup / mutation
# --------------------------------------------------------------------------- #
def find_blocker(register, blocker_id):
    for entry in register["blockers"]:
        if entry.get("id") == blocker_id:
            return entry
    raise RegisterError("no such blocker: {0}".format(blocker_id))


def waive_blocker(entry, rationale, waived_by):
    if not rationale or not rationale.strip():
        raise RegisterError("a non-empty waiver rationale is required")
    if entry["status"] == STATUS_CLOSED:
        raise RegisterError("cannot waive a closed blocker: {0}".format(entry["id"]))
    entry["status"] = STATUS_WAIVED
    entry["waiver"] = {
        "rationale": rationale.strip(),
        "waivedBy": waived_by,
        "waivedAt": _now_iso(),
    }
    entry["resolvedAt"] = entry["waiver"]["waivedAt"]


def close_blocker(entry):
    if entry["status"] == STATUS_CLOSED:
        return  # idempotent
    entry["status"] = STATUS_CLOSED
    entry["resolvedAt"] = _now_iso()


def reopen_blocker(entry):
    entry["status"] = STATUS_OPEN
    entry["resolvedAt"] = None
    # A prior waiver no longer applies once reopened; require a fresh waiver.
    entry["waiver"] = None


def filter_blockers(blockers, status=None, platform=None, run_id=None):
    out = []
    for entry in blockers:
        if status is not None and entry.get("status") != status:
            continue
        if platform is not None and entry.get("platform") != platform:
            continue
        if run_id is not None and entry.get("runId") != run_id:
            continue
        out.append(entry)
    return out


def gate_evaluate(register, run_id=None):
    """Return ``(open_count, breakdown)`` for the gate.

    ``breakdown`` maps platform → count of open blockers. The register reports
    state only; it never certifies a release as ready.
    """
    open_blockers = filter_blockers(
        register["blockers"], status=STATUS_OPEN, run_id=run_id
    )
    breakdown = {}
    for entry in open_blockers:
        plat = entry.get("platform", "(unknown)")
        breakdown[plat] = breakdown.get(plat, 0) + 1
    return len(open_blockers), breakdown


# --------------------------------------------------------------------------- #
# Rendering
# --------------------------------------------------------------------------- #
def render_blocker_brief(entry):
    return "{id} [{sev}] {status}  run={run}  platform={platform}  path={path}".format(
        id=entry.get("id", "?"),
        sev=entry.get("severity", "?"),
        status=entry.get("status", "?"),
        run=entry.get("runId") or "-",
        platform=entry.get("platform") or "-",
        path=entry.get("fieldPath") or "-",
    )


def render_blocker_detail(entry):
    lines = [
        "blocker {0}".format(entry.get("id", "?")),
        "  status: {0}".format(entry.get("status", "?")),
        "  severity: {0}".format(entry.get("severity", "?")),
        "  runId: {0}".format(entry.get("runId") or "-"),
        "  platform: {0}".format(entry.get("platform") or "-"),
        "  fieldPath: {0}".format(entry.get("fieldPath") or "-"),
        "  kind: {0}".format(entry.get("kind", "-")),
        "  reason: {0}".format(entry.get("reason", "-")),
        "  canonicalSha256: {0}".format(entry.get("canonicalSha256") or "-"),
        "  candidateSha256: {0}".format(entry.get("candidateSha256") or "-"),
        "  canonicalizedSha256: {0}".format(
            entry.get("canonicalizedSha256") or "-"
        ),
        "  candidateCanonicalizedSha256: {0}".format(
            entry.get("candidateCanonicalizedSha256") or "-"
        ),
        "  canonicalSnippet: {0}".format(entry.get("canonicalSnippet")),
        "  candidateSnippet: {0}".format(entry.get("candidateSnippet")),
        "  createdAt: {0}".format(entry.get("createdAt", "-")),
        "  resolvedAt: {0}".format(entry.get("resolvedAt") or "-"),
    ]
    waiver = entry.get("waiver")
    if isinstance(waiver, dict):
        lines.append("  waiver:")
        lines.append("    rationale: {0}".format(waiver.get("rationale", "")))
        lines.append("    waivedBy: {0}".format(waiver.get("waivedBy", "-")))
        lines.append("    waivedAt: {0}".format(waiver.get("waivedAt", "-")))
    else:
        lines.append("  waiver: (none)")
    return "\n".join(lines) + "\n"


def render_list(blockers):
    if not blockers:
        return "(no blockers match)\n"
    return "\n".join(render_blocker_brief(e) for e in blockers) + "\n"


def render_gate(open_count, breakdown):
    lines = ["release blocker gate", "  open blockers: {0}".format(open_count)]
    if breakdown:
        lines.append("  by platform:")
        for plat in sorted(breakdown.keys()):
            lines.append("    {0}: {1}".format(plat, breakdown[plat]))
    lines.append(
        "  note: the register reports open-blocker state; it does not "
        "certify a release."
    )
    return "\n".join(lines) + "\n"


# --------------------------------------------------------------------------- #
# CLI
# --------------------------------------------------------------------------- #
def _load_diff(path):
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return json.load(handle)
    except FileNotFoundError:
        raise RegisterError("diff-result file not found: {0}".format(path))
    except json.JSONDecodeError as err:
        raise RegisterError("invalid JSON in diff-result {0}: {1}".format(path, err))


def parse_args(argv):
    parser = argparse.ArgumentParser(
        prog=TOOL_NAME,
        description=(
            "Register cross-platform corpus divergences as release blockers, "
            "with waiver / close / gate workflow. Does not run benchmarks and "
            "does not certify a release."
        ),
    )
    parser.add_argument(
        "--register",
        default=None,
        help=(
            "Path to the register JSON file. Default: "
            "/private/tmp/{0}. Must not be under ~/Documents.".format(
                DEFAULT_REGISTER_NAME
            )
        ),
    )
    sub = parser.add_subparsers(dest="command")
    sub.required = True

    p_add = sub.add_parser(
        "add-from-diff",
        help="Derive blockers from a diff-result.json (non-matching candidates).",
    )
    p_add.add_argument("diff_result", help="Path to a diff-result.json file.")
    p_add.add_argument("--run-id", default=None, help="Run id to tag blockers with.")
    p_add.add_argument(
        "--severity", choices=_SEVERITIES, default=SEVERITY_MEDIUM,
        help="Severity assigned to derived blockers (default: medium).",
    )

    p_manual = sub.add_parser("add", help="Manually register one blocker.")
    p_manual.add_argument("--run-id", default=None)
    p_manual.add_argument("--platform", required=True)
    p_manual.add_argument("--field-path", required=True)
    p_manual.add_argument(
        "--severity", choices=_SEVERITIES, default=SEVERITY_MEDIUM,
    )
    p_manual.add_argument("--reason", default=None)
    p_manual.add_argument("--canonical-sha256", default="")
    p_manual.add_argument("--candidate-sha256", default="")
    p_manual.add_argument("--canonicalized-sha256", default="")
    p_manual.add_argument("--candidate-canonicalized-sha256", default="")

    p_list = sub.add_parser("list", help="List blockers (optionally filtered).")
    p_list.add_argument("--status", choices=_STATUSES, default=None)
    p_list.add_argument("--platform", default=None)
    p_list.add_argument("--run-id", default=None)
    p_list.add_argument("--json", action="store_true", help="Emit JSON instead of text.")

    p_show = sub.add_parser("show", help="Show one blocker in detail.")
    p_show.add_argument("id")

    p_waive = sub.add_parser("waive", help="Waive a blocker with a rationale.")
    p_waive.add_argument("id")
    p_waive.add_argument("--rationale", required=True, help="Why this blocker is accepted.")
    p_waive.add_argument(
        "--by", default=None,
        help="Who waived it (default: current user $USER / hostname).",
    )

    p_close = sub.add_parser("close", help="Close a blocker (e.g. fixed).")
    p_close.add_argument("id")

    p_reopen = sub.add_parser("reopen", help="Reopen a closed/waived blocker.")
    p_reopen.add_argument("id")

    p_gate = sub.add_parser(
        "gate",
        help="Report open-blocker count. Exit 0 if none, 1 if any open.",
    )
    p_gate.add_argument("--run-id", default=None)

    return parser.parse_args(argv)


def _default_waiver_by():
    user = os.environ.get("USER") or os.environ.get("LOGNAME")
    if not user:
        user = socket.gethostname()
    return user


def main(argv=None):
    if argv is None:
        argv = sys.argv[1:]
    args = parse_args(argv)

    try:
        register_path = resolve_register_path(args.register)
    except RegisterError as err:
        sys.stderr.write("error: {0}\n".format(err))
        return 2

    try:
        if args.command == "add-from-diff":
            diff_result = _load_diff(args.diff_result)
            register = load_register(register_path)
            added = add_blockers_from_diff(
                register, diff_result, args.run_id, args.severity
            )
            save_register(register_path, register)
            sys.stdout.write(
                "added {0} blocker(s) from {1}\n".format(len(added), args.diff_result)
            )
            for entry in added:
                sys.stdout.write("  " + render_blocker_brief(entry) + "\n")
            return 0

        if args.command == "add":
            register = load_register(register_path)
            entry = add_manual_blocker(
                register,
                run_id=args.run_id,
                platform=args.platform,
                field_path=args.field_path,
                severity=args.severity,
                reason=args.reason,
                canonical_sha=args.canonical_sha256,
                candidate_sha=args.candidate_sha256,
                canonicalized_sha=args.canonicalized_sha256,
                candidate_canonicalized_sha=args.candidate_canonicalized_sha256,
            )
            save_register(register_path, register)
            sys.stdout.write(render_blocker_detail(entry))
            return 0

        if args.command == "list":
            register = load_register(register_path)
            blockers = filter_blockers(
                register["blockers"],
                status=args.status,
                platform=args.platform,
                run_id=args.run_id,
            )
            if args.json:
                sys.stdout.write(
                    json.dumps(blockers, indent=2, ensure_ascii=False) + "\n"
                )
            else:
                sys.stdout.write(render_list(blockers))
            return 0

        if args.command == "show":
            register = load_register(register_path)
            entry = find_blocker(register, args.id)
            sys.stdout.write(render_blocker_detail(entry))
            return 0

        if args.command == "waive":
            register = load_register(register_path)
            entry = find_blocker(register, args.id)
            waive_blocker(entry, args.rationale, args.by or _default_waiver_by())
            save_register(register_path, register)
            sys.stdout.write(render_blocker_detail(entry))
            return 0

        if args.command == "close":
            register = load_register(register_path)
            entry = find_blocker(register, args.id)
            close_blocker(entry)
            save_register(register_path, register)
            sys.stdout.write(render_blocker_detail(entry))
            return 0

        if args.command == "reopen":
            register = load_register(register_path)
            entry = find_blocker(register, args.id)
            reopen_blocker(entry)
            save_register(register_path, register)
            sys.stdout.write(render_blocker_detail(entry))
            return 0

        if args.command == "gate":
            register = load_register(register_path)
            open_count, breakdown = gate_evaluate(register, run_id=args.run_id)
            sys.stdout.write(render_gate(open_count, breakdown))
            return 0 if open_count == 0 else 1

    except RegisterError as err:
        sys.stderr.write("error: {0}\n".format(err))
        return 2

    # Unreachable (subparser required=True), but keep a defensive exit.
    sys.stderr.write("error: no command\n")
    return 2


if __name__ == "__main__":
    sys.exit(main())
