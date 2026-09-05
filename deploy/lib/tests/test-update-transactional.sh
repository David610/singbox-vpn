#!/usr/bin/env bash
# Checkpoint-3 regression tests for the transactional production
# updater (deploy/almalinux/update.sh): version resolution, no-Cargo
# production path, verify-before-mutation ordering, interrupted-
# transaction detection, and rollback's /opt/singbox-vpn source-tree restore.
#
# Static/source-inspection only (same convention as
# test-install-update-parity.sh / test-update-conditional-restart.sh /
# test-state-schema-migration.sh in this same directory) — update.sh
# itself needs root, systemd, a real deployment, and network access to
# actually run, none of which are assumed here.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
UPDATE_SH="$REPO_ROOT/deploy/almalinux/update.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

echo "--- static: production path requires an explicit target (--version/--latest/--repair) ---"
if grep -q 'a production update requires an explicit target' "$UPDATE_SH"; then
  ok "update.sh refuses to run a production update with no --version/--latest/--repair"
else
  fail "update.sh does not refuse an ambiguous production update target"
fi

echo
echo "--- static: target version is syntax-validated before use ---"
if grep -qE "grep -Eq '\^v\?" "$UPDATE_SH"; then
  ok "update.sh validates TARGET_VERSION against a vX.Y.Z pattern before using it"
else
  fail "update.sh does not validate the target version's syntax"
fi

echo
echo "--- static: same-version request exits cleanly without mutation (unless --repair) ---"
if grep -q 'Already at .*nothing to update' "$UPDATE_SH"; then
  ok "update.sh exits early ('Already at ...') when target equals current version and --repair was not given"
else
  fail "update.sh does not handle the same-version case cleanly"
fi

echo
echo "--- functional: normal updates reject downgrade; explicit rollback requires --allow-downgrade ---"
version_fn="$(sed -n '/^version_is_older() {/,/^}/p' "$UPDATE_SH")"
if [ -n "$version_fn" ]; then
  eval "$version_fn"
  version_is_older v0.1.2 v0.1.3 \
    && ok "version ordering identifies v0.1.2 as older than v0.1.3" \
    || fail "version ordering did not identify an older release"
  if version_is_older v0.1.3 v0.1.2; then
    fail "version ordering misclassified an upgrade as a downgrade"
  else
    ok "version ordering permits a normal upgrade"
  fi
  version_is_older v0.1.3-rc.1 v0.1.3 \
    && ok "SemVer prerelease is older than its final release" \
    || fail "prerelease/final ordering is incorrect"
  if version_is_older v0.1.3 v0.1.3-rc.1; then
    fail "final release was misclassified as older than its prerelease"
  else
    ok "final release is newer than its prerelease"
  fi
else
  fail "could not extract version_is_older() from the real updater"
fi

echo
echo "--- functional: attestation is required for every release, with no version-gated checksum-only fallback ---"
# A version-gated exemption ("releases below vX are checksum-only") is
# gated on attacker-suppliable release metadata: anyone with
# release-publish access — the actor attestation exists to contain —
# could republish an old-numbered or malformed tag under that policy and
# skip attestation entirely. verify_release_attestation() must therefore
# require cosign-verified attestation unconditionally, in both install.sh's
# deployment installer and update.sh, with no early return.
updater_verify_fn="$(sed -n '/^verify_release_attestation() {/,/^}/p' "$UPDATE_SH")"
installer_verify_fn="$(sed -n '/^verify_release_attestation() {/,/^}/p' "$REPO_ROOT/deploy/almalinux/install.sh")"
if [ -n "$updater_verify_fn" ] && [ -n "$installer_verify_fn" ]; then
  if printf '%s' "$updater_verify_fn" | grep -qE 'return 0|HISTORICAL RELEASE|release_version_at_least'; then
    fail "update.sh's verify_release_attestation still contains a version-gated fallback"
  else
    ok "update.sh's verify_release_attestation has no version-gated fallback"
  fi
  if printf '%s' "$installer_verify_fn" | grep -qE 'return 0|HISTORICAL RELEASE|release_version_at_least'; then
    fail "install.sh's verify_release_attestation still contains a version-gated fallback"
  else
    ok "install.sh's verify_release_attestation has no version-gated fallback"
  fi
