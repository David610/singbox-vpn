#!/usr/bin/env bash
# Regression test for the install-state manifest timing bug: the
# manifest used to be written ONLY inside print_status(), which main()
# only reaches after acceptance_stage() succeeds. An install that got as
# far as "services running and confirmed listening" (start_stage) but
# then failed the separate acceptance test left NO manifest on disk, so
# existing_install_present() (the only thing a re-run checks) reported
# "fresh install" instead of "repair" — re-triggering port-conflict
# checks against ports vpn1 itself already legitimately owns.
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
start_stage_line="$(echo "$main_body" | grep -n 'start_stage' | head -n1 | cut -d: -f1)"
acceptance_stage_line="$(echo "$main_body" | grep -n 'acceptance_stage' | head -n1 | cut -d: -f1)"
print_status_line="$(echo "$main_body" | grep -n 'print_status' | head -n1 | cut -d: -f1)"
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
if echo "$start_stage_body" | grep -q 'write_install_state_manifest "pending"'; then
  echo "ok: start_stage() writes the manifest with acceptance=pending"
else
  echo "FAIL: start_stage() does not write a pending manifest — a failure in acceptance_stage would again leave no manifest at all"
  failures=$((failures + 1))
fi
print_status_body="$(sed -n '/^print_status() {/,/^}/p' "$INSTALL_SH")"
if echo "$print_status_body" | grep -q 'write_install_state_manifest "accepted"'; then
  echo "ok: print_status() upgrades the manifest to acceptance=accepted"
else
  echo "FAIL: print_status() does not record acceptance=accepted"
  failures=$((failures + 1))
fi
# print_status()'s hard success asserts must remain untouched by this change.
if echo "$print_status_body" | grep -q 'VLESS_REALITY_OK.*-eq 1.*||.*die' \
    && echo "$print_status_body" | grep -q 'HYSTERIA2_OK.*-eq 1.*||.*die' \
    && echo "$print_status_body" | grep -q 'SUBSCRIPTION_BACKEND_OK.*-eq 1.*||.*die'; then
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
FAKE_STATE_DIR="$TMPDIR_TEST/var-lib-vpn1"
REAL_STATE_PATH_PREFIX="/var/lib/vpn1"

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
    # /var/lib/vpn1/install-state.json with no override variable. Rather
    # than duplicating their logic, redefine them using their OWN real
    # bodies (extracted from the actually-sourced functions via `declare
    # -f`) with only the literal /var/lib/vpn1 path string substituted
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
if echo "$out" | grep -q '^PENDING_OK$'; then
  echo "ok: a manifest written after start_stage (acceptance=pending) makes existing_install_present() true"
else
  echo "FAIL: pending-manifest / existing_install_present() interaction is broken"
  echo "$out"
  failures=$((failures + 1))
fi
if echo "$out" | grep -q '^ACCEPTED_OK$'; then
  echo "ok: upgrading the manifest to acceptance=accepted still keeps existing_install_present() true (a repair run stays a repair run)"
else
  echo "FAIL: accepted-manifest / existing_install_present() interaction is broken"
  echo "$out"
  failures=$((failures + 1))
fi
if echo "$out" | grep -q '^NOARG_REJECTED_OK$'; then
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
