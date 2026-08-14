#!/usr/bin/env bash
# Regression test for the REALITY-decoy ordering incident this task was
# filed about: install.sh used to only discover a missing
# REALITY_HANDSHAKE_SERVER deep in stage 10 (render_deployment_toml),
# AFTER packages, sing-box, users, directories and a real Let's Encrypt
# certificate had already been created. Fatal preconditions must now be
# resolved/validated in stage 1 (preflight_stage), before
# packages_stage runs.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

echo "--- static: main() calls preflight_stage before packages_stage ---"
main_body="$(sed -n '/^main() {/,/^}/p' "$INSTALL_SH")"
preflight_line="$(echo "$main_body" | grep -n '^\s*preflight_stage$' | head -n1 | cut -d: -f1)"
packages_line="$(echo "$main_body" | grep -n '^\s*packages_stage$' | head -n1 | cut -d: -f1)"
if [ -n "$preflight_line" ] && [ -n "$packages_line" ] && [ "$preflight_line" -lt "$packages_line" ]; then
  ok "preflight_stage runs before packages_stage"
else
  fail "preflight_stage does not run strictly before packages_stage in main()"
fi

echo
echo "--- static: preflight_stage() resolves REALITY_HANDSHAKE_SERVER and host config before persist_source_tree/packages ---"
preflight_body="$(sed -n '/^preflight_stage() {/,/^}/p' "$INSTALL_SH")"
if echo "$preflight_body" | grep -q 'resolve_reality_handshake_server'; then
  ok "preflight_stage() calls resolve_reality_handshake_server"
else
  fail "preflight_stage() no longer calls resolve_reality_handshake_server — the fatal REALITY precondition may again be discovered too late"
fi
if echo "$preflight_body" | grep -q 'resolve_host_config'; then
  ok "preflight_stage() calls resolve_host_config"
else
  fail "preflight_stage() no longer calls resolve_host_config"
fi

echo
echo "--- static: render_deployment_toml() (stage 10) still validates REALITY_HANDSHAKE_SERVER too (defense in depth) ---"
render_body="$(sed -n '/^render_deployment_toml() {/,/^}/p' "$INSTALL_SH")"
if echo "$render_body" | grep -q 'REALITY_HANDSHAKE_SERVER:?'; then
  ok "render_deployment_toml() still requires REALITY_HANDSHAKE_SERVER (belt-and-suspenders, in case it is ever called directly)"
else
  fail "render_deployment_toml() dropped its own REALITY_HANDSHAKE_SERVER guard"
fi

echo
echo "--- functional: resolve_reality_handshake_server(), sourced from the real install.sh, refuses to proceed with zero host mutation when non-interactive and unset ---"
TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT

run_resolve_reality() {
  (
    set -Eeuo pipefail
    # shellcheck disable=SC2034 # read by install.sh's top-level probe on source
    DEPLOYMENT_TOML="$TMPDIR_TEST/no-such-deployment.toml"
    # shellcheck source=/dev/null
    source "$INSTALL_SH"
    # shellcheck disable=SC2034 # read by resolve_reality_handshake_server
    NONINTERACTIVE=1
    unset REALITY_HANDSHAKE_SERVER
    resolve_reality_handshake_server
  )
}
rc=0
out="$(run_resolve_reality 2>&1)" || rc=$?
if [ "$rc" -ne 0 ]; then
  ok "resolve_reality_handshake_server() fails (rc=$rc) instead of silently proceeding when REALITY_HANDSHAKE_SERVER is unset and non-interactive"
else
  fail "resolve_reality_handshake_server() did NOT fail with REALITY_HANDSHAKE_SERVER unset in non-interactive mode"
fi
if echo "$out" | grep -qi "no universally safe default"; then
  ok "the failure message explains why no default is invented"
else
  fail "the failure message does not explain the no-default rationale; got: $out"
fi
if echo "$out" | grep -qi "reality-handshake-server\|REALITY_HANDSHAKE_SERVER"; then
  ok "the failure message names the flag/env var to fix it"
else
  fail "the failure message does not point at --reality-handshake-server/REALITY_HANDSHAKE_SERVER"
fi

echo
echo "--- functional: resolve_reality_handshake_server() accepts and validates a supplied value, applying punycode normalization ---"
run_resolve_reality_with_value() {
  (
    set -Eeuo pipefail
    # shellcheck disable=SC2034 # read by install.sh's top-level probe on source
    DEPLOYMENT_TOML="$TMPDIR_TEST/no-such-deployment.toml"
    # shellcheck source=/dev/null
    source "$INSTALL_SH"
    # shellcheck disable=SC2034 # read by resolve_reality_handshake_server/resolve_host_config
    NONINTERACTIVE=1
    REALITY_HANDSHAKE_SERVER="www.example.com"
    resolve_reality_handshake_server
    echo "RESOLVED=$REALITY_HANDSHAKE_SERVER"
  )
}
out2="$(run_resolve_reality_with_value 2>&1)" || true
if echo "$out2" | grep -q '^RESOLVED=www.example.com$'; then
  ok "a valid ASCII REALITY_HANDSHAKE_SERVER passes through resolve_reality_handshake_server unchanged"
else
  fail "resolve_reality_handshake_server() did not accept a valid supplied value; got: $out2"
fi