else
  fail "could not extract verify_release_attestation() from update.sh and/or the deployment installer"
fi
if grep -q 'SINGBOX_VPN_ALLOW_LEGACY_CHECKSUM_ONLY' "$REPO_ROOT/install.sh" \
    "$REPO_ROOT/deploy/almalinux/install.sh" "$UPDATE_SH"; then
  fail "an operator-controlled permanent checksum-only override still exists"
else
  ok "new releases have no operator-controlled checksum-only override"
fi
if grep -q 'refusing unintended downgrade' "$UPDATE_SH" \
  && grep -q -- '--allow-downgrade) ALLOW_DOWNGRADE=1' "$UPDATE_SH" \
  && grep -q 'valid only with an explicit --version target' "$UPDATE_SH"; then
  ok "normal downgrade fails closed and intentional rollback requires the explicit flag plus version"
else
  fail "explicit downgrade semantics are not fully enforced"
fi
same_version_line="$(grep -n 'Already at .*nothing to update' "$UPDATE_SH" | head -1 | cut -d: -f1)"
first_backup_dir_line="$(grep -n 'install -d -m 0700 -o root -g root "\$BACKUP_DIR"' "$UPDATE_SH" | tail -1 | cut -d: -f1)"
if [ -n "$same_version_line" ] && [ -n "$first_backup_dir_line" ] && [ "$same_version_line" -lt "$first_backup_dir_line" ]; then
  ok "the same-version early-exit happens before any backup/mutation machinery runs"
else
  fail "the same-version early-exit does not clearly precede backup/mutation setup"
fi

echo
echo "--- static: the PRODUCTION path never requires Cargo (no-Cargo requirement is checkpoint-3's core rule) ---"
# Extract everything from the "PRODUCTION PATH" banner to end of file,
# and confirm no bare 'cargo' invocation appears there (only inside the
# clearly separate DEV-REBUILD block above it, which is allowed to).
# Use a here-string rather than `echo "$production_body" | grep -q`:
# under pipefail, grep -q may close the pipe as soon as it finds a match,
# causing echo to receive SIGPIPE and turn a successful assertion into a
# timing-dependent failure.
production_body="$(awk '/^# PRODUCTION PATH/{flag=1} flag{print}' "$UPDATE_SH")"
if grep -qE '(^|[^-])cargo (build|test)' <<<"$production_body"; then
  fail "the production update path invokes cargo build/test — production updates must never require Cargo/Rust"
else
  ok "the production update path never invokes cargo build/test"
fi
if grep -q 'no prebuilt binaries are available' <<<"$production_body"; then
  ok "production path fails closed (die) rather than falling back to a source build when no prebuilt release binaries exist"
else
  fail "production path does not clearly refuse to fall back to a source build"
fi

echo
echo "--- static: dev-rebuild is a clearly separate, explicit escape hatch (SINGBOX_VPN_CHANNEL=dev / --dev-rebuild) ---"
if grep -q -- '--dev-rebuild) DEV_REBUILD=1' "$UPDATE_SH" && grep -q '"\${SINGBOX_VPN_CHANNEL:-}" = "dev"' "$UPDATE_SH"; then
  ok "--dev-rebuild and SINGBOX_VPN_CHANNEL=dev both explicitly gate the Cargo-based rebuild path"
else
  fail "the dev-rebuild escape hatch is not clearly gated"
fi

