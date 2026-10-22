#!/usr/bin/env python3
"""Summarise sshd authentication events for an owner who must decide whether a
host is trusted (issue #167). Read-only: it parses text, it does not touch sshd.

Two outputs, deliberately different:

* stdout, REDACTED and safe to paste into an issue or chat: counts, and a
  timeline of successful logins in which every source address is replaced by a
  stable label (S1, S2, ...). Addresses, invalid-user names (people mistype
  passwords there) and key material never appear.
* --private-out FILE (mode 0600, refused if it exists), for the owner only:
  the label -> address mapping and the raw successful-login lines. The owner
  matches S1.. to their own phone/ISP/VPN addresses and answers "mine /
  not mine" without publishing the addresses.

Anomaly flags are hints for a human, not a verdict:
  ROOT_PASSWORD_SUCCESS   root authenticated with a password
  SUCCESS_AFTER_FAILURES  a source succeeded after >= N failures (guessing pattern)
  UNEXPECTED_SOURCE       success from a source not in --expected (private list)
"""
import argparse
import collections
import json
import os
import re
import sys

ACCEPTED = re.compile(
    r"Accepted (?P<method>\S+) for (?P<user>\S+) from (?P<src>\S+) port \d+ ssh2"
    r"(?:: (?P<keytype>\S+) (?P<fp>SHA256:\S+))?"
)
FAILED = re.compile(r"Failed (?P<method>\S+) for (?:invalid user )?\S+ from (?P<src>\S+) port \d+")
INVALID = re.compile(r"Invalid user .* from (?P<src>\S+)")
ISO_TS = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)")
SYSLOG_TS = re.compile(r"^([A-Z][a-z]{2} +\d+ \d\d:\d\d:\d\d)")


def timestamp(line):
    for pattern in (ISO_TS, SYSLOG_TS):
        match = pattern.match(line)
        if match:
            return match.group(1)
    return ""


def analyse(lines, expected=(), fail_threshold=5):
    labels = {}
    failures = collections.Counter()
    methods_failed = collections.Counter()
    invalid_attempts = 0
    successes = []
    flags = []

    def label(src):
        if src not in labels:
            labels[src] = f"S{len(labels) + 1}"
        return labels[src]

    for line in lines:
        accepted = ACCEPTED.search(line)
        if accepted:
            src = accepted["src"]
            record = {
                "time": timestamp(line),
                "account": accepted["user"],
                "method": accepted["method"],
                "source": label(src),
                "prior_failures_from_source": failures[src],
                "key_fingerprint": accepted["fp"] or "",
                "_src": src,
                "_raw": line.rstrip("\n"),
            }
            successes.append(record)
            if accepted["user"] == "root" and accepted["method"] == "password":
                flags.append(("ROOT_PASSWORD_SUCCESS", record))
            if failures[src] >= fail_threshold:
                flags.append(("SUCCESS_AFTER_FAILURES", record))
            if expected and src not in expected:
                flags.append(("UNEXPECTED_SOURCE", record))
            continue
        failed = FAILED.search(line)
        if failed:
            failures[failed["src"]] += 1
            methods_failed[failed["method"]] += 1
            continue
        if INVALID.search(line):
            invalid_attempts += 1
            failures[INVALID.search(line)["src"]] += 1

    public = {
        "successful_logins": [
            {k: v for k, v in s.items() if not k.startswith("_")} for s in successes
        ],
        "successful_by_method": dict(collections.Counter(s["method"] for s in successes)),
        "distinct_success_sources": len({s["_src"] for s in successes}),
        "failed_by_method": dict(methods_failed),
        "invalid_user_attempts": invalid_attempts,
        "distinct_failing_sources": len(failures),
        "flags": [
            {"flag": name, "time": rec["time"], "account": rec["account"], "source": rec["source"]}
            for name, rec in flags
        ],
        "note": "Sources are labels; the label->address map is in the private file only.",
    }
    private = {
        "labels": {label_: src for src, label_ in labels.items()},
        "successful_login_lines": [s["_raw"] for s in successes],
    }
    return public, private


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--input", default="-", help="log text file, or - for stdin")
    parser.add_argument("--expected", help="file with one expected source address per line (private)")
    parser.add_argument("--private-out", help="write the owner-only mapping here (mode 0600)")
    parser.add_argument("--fail-threshold", type=int, default=5)
    args = parser.parse_args(argv)
    try:
        text = sys.stdin.read() if args.input == "-" else open(args.input, encoding="utf-8", errors="replace").read()
        expected = set()
        if args.expected:
            expected = {l.strip() for l in open(args.expected, encoding="utf-8") if l.strip() and not l.startswith("#")}
    except OSError as error:
        print(f"ssh_auth_summary: {error}", file=sys.stderr)
        return 2
    public, private = analyse(text.splitlines(), expected, args.fail_threshold)
    if args.private_out:
        try:
            fd = os.open(args.private_out, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        except OSError as error:
            print(f"ssh_auth_summary: cannot create private output (it must not already exist): {error}", file=sys.stderr)
            return 2
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(private, handle, indent=2, sort_keys=True)
            handle.write("\n")
    print(json.dumps(public, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
