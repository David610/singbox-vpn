#!/usr/bin/env bash
# Regression test for the install-state manifest timing bug: the
# manifest used to be written ONLY inside print_status(), which main()
# only reaches after acceptance_stage() succeeds. An install that got as
# far as "services running and confirmed listening" (start_stage) but
# then failed the separate acceptance test left NO manifest on disk, so
# existing_install_present() (the only thing a re-run checks) reported
# "fresh install" instead of "repair" — re-triggering port-conflict
# checks against ports singbox-vpn itself already legitimately owns.
#
# This test `source`s the REAL deploy/almalinux/install.sh (guarded so
# that sourcing does not invoke main() — see the BASH_SOURCE[0]==$0
# check at the bottom of install.sh) and calls the real
# existing_install_present()/write_install_state_manifest() functions
# directly, with STATE_DIR/DEPLOYMENT_TOML-derived paths overridden to a
# throwaway directory beforehand — no root/systemd required, no real
# dnf/certbot/sing-box calls, and no reimplementation of the functions
# under test. A few ordering/content assertions against the real source
# are also kept, clearly labeled as static/structural checks (they
# cannot, by themselves, prove the functions behave correctly at
# runtime — that's what the functional part below is for).
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"

failures=0

echo "--- static: main() writes the manifest at start_stage, before acceptance_stage ---"
main_body="$(sed -n '/^main() {/,/^}/p' "$INSTALL_SH")"
start_stage_line="$(grep -n 'start_stage' <<<"$main_body" | head -n1 | cut -d: -f1)"
acceptance_stage_line="$(grep -n 'acceptance_stage' <<<"$main_body" | head -n1 | cut -d: -f1)"
print_status_line="$(grep -n 'print_status' <<<"$main_body" | head -n1 | cut -d: -f1)"
if [ -n "$start_stage_line" ] && [ -n "$acceptance_stage_line" ] && [ -n "$print_status_line" ] \
    && [ "$start_stage_line" -lt "$acceptance_stage_line" ] \
    && [ "$acceptance_stage_line" -lt "$print_status_line" ]; then
  echo "ok: main() runs start_stage, then acceptance_stage, then print_status, in that order"
else
  echo "FAIL: main()'s stage ordering is not start_stage < acceptance_stage < print_status"
  failures=$((failures + 1))
fi

echo
echo "--- static: start_stage() writes a 'pending' manifest; print_status() writes 'accepted' ---"
start_stage_body="$(sed -n '/^start_stage() {/,/^}/p' "$INSTALL_SH")"
if grep -q 'write_install_state_manifest "pending"' <<<"$start_stage_body"; then
  echo "ok: start_stage() writes the manifest with acceptance=pending"
else
  echo "FAIL: start_stage() does not write a pending manifest — a failure in acceptance_stage would again leave no manifest at all"
  failures=$((failures + 1))
fi
print_status_body="$(sed -n '/^print_status() {/,/^}/p' "$INSTALL_SH")"
if grep -q 'write_install_state_manifest "accepted"' <<<"$print_status_body"; then
  echo "ok: print_status() upgrades the manifest to acceptance=accepted"
else
  echo "FAIL: print_status() does not record acceptance=accepted"
  failures=$((failures + 1))
fi
# print_status()'s hard success asserts must remain untouched by this change.
if grep -q 'VLESS_REALITY_OK.*-eq 1.*||.*die' <<<"$print_status_body" \
    && grep -q 'HYSTERIA2_OK.*-eq 1.*||.*die' <<<"$print_status_body" \
    && grep -q 'SUBSCRIPTION_BACKEND_OK.*-eq 1.*||.*die' <<<"$print_status_body"; then
  echo "ok: print_status() still hard-asserts VLESS_REALITY_OK/HYSTERIA2_OK/SUBSCRIPTION_BACKEND_OK before claiming success"
else
  echo "FAIL: print_status()'s success hard-asserts appear to have been weakened or removed"
  failures=$((failures + 1))
fi

echo
echo "--- static: sourcing install.sh must not invoke main() ---"
if grep -qE '^if \[\[ "\$\{BASH_SOURCE\[0\]\}" == "\$\{0\}" \]\]; then$' "$INSTALL_SH" \
    && tail -n5 "$INSTALL_SH" | grep -q 'main "\$@"'; then
  echo "ok: install.sh guards its trailing main \"\$@\" call so sourcing (BASH_SOURCE[0] != \$0) skips it"
else
  echo "FAIL: install.sh's trailing main \"\$@\" call is not guarded as expected — sourcing it below may run the production installer"
  failures=$((failures + 1))
fi

echo
echo "--- functional: existing_install_present()/write_install_state_manifest() [real functions, sourced from install.sh] against a throwaway dir ---"
TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT
FAKE_STATE_DIR="$TMPDIR_TEST/var-lib-singbox-vpn"
REAL_STATE_PATH_PREFIX="/var/lib/singbox-vpn"

