#!/usr/bin/env python3
"""Worktree conflict checker — a read-only dev-time tool.

Scans all git worktrees for dirty files, ahead-commits, and overlapping
modified paths; emits a merge-risk report JSON.

STRICTLY READ-ONLY. The only git commands ever issued are:
  - git worktree list --porcelain
  - git -C <wt> status --porcelain
  - git -C <wt> rev-list --count --left-right @{u}...HEAD

All git access goes through an injectable ``runner(args) -> (rc, stdout, stderr)``
and a ``worktrees_provider()`` indirection so the tool is fully testable.

Python 3.9+ stdlib only.
"""

import argparse
import datetime
import json
import subprocess
import sys


# --- git boundary -----------------------------------------------------------

def _default_runner(args):
    """Production runner: real subprocess, no mutation, capture output."""
    result = subprocess.run(args, capture_output=True, text=True)
    return (result.returncode, result.stdout, result.stderr)


# --- parsers ----------------------------------------------------------------

def parse_worktree_list(out):
    """Parse ``git worktree list --porcelain`` output.

    Returns a list of dicts ``{"path":..., "branch":..., "head":...}``.
    Records are blank-line separated. ``branch`` is the short name
    (``refs/heads/`` stripped); missing branch line -> ``"detached"``.
    """
    records = []
    current = {}
    for line in out.splitlines():
        if not line.strip():
            if current:
                records.append(current)
                current = {}
            continue
        if line.startswith("worktree "):
            current["path"] = line[len("worktree "):]
        elif line.startswith("HEAD "):
            current["head"] = line[len("HEAD "):]
        elif line.startswith("branch "):
            ref = line[len("branch "):]
            if ref.startswith("refs/heads/"):
                ref = ref[len("refs/heads/"):]
            current["branch"] = ref
    if current:
        records.append(current)

    result = []
    for r in records:
        branch = r.get("branch")
        if not branch:
            branch = "detached"
        result.append({
            "path": r.get("path", ""),
            "head": r.get("head", ""),
            "branch": branch,
        })
    return result


def dirty_files(wt_path, runner):
    """Return repo-relative dirty paths from ``git -C <wt> status --porcelain``.

    Porcelain v1 lines are ``XY <path>``; the path is the substring from
    col 3. Blank lines are ignored. All entries (including untracked) are
    included.
    """
    rc, out, _err = runner(
        ["git", "-C", wt_path, "status", "--porcelain"]
    )
    paths = []
    for line in out.splitlines():
        if not line.strip():
            continue
        path = line[3:]
        if path:
            paths.append(path)
    return paths


def ahead_behind(wt_path, runner):
    """Return ``(ahead, behind, note)`` vs upstream.

    Uses ``git -C <wt> rev-list --count --left-right @{u}...HEAD`` whose
    output is ``"<behind>\\t<ahead>"``. On no-upstream (non-zero exit)
    returns ``(0, 0, "no-upstream")``.
    """
    rc, out, _err = runner(
        ["git", "-C", wt_path, "rev-list", "--count", "--left-right",
         "@{u}...HEAD"]
    )
    if rc != 0:
        return (0, 0, "no-upstream")
    parts = out.split()
    if len(parts) >= 2:
        try:
            behind = int(parts[0])
            ahead = int(parts[1])
            return (ahead, behind, "")
        except ValueError:
            return (0, 0, "")
    return (0, 0, "")


# --- core collect -----------------------------------------------------------

