"""Tests for tools/worktree-conflict/worktree_conflict.py.

All git access goes through an injectable ``runner(args) -> (rc, stdout, stderr)``
and a ``worktrees_provider()`` indirection, so these tests are fully deterministic
and never touch the real repo's worktrees.
"""

import io
import contextlib
import json
import os
import sys
import unittest

# The tool lives under tools/worktree-conflict/ (hyphen -> not importable as a
# dotted package). Add the directory to sys.path so we can import the module.
_HERE = os.path.dirname(os.path.abspath(__file__))
_TOOL_DIR = os.path.abspath(
    os.path.join(_HERE, "..", "..", "tools", "worktree-conflict")
)
if _TOOL_DIR not in sys.path:
    sys.path.insert(0, _TOOL_DIR)

import worktree_conflict  # noqa: E402
from worktree_conflict import (  # noqa: E402
    parse_worktree_list,
    dirty_files,
    ahead_behind,
    collect,
    main,
)


def _run_with(argspec, responses):
    """Build a fake runner that dispatches on substrings in the argv list.

    ``responses`` maps a key substring -> (rc, stdout, stderr). The first
    matching key wins. Unmatched calls return (0, "", "").
    """
    def runner(args):
        joined = " ".join(args)
        for key, resp in responses.items():
            if key in joined:
                return resp
        return (0, "", "")
    return runner


class TestParseWorktreeList(unittest.TestCase):
    def test_parse_basic_and_detached(self):
        out = (
            "worktree /wt1\n"
            "HEAD aaa111222333\n"
            "branch refs/heads/main\n"
            "\n"
            "worktree /wt2\n"
            "HEAD bbb444555666\n"
        )
        wts = parse_worktree_list(out)
        self.assertEqual(len(wts), 2)
        self.assertEqual(wts[0]["path"], "/wt1")
        self.assertEqual(wts[0]["branch"], "main")
        self.assertEqual(wts[0]["head"], "aaa111222333")
        # detached when no branch line
        self.assertEqual(wts[1]["path"], "/wt2")
        self.assertEqual(wts[1]["branch"], "detached")
        self.assertEqual(wts[1]["head"], "bbb444555666")

    def test_parse_empty(self):
        self.assertEqual(parse_worktree_list(""), [])
        self.assertEqual(parse_worktree_list("\n\n"), [])


class TestDirtyFiles(unittest.TestCase):
    def test_strips_two_char_status_prefix(self):
        runner = _run_with(
            ["status"],
            {"status": (0, " M foo.txt\n?? bar.txt\nA  new.txt\n", "")},
        )
        paths = dirty_files("/wt1", runner)
        self.assertEqual(paths, ["foo.txt", "bar.txt", "new.txt"])

    def test_ignores_blank_lines(self):
        runner = _run_with(
            ["status"], {"status": (0, " M foo.txt\n\n M bar.txt\n", "")}
        )
        paths = dirty_files("/wt1", runner)
        self.assertEqual(paths, ["foo.txt", "bar.txt"])

    def test_empty_when_clean(self):
        runner = _run_with(["status"], {"status": (0, "", "")})
        self.assertEqual(dirty_files("/wt1", runner), [])


class TestAheadBehind(unittest.TestCase):
    def test_parses_left_right_count(self):
        # Output is "behind\tahead" (--left-right @{u}...HEAD).
        runner = _run_with(["rev-list"], {"rev-list": (0, "3\t12\n", "")})
        ahead, behind, note = ahead_behind("/wt1", runner)
        self.assertEqual(ahead, 12)
        self.assertEqual(behind, 3)
        self.assertEqual(note, "")

    def test_no_upstream(self):
        runner = _run_with(
            ["rev-list"], {"rev-list": (128, "", "fatal: no upstream")}
        )
        ahead, behind, note = ahead_behind("/wt1", runner)
        self.assertEqual((ahead, behind), (0, 0))
        self.assertEqual(note, "no-upstream")