echo
echo "--- functional: resolve_reality_handshake_server() rejects an invalid (shell-metacharacter-bearing) value ---"
run_resolve_reality_invalid() {
  (
    set -Eeuo pipefail
    # shellcheck disable=SC2034 # read by install.sh's top-level probe on source
    DEPLOYMENT_TOML="$TMPDIR_TEST/no-such-deployment.toml"
    # shellcheck source=/dev/null
    source "$INSTALL_SH"
    # shellcheck disable=SC2034 # read by resolve_reality_handshake_server/resolve_host_config
    NONINTERACTIVE=1
    REALITY_HANDSHAKE_SERVER='evil.com; rm -rf /'
    resolve_reality_handshake_server
  )
}
rc3=0
run_resolve_reality_invalid >/dev/null 2>&1 || rc3=$?
[ "$rc3" -ne 0 ] && ok "an invalid REALITY_HANDSHAKE_SERVER value is rejected, not interpolated" || fail "an invalid REALITY_HANDSHAKE_SERVER value was NOT rejected"

echo
echo "--- static: an ERR trap installs automatic rollback (transactional install) ---"
if grep -qE "trap .*on_fatal_error.* ERR" "$INSTALL_SH" && grep -q "^on_fatal_error()" "$INSTALL_SH"; then
  ok "install.sh installs a global ERR trap that triggers automatic rollback on fatal failure"
else
  fail "install.sh no longer installs an ERR-trap-based automatic rollback"
fi
if grep -q 'ownership_is_marked INSTALL_ATTEMPTED' "$INSTALL_SH"; then
  ok "the rollback trap gates on whether any mutation was actually attempted (no-op if preflight itself failed before touching anything)"
else
  fail "the rollback trap no longer gates on INSTALL_ATTEMPTED"
fi

echo
echo "--- static: auto-rollback is NEVER applied to a failed REPAIR of an existing install (must not destroy a live deployment's users/keys/certs over a transient failure) ---"
on_fatal_error_body="$(sed -n '/^on_fatal_error() {/,/^}/p' "$INSTALL_SH")"
if echo "$on_fatal_error_body" | grep -q 'IS_FRESH_INSTALL'; then
  ok "on_fatal_error() checks IS_FRESH_INSTALL before ever running the uninstaller"
else
  fail "on_fatal_error() no longer distinguishes a fresh install from a repair — a failed repair run would auto-uninstall a live, previously-working deployment"
fi
if echo "$on_fatal_error_body" | grep -qE 'IS_FRESH_INSTALL.*-ne 1|IS_FRESH_INSTALL.*!= *1'; then
  ok "on_fatal_error() skips auto-rollback specifically when IS_FRESH_INSTALL != 1 (i.e. on a repair run)"
else
  fail "on_fatal_error()'s IS_FRESH_INSTALL check does not look like the expected 'skip rollback on repair' guard"
fi
if grep -qE '^IS_FRESH_INSTALL=0' "$INSTALL_SH"; then
  ok "IS_FRESH_INSTALL defaults to 0 (repair/fail-safe) at top level, before preflight_stage ever determines the real answer"
else
  fail "IS_FRESH_INSTALL has no fail-safe default — a very early failure (before preflight_stage sets it) could hit an unbound variable or default the wrong way"
fi

echo
echo "--- static: ownership tracking (INSTALL_ATTEMPTED/OPT_VPN1_PRE_EXISTED baseline) is marked BEFORE the first persistent mutation (persist_source_tree/install_idn_support) ---"
preflight_body2="$(sed -n '/^preflight_stage() {/,/^}/p' "$INSTALL_SH")"
mark_line="$(echo "$preflight_body2" | grep -n '^\s*ownership_mark INSTALL_ATTEMPTED\s*$' | head -n1 | cut -d: -f1)"
persist_line="$(echo "$preflight_body2" | grep -n '^\s*persist_source_tree\s*$' | head -n1 | cut -d: -f1)"
idn_line="$(echo "$preflight_body2" | grep -n '^\s*install_idn_support\s*$' | head -n1 | cut -d: -f1)"
if [ -n "$mark_line" ] && [ -n "$persist_line" ] && [ -n "$idn_line" ] \
    && [ "$mark_line" -lt "$persist_line" ] && [ "$mark_line" -lt "$idn_line" ]; then
  ok "ownership_mark INSTALL_ATTEMPTED runs before persist_source_tree and install_idn_support (the first two host mutations) — a failure inside either is now rolled back"
else
  fail "ownership_mark INSTALL_ATTEMPTED does not run before the first host mutations (mark=$mark_line persist=$persist_line idn=$idn_line)"
fi

echo
echo "--- static: the SSH port is resolved in preflight_stage before packages_stage (and thus before firewalld is ever touched) ---"
ssh_resolve_line="$(echo "$preflight_body2" | grep -n '^\s*resolve_ssh_port\s*$' | head -n1 | cut -d: -f1)"
if [ -n "$ssh_resolve_line" ] && [ -n "$mark_line" ] && [ "$ssh_resolve_line" -lt "$mark_line" ]; then
  ok "resolve_ssh_port() runs early in preflight_stage, before any mutation-tracking/mutation begins"
else
  fail "resolve_ssh_port() does not run early enough in preflight_stage (resolve=$ssh_resolve_line mark=$mark_line)"
fi

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all preflight-ordering tests passed"