def collect(root, runner=None, worktrees_provider=None):
    """Build the merge-risk report for all worktrees.

    ``root``: repo path used to discover worktrees when no provider is given.
    ``runner``: ``args -> (rc, stdout, stderr)``; defaults to real subprocess.
    ``worktrees_provider``: callable returning ``git worktree list --porcelain``
    text; defaults to running git via ``runner``.
    """
    if runner is None:
        runner = _default_runner

    if worktrees_provider is None:
        _rc, out, _err = runner(
            ["git", "-C", root, "worktree", "list", "--porcelain"]
        )
        wt_text = out
    else:
        wt_text = worktrees_provider()

    worktrees = parse_worktree_list(wt_text)

    wt_reports = []
    path_to_wts = {}  # dirty path -> [wt paths that have it]
    for wt in worktrees:
        wt_path = wt["path"]
        dirty = dirty_files(wt_path, runner)
        ahead, behind, note = ahead_behind(wt_path, runner)
        wt_reports.append({
            "path": wt_path,
            "branch": wt["branch"],
            "head": wt["head"],
            "dirty_files": len(dirty),
            "dirty_paths": dirty,
            "ahead": ahead,
            "behind": behind,
            "notes": note,
        })
        for p in dirty:
            path_to_wts.setdefault(p, []).append(wt_path)

    overlaps = [
        {"path": p, "worktrees": list(wts)}
        for p, wts in path_to_wts.items()
        if len(wts) > 1
    ]
    overlaps.sort(key=lambda o: o["path"])

    risks = []
    # overlap -> high
    for ov in overlaps:
        risks.append({
            "severity": "high",
            "kind": "overlap",
            "description": "Path '{}' modified in {} worktrees".format(
                ov["path"], len(ov["worktrees"])),
            "worktrees": list(ov["worktrees"]),
        })
    # per-worktree dirty / ahead / no-upstream
    for wtr in wt_reports:
        dn = wtr["dirty_files"]
        if dn >= 5:
            risks.append({
                "severity": "medium",
                "kind": "dirty",
                "description": "Worktree {} has {} dirty files".format(
                    wtr["path"], dn),
                "worktrees": [wtr["path"]],
            })
        elif dn > 0:
            risks.append({
                "severity": "low",
                "kind": "dirty",
                "description": "Worktree {} has {} dirty file(s)".format(
                    wtr["path"], dn),
                "worktrees": [wtr["path"]],
            })
        a = wtr["ahead"]
        if a >= 10:
            risks.append({
                "severity": "medium",
                "kind": "ahead",
                "description": "Worktree {} is {} commits ahead of upstream".format(
                    wtr["path"], a),
                "worktrees": [wtr["path"]],
            })
        elif a > 0:
            risks.append({
                "severity": "low",
                "kind": "ahead",
                "description": "Worktree {} is {} commit(s) ahead of upstream".format(
                    wtr["path"], a),
                "worktrees": [wtr["path"]],
            })
        if wtr["notes"] == "no-upstream":
            risks.append({
                "severity": "low",
                "kind": "no-upstream",
                "description": "Worktree {} has no upstream configured".format(
                    wtr["path"]),
                "worktrees": [wtr["path"]],
            })

    rank = {"high": 0, "medium": 1, "low": 2}
    risks.sort(key=lambda r: (rank[r["severity"]], r["kind"]))

    return {
        "version": "worktree-conflict-report/1",
        "generated_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "tool": "worktree-conflict-checker",
        "worktrees": wt_reports,
        "overlaps": overlaps,
        "risks": risks,
        "summary": {
            "worktrees": len(wt_reports),
            "dirty_total": sum(w["dirty_files"] for w in wt_reports),
            "ahead_total": sum(w["ahead"] for w in wt_reports),
            "overlaps": len(overlaps),
            "risks": len(risks),
        },
    }


# --- CLI --------------------------------------------------------------------

def main(argv=None, runner=None, worktrees_provider=None):
    """CLI entry. Returns exit code (0 on success, 2 on usage error)."""
    parser = argparse.ArgumentParser(
        prog="worktree-conflict",
        description="Scan git worktrees for merge risk (read-only).",
    )
    parser.add_argument("root", nargs="?", default=".",
                        help="repo root to scan (default: cwd)")
    parser.add_argument("--indent", type=int, default=None,
                        help="JSON indent width")
    parser.add_argument("--pretty", action="store_true",
                        help="pretty-print JSON (indent=2, sorted keys)")
    args = parser.parse_args(argv)

    report = collect(
        args.root, runner=runner, worktrees_provider=worktrees_provider
    )

    if args.pretty:
        out = json.dumps(report, indent=2, sort_keys=True)
    elif args.indent is not None:
        out = json.dumps(report, indent=args.indent)
    else:
        out = json.dumps(report)
    print(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
