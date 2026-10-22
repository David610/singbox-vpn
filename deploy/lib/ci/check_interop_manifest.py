#!/usr/bin/env python3
"""Exact inventory check for CI's real-sing-box interop step (singbox-validate).

  check_interop_manifest.py MANIFEST LOG

Exits 0 only when every manifest entry passed exactly once in its own suite, and no test outside the manifest ran,
failed, was ignored or skipped. This replaces a hard-coded pass count, which drifted when the decoy-budget suite
gained a test (12 passed, 11 expected: run 39799935640).
"""
from __future__ import annotations

import os
import re
import sys

_ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")   # CI runs with CARGO_TERM_COLOR=always
_RUNNING = re.compile(r"^\s*Running (?:\S*/)?(\w+)\.rs\b")
_RESULT = re.compile(r"^test (\S+) \.\.\. (\w+)\s*$")
_TEST_FN = re.compile(r"#\[(?:tokio::)?test(?:\([^)]*\))?\]\s*(?:#\[[^\]]*\]\s*)*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)")


def parse_manifest(text: str) -> list[tuple[str, str]]:
    out = []
    for n, line in enumerate(text.splitlines(), 1):
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        suite, sep, name = line.partition("::")
        if not sep or not re.fullmatch(r"\w+", suite) or not re.fullmatch(r"\w+", name):
            raise ValueError(f"manifest line {n}: expected <suite>::<test>, got {line!r}")
        out.append((suite, name))
    if len(set(out)) != len(out):
        raise ValueError("manifest lists a test twice")
    return out


def load_manifest(path: str) -> list[tuple[str, str]]:
    with open(path) as f:
        return parse_manifest(f.read())


def source_inventory(root: str, suites) -> set[tuple[str, str]]:
    found = set()
    for s in suites:
        with open(os.path.join(root, "crates", "compat-config", "tests", s + ".rs")) as f:
            found |= {(s, n) for n in _TEST_FN.findall(f.read())}
    return found


def results(log: str) -> list[tuple[str | None, str, str]]:
    out, suite = [], None
    for line in _ANSI.sub("", log).splitlines():
        m = _RUNNING.match(line)
        if m:
            suite = m.group(1)
            continue
        m = _RESULT.match(line)
        if m:
            out.append((suite, m.group(1), m.group(2)))
    return out


def passed(log: str) -> list[tuple[str | None, str]]:
    return [(s, n) for s, n, r in results(log) if r == "ok"]


def check(log: str, manifest: list[tuple[str, str]]) -> list[str]:
    errs = []
    want = set(manifest)
    ok = passed(log)
    for entry in manifest:
        c = ok.count(entry)
        if c == 0:
            errs.append(f"{entry[0]}::{entry[1]} did not pass")
        elif c > 1:
            errs.append(f"{entry[0]}::{entry[1]} passed {c} times")
    for s, n, r in results(log):
        if (s, n) not in want:
            errs.append(f"{s}::{n} ({r}) is not in the manifest")
        elif r != "ok":
            errs.append(f"{s}::{n} result {r}")
    if any(line.startswith("skipping:") for line in _ANSI.sub("", log).splitlines()):
        errs.append("a test printed 'skipping:' despite SINGBOX_VPN_REQUIRE_REAL_INTEROP=1")
    return errs


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(__doc__.strip().splitlines()[2].strip(), file=sys.stderr)
        return 2
    manifest = load_manifest(argv[0])
    with open(argv[1]) as f:
        errs = check(f.read(), manifest)
    for e in errs:
        print(f"::error::{e}")
    if errs:
        return 1
    print(f"all {len(manifest)} manifest interop tests executed and passed")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
