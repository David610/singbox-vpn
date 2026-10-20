#!/usr/bin/env python3
"""Lint (I-53): early-exit consumers fed by a pipe in scripts that use
`set -o pipefail`.

`grep -q`/`-m`/`-l` stop reading at the first match; the producer then gets
SIGPIPE and, under pipefail, the whole pipeline reports failure even though
the pattern WAS found. With a value larger than one write (~4 KiB) this
flakes at random (measured 14/6000 for a 10.7 KB value; 0 for the here-string
form). Rules, in files that set pipefail:

  R1  echo/printf/cat ... | grep -q/-l/-m ... -> grep ... <<<"$value"  (or < file)
      (a non-early-exit grep such as -o/-c/-n reads all input: no SIGPIPE, not flagged)
  R3  a command / echo / printf / cat piped into an early-exit consumer
        (awk ... exit, head, sed ... q, grep -m, read) with no `|| ...` on the
        same line: the producer is killed by SIGPIPE, pipefail reports 141, and
        under `set -e` the assignment aborts the script. Append `|| true` (the
        value is already captured) or capture the output first.
  R2  a known large-output command | grep -q/-m/-l or | head
        (journalctl, nft list, iptables-save, openssl s_client, vpn-admin doctor,
         systemctl list-unit-files, dpkg -l, rpm -qa) -> capture first, then grep <<<

Suppress a reviewed line with a trailing `# pipefail-ok: reason`.
Usage: lint_pipefail_pipes.py [paths...]   (default: whole repo)   exit 1 on findings.
"""
import glob
import os
import re
import sys

R1 = re.compile(r"(?:^|[;&|(]|\bthen\b|\bdo\b|\bif\b|\belif\b|!)\s*(?:echo\b|printf\b|cat\b)(?P<prod>[^|\n]*)\|\s*grep\b(?P<opts>(?:\s+-{1,2}[A-Za-z][A-Za-z-]*(?:=\S+)?)*)")
EARLY = re.compile(r"(?:^|\s)(?:-[A-Za-z]*[qlm][A-Za-z]*|--quiet|--silent|--max-count(?:=\S+)?|--files-with-matches)(?=\s|$)")


def r1_hit(code):
    for m in R1.finditer(code):
        prod = m.group("prod")
        if prod.count("$(") > prod.count(")"):        # the pipe is inside a command substitution
            continue
        if EARLY.search(m.group("opts") or ""):
            return True
    return False

R2 = re.compile(r"\b(?:journalctl|nft\s+list|iptables-save|ip6tables-save|openssl\s+s_client|vpn-admin\s+doctor|systemctl\s+list-unit-files|dpkg\s+-l|rpm\s+-qa)\b[^|\n]*\|\s*(?:grep\s+-[A-Za-z]*[qlm]|head\b)")
R3 = re.compile(r"(?P<prod>[^|;&()]+?)\|\s*(?:awk\b[^|]*\bexit\b|head\b|sed\b[^|]*\bq\b|grep\s+-[A-Za-z]*m\b|read\b)")
FILTERS = ("grep", "sed", "awk", "tr", "cut", "sort", "uniq", "wc", "tail", "head", "jq", "xargs", "tee")


def r3_hit(code):
    if "||" in code:
        return False                                    # tolerated: status is handled
    for m in R3.finditer(code):
        prod = m.group("prod").strip().split("|")[-1].strip()
        first = prod.split()[0] if prod.split() else ""
        if first in FILTERS or first.startswith(("$(", '"$(')):
            continue                                    # small filtered stream, not a raw producer
        if first in ("echo", "printf") and "$" not in prod:
            continue                                    # constant string, tiny
        return True
    return False


HERE = re.compile(r"<<-?\s*(['\"]?)([A-Za-z_][A-Za-z0-9_]*)\1")


ALLOW_FILE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "pipefail_lint_allow.txt")


def load_allow():
    out = []
    try:
        for line in open(ALLOW_FILE):
            line = line.strip()
            if line and not line.startswith("#"):
                path, _, rest = line.partition("|")
                needle, _, _why = rest.partition("|")
                out.append((path.strip(), needle.strip()))
    except OSError:
        pass
    return out


ALLOW = load_allow()


def scan(path):
    try:
        text = open(path, errors="replace").read()
    except OSError:
        return []
    if "pipefail" not in text:
        return []
    out, here = [], None
    for n, line in enumerate(text.split("\n"), 1):
        if here:
            if line.strip() == here:
                here = None
            continue
        m = HERE.search(line)
        s = line.strip()
        if m:
            here = m.group(2)
        if s.startswith("#") or "pipefail-ok" in line:
            continue
        if any(path.endswith(a) and needle in line for a, needle in ALLOW):
            continue
        code = re.sub(r"'[^']*'", "''", line)          # blank out single-quoted text
        if r1_hit(code):
            out.append((path, n, "R1", s[:120]))
        elif R2.search(code):
            out.append((path, n, "R2", s[:120]))
        elif r3_hit(line):
            out.append((path, n, "R3", s[:120]))
    return out


def main():
    args = sys.argv[1:]
    if args:
        files = args
    else:
        files = [f for f in glob.glob("**/*.sh", recursive=True) + ["install.sh", "uninstall.sh", "bin/singbox-vpn-uninstall"]
                 if not f.startswith(("target/", "node_modules/", "deploy/lab/", "docs/")) and os.path.isfile(f)]
    findings = [x for f in sorted(set(files)) for x in scan(f)]
    for p, n, r, s in findings:
        print(f"{p}:{n}: {r}: {s}")
    print(f"{len(findings)} finding(s)")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
