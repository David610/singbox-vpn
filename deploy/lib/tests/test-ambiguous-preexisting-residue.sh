#!/usr/bin/env bash
# Regression test for the real-VPS rollback-residue incident (Part H):
# after a failed install.sh run auto-rolled back, vpn-expiry-reconcile.
# {service,timer} and vpn-service-watchdog.{service,timer} were left
# behind and had to be removed by hand. Investigation showed this was
# NOT a deletion/ownership bug in uninstall.sh's restore_or_remove_
# fixed_path() (it is deliberately conservative and, by design, leaves a
# fixed path alone when it has no ownership record at all — see
# test-uninstall-ownership-checkpoint2.sh) — the run that failed
# (fetch_release_binaries() dies in stage 4, strictly before stage 6
# "systemd" ever installs those units) could not have created them, so
# they were already-ambiguous residue from an earlier, separate attempt
# on that host, and existing_install_present() alone (which only checks
# for install-state.json) had no way to see that.
#
# check_no_ambiguous_preexisting_residue() (deploy/almalinux/install.sh)
# closes that gap: before treating a host as a fresh install, it refuses
# to proceed silently if a known singbox-vpn fixed path exists with no
# ownership record at all — the exact ambiguous state that leads to this
# residue. This test exercises the REAL function (extracted via
# `declare -f` after sourcing install.sh, with the hardcoded
# /etc/systemd/system prefix substituted for a throwaway fixture
# directory — the same technique test-install-manifest-idempotency.sh
# uses for install.sh functions with no path-override variable), never
# a reimplementation, and never touches a real system path.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT
FIXTURE_SYSTEMD_DIR="$TMPDIR_TEST/etc-systemd-system"
mkdir -p "$FIXTURE_SYSTEMD_DIR"

echo "--- static: check_no_ambiguous_preexisting_residue() is called only on the fresh-install path, never for a recognized repair ---"
preflight_body="$(sed -n '/^preflight_stage() {/,/^}/p' "$INSTALL_SH")"
if echo "$preflight_body" | grep -A2 'else$' | grep -q 'check_no_ambiguous_preexisting_residue'; then
  ok "check_no_ambiguous_preexisting_residue() is called in the 'no existing installation detected' branch"
else
  fail "check_no_ambiguous_preexisting_residue() is not wired into preflight_stage()'s fresh-install branch"
fi
if echo "$preflight_body" | grep -B4 'check_no_ambiguous_preexisting_residue' | grep -q 'if existing_install_present; then'; then
  ok "the call site is gated behind the same existing_install_present() branch structure (never runs for a recognized repair)"
else
  fail "could not confirm check_no_ambiguous_preexisting_residue() is gated on existing_install_present() being false"
fi

echo
echo "--- functional: sourcing install.sh defines the real function and ownership_get() ---"
export OWNERSHIP_DIR="$TMPDIR_TEST/state"
export OWNERSHIP_FILE="$OWNERSHIP_DIR/ownership.env"
# shellcheck disable=SC1090
. "$INSTALL_SH"
if declare -f check_no_ambiguous_preexisting_residue >/dev/null && declare -f ownership_get >/dev/null; then
  ok "sourcing install.sh defines check_no_ambiguous_preexisting_residue() and ownership_get()"
else
  echo "FATAL: required functions not defined after sourcing $INSTALL_SH"
  exit 1
fi

# Re-define the real function with only the hardcoded /etc/systemd/system
# prefix substituted for the fixture directory — same body, same logic,
# a fixture-safe path is the only change.
residue_fn_body="$(declare -f check_no_ambiguous_preexisting_residue | sed "s#/etc/systemd/system#$FIXTURE_SYSTEMD_DIR#g")"
eval "$residue_fn_body"
REPO_ROOT_FOR_MSG="$TMPDIR_TEST/repo"
mkdir -p "$REPO_ROOT_FOR_MSG"
REPO_ROOT="$REPO_ROOT_FOR_MSG"

run_residue_check() {
  (
    die() { echo "DIE:$*"; exit 1; }
    check_no_ambiguous_preexisting_residue
    echo "NO_DIE"
  ) >"$TMPDIR_TEST/out" 2>&1
}

