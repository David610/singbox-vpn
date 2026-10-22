#!/usr/bin/env python3
"""Machine-verify the `main` branch ruleset (issue #170).

Input is the JSON GitHub returns for one ruleset:
    gh api repos/OWNER/REPO/rulesets/ID > ruleset.json
    python3 deploy/lib/verify-main-protection.py ruleset.json

Exit 0 only if every check passes. It never changes anything. It checks that
the required status checks are present AND that nothing already in place was
weakened (pull request rule, deletion / non-fast-forward blocks, no bypass).
Classic branch protection cannot be read with an integration token (403); this
tool therefore verifies the ruleset only and says so.
"""
import argparse
import json
import sys

REQUIRED_CHECKS = ("offline-full-verdict", "codeql-rust", "codeql-actions")


def verify(ruleset, required=REQUIRED_CHECKS):
    findings = []

    def check(name, ok, detail=""):
        findings.append({"check": name, "ok": bool(ok), "detail": detail})

    rules = {}
    for rule in ruleset.get("rules") or []:
        rules.setdefault(rule.get("type"), []).append(rule)

    check("enforcement is active", ruleset.get("enforcement") == "active", str(ruleset.get("enforcement")))
    check("target is branch", ruleset.get("target") == "branch", str(ruleset.get("target")))
    ref = (ruleset.get("conditions") or {}).get("ref_name") or {}
    include = ref.get("include") or []
    check(
        "applies to the default branch",
        any(i in ("~DEFAULT_BRANCH", "refs/heads/main", "~ALL") for i in include) and not ref.get("exclude"),
        f"include={include} exclude={ref.get('exclude')}",
    )
    check("no bypass actors", not ruleset.get("bypass_actors"), str(ruleset.get("bypass_actors")))
    check("deletion is blocked", "deletion" in rules)
    check("non-fast-forward updates are blocked", "non_fast_forward" in rules)
    check("a pull request is required", "pull_request" in rules)

    status_rules = rules.get("required_status_checks") or []
    configured = {}
    strict = False
    for rule in status_rules:
        params = rule.get("parameters") or {}
        strict = strict or bool(params.get("strict_required_status_checks_policy"))
        for entry in params.get("required_status_checks") or []:
            configured[entry.get("context")] = entry
    check("a required_status_checks rule exists", bool(status_rules))
    for context in required:
        check(f"required check '{context}'", context in configured)
    check("checks must be up to date with the base (current commits)", strict)
    return findings


def update_payload(ruleset, required=REQUIRED_CHECKS):
    """The body for `PUT /repos/OWNER/REPO/rulesets/ID` that adds the required
    checks and keeps every existing rule, condition and (empty) bypass list
    byte-for-byte. A PUT replaces the whole rule list, so building it from the
    live document is what prevents an accidental weakening."""
    body = {
        key: ruleset[key]
        for key in ("name", "target", "enforcement", "conditions", "rules", "bypass_actors")
        if key in ruleset
    }
    body["rules"] = [dict(r) for r in body.get("rules", []) if r.get("type") != "required_status_checks"]
    body["rules"].append({
        "type": "required_status_checks",
        "parameters": {
            "strict_required_status_checks_policy": True,
            "do_not_enforce_on_create": False,
            "required_status_checks": [{"context": c} for c in required],
        },
    })
    return body


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("ruleset_json")
    parser.add_argument("--require", default=",".join(REQUIRED_CHECKS))
    parser.add_argument("--json", action="store_true", help="print machine-readable findings")
    parser.add_argument("--emit-update-payload", action="store_true",
                        help="print the PUT body that adds the checks and preserves every existing rule; changes nothing")
    args = parser.parse_args(argv)
    try:
        with open(args.ruleset_json, encoding="utf-8") as handle:
            ruleset = json.load(handle)
    except (OSError, ValueError) as error:
        print(f"verify-main-protection: cannot read ruleset JSON: {error}", file=sys.stderr)
        return 2
    if not isinstance(ruleset, dict):
        print("verify-main-protection: ruleset JSON must be an object", file=sys.stderr)
        return 2
    required = tuple(c for c in args.require.split(",") if c)
    if args.emit_update_payload:
        print(json.dumps(update_payload(ruleset, required), indent=2, sort_keys=True))
        return 0
    findings = verify(ruleset, required)
    failed = [f for f in findings if not f["ok"]]
    if args.json:
        print(json.dumps({"ok": not failed, "findings": findings}, indent=2))
    else:
        for f in findings:
            print(("PASS " if f["ok"] else "FAIL ") + f["check"] + (f"  [{f['detail']}]" if f["detail"] and not f["ok"] else ""))
        print("main protection: " + ("PASS" if not failed else f"NOT ENFORCED ({len(failed)} check(s) failing)"))
    return 0 if not failed else 1


if __name__ == "__main__":
    sys.exit(main())
