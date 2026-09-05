#!/usr/bin/env bash
# Regression tests for persistent-state schema versioning/migration
# wiring (deploy/almalinux/install.sh's check_state_schema(),
# deploy/almalinux/update.sh's equivalent inline block): reinstall/
# update must explicitly report FRESH/REPAIR/UPGRADE/MIGRATION_REQUIRED/
# INVALID and never silently continue on a schema this vpn-admin cannot
# safely interpret. The actual migration LOGIC (backup/validate/atomic
# commit) is covered by crates/compat-config's own Rust tests
# (deployment::tests, store::tests) and apps/admin/tests/cli.rs's
# config_* tests — this file only covers the shell-side wiring: does
# install.sh/update.sh call `vpn-admin config validate`/`config migrate`
# in the right place, and does it react correctly to each exit code.
#
# shellcheck disable=SC2034
# (BIN_DIR/DEPLOYMENT_TOML/IS_FRESH_INSTALL/SINGBOX_VPN_VERSION are read by
# check_state_schema() after `source "$INSTALL_SH"` — shellcheck cannot
# see that dynamic use.)
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"
UPDATE_SH="$REPO_ROOT/deploy/almalinux/update.sh"
STATE_SCHEMA_SH="$REPO_ROOT/deploy/lib/state-schema.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

echo "--- static: main() calls check_state_schema after binaries_stage, before reality_keys_stage ---"
main_body="$(sed -n '/^main() {/,/^}/p' "$INSTALL_SH")"
binaries_line="$(echo "$main_body" | grep -n '^\s*binaries_stage$' | head -n1 | cut -d: -f1)"
schema_line="$(echo "$main_body" | grep -n '^\s*check_state_schema$' | head -n1 | cut -d: -f1)"
reality_line="$(echo "$main_body" | grep -n '^\s*reality_keys_stage$' | head -n1 | cut -d: -f1)"
if [ -n "$binaries_line" ] && [ -n "$schema_line" ] && [ -n "$reality_line" ] \
    && [ "$binaries_line" -lt "$schema_line" ] && [ "$schema_line" -lt "$reality_line" ]; then
  ok "check_state_schema runs strictly between binaries_stage and reality_keys_stage"
else
  fail "check_state_schema is not correctly ordered in main() (binaries=$binaries_line schema=$schema_line reality=$reality_line)"
fi

echo
echo "--- static: shared deploy/lib/state-schema.sh does the actual validate/migrate work ---"
# Phase 7 (deployment modularization) extracted this exact validate/
# migrate/branch block into one shared function -- it used to be
# triplicated near-verbatim across install.sh and update.sh's two
# paths. Check the shared implementation directly rather than each
# call site's now-thin wrapper.
state_schema_body="$(cat "$STATE_SCHEMA_SH")"
for marker in 'MIGRATION REQUIRED' 'config migrate' 'config validate'; do
  if echo "$state_schema_body" | grep -qF "$marker"; then
    ok "deploy/lib/state-schema.sh mentions '$marker'"
  else
    fail "deploy/lib/state-schema.sh does not mention '$marker'"
  fi
done

echo
echo "--- static: check_state_schema() reports every required mode, delegates to the shared function, and fails closed ---"
schema_body="$(sed -n '/^check_state_schema() {/,/^}/p' "$INSTALL_SH")"
for marker in 'FRESH' 'REPAIR' 'UPGRADE'; do
  if echo "$schema_body" | grep -qF "$marker"; then
    ok "check_state_schema() reports '$marker'"
  else
    fail "check_state_schema() does not mention '$marker'"
  fi
done
if echo "$schema_body" | grep -q 'state_schema_validate_and_migrate'; then
  ok "check_state_schema() delegates to the shared state_schema_validate_and_migrate()"
else
  fail "check_state_schema() does not call state_schema_validate_and_migrate -- has the shared extraction been bypassed?"
fi
if echo "$schema_body" | grep -q 'die "persistent state is INVALID'; then
  ok "check_state_schema() dies (non-zero, no further mutation) on an INVALID/unsupported schema"
else
  fail "check_state_schema() does not fail closed on an unsupported schema"
fi
if echo "$schema_body" | grep -q 'die "persistent state migration failed'; then
  ok "check_state_schema() dies on a migration failure (schema_rc=3), not just an invalid schema"
else
  fail "check_state_schema() does not handle a migration failure (schema_rc=3) distinctly"
fi

echo
echo "--- static: install.sh and update.sh both source deploy/lib/state-schema.sh ---"
for f in "$INSTALL_SH" "$UPDATE_SH"; do
  if grep -q 'deploy/lib/state-schema\.sh' "$f"; then
    ok "$(basename "$f") sources deploy/lib/state-schema.sh"
  else
    fail "$(basename "$f") does not source deploy/lib/state-schema.sh"
  fi
done

