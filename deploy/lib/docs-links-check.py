#!/usr/bin/env python3
"""Deterministic documentation link check (no network).

Checks every tracked Markdown file for:
  * relative links whose target file or directory does not exist;
  * `#anchor` fragments (same-file or cross-file) with no matching heading;
  * (with --code-paths) repo-looking paths in `code spans` that do not exist.
Fenced code blocks are ignored. External URLs are NOT fetched here (see
docs-external-urls.py, which is advisory only).

Historical records are reported but do not fail the check: a file is
historical when it starts with a "HISTORICAL" banner, or lives under a path in
HISTORICAL_PREFIXES. Everything else must be clean.

Usage: docs-links-check.py [--root DIR] [--code-paths] [--json]
Exit 0 = no findings in current docs; 1 = findings; 2 = usage error.
"""
import argparse, json, os, re, subprocess, sys
from urllib.parse import unquote

HISTORICAL_PREFIXES = (
    "docs/archive/", "docs/handoffs/", "docs/qualification/", "docs/reviews/",
    "docs/incidents/", "docs/release-notes/", "docs/health-probe-evidence/",
    "docs/b2-evidence/", "docs/superpowers/", "TASKS.md",
)
LINK = re.compile(r"(?<!\!)\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)|^\s*\[[^\]]+\]:\s*(\S+)", re.M)
FENCE = re.compile(r"^(```|~~~)")
HEADING = re.compile(r"^(#{1,6})\s+(.*?)\s*#*\s*$")
SPAN = re.compile(r"`([^`\n]+)`")
PATHLIKE = re.compile(r"^(?:\./)?(?:docs|deploy|crates|apps|services|fixtures|bin|fuzz|\.github)/[A-Za-z0-9_./-]+$")


def slug(text):
    text = re.sub(r"`([^`]*)`", r"\1", text)
    text = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", text)
    text = re.sub(r"<[^>]+>", "", text).strip().lower()
    text = re.sub(r"[^\w\- ]", "", text, flags=re.U)
    return text.replace(" ", "-")


def strip_fences(lines):
    out, fence = [], None
    for line in lines:
        m = FENCE.match(line.lstrip())
        if m:
            fence = None if fence == m.group(1) else (fence or m.group(1))
            out.append("")
        else:
            out.append("" if fence else line)
    return out


def anchors(path, cache={}):
    if path not in cache:
        seen, result = {}, set()
        try:
            lines = strip_fences(open(path, encoding="utf-8").read().splitlines())
        except OSError:
            lines = []
        for line in lines:
            m = HEADING.match(line)
            if m:
                s = slug(m.group(2)); n = seen.get(s, 0); seen[s] = n + 1
                result.add(s if n == 0 else f"{s}-{n}")
            for a in re.findall(r'<a\s+(?:name|id)="([^"]+)"', line):
                result.add(a)
        cache[path] = result
    return cache[path]


def tracked_markdown(root):
    out = subprocess.run(["git", "-C", root, "ls-files", "*.md"], capture_output=True, text=True, check=True).stdout
    return sorted(p for p in out.splitlines() if os.path.exists(os.path.join(root, p)))


def is_historical(rel, first_lines):
    return rel.startswith(HISTORICAL_PREFIXES) or any("HISTORICAL" in l.upper() for l in first_lines[:6])


def check(root, code_paths):
    findings = []
    for rel in tracked_markdown(root):
        full = os.path.join(root, rel)
        raw = open(full, encoding="utf-8", errors="replace").read().splitlines()
        hist = is_historical(rel, raw)
        for no, line in enumerate(strip_fences(raw), 1):
            targets = [a or b for a, b in LINK.findall(line)]
            for t in targets:
                t = t.strip("<>")
                if re.match(r"^[a-zA-Z][a-zA-Z0-9+.-]*:", t) or t.startswith("//"):
                    continue
                target, _, frag = t.partition("#")
                target = unquote(target)
                dest = full if target == "" else os.path.normpath(os.path.join(os.path.dirname(full), target))
                if target.startswith("/"):
                    dest = os.path.normpath(os.path.join(root, target.lstrip("/")))
                if not os.path.exists(dest):
                    findings.append((rel, no, "missing-target", t, hist)); continue
                if frag and os.path.isfile(dest) and dest.endswith(".md") and unquote(frag).lower() not in anchors(dest):
                    findings.append((rel, no, "missing-anchor", t, hist))
            if code_paths:
                for span in SPAN.findall(line):
                    cand = span.strip().rstrip(",.;:")
                    if PATHLIKE.match(cand) and not any(c in cand for c in "*<>{}$") and not os.path.exists(os.path.join(root, cand)):
                        findings.append((rel, no, "missing-code-path", cand, hist))
    if code_paths:
        # Paths named inside fenced command blocks: a stale script path there breaks a reader.
        for rel in tracked_markdown(root):
            raw = open(os.path.join(root, rel), encoding="utf-8", errors="replace").read().splitlines()
            hist = is_historical(rel, raw)
            fence = None
            for no, line in enumerate(raw, 1):
                m = FENCE.match(line.lstrip())
                if m:
                    fence = None if fence == m.group(1) else (fence or m.group(1)); continue
                if fence:
                    for tok in re.findall(r"[A-Za-z0-9_./-]+", line):
                        tok = tok.rstrip(".,;:")
                        tok = tok[2:] if tok.startswith("./") else tok
                        if PATHLIKE.match(tok) and not os.path.exists(os.path.join(root, tok)):
                            findings.append((rel, no, "missing-fenced-path", tok, hist))
    return findings


def main():
    ap = argparse.ArgumentParser(); ap.add_argument("--root", default="."); ap.add_argument("--code-paths", action="store_true"); ap.add_argument("--json", action="store_true")
    a = ap.parse_args()
    f = check(os.path.abspath(a.root), a.code_paths)
    current = [x for x in f if not x[4]]
    if a.json:
        print(json.dumps([dict(zip(("file", "line", "kind", "target", "historical"), x)) for x in f], indent=1))
    else:
        for rel, no, kind, t, hist in f:
            print(f"{'HIST ' if hist else 'FAIL '}{rel}:{no}: {kind}: {t}")
        print(f"\n{len(current)} finding(s) in current docs, {len(f) - len(current)} in historical records")
    return 1 if current else 0


if __name__ == "__main__":
    sys.exit(main())