def _ok_runner(status_map, ahead_map):
    """Build a runner returning canned status/rev-list per worktree path."""
    def runner(args):
        joined = " ".join(args)
        if "status" in joined:
            for wt, out in status_map.items():
                if wt in joined:
                    return (0, out, "")
        if "rev-list" in joined:
            for wt, out in ahead_map.items():
                if wt in joined:
                    return (0, out, "")
        return (0, "", "")
    return runner


class TestCollect(unittest.TestCase):
    def test_overlap_high_severity(self):
        porcelain = (
            "worktree /wt1\nHEAD aaa\nbranch refs/heads/b1\n\n"
            "worktree /wt2\nHEAD bbb\nbranch refs/heads/b2\n"
        )
        runner = _ok_runner(
            {"/wt1": " M shared.txt\n", "/wt2": " M shared.txt\n"},
            {"/wt1": "0\t0\n", "/wt2": "0\t0\n"},
        )
        report = collect(
            "/root", runner=runner, worktrees_provider=lambda: porcelain
        )
        overlaps = [o for o in report["overlaps"] if o["path"] == "shared.txt"]
        self.assertEqual(len(overlaps), 1)
        self.assertEqual(set(overlaps[0]["worktrees"]), {"/wt1", "/wt2"})
        high_overlap = [
            r for r in report["risks"]
            if r["severity"] == "high" and r["kind"] == "overlap"
        ]
        self.assertTrue(high_overlap, "expected a high-severity overlap risk")

    def test_dirty_six_files_medium(self):
        porcelain = "worktree /wt1\nHEAD aaa\nbranch refs/heads/b1\n"
        status_out = "".join(" M f{}.txt\n".format(i) for i in range(6))
        runner = _ok_runner({"/wt1": status_out}, {"/wt1": "0\t0\n"})
        report = collect(
            "/root", runner=runner, worktrees_provider=lambda: porcelain
        )
        medium_dirty = [
            r for r in report["risks"]
            if r["kind"] == "dirty" and r["severity"] == "medium"
        ]
        self.assertTrue(medium_dirty, "expected a medium-severity dirty risk")
        # exactly one worktree in the risk
        self.assertEqual(medium_dirty[0]["worktrees"], ["/wt1"])

    def test_ahead_twelve_medium(self):
        porcelain = "worktree /wt1\nHEAD aaa\nbranch refs/heads/b1\n"
        runner = _ok_runner({"/wt1": ""}, {"/wt1": "0\t12\n"})
        report = collect(
            "/root", runner=runner, worktrees_provider=lambda: porcelain
        )
        medium_ahead = [
            r for r in report["risks"]
            if r["kind"] == "ahead" and r["severity"] == "medium"
        ]
        self.assertTrue(medium_ahead, "expected a medium-severity ahead risk")

    def test_ahead_below_threshold_low(self):
        porcelain = "worktree /wt1\nHEAD aaa\nbranch refs/heads/b1\n"
        runner = _ok_runner({"/wt1": ""}, {"/wt1": "0\t3\n"})
        report = collect(
            "/root", runner=runner, worktrees_provider=lambda: porcelain
        )
        low_ahead = [
            r for r in report["risks"]
            if r["kind"] == "ahead" and r["severity"] == "low"
        ]
        self.assertTrue(low_ahead, "expected a low-severity ahead risk")

    def test_summary_counts(self):
        porcelain = (
            "worktree /wt1\nHEAD aaa\nbranch refs/heads/b1\n\n"
            "worktree /wt2\nHEAD bbb\nbranch refs/heads/b2\n"
        )
        runner = _ok_runner(
            {"/wt1": " M a.txt\n M b.txt\n", "/wt2": " M c.txt\n"},
            {"/wt1": "0\t5\n", "/wt2": "0\t0\n"},
        )
        report = collect(
            "/root", runner=runner, worktrees_provider=lambda: porcelain
        )
        s = report["summary"]
        self.assertEqual(s["worktrees"], 2)
        self.assertEqual(s["dirty_total"], 3)
        self.assertEqual(s["ahead_total"], 5)
        self.assertEqual(s["overlaps"], 0)
        self.assertEqual(s["risks"], len(report["risks"]))

    def test_report_shape(self):
        porcelain = "worktree /wt1\nHEAD aaa\nbranch refs/heads/b1\n"
        runner = _ok_runner({"/wt1": " M a.txt\n"}, {"/wt1": "0\t0\n"})
        report = collect(
            "/root", runner=runner, worktrees_provider=lambda: porcelain
        )
        self.assertEqual(report["version"], "worktree-conflict-report/1")
        self.assertEqual(report["tool"], "worktree-conflict-checker")
        self.assertIn("generated_at", report)
        wt = report["worktrees"][0]
        for key in ("path", "branch", "head", "dirty_files",
                    "dirty_paths", "ahead", "behind", "notes"):
            self.assertIn(key, wt)
        self.assertEqual(wt["dirty_files"], 1)
        self.assertEqual(wt["dirty_paths"], ["a.txt"])
        self.assertEqual(wt["ahead"], 0)
        self.assertEqual(wt["behind"], 0)