run_manifest_flow() {
  (
    set -Eeuo pipefail
    # DEPLOYMENT_TOML is pointed at a nonexistent path so the top-level
    # SUBSCRIPTION_BACKEND_PORT awk probe (guarded with `2>/dev/null ||
    # true`) is a safe no-op rather than reading a real host's config.
    # REPO_ROOT is left as the real repo root (harmless — read-only, used
    # for e.g. banner text).
    # shellcheck disable=SC2034 # read by install.sh's top-level probe on source
    DEPLOYMENT_TOML="$TMPDIR_TEST/no-such-deployment.toml"
    # shellcheck source=/dev/null
    source "$INSTALL_SH"

    # existing_install_present()/write_install_state_manifest() hardcode
    # /var/lib/singbox-vpn/install-state.json with no override variable. Rather
    # than duplicating their logic, redefine them using their OWN real
    # bodies (extracted from the actually-sourced functions via `declare
    # -f`) with only the literal /var/lib/singbox-vpn path string substituted
    # for a throwaway directory — every line of actual logic is exactly
    # what was just sourced from install.sh, untouched.
    if ! declare -f write_install_state_manifest >/dev/null || ! declare -f existing_install_present >/dev/null; then
      echo "UNEXPECTED: write_install_state_manifest/existing_install_present were not defined by sourcing $INSTALL_SH"
      exit 1
    fi
    write_body="$(declare -f write_install_state_manifest)"
    write_body="${write_body//$REAL_STATE_PATH_PREFIX/$FAKE_STATE_DIR}"
    eval "$write_body"
    exist_body="$(declare -f existing_install_present)"
    exist_body="${exist_body//$REAL_STATE_PATH_PREFIX/$FAKE_STATE_DIR}"
    eval "$exist_body"

    manifest_path="$FAKE_STATE_DIR/install-state.json"
    if ! declare -f write_install_state_manifest | grep -qF "$manifest_path"; then
      echo "UNEXPECTED: path substitution into write_install_state_manifest()'s real body did not take effect as expected"
      exit 1
    fi

    # write_install_state_manifest() reads PUBLIC_HOST/SUBSCRIPTION_HOST,
    # which are normally only set by resolve_host_config() (part of the
    # real host_config_stage, not exercised by this test). Under `set
    # -u` (inherited from the sourced install.sh) referencing them unset
    # would abort the whole subshell — set harmless test values, matching
    # how a real run would have them populated by the time start_stage
    # calls write_install_state_manifest.
    # shellcheck disable=SC2034 # read by write_install_state_manifest() (eval'd from the real sourced body)
    PUBLIC_HOST="test.example.com"
    # shellcheck disable=SC2034
    SUBSCRIPTION_HOST="test.example.com"
    # Likewise FIREWALL_BACKEND/OS_FAMILY are normally set by
    # detect_os() (preflight_stage) — set harmless test values instead
    # of running detect_os() against this real host.
    # shellcheck disable=SC2034
    FIREWALL_BACKEND="firewalld"
    # shellcheck disable=SC2034
    OS_FAMILY="rhel"
    # Likewise ARCH is normally set by detect_arch() (preflight_stage) —
    # write_install_state_manifest() now also records it (and the
    # matching pinned sing-box checksum) for reproducibility diagnosis.
    # shellcheck disable=SC2034
    ARCH="amd64"

    # 1. Before start_stage-equivalent: no manifest, fresh install.
    if existing_install_present; then
      echo "UNEXPECTED: manifest present before any write"
      exit 1
    fi

    # 2. start_stage-equivalent: services confirmed listening, manifest
    #    written as pending (acceptance_stage has not run yet — this
    #    simulates a partial-failure re-run scenario).
    write_install_state_manifest "pending"
    if ! existing_install_present; then
      echo "UNEXPECTED: existing_install_present() is false right after a pending manifest was written"
      exit 1
    fi
    acceptance_field="$(grep -oE '"acceptance": *"[a-z]+"' "$manifest_path" | grep -oE '"[a-z]+"$' | tr -d '"')"
    [ "$acceptance_field" = "pending" ] || { echo "UNEXPECTED: acceptance field is '$acceptance_field', expected 'pending'"; exit 1; }
    echo "PENDING_OK"

    # 2b. Release identity: the manifest records the SHA-256 of the executables
    #     actually installed in $BIN_DIR (what `vpn-admin doctor` compares), and
    #     leaves the fields empty — never invented — when they do not exist yet.
    [ -z "$(grep -oE '"vpn_admin_sha256": *"[0-9a-f]+"' "$manifest_path")" ] \
      || { echo "UNEXPECTED: a hash was recorded although no vpn-admin was installed"; exit 1; }
    BIN_DIR="$TMPDIR_TEST/fakebin"
    mkdir -p "$BIN_DIR"
    printf 'fake vpn-admin build\n' >"$BIN_DIR/vpn-admin"
    printf 'fake subscription build\n' >"$BIN_DIR/vpn-subscription-svc"
    write_install_state_manifest "pending"
    want_admin="$(sha256sum "$BIN_DIR/vpn-admin" | awk '{print $1}')"
    want_sub="$(sha256sum "$BIN_DIR/vpn-subscription-svc" | awk '{print $1}')"
    grep -q "\"vpn_admin_sha256\": \"$want_admin\"" "$manifest_path" \
      || { echo "UNEXPECTED: vpn_admin_sha256 missing/wrong in the manifest"; exit 1; }
    grep -q "\"subscription_sha256\": \"$want_sub\"" "$manifest_path" \
      || { echo "UNEXPECTED: subscription_sha256 missing/wrong in the manifest"; exit 1; }
    echo "IDENTITY_OK"

    # 3. print_status-equivalent: acceptance_stage succeeded, manifest
    #    upgraded to accepted. Re-running existing_install_present()
    #    must still report an existing install (a repair run must never
    #    re-run fresh-install-only logic just because acceptance status
    #    changed).
    write_install_state_manifest "accepted"
    if ! existing_install_present; then
      echo "UNEXPECTED: existing_install_present() is false after upgrading to accepted"
      exit 1
    fi
    acceptance_field="$(grep -oE '"acceptance": *"[a-z]+"' "$manifest_path" | grep -oE '"[a-z]+"$' | tr -d '"')"
    [ "$acceptance_field" = "accepted" ] || { echo "UNEXPECTED: acceptance field is '$acceptance_field', expected 'accepted'"; exit 1; }
    echo "ACCEPTED_OK"

    # 4. write_install_state_manifest() called with no argument must
    #    fail loudly (bash's ${1:?...}), not silently write a manifest
    #    with an empty/undefined acceptance field. `${1:?...}` expansion
    #    failures are fatal to the whole (sub)shell immediately,
    #    bypassing normal `if CMD; then` protection — so this is
    #    deliberately run in its OWN nested subshell to observe that
    #    exit status without also killing this outer one.
    noarg_rc=0
    ( write_install_state_manifest ) 2>/dev/null || noarg_rc=$?
    if [ "$noarg_rc" -eq 0 ]; then
      echo "UNEXPECTED: write_install_state_manifest with no argument did not fail"
      exit 1
    fi
    echo "NOARG_REJECTED_OK"
  )
}

