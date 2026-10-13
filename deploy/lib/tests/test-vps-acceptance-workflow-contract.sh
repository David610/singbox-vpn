#!/usr/bin/env bash
# Contract test for .github/workflows/vps-acceptance.yml: release acceptance
# must request the egress-policy stage, include the reboot, use a verified
# SSH host key, and independently require the stage's PASS in the log. These
# are static workflow assertions plus an execution of the workflow's own
# prerequisite and log-evidence shell snippets (extracted, not re-typed).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
WF="$ROOT/.github/workflows/vps-acceptance.yml"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
failures=0
fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }
ok() { echo "ok: $*"; }

python3 - "$WF" "$TMP" <<'PY'
import sys, pathlib
try:
    import yaml
except ImportError:
    sys.exit("PyYAML is required for the workflow contract test")
wf = yaml.safe_load(open(sys.argv[1], encoding="utf-8"))
tmp = pathlib.Path(sys.argv[2])
# PyYAML parses the bare key `on` as boolean True.
inputs = (wf.get("on") or wf.get(True))["workflow_dispatch"]["inputs"]
for name in ("verify_egress_policy", "ssh_known_hosts", "version", "skip_reboot"):
    assert name in inputs, f"missing input {name}"
assert inputs["verify_egress_policy"]["default"] is True, "egress stage must default to on"
steps = wf["jobs"]["guard"]["steps"] + wf["jobs"]["acceptance"]["steps"]
by_name = {s.get("name", ""): s for s in steps}
(tmp / "prereq.sh").write_text(by_name["Enforce release-acceptance prerequisites"]["run"])
(tmp / "evidence.sh").write_text(by_name["Require the egress-policy stage evidence in the log"]["run"])
run = by_name["Run the destructive lifecycle acceptance gate"]["run"]
(tmp / "run.sh").write_text(run)
assert "--verify-egress-policy" in run and "--known-hosts-file" in run
assert "${{" not in run, "inputs must reach the script only through env:"
PY
ok "workflow declares the egress/known_hosts inputs and wires both flags through env"

prereq() { RELEASE_VERSION="$1" VERIFY_EGRESS_POLICY="$2" SKIP_REBOOT="$3" SSH_KNOWN_HOSTS="$4" bash "$TMP/prereq.sh" >/dev/null 2>&1; }
prereq "" false true "" && ok "a development run (no version) is not blocked by release prerequisites" \
  || fail "development runs must stay possible"
prereq v9.9.9-rc.1 true false "host ssh-ed25519 AAAA" && ok "a complete release-acceptance request passes the prerequisite step" \
  || fail "a complete release request was rejected"
! prereq v9.9.9-rc.1 false false "host ssh-ed25519 AAAA" && ok "release acceptance without the egress stage is refused" \
  || fail "release acceptance may not drop the egress stage"
! prereq v9.9.9-rc.1 true true "host ssh-ed25519 AAAA" && ok "release acceptance cannot skip the reboot" \
  || fail "release acceptance may not skip the reboot"
! prereq v9.9.9-rc.1 true false "" && ok "release acceptance requires a verified host key" \
  || fail "release acceptance may not trust an unknown host key"

good="$TMP/good.log"
cat > "$good" <<'LOG'
=== 16c. candidate standalone exit egress policy convergence ===
[PASS] candidate exit egress apply/reconcile/doctor (restored initial policy)
LIFECYCLE GATE: PASS
LOG
evidence() { (cd "$TMP" && GITHUB_STEP_SUMMARY="$TMP/summary.md" bash "$TMP/evidence.sh" >/dev/null 2>&1); }
cp "$good" "$TMP/lifecycle-acceptance.log"
evidence && ok "the evidence step accepts a log with the stage PASS and the gate PASS" || fail "good log rejected"
grep -q 'egress-policy stage 16c: PASS' "$TMP/summary.md" && ok "the result is written to the job summary" || fail "no job summary"
grep -v '^=== 16c' "$good" > "$TMP/lifecycle-acceptance.log"
! evidence && ok "a log without stage 16c fails the workflow" || fail "absent stage accepted"
grep -v '^\[PASS\] candidate exit egress' "$good" > "$TMP/lifecycle-acceptance.log"
! evidence && ok "a log without the stage PASS fails the workflow" || fail "stage without PASS accepted"
grep -v '^LIFECYCLE GATE' "$good" > "$TMP/lifecycle-acceptance.log"
! evidence && ok "a log without the overall gate PASS fails the workflow" || fail "missing gate PASS accepted"

if [ "$failures" -ne 0 ]; then
  echo "FAIL: test-vps-acceptance-workflow-contract.sh ($failures failure(s))" >&2
  exit 1
fi
echo "PASS: test-vps-acceptance-workflow-contract.sh"