echo
echo "--- static: update.sh's production path also validates/migrates state and dies closed on an unsupported schema ---"
# Checks the PRODUCTION update path (checkpoint 3) — the one this
# checkpoint's transactional release-to-release updater actually
# introduced. The separate --dev-rebuild/SINGBOX_VPN_CHANNEL=dev escape hatch
# has its own equivalent block (an intentional near-verbatim copy,
# through the same shared state_schema_validate_and_migrate()) and is
# not re-checked here.
update_body="$(sed -n '/^log "install mode: UPDATE/,/^log "rendering current authoritative/p' "$UPDATE_SH")"
if echo "$update_body" | grep -q 'state_schema_validate_and_migrate'; then
  ok "update.sh's production path delegates to the shared state_schema_validate_and_migrate()"
else
  fail "update.sh's production path does not call state_schema_validate_and_migrate"
fi
if echo "$update_body" | grep -q 'die "persistent state is INVALID'; then
  ok "update.sh dies closed on an INVALID/unsupported schema (rollback fires via the existing trap)"
else
  fail "update.sh does not fail closed on an unsupported schema"
fi
if echo "$update_body" | grep -q 'die "persistent state migration failed'; then
  ok "update.sh's production path dies on a migration failure (schema_rc=3) distinctly"
else
  fail "update.sh's production path does not handle a migration failure (schema_rc=3) distinctly"
fi
if echo "$update_body" | grep -q 'lifecycle_gate_abort_hook after_migration'; then
  ok "update.sh's production path still fires the after_migration lifecycle-gate hook only on an actual migration (schema_rc=2)"
else
  fail "update.sh's production path lost its after_migration lifecycle-gate hook"
fi

# The functional checks below source the REAL install.sh (guarded
# against auto-running main() — same pattern as
# test-install-manifest-idempotency.sh) rather than re-extracting
# individual function bodies via sed: check_state_schema() calls the
# real (multi-line) die()/log(), and sourcing the whole file is the only
# way to get those for real instead of re-implementing them a second
# time in this test.
TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT
FAKE_BIN="$TMPDIR_TEST/bin"
mkdir -p "$FAKE_BIN"
CALL_LOG="$TMPDIR_TEST/calls.log"
# Mirrors a real vpn-admin: `validate`'s exit code is controlled by the
# test (0/2/3); `migrate` always succeeds unless FAKE_MIGRATE_EXIT says
# otherwise — a fixed exit code for every subcommand would not exercise
# check_state_schema()'s actual branching (validate MIGRATION_REQUIRED ->
# then call migrate -> then continue).
cat > "$FAKE_BIN/vpn-admin" <<EOF
#!/usr/bin/env bash
echo "\$*" >> "$CALL_LOG"
case " \$* " in
  *" validate "*) exit "\${FAKE_VALIDATE_EXIT:-0}" ;;
  *" migrate "*) exit "\${FAKE_MIGRATE_EXIT:-0}" ;;
esac
exit 0
EOF
chmod +x "$FAKE_BIN/vpn-admin"

echo
echo "--- functional: check_state_schema() fresh install never calls vpn-admin ---"
: > "$CALL_LOG"
(
  # shellcheck disable=SC1090
  source "$INSTALL_SH"
  BIN_DIR="$FAKE_BIN"
  DEPLOYMENT_TOML="$TMPDIR_TEST/does-not-exist.toml"
  IS_FRESH_INSTALL=1
  check_state_schema
)
if [ ! -s "$CALL_LOG" ]; then
  ok "fresh install (IS_FRESH_INSTALL=1) never invokes vpn-admin"
else
  fail "fresh install unexpectedly invoked vpn-admin: $(cat "$CALL_LOG")"
fi

echo
echo "--- functional: check_state_schema() does not abort when deployment.toml is absent (return code bug regression) ---"
# Regression test: check_state_schema() used to end its no-deployment.toml
# early-exit with a bare `[ -f "$DEPLOYMENT_TOML" ] || return`, which
# returns the FAILED test's exit status (1) rather than success — under
# `set -e` in the real install.sh, that silently aborted a legitimate
# no-op (existing install-state.json but no deployment.toml yet) as if
# it were a real failure. Fixed to `return 0` explicitly.
rc_noop=1
(
  # shellcheck disable=SC1090
  source "$INSTALL_SH"
  BIN_DIR="$FAKE_BIN"
  DEPLOYMENT_TOML="$TMPDIR_TEST/does-not-exist.toml"
  IS_FRESH_INSTALL=0
  SINGBOX_VPN_VERSION="v1.2.3"
  check_state_schema
) && rc_noop=0
if [ "$rc_noop" -eq 0 ]; then
  ok "check_state_schema() with IS_FRESH_INSTALL=0 and no deployment.toml yet returns success, not the failed -f test's status"
else
  fail "check_state_schema() incorrectly propagates the failed [-f] test's exit status"
fi