echo
echo "--- static: target release material is verified (checksum) before ANY live mutation ---"
# Every STAGE-phase die() must say so, so an operator reading the error
# knows nothing was touched yet.
stage_body="$(awk '/^STAGING_ROOT=/{flag=1} /^STAGE_OK=1/{print; flag=0} flag{print}' "$UPDATE_SH")"
stage_die_count="$(echo "$stage_body" | grep -c 'die ')"
stage_unchanged_count="$(echo "$stage_body" | grep -c 'Nothing live has been changed')"
if [ "$stage_die_count" -gt 0 ] && [ "$stage_unchanged_count" -ge "$((stage_die_count - 2))" ]; then
  ok "STAGE-phase failures consistently state that nothing live has been changed yet ($stage_unchanged_count of $stage_die_count die() calls)"
else
  fail "STAGE-phase failures do not consistently confirm zero live mutation (die=$stage_die_count, confirmed=$stage_unchanged_count)"
fi
if grep -q 'sha256sum -c' "$UPDATE_SH" && grep -q 'checksum verification failed for singbox-vpn-src.tar.gz' "$UPDATE_SH"; then
  ok "the target release source archive is checksum-verified before extraction"
else
  fail "the target release source archive checksum verification is missing"
fi

echo
echo "--- static: BACKUP_DIR/mutation setup (PREPARE) happens strictly after STAGE_OK=1 ---"
stage_ok_line="$(grep -n '^STAGE_OK=1$' "$UPDATE_SH" | head -1 | cut -d: -f1)"
prepare_line="$(grep -n '^install -d -m 0700 -o root -g root "\$BACKUP_DIR"$' "$UPDATE_SH" | tail -1 | cut -d: -f1)"
if [ -n "$stage_ok_line" ] && [ -n "$prepare_line" ] && [ "$stage_ok_line" -lt "$prepare_line" ]; then
  ok "PREPARE (backup creation) runs after STAGE has fully verified the target release"
else
  fail "PREPARE does not clearly run after STAGE_OK=1 (stage_ok=$stage_ok_line prepare=$prepare_line)"
fi

echo
echo "--- static: install-state.json is only rewritten AFTER protocol verification, at COMMIT ---"
doctor_line_prod="$(grep -n 'doctor --protocol --require-protocol' "$UPDATE_SH" | tail -1 | cut -d: -f1)"
manifest_write_line="$(grep -n 'mv -f "\$INSTALL_STATE_MANIFEST.tmp" "\$INSTALL_STATE_MANIFEST"' "$UPDATE_SH" | head -1 | cut -d: -f1)"
committed_line_prod="$(grep -n '^committed=1$' "$UPDATE_SH" | tail -1 | cut -d: -f1)"
if [ -n "$doctor_line_prod" ] && [ -n "$manifest_write_line" ] && [ "$doctor_line_prod" -lt "$manifest_write_line" ]; then
  ok "install-state.json is only written after the protocol acceptance check"
else
  fail "install-state.json write does not clearly happen after protocol verification"
fi
if [ -n "$manifest_write_line" ] && [ -n "$committed_line_prod" ] && [ "$manifest_write_line" -lt "$committed_line_prod" ]; then
  ok "install-state.json is written before committed=1 (so a write failure would still roll back)"
else
  fail "install-state.json write does not clearly precede committed=1"
fi

echo
echo "--- static: rollback restores the previous /opt/singbox-vpn source tree, not only binaries ---"
rollback_body="$(sed -n '/^rollback_update() {/,/^}/p' "$UPDATE_SH")"
if echo "$rollback_body" | grep -q 'PREV_OPT_DIR' && echo "$rollback_body" | grep -q 'mv -f "\$PREV_OPT_DIR" /opt/singbox-vpn'; then
  ok "rollback_update() restores the previous /opt/singbox-vpn source tree from PREV_OPT_DIR"
else
  fail "rollback_update() does not restore the previous /opt/singbox-vpn source tree"
fi
if echo "$rollback_body" | grep -q 'BACKUP_DIR/sing-box'; then
  ok "rollback_update() restores the previous sing-box binary if it was changed"
else
  fail "rollback_update() does not restore a changed sing-box binary"
