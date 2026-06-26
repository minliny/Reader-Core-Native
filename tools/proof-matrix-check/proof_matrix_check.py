#!/usr/bin/env python3
"""Compatibility wrapper for the legacy fixture proof-matrix check."""

import os
import sys


ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(ROOT, "legacy-fixture-register"))

import legacy_fixture_register as lfr  # noqa: E402


def main(argv=None):
    argv = list(argv or sys.argv[1:])
    if "validate" not in argv and "proof" not in argv:
        prefix = []
        rest = []
        i = 0
        while i < len(argv):
            if argv[i] == "--repo-root" and i + 1 < len(argv):
                prefix.extend(argv[i:i + 2])
                i += 2
                continue
            rest = argv[i:]
            break
        argv = prefix + ["proof"] + rest
    return lfr.main(argv)


if __name__ == "__main__":
    sys.exit(main())
