#!/usr/bin/env bash
# Regression test: install acceptance must provision the reserved probe
# principal BEFORE running the L5-6 protocol self-test.
#
# Since C-16 (egress isolation), a node's own loopback listener is reachable
# only by a RESERVED PROBE principal (`vpn-admin user create-probe`); an
# ordinary user is deliberately fail-closed out of it. The protocol self-test
# prefers a probe identity and otherwise falls back to the first ordinary
# user, which then comes back INCONCLUSIVE. `doctor --protocol
# --require-protocol` turns that into a hard failure, so a fresh install
# rolled itself back on every host (found installing v1.1.0-rc.8 on three real
# VPSs). CI never noticed because its jobs call `user create-probe`
# explicitly. `create-probe` documents itself as safe for install.sh to call
# unconditionally.
#
# This sources the real install.sh and runs the real acceptance_stage() against
# a mocked `vpn` binary that records every invocation.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

mkdir -p "$work/bin"
cat >"$work/bin/vpn" <<'EOF'
#!/usr/bin/env bash
# Records the sub-command (everything after --config <path>) and can be told to
# fail the create-probe call.
shift 2
echo "$*" >>"$MOCK_VPN_LOG"
case "$*" in
  "user create-probe") exit "${MOCK_PROBE_RC:-0}" ;;
  *) exit 0 ;;
esac
EOF
cat >"$work/bin/vpn-health-check" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
chmod +x "$work/bin/vpn" "$work/bin/vpn-health-check"

# Runs acceptance_stage() for the given role / prior state and prints the
# recorded vpn invocations, one per line, followed by a final RESULT line.
run_acceptance() {
  local role="$1" prior="$2" probe_rc="${3:-0}"
  : >"$work/vpn.log"
  (
    export MOCK_VPN_LOG="$work/vpn.log" MOCK_PROBE_RC="$probe_rc"
    # shellcheck disable=SC1090
    . "$INSTALL_SH"
    # These are read by the sourced installer's functions.
    # shellcheck disable=SC2034
    BIN_DIR="$work/bin"
    # shellcheck disable=SC2034
    DEPLOYMENT_TOML="$work/deployment.toml"
    # shellcheck disable=SC2034
    NODE_ROLE="$role"
    # shellcheck disable=SC2034
    PRIOR_ACCEPTANCE_STATE="$prior"
    stage() { :; }
    log() { :; }
    ensure_first_user() { :; }
    verify_subscription_through_nginx() { :; }
    verify_relay_subscription_fail_closed_through_nginx() { :; }
    die() { echo "DIE: $*" >>"$work/vpn.log"; exit 1; }
    acceptance_stage
  ) >/dev/null 2>&1 && echo "RESULT: accepted" >>"$work/vpn.log" || echo "RESULT: refused" >>"$work/vpn.log"
  cat "$work/vpn.log"
}

line_of() { grep -n -F -- "$1" <<<"$2" | head -1 | cut -d: -f1 || true; }

for scenario in "exit fresh" "relay fresh" "exit accepted"; do
  read -r role prior <<<"$scenario"
  echo
  echo "--- functional: $role node, prior acceptance state '$prior': probe is provisioned before the protocol self-test ---"
  out="$(run_acceptance "$role" "$prior")"
  probe_line="$(line_of 'user create-probe' "$out")"
  doctor_line="$(line_of 'doctor --protocol --require-protocol' "$out")"
  if [ -z "$probe_line" ]; then
    fail "$role/$prior: acceptance never ran 'user create-probe'"
  elif [ -z "$doctor_line" ]; then
    fail "$role/$prior: acceptance never ran the protocol self-test"
  elif [ "$probe_line" -lt "$doctor_line" ]; then
    ok "$role/$prior: 'user create-probe' runs before 'doctor --protocol --require-protocol'"
  else
    fail "$role/$prior: 'user create-probe' ran AFTER the protocol self-test"
  fi
  grep -q 'RESULT: accepted' <<<"$out" \
    && ok "$role/$prior: acceptance completes when both calls succeed" \
    || fail "$role/$prior: acceptance did not complete: $out"
done

echo
echo "--- functional: a failing create-probe stops installation before the protocol self-test ---"
out="$(run_acceptance exit fresh 1)"
if grep -q 'RESULT: refused' <<<"$out" && grep -q '^DIE:' <<<"$out"; then
  ok "acceptance is refused when the probe principal cannot be provisioned"
else
  fail "acceptance was not refused when create-probe failed: $out"
fi
if grep -q 'doctor --protocol' <<<"$out"; then
  fail "the protocol self-test still ran after create-probe failed"
else
  ok "the protocol self-test is not attempted without a probe principal"
fi

echo
if [ "$failures" -ne 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "PASS: install acceptance provisions the probe principal before the protocol self-test"