out="$(run_manifest_flow)" || true
if grep -q '^PENDING_OK$' <<<"$out"; then
  echo "ok: a manifest written after start_stage (acceptance=pending) makes existing_install_present() true"
else
  echo "FAIL: pending-manifest / existing_install_present() interaction is broken"
  echo "$out"
  failures=$((failures + 1))
fi
if grep -q '^IDENTITY_OK$' <<<"$out"; then
  echo "ok: the manifest records the SHA-256 of the installed executables (empty, not invented, before they exist)"
else
  echo "FAIL: release-identity hashes are not recorded in the install-state manifest"
  echo "$out"
  failures=$((failures + 1))
fi
update_commit="$(sed -n '/^# COMMIT/,/^committed=1/p' "$REPO_ROOT/deploy/almalinux/update.sh")"
if grep -q '"vpn_admin_sha256": "\$vpn_admin_sha256"' <<<"$update_commit" \
    && grep -q '"subscription_sha256": "\$subscription_sha256"' <<<"$update_commit" \
    && grep -q 'sha256sum "\$BIN_DIR/vpn-admin"' <<<"$update_commit"; then
  echo "ok: update.sh's COMMIT step records the identity of the executables it just installed"
else
  echo "FAIL: update.sh does not record executable hashes in the manifest it commits"
  failures=$((failures + 1))
fi
pending_runs="$(grep -c 'SINGBOX_VPN_RELEASE_IDENTITY_PENDING=1 "\$BIN_DIR/vpn" --config "\$DEPLOYMENT_TOML" doctor' "$REPO_ROOT/deploy/almalinux/update.sh")"
if [ "$pending_runs" = 2 ] \
    && [ "$(grep -c 'doctor --protocol --require-protocol' "$REPO_ROOT/deploy/almalinux/update.sh")" = "$pending_runs" ]; then
  echo "ok: every pre-commit verification doctor run in update.sh is marked update-in-progress"
else
  echo "FAIL: a pre-commit doctor run in update.sh is not marked SINGBOX_VPN_RELEASE_IDENTITY_PENDING=1 (would report a false inconsistency)"
  failures=$((failures + 1))
fi
if grep -q '^ACCEPTED_OK$' <<<"$out"; then
  echo "ok: upgrading the manifest to acceptance=accepted still keeps existing_install_present() true (a repair run stays a repair run)"
else
  echo "FAIL: accepted-manifest / existing_install_present() interaction is broken"
  echo "$out"
  failures=$((failures + 1))
fi
if grep -q '^NOARG_REJECTED_OK$' <<<"$out"; then
  echo "ok: write_install_state_manifest() with no acceptance argument fails loudly instead of writing an incomplete manifest"
else
  echo "FAIL: write_install_state_manifest() with no argument did not fail as expected"
  echo "$out"
  failures=$((failures + 1))
fi

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all tests passed"
