#!/usr/bin/env python3
"""C ABI symbol checker for Reader-Core native libraries.

Verifies that a built static (.a) or dynamic (.so / .dylib / .dll) library
exports the expected ``rc_*`` symbols declared in ``include/reader_core.h``.
This catches missing ABI symbols at build time instead of at wrapper-integration
time (iOS Swift wrapper / Android JNI / HarmonyOS NAPI / C/C++ smoke).

Usage::

    abi-symbol-check.py <library-path> [--required NAME ...] [--verbose]
    abi-symbol-check.py --help

The tool auto-detects the platform and picks the best available symbol reader.
It tries, in order: ``llvm-nm`` (from the Rust toolchain, handles newest bitcode),
system ``nm``, ``readelf`` (Linux), and ``otool`` (macOS). It works on both
static archives and shared libraries.

Required symbols (default set, matching ``include/reader_core.h``)::

    rc_abi_version
    rc_runtime_create
    rc_runtime_send
    rc_runtime_cancel
    rc_runtime_destroy

Exit codes::

    0 — all required symbols are defined in the library
    1 — one or more required symbols are missing (FAIL)
    2 — tool/usage error (file not found, no symbol reader available, ...)
"""

from __future__ import annotations

import argparse
import glob
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass, field

# Platform flag for macOS (Mach-O uses -gU; Linux ELF uses -D for .so).
_IS_DARWIN = sys.platform == "darwin"

# Default required ABI surface. Keep in sync with include/reader_core.h.
# Do NOT read the header at runtime — the whole point is to catch drift between
# the header and the build artifact, so the expected set is hardcoded here.
DEFAULT_REQUIRED_SYMBOLS: tuple[str, ...] = (
    "rc_abi_version",
    "rc_runtime_create",
    "rc_runtime_send",
    "rc_runtime_cancel",
    "rc_runtime_destroy",
)

# nm type letters that mean "defined symbol" (as opposed to 'U' = undefined).
# Uppercase = global/external, lowercase = local. We accept both for static
# archives (where everything is available to the linker anyway) and restrict
# to uppercase globals for shared libraries (the actual exported surface).
DEFINED_NM_TYPES = set("TtRrDdBbSsGgCcVvWwIi")


@dataclass
class CheckResult:
    """Outcome of a symbol check."""

    library: str
    required: list[str]
    found: set[str] = field(default_factory=set)
    missing: list[str] = field(default_factory=list)
    symbol_tool: str = ""
    is_shared: bool = False
    error: str | None = None

    @property
    def passed(self) -> bool:
        return self.error is None and not self.missing