echo
echo "--- functional: a pristine host (no fixture unit files at all) passes silently ---"
rc=0; run_residue_check || rc=$?
if [ "$rc" -eq 0 ] && grep -q NO_DIE "$TMPDIR_TEST/out"; then
  ok "no fixture units present -> no ambiguity, proceeds as fresh"
else
  fail "a pristine host incorrectly triggered die(): rc=$rc out='$(cat "$TMPDIR_TEST/out")'"
fi

echo
echo "--- functional: a known singbox-vpn unit with NO ownership record at all -> refuses to proceed (the exact incident) ---"
rm -f "$OWNERSHIP_FILE"
echo "leftover unit content" > "$FIXTURE_SYSTEMD_DIR/vpn-expiry-reconcile.service"
rc=0; run_residue_check || rc=$?
if [ "$rc" -ne 0 ] && grep -q 'vpn-expiry-reconcile.service' "$TMPDIR_TEST/out" && grep -qi 'ambiguous\|no ownership record' "$TMPDIR_TEST/out"; then
  ok "an unrecognized pre-existing singbox-vpn unit refuses to let the host be treated as pristine, and names the exact path"
else
  fail "did not refuse on ambiguous residue: rc=$rc out='$(cat "$TMPDIR_TEST/out")'"
fi
rm -f "$FIXTURE_SYSTEMD_DIR/vpn-expiry-reconcile.service"

echo
echo "--- functional: the same unit WITH an ownership record (PRE_EXISTED=0, singbox-vpn's own) -> proceeds (known, not ambiguous) ---"
echo "singbox-vpn's own unit content" > "$FIXTURE_SYSTEMD_DIR/vpn-expiry-reconcile.service"
ownership_set "FIXEDPATH_EXPIRY_SVC_UNIT_PRE_EXISTED" "0"
rc=0; run_residue_check || rc=$?
if [ "$rc" -eq 0 ] && grep -q NO_DIE "$TMPDIR_TEST/out"; then
  ok "a unit with a recorded ownership fact (even singbox-vpn's own) is not ambiguous — proceeds"
else
  fail "a known-ownership unit incorrectly blocked a fresh install: rc=$rc out='$(cat "$TMPDIR_TEST/out")'"
fi

echo
echo "--- functional: this never deletes or modifies anything — purely a read-only check ---"
if [ -f "$FIXTURE_SYSTEMD_DIR/vpn-expiry-reconcile.service" ] \
   && [ "$(cat "$FIXTURE_SYSTEMD_DIR/vpn-expiry-reconcile.service")" = "singbox-vpn's own unit content" ]; then
  ok "the fixture unit file was left completely untouched by the check"
else
  fail "the fixture unit file was modified or removed — this check must be read-only"
fi

echo
echo "--- functional: multiple ambiguous units are all named in a single refusal (not just the first) ---"
rm -f "$OWNERSHIP_FILE" "$FIXTURE_SYSTEMD_DIR/vpn-expiry-reconcile.service"
echo x > "$FIXTURE_SYSTEMD_DIR/vpn-expiry-reconcile.service"
echo x > "$FIXTURE_SYSTEMD_DIR/vpn-expiry-reconcile.timer"
echo x > "$FIXTURE_SYSTEMD_DIR/vpn-service-watchdog.service"
echo x > "$FIXTURE_SYSTEMD_DIR/vpn-service-watchdog.timer"
rc=0; run_residue_check || rc=$?
all_named=1
for u in vpn-expiry-reconcile.service vpn-expiry-reconcile.timer vpn-service-watchdog.service vpn-service-watchdog.timer; do
  grep -q "$u" "$TMPDIR_TEST/out" || all_named=0
done
if [ "$rc" -ne 0 ] && [ "$all_named" -eq 1 ]; then
  ok "all 4 real-incident units are named together in a single refusal, not just the first found"
else
  fail "not all ambiguous units were named: rc=$rc all_named=$all_named out='$(cat "$TMPDIR_TEST/out")'"
fi

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all ambiguous-pre-existing-residue tests passed"