class TestCollectEdgeCases(unittest.TestCase):
    def test_zero_worktrees(self):
        report = collect(
            "/root",
            runner=_run_with([], {}),
            worktrees_provider=lambda: "",
        )
        self.assertEqual(report["worktrees"], [])
        self.assertEqual(report["overlaps"], [])
        self.assertEqual(report["risks"], [])
        s = report["summary"]
        self.assertEqual(s["worktrees"], 0)
        self.assertEqual(s["dirty_total"], 0)
        self.assertEqual(s["ahead_total"], 0)
        self.assertEqual(s["overlaps"], 0)
        self.assertEqual(s["risks"], 0)

    def test_risks_sorted_severity_then_kind(self):
        porcelain = (
            "worktree /wt1\nHEAD aaa\nbranch refs/heads/b1\n\n"
            "worktree /wt2\nHEAD bbb\nbranch refs/heads/b2\n"
        )
        # wt1: dirty 6 (medium dirty), ahead 12 (medium ahead), shared path.
        # wt2: dirty 1 (low dirty), shared path -> overlap (high).
        wt1_status = " M shared.txt\n" + "".join(
            " M g{}.txt\n".format(i) for i in range(5)
        )
        runner = _ok_runner(
            {"/wt1": wt1_status, "/wt2": " M shared.txt\n"},
            {"/wt1": "0\t12\n", "/wt2": "0\t0\n"},
        )
        report = collect(
            "/root", runner=runner, worktrees_provider=lambda: porcelain
        )
        risks = report["risks"]
        # there must be at least one of each: high, medium, low
        sevs = {r["severity"] for r in risks}
        self.assertIn("high", sevs)
        self.assertIn("medium", sevs)
        rank = {"high": 0, "medium": 1, "low": 2}
        keys = [(rank[r["severity"]], r["kind"]) for r in risks]
        self.assertEqual(keys, sorted(keys),
                         "risks not sorted by (severity desc, kind asc): %r" % risks)


class TestCLI(unittest.TestCase):
    def test_pretty_outputs_valid_json_exit0(self):
        porcelain = "worktree /wt1\nHEAD aaa\nbranch refs/heads/b1\n"
        runner = _ok_runner({"/wt1": ""}, {"/wt1": "0\t0\n"})
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            code = main(
                ["--pretty"],
                runner=runner,
                worktrees_provider=lambda: porcelain,
            )
        out = buf.getvalue().strip()
        self.assertEqual(code, 0)
        data = json.loads(out)  # must parse
        self.assertEqual(data["version"], "worktree-conflict-report/1")
        self.assertEqual(data["summary"]["worktrees"], 1)

    def test_default_root_is_cwd(self):
        # root positional defaults to "."; ensure no crash, exit 0.
        porcelain = "worktree /wt1\nHEAD aaa\nbranch refs/heads/b1\n"
        runner = _ok_runner({"/wt1": ""}, {"/wt1": "0\t0\n"})
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            code = main(
                [],
                runner=runner,
                worktrees_provider=lambda: porcelain,
            )
        self.assertEqual(code, 0)
        json.loads(buf.getvalue())  # parses


if __name__ == "__main__":
    unittest.main()