def _run(cmd: list[str]) -> tuple[int, str, str]:
    """Run a command and return (returncode, stdout, stderr)."""
    proc = subprocess.run(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return proc.returncode, proc.stdout, proc.stderr


def _has_tool(name: str) -> bool:
    return shutil.which(name) is not None


def _find_llvm_nm() -> str | None:
    """Locate ``llvm-nm`` bundled with the Rust toolchain, if present.

    The Rust ``llvm-tools`` component ships ``llvm-nm`` under
    ``<sysroot>/lib/rustlib/<host-triple>/bin/llvm-nm``. Unlike the system
    ``nm`` on macOS, it can parse object files produced by newer Rust/LLVM
    toolchains (the Xcode ``nm`` reader lags Rust's LLVM and emits
    "Unknown attribute kind" errors on static archives).
    """
    # On PATH?
    on_path = shutil.which("llvm-nm")
    if on_path:
        return on_path
    # Bundled with rustup?
    rc, out, _ = _run(["rustc", "--print", "sysroot"])
    if rc != 0 or not out.strip():
        return None
    sysroot = out.strip()
    # The host triple directory name varies; glob for any llvm-nm under bin/.
    candidates = glob.glob(
        os.path.join(sysroot, "lib", "rustlib", "*", "bin", "llvm-nm")
    )
    if candidates:
        return candidates[0]
    return None


def _looks_shared(path: str) -> bool:
    """Heuristic: is this a shared library rather than a static archive?"""
    ext = os.path.splitext(path)[1].lower()
    if ext in (".a", ".lib"):
        return False
    if ext in (".so", ".dylib", ".dll"):
        return True
    # Fall back to the `file` command for extensionless / unusual names.
    if _has_tool("file"):
        rc, out, _ = _run(["file", "-b", path])
        if rc == 0:
            low = out.lower()
            if "shared" in low or "dynamic" in low or "pie executable" in low:
                return True
            if "archive" in low or "static" in low:
                return False
    # Unknown → assume shared (stricter check: only globals count).
    return True


def _strip_macho_underscore(sym: str) -> str:
    """Strip a single leading underscore (Mach-O C-symbol convention).

    On macOS, a C symbol ``rc_abi_version`` appears in the symbol table as
    ``_rc_abi_version``. We normalize so the same required-name list works on
    every platform. Only ONE leading underscore is stripped, and only if the
    remainder is non-empty — this matches how the Mach-O linker names C symbols.
    """
    if len(sym) >= 2 and sym.startswith("_") and not sym.startswith("__"):
        return sym[1:]
    return sym


def _parse_nm_line(line: str) -> tuple[str, str] | None:
    """Parse one line of `nm` output into (type_char, name).

    nm output formats encountered::

        0000000100000abc T rc_abi_version      # addr type name
                         U _printf             # indented, undefined
        ---------------- T _rc_abi_version      # llvm-nm uses dashes for no addr
        T rc_abi_version                        # no addr (some -D outputs)

    Returns None for blank lines, header lines, or unparseable lines.
    """
    line = line.rstrip("\n")
    if not line.strip():
        return None
    parts = line.split()
    # Drop a leading hex address, or a run of dashes (llvm-nm "no address").
    if parts and all(c in "0123456789abcdefABCDEF" for c in parts[0]) and len(parts[0]) > 0:
        parts = parts[1:]
    elif parts and set(parts[0]) <= {"-"}:
        parts = parts[1:]
    if len(parts) < 2:
        return None
    type_field = parts[0]
    name = parts[1]
    # nm type is a single letter (sometimes followed by a space then name).
    if len(type_field) != 1:
        return None
    return type_field, name


def _shared_nm_flags() -> list[str]:
    """nm flags for reading a shared library's exported symbols.

    On macOS (Mach-O) ``-D`` selects the dynamic symbol table but, unlike Linux,
    does not by itself restrict to defined globals — ``-gU`` (global + defined
    only) is the right combo and works on ``.dylib`` files. On Linux (ELF)
    ``-D`` reads the ``.dynsym`` section which is the actual export table.
    """
    if _IS_DARWIN:
        return ["-g", "-U"]
    return ["-D"]


def _collect_nm_symbols(
    nm_bin: str, path: str, *, shared: bool, extra_flags: list[str]
) -> set[str]:
    """Run nm and return defined symbol names.

    nm may return a non-zero exit code yet still emit valid symbol lines for
    some object files (e.g. when it can't parse bitcode attributes of
    compiler_builtins but succeeds for reader-ffi). We treat the run as
    best-effort: if stdout contains any parseable defined symbols, we use them.
    Raises RuntimeError only if the run produces zero usable symbols.
    """
    cmd = [nm_bin]
    if shared:
        cmd += _shared_nm_flags()
    cmd += extra_flags
    cmd.append(path)
    rc, out, err = _run(cmd)
    found: set[str] = set()
    for line in out.splitlines():
        parsed = _parse_nm_line(line)
        if parsed is None:
            continue
        type_char, name = parsed
        if type_char == "U":
            continue  # undefined — not what we want
        if type_char not in DEFINED_NM_TYPES:
            continue
        found.add(_strip_macho_underscore(name))
    if not found and rc != 0:
        raise RuntimeError(f"{nm_bin} failed (rc={rc}): {err.strip() or out.strip()}")
    return found


def _collect_otool_symbols(path: str) -> set[str]:
    """Use ``otool -T`` (macOS dylib) to list exported symbols.

    Note: Rust ``cdylib`` crates frequently export a *symbol export trie* but
    leave the Mach-O *table of contents* empty, in which case ``otool -T``
    prints ``Table of contents (0 entries)`` and no symbols. This function
    raises RuntimeError in that case so the caller falls through to a better
    reader (``nm -gU`` or ``llvm-nm``).
    """
    rc, out, err = _run(["otool", "-T", path])
    if rc != 0:
        raise RuntimeError(f"otool -T failed (rc={rc}): {err.strip()}")
    found: set[str] = set()
    for line in out.splitlines():
        stripped = line.strip()
        if not stripped:
            continue
        # Skip header / summary lines.
        if stripped == path or stripped.startswith(path + ":"):
            continue
        if stripped.startswith(("Table of contents", "module index", "Exports")):
            continue
        # A real exported-symbol line is a single token, optionally with a
        # leading address. Reject lines with spaces inside the token (e.g.
        # "module index symbol index") or parentheses.
        parts = stripped.split()
        candidate = parts[-1]
        if any(c in candidate for c in "() \t"):
            continue
        # Mach-O C symbols start with '_'; allow bare names too (rare).
        if not (candidate[0].isalpha() or candidate[0] == "_"):
            continue
        found.add(_strip_macho_underscore(candidate))
    if not found:
        raise RuntimeError("otool -T returned no exported symbols (empty TOC)")
    return found


def _collect_otool_tv_labels(path: str) -> set[str]:
    """Last-resort fallback for static archives on macOS: parse disassembly.

    ``otool -tv`` prints function labels as ``_symbol_name:`` at the start of
    each function's disassembly. This is slow on large archives but works even
    when the system ``nm`` cannot read newer LLVM bitcode attributes and
    ``llvm-nm`` is not installed. Only the label lines are collected; the
    disassembly body is discarded.
    """
    rc, out, err = _run(["otool", "-tv", path])
    if rc != 0:
        raise RuntimeError(f"otool -tv failed (rc={rc}): {err.strip()}")
    found: set[str] = set()
    for line in out.splitlines():
        stripped = line.strip()
        # Label lines look like ``_rc_abi_version:`` with nothing else.
        if stripped.endswith(":") and not stripped.startswith("0x"):
            label = stripped[:-1]
            # Filter out obviously non-symbol labels (e.g. section markers).
            if label and (label[0].isalpha() or label[0] == "_"):
                found.add(_strip_macho_underscore(label))
    if not found:
        raise RuntimeError("otool -tv returned no symbol labels")
    return found


def _collect_readelf_symbols(path: str) -> set[str]:
    """Use ``readelf --dyn-syms`` (Linux) to list dynamic symbols."""
    rc, out, err = _run(["readelf", "-sW", "--dyn-syms", path])
    if rc != 0:
        raise RuntimeError(f"readelf failed (rc={rc}): {err.strip()}")
    found: set[str] = set()
    for line in out.splitlines():
        # readelf symbol table rows are column-aligned, e.g.:
        #    7: 0000000000002a30   176 FUNC    GLOBAL DEFAULT   15 rc_abi_version
        # We only care about defined GLOBAL/WEAK FUNC/OBJECT symbols.
        if "UND" in line.split():
            continue
        if "GLOBAL" not in line and "WEAK" not in line:
            continue
        parts = line.split()
        if len(parts) < 8:
            continue
        name = parts[-1]
        if name and name != "UND":
            found.add(name)
    if not found:
        raise RuntimeError("readelf returned no defined symbols")
    return found


def collect_symbols(path: str) -> tuple[set[str], str, bool]:
    """Collect defined symbol names from a library.

    Returns (symbol_set, tool_used, is_shared). Raises RuntimeError if no
    usable symbol reader is available or all readers fail.

    Strategy order (first non-empty result wins):
      Shared libs:
        1. system nm -D            (works on macOS dylibs + Linux .so)
        2. readelf --dyn-syms      (Linux fallback)
        3. otool -T                (macOS fallback; often empty for Rust cdylibs)
        4. llvm-nm -D              (last resort)
      Static archives:
        1. llvm-nm                 (handles newest Rust/LLVM bitcode)
        2. system nm               (works on older toolchains / Linux)
        3. readelf -s              (Linux fallback)
        4. otool -tv               (macOS last resort; parses disassembly labels)
    """
    shared = _looks_shared(path)
    errors: list[str] = []
    llvm_nm = _find_llvm_nm()

    if shared:
        if _has_tool("nm"):
            try:
                found = _collect_nm_symbols("nm", path, shared=True, extra_flags=["-g"])
                if found:
                    return found, "nm", shared
            except RuntimeError as e:
                errors.append(str(e))
        if _has_tool("readelf"):
            try:
                found = _collect_readelf_symbols(path)
                if found:
                    return found, "readelf", shared
            except RuntimeError as e:
                errors.append(str(e))
        if _has_tool("otool"):
            try:
                found = _collect_otool_symbols(path)
                if found:
                    return found, "otool", shared
            except RuntimeError as e:
                errors.append(str(e))
        if llvm_nm:
            try:
                found = _collect_nm_symbols(llvm_nm, path, shared=True, extra_flags=["-g"])
                if found:
                    return found, "llvm-nm", shared
            except RuntimeError as e:
                errors.append(str(e))
    else:
        if llvm_nm:
            try:
                found = _collect_nm_symbols(llvm_nm, path, shared=False, extra_flags=[])
                if found:
                    return found, "llvm-nm", shared
            except RuntimeError as e:
                errors.append(str(e))
        if _has_tool("nm"):
            try:
                found = _collect_nm_symbols("nm", path, shared=False, extra_flags=[])
                if found:
                    return found, "nm", shared
            except RuntimeError as e:
                errors.append(str(e))
        if _has_tool("readelf"):
            try:
                found = _collect_readelf_symbols(path)
                if found:
                    return found, "readelf", shared
            except RuntimeError as e:
                errors.append(str(e))
        if _has_tool("otool"):
            try:
                found = _collect_otool_tv_labels(path)
                if found:
                    return found, "otool-tv", shared
            except RuntimeError as e:
                errors.append(str(e))

    raise RuntimeError(
        "no usable symbol reader succeeded. "
        f"Tried: {errors or 'nm/otool/readelf not found on PATH'}. "
        "Hint: on macOS, install the Rust llvm-tools component "
        "(`rustup component add llvm-tools`) so llvm-nm can read static "
        "archives built with newer Rust toolchains."
    )


def check_library(path: str, required: list[str], *, verbose: bool = False) -> CheckResult:
    """Check that `path` defines all `required` symbols."""
    if not os.path.isfile(path):
        return CheckResult(
            library=path, required=required, error=f"file not found: {path}"
        )

    try:
        found, tool, is_shared = collect_symbols(path)
    except RuntimeError as e:
        return CheckResult(library=path, required=required, error=str(e))

    missing = [s for s in required if s not in found]
    result = CheckResult(
        library=path,
        required=required,
        found=found,
        missing=missing,
        symbol_tool=tool,
        is_shared=is_shared,
    )

    if verbose:
        print(f"[info] library:    {path}")
        print(f"[info] shared:     {is_shared}")
        print(f"[info] symbol tool:{tool}")
        print(f"[info] found {len(found)} defined symbols")
        for name in sorted(found):
            print(f"       + {name}")
    return result


def report(result: CheckResult, *, stream=sys.stdout) -> None:
    """Print a human-readable pass/fail report."""
    if result.error is not None:
        print(f"ERROR: {result.error}", file=stream)
        return

    label = "PASS" if result.passed else "FAIL"
    print(f"{label}: {result.library}", file=stream)
    print(f"  symbol reader : {result.symbol_tool}", file=stream)
    print(f"  library type  : {'shared' if result.is_shared else 'static archive'}", file=stream)
    print(f"  required      : {len(result.required)} symbol(s)", file=stream)
    print(f"  found         : {len(result.required) - len(result.missing)} / {len(result.required)}", file=stream)
    if result.missing:
        print(f"  MISSING ({len(result.missing)}):", file=stream)
        for name in result.missing:
            print(f"    - {name}", file=stream)
    else:
        print("  all required symbols present", file=stream)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="abi-symbol-check",
        description="Check a Reader-Core native library for required rc_* ABI symbols.",
    )
    parser.add_argument(
        "library",
        help="Path to the built library (.a, .so, .dylib, or extensionless).",
    )
    parser.add_argument(
        "--required",
        nargs="*",
        default=list(DEFAULT_REQUIRED_SYMBOLS),
        help=(
            "Symbol names that must be defined. Defaults to the ABI v1 surface: "
            + ", ".join(DEFAULT_REQUIRED_SYMBOLS)
        ),
    )
    parser.add_argument(
        "--verbose", "-v",
        action="store_true",
        help="List every defined symbol found in addition to the pass/fail report.",
    )
    args = parser.parse_args(argv)

    result = check_library(args.library, args.required, verbose=args.verbose)
    report(result)
    if result.error is not None:
        return 2
    return 0 if result.passed else 1


if __name__ == "__main__":
    sys.exit(main())