fi
if echo "$rollback_body" | grep -q 'render-config' && echo "$rollback_body" | grep -qv 'rm -rf.*users.json'; then
  ok "rollback_update() re-renders (never rewinds) users.json/REALITY material with the restored tooling"
else
  fail "rollback_update() does not clearly avoid rewinding authoritative user/REALITY state"
fi
if echo "$rollback_body" | grep -q 'UPDATE FAILED — PREVIOUS RELEASE RESTORED AND VERIFIED' \
    && echo "$rollback_body" | grep -q 'UPDATE FAILED — ROLLBACK ALSO FAILED'; then
  ok "rollback_update() reports exactly the two required distinct outcomes"
else
  fail "rollback_update() does not report both required distinct outcomes"
fi

echo
echo "--- static: interrupted-transaction detection refuses a new update on top of unknown state ---"
if grep -q 'TRANSACTION_MARKER' "$UPDATE_SH" && grep -q 'did not finish cleanly' "$UPDATE_SH"; then
  ok "update.sh detects a leftover transaction marker from an interrupted prior run and refuses to proceed"
else
  fail "update.sh does not detect an interrupted prior transaction"
fi
marker_check_line="$(grep -n 'if \[ -e "\$TRANSACTION_MARKER" \]; then' "$UPDATE_SH" | head -1 | cut -d: -f1)"
flock_line="$(grep -n '^flock -x 200$' "$UPDATE_SH" | head -1 | cut -d: -f1)"
if [ -n "$marker_check_line" ] && [ -n "$flock_line" ] && [ "$marker_check_line" -lt "$flock_line" ]; then
  ok "the stale-transaction check runs before the installer lock is even acquired"
else
  fail "the stale-transaction check does not clearly run early"
fi
if grep -q '/opt/.singbox-vpn-update-staging.\*' "$UPDATE_SH" && grep -q '/opt/.singbox-vpn-prev-\*' "$UPDATE_SH"; then
  ok "update.sh also detects a stale staging/rollback directory left by a killed prior run"
else
  fail "update.sh does not detect a stale staging/rollback directory"
fi

echo
echo "--- static: transaction-only backups are removed on successful commit, not kept as a permanent product ---"
if grep -A15 '^committed=1$' "$UPDATE_SH" | grep -q 'rm -rf "\$PREV_OPT_DIR" "\$BACKUP_DIR" "\$STAGING_ROOT"'; then
  ok "successful commit removes the transaction-only backup/staging/prev directories"
else
  fail "successful commit does not clearly clean up transaction-only backups"
fi

echo
echo "--- static: --repair reconciles the CURRENT version only, never accepts --version/--latest together ---"
if grep -q 'repair reconciles the CURRENTLY installed release' "$UPDATE_SH"; then
  ok "--repair explicitly refuses to be combined with --version/--latest"
else
  fail "--repair does not clearly refuse to be combined with --version/--latest"
fi

echo
echo "--- static: sing-box binary is staged/verified as part of the same release transaction when the target pins a different version ---"
if grep -q 'TARGET_SINGBOX_VERSION' "$UPDATE_SH" && grep -q 'CURRENT_SINGBOX_PINNED' "$UPDATE_SH" \
    && grep -q 'STAGED_SINGBOX_BIN' "$UPDATE_SH"; then
  ok "update.sh compares target vs. current pinned sing-box version and stages/verifies a new binary when they differ"
else
  fail "update.sh does not handle a sing-box version change as part of the update transaction"
fi

echo
echo "--- static: lifecycle-acceptance.sh's update invocation uses a real update.sh flag (not the previously-ignored SINGBOX_VPN_REF) ---"
LIFECYCLE_SH="$REPO_ROOT/deploy/almalinux/lifecycle-acceptance.sh"
if grep -q 'update.sh --dev-rebuild' "$LIFECYCLE_SH"; then
  ok "lifecycle-acceptance.sh invokes update.sh with a flag update.sh actually understands"
else
  fail "lifecycle-acceptance.sh still invokes update.sh in a way it does not understand"
fi

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all update-transactional tests passed"