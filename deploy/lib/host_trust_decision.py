#!/usr/bin/env python3
"""Validate the owner's host-trust decision record (issue #167).

The record is a small JSON file the OWNER writes after reading their private
evidence (see docs/qualification/2026-10-22/HOST-TRUST-DECISION.md). This tool
only checks that the record is internally consistent and contains no secrets or
addresses. It never contacts a host, and a valid record is not a security
assessment: it is the owner's recorded decision.

Exit status: 0 TRUSTED (consistent), 3 UNTRUSTED (rebuild required),
4 UNDETERMINED (still blocked), 1 invalid record, 2 unreadable file.
"""
import json
import re
import sys

DECISIONS = {"TRUSTED": 0, "UNTRUSTED": 3, "UNDETERMINED": 4}
CLASSES = {"OWNER", "NOT_OWNER", "UNKNOWN"}
IPV4 = re.compile(r"\b(?:\d{1,3}\.){3}\d{1,3}\b")
HEXCOLON = re.compile(r"[0-9A-Fa-f:.]{3,}")
SECRET_SHAPES = (
    re.compile(r"BEGIN [A-Z ]*PRIVATE KEY"),
    re.compile(r"\bssh-(?:ed25519|rsa|dss)\s+AAAA"),
    re.compile(r"\b[A-Za-z0-9+/_-]{40,}={0,2}\b"),
)
REQUIRED = ("schema", "host_label", "decided_on", "decided_by", "decision", "sessions",
            "provider_snapshot_taken", "evidence_private_location")


def contains_ipv6(text):
    """True for full and `::`-compressed IPv6 literals; not for times like 08:42:11."""
    for match in HEXCOLON.finditer(text):
        token = match.group(0)
        colons = token.count(":")
        if colons >= 2 and ("::" in token or colons == 7) and re.fullmatch(r"[0-9A-Fa-f:.]+", token):
            return True
    return False


def strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for item in value.values():
            yield from strings(item)
    elif isinstance(value, list):
        for item in value:
            yield from strings(item)


def validate(record):
    errors = []
    if not isinstance(record, dict):
        return ["record must be a JSON object"]
    for key in REQUIRED:
        if key not in record:
            errors.append(f"missing field: {key}")
    if errors:
        return errors
    if record["schema"] != 1:
        errors.append("schema must be 1")
    if record["decision"] not in DECISIONS:
        errors.append("decision must be TRUSTED, UNTRUSTED or UNDETERMINED")
    if not re.fullmatch(r"\d{4}-\d\d-\d\d", str(record["decided_on"])):
        errors.append("decided_on must be YYYY-MM-DD")
    for field in ("host_label", "decided_by", "evidence_private_location"):
        if not str(record[field]).strip():
            errors.append(f"{field} must not be empty")
    sessions = record["sessions"]
    if not isinstance(sessions, list):
        errors.append("sessions must be a list")
        sessions = []
    ids = set()
    for session in sessions:
        if not isinstance(session, dict) or session.get("classification") not in CLASSES:
            errors.append("every session needs classification OWNER, NOT_OWNER or UNKNOWN")
            continue
        sid = session.get("id")
        if not sid or sid in ids:
            errors.append("every session needs a unique id (for example L1)")
        ids.add(sid)
    for text in strings(record):
        if IPV4.search(text) or contains_ipv6(text):
            errors.append("the record must not contain network addresses (use S1/L1 labels)")
            break
    for text in strings(record):
        if any(shape.search(text) for shape in SECRET_SHAPES):
            errors.append("the record looks like it contains key or token material")
            break
    if errors:
        return errors
    classes = [s["classification"] for s in sessions]
    if record["decision"] == "TRUSTED":
        if not sessions:
            errors.append("TRUSTED needs the historical sessions listed and classified")
        if any(c != "OWNER" for c in classes):
            errors.append("TRUSTED is inconsistent with a NOT_OWNER or UNKNOWN session")
        if record["provider_snapshot_taken"] is not True:
            errors.append("TRUSTED requires a provider snapshot to have been taken first")
    if record["decision"] == "UNDETERMINED" and sessions and all(c == "OWNER" for c in classes):
        errors.append("every session is OWNER: record TRUSTED, or say which fact is still missing in notes")
    return errors


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if len(argv) != 1:
        print("usage: host_trust_decision.py DECISION.json", file=sys.stderr)
        return 2
    try:
        record = json.load(open(argv[0], encoding="utf-8"))
    except (OSError, ValueError) as error:
        print(f"host_trust_decision: cannot read record: {error}", file=sys.stderr)
        return 2
    errors = validate(record)
    if errors:
        for error in errors:
            print(f"INVALID: {error}")
        return 1
    print(f"host_trust_decision: {record['decision']} (host {record['host_label']}, decided {record['decided_on']})")
    return DECISIONS[record["decision"]]


if __name__ == "__main__":
    sys.exit(main())