echo
echo "--- functional: check_state_schema() migrates on exit code 2, dies on exit code 3 ---"
: > "$CALL_LOG"
: > "$TMPDIR_TEST/deployment.toml"
rc2=0
(
  # shellcheck disable=SC1090
  source "$INSTALL_SH"
  BIN_DIR="$FAKE_BIN"
  DEPLOYMENT_TOML="$TMPDIR_TEST/deployment.toml"
  IS_FRESH_INSTALL=0
  SINGBOX_VPN_VERSION="v1.2.3"
  export FAKE_VALIDATE_EXIT=2
  check_state_schema
  echo "reached end without dying"
) >"$TMPDIR_TEST/exit2.out" 2>&1 || rc2=$?
if [ "$rc2" -eq 0 ] && grep -q 'config migrate' "$CALL_LOG" && grep -q 'reached end without dying' "$TMPDIR_TEST/exit2.out"; then
  ok "exit code 2 (MIGRATION_REQUIRED) triggers 'config migrate' and does not abort"
else
  fail "exit code 2 handling incorrect (rc=$rc2; see $TMPDIR_TEST/exit2.out, calls: $(cat "$CALL_LOG" 2>/dev/null))"
fi

rc3=0
(
  # shellcheck disable=SC1090
  source "$INSTALL_SH"
  BIN_DIR="$FAKE_BIN"
  DEPLOYMENT_TOML="$TMPDIR_TEST/deployment.toml"
  IS_FRESH_INSTALL=0
  SINGBOX_VPN_VERSION="v1.2.3"
  export FAKE_VALIDATE_EXIT=3
  check_state_schema
) >/tmp/check_state_schema_invalid_test.out 2>&1 || rc3=$?
if [ "$rc3" -ne 0 ]; then
  ok "exit code 3 (INVALID) aborts (die) rather than continuing (see /tmp/check_state_schema_invalid_test.out)"
else
  fail "exit code 3 (INVALID) did not abort"
fi

rc4=0
: > "$CALL_LOG"
(
  # shellcheck disable=SC1090
  source "$INSTALL_SH"
  BIN_DIR="$FAKE_BIN"
  DEPLOYMENT_TOML="$TMPDIR_TEST/deployment.toml"
  IS_FRESH_INSTALL=0
  SINGBOX_VPN_VERSION="v1.2.3"
  export FAKE_VALIDATE_EXIT=2
  export FAKE_MIGRATE_EXIT=1
  check_state_schema
) >/tmp/check_state_schema_migrate_fail_test.out 2>&1 || rc4=$?
if [ "$rc4" -ne 0 ]; then
  ok "a failing 'config migrate' (unexpected internal error) also aborts (die), not silently continues"
else
  fail "a failing config migrate did not abort"
fi

echo
echo "--- static: update.sh rejects an incompatible schema BEFORE SWITCH, using the STAGED (not-yet-installed) target binary ---"
switch_line="$(grep -n '^log "SWITCHING to' "$UPDATE_SH" | head -n1 | cut -d: -f1)"
precheck_line="$(grep -n 'reject an impossible state-schema transition before ANY' "$UPDATE_SH" | head -n1 | cut -d: -f1)"
if [ -n "$precheck_line" ] && [ -n "$switch_line" ] && [ "$precheck_line" -lt "$switch_line" ]; then
  ok "the pre-switch schema compatibility check runs strictly before SWITCH"
else
  fail "the pre-switch schema compatibility check is missing, or does not run before SWITCH (precheck_line=$precheck_line switch_line=$switch_line)"
fi
precheck_body="$(sed -n '/reject an impossible state-schema transition before ANY/,/^fi$/p' "$UPDATE_SH")"
if echo "$precheck_body" | grep -q '"\$STAGED_BIN_DIR/vpn-admin" --config "\$DEPLOYMENT_TOML" config validate'; then
  ok "the pre-switch check validates against the STAGED target vpn-admin, not the currently-installed one"
else
  fail "the pre-switch check does not use \$STAGED_BIN_DIR/vpn-admin -- it would be validating with the OLD binary, proving nothing about the target's compatibility"
fi
if echo "$precheck_body" | grep -q 'Nothing live has been changed'; then
  ok "the pre-switch check's failure message confirms zero live mutation"
else
  fail "the pre-switch check's failure message does not state that nothing live was changed"
fi
if echo "$precheck_body" | grep -qE '0 \| 2\)'; then
  ok "the pre-switch check treats both 'current' (0) and 'migration required' (2) as compatible -- migration itself still happens post-SWITCH with the live binary"
else
  fail "the pre-switch check does not accept the MIGRATION_REQUIRED (2) status as compatible"
fi

if [ "$failures" -eq 0 ]; then
  echo
  echo "all state-schema-migration wiring tests passed"
else
  echo
  echo "$failures test(s) FAILED"
  exit 1
fi
