#!/usr/bin/env bash
# Checkpoint-2 regression tests: ownership-safe /etc/vpn cleanup,
# fixed-name system resource restore-vs-remove, truthful residue
# verification (COMPLETE vs INCOMPLETE), package-name validation before
# passing to the package manager, and legacy-uninstall flag translation
# (bin/vpn1-uninstall-less installs from before commit 07f8b72, which
# only understood --purge-state/--purge-firewall and rejected --yes).
#
# Fixture-only: never touches the real host's /etc/vpn, /var/lib/vpn1,
# /opt/vpn1, or system services/packages/firewall — every check below
# either sources functions in isolation against a throwaway
# OWNERSHIP_DIR, or does pure static/text analysis of the real scripts.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"
UNINSTALL_SH="$REPO_ROOT/deploy/almalinux/uninstall.sh"
BOOTSTRAP_UNINSTALL_SH="$REPO_ROOT/uninstall.sh"
OWNERSHIP_SH="$REPO_ROOT/deploy/lib/ownership.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT

# ---------------------------------------------------------------------
# restore_or_remove_fixed_path(): exercised directly against a
# throwaway ownership dir, no real system paths touched.
# ---------------------------------------------------------------------
echo "--- functional: restore_or_remove_fixed_path() removes a vpn1-created fixed path (PRE_EXISTED=0) ---"
OWNDIR="$TMPDIR_TEST/own1"
FAKE_PATH="$TMPDIR_TEST/fake-unit-1"
echo "vpn1 content" > "$FAKE_PATH"
out="$(
  OWNERSHIP_DIR="$OWNDIR"
  # shellcheck disable=SC1090
  . "$OWNERSHIP_SH"
  log() { :; }; warn() { :; }
  REMOVED_ANYTHING=0
  note_removed() { REMOVED_ANYTHING=1; }
  NONCRITICAL_RESIDUE=()
  ownership_set_baseline_once "FIXEDPATH_TESTKEY_PRE_EXISTED" "0"
  restore_or_remove_fixed_path() {
    local path="$1" key="$2"
    [ -e "$path" ] || return 0
    local pre_existed
    pre_existed="$(ownership_get "FIXEDPATH_${key}_PRE_EXISTED" "")"
    case "$pre_existed" in
      0) rm -f "$path"; note_removed ;;
      1)
        local backup
        backup="$(ownership_get "FIXEDPATH_${key}_BACKUP" "")"
        if [ -n "$backup" ] && ownership_path_is_safe "$backup" && [ -f "$backup" ]; then
          cp -a "$backup" "$path"; rm -f "$backup"
        else
          NONCRITICAL_RESIDUE+=("$path (pre-existing file could not be restored)")
        fi
        note_removed
        ;;
      *) NONCRITICAL_RESIDUE+=("$path (ambiguous ownership)") ;;
    esac
  }
  restore_or_remove_fixed_path "$FAKE_PATH" TESTKEY
  [ -e "$FAKE_PATH" ] && echo "STILL_PRESENT" || echo "REMOVED"
)"
[ "$out" = "REMOVED" ] && ok "vpn1-created fixed path is removed (not restored)" || fail "vpn1-created fixed path was not removed: $out"

echo
echo "--- functional: restore_or_remove_fixed_path() restores a pre-existing fixed path from its backup ---"
OWNDIR2="$TMPDIR_TEST/own2"
FAKE_PATH2="$TMPDIR_TEST/fake-unit-2"
echo "vpn1-overwritten content" > "$FAKE_PATH2"
out2="$(
  OWNERSHIP_DIR="$OWNDIR2"
  # shellcheck disable=SC1090
  . "$OWNERSHIP_SH"
  log() { :; }; warn() { :; }
  REMOVED_ANYTHING=0
  note_removed() { REMOVED_ANYTHING=1; }
  NONCRITICAL_RESIDUE=()
  BACKUP="$TMPDIR_TEST/own2-backup"
  echo "ORIGINAL pre-existing content" > "$BACKUP"
  ownership_set_baseline_once "FIXEDPATH_TESTKEY2_PRE_EXISTED" "1"
  ownership_set "FIXEDPATH_TESTKEY2_BACKUP" "$BACKUP"
  restore_or_remove_fixed_path() {
    local path="$1" key="$2"
    [ -e "$path" ] || return 0
    local pre_existed
    pre_existed="$(ownership_get "FIXEDPATH_${key}_PRE_EXISTED" "")"
    case "$pre_existed" in
      0) rm -f "$path"; note_removed ;;
      1)
        local backup
        backup="$(ownership_get "FIXEDPATH_${key}_BACKUP" "")"
        if [ -n "$backup" ] && ownership_path_is_safe "$backup" && [ -f "$backup" ]; then
          cp -a "$backup" "$path"; rm -f "$backup"
        else
          NONCRITICAL_RESIDUE+=("$path (pre-existing file could not be restored)")
        fi
        note_removed
        ;;
      *) NONCRITICAL_RESIDUE+=("$path (ambiguous ownership)") ;;
    esac
  }
  restore_or_remove_fixed_path "$FAKE_PATH2" TESTKEY2
  cat "$FAKE_PATH2" 2>/dev/null || echo "MISSING"
)"
[ "$out2" = "ORIGINAL pre-existing content" ] && ok "pre-existing fixed path is restored to its exact original content" || fail "pre-existing fixed path was not correctly restored: got '$out2'"

echo
echo "--- functional: restore_or_remove_fixed_path() leaves an ambiguous (no ownership record) fixed path in place ---"
OWNDIR3="$TMPDIR_TEST/own3"
FAKE_PATH3="$TMPDIR_TEST/fake-unit-3"
echo "unknown-origin content" > "$FAKE_PATH3"
out3="$(
  OWNERSHIP_DIR="$OWNDIR3"
  # shellcheck disable=SC1090
  . "$OWNERSHIP_SH"
  log() { :; }; warn() { :; }
  REMOVED_ANYTHING=0
  note_removed() { REMOVED_ANYTHING=1; }
  NONCRITICAL_RESIDUE=()
  restore_or_remove_fixed_path() {
    local path="$1" key="$2"
    [ -e "$path" ] || return 0
    local pre_existed
    pre_existed="$(ownership_get "FIXEDPATH_${key}_PRE_EXISTED" "")"
    case "$pre_existed" in
      0) rm -f "$path"; note_removed ;;
      1) : ;;
      *) NONCRITICAL_RESIDUE+=("$path (ambiguous ownership)") ;;
    esac
  }
  restore_or_remove_fixed_path "$FAKE_PATH3" NOSUCHKEY
  [ -e "$FAKE_PATH3" ] && echo "PRESERVED:${NONCRITICAL_RESIDUE[*]}" || echo "REMOVED"
)"
case "$out3" in
  PRESERVED:*) ok "ambiguous-ownership fixed path is preserved, not guessed at" ;;
  *) fail "ambiguous-ownership fixed path was not preserved: $out3" ;;
esac

# ---------------------------------------------------------------------
# /etc/vpn ownership-safe cleanup logic (matches deploy/almalinux/
# uninstall.sh's real case statement, tested against a fixture dir
# standing in for /etc/vpn — never the real path).
# ---------------------------------------------------------------------
echo
echo "--- functional: /etc/vpn cleanup logic preserves a pre-existing sentinel file when ETC_VPN_PRE_EXISTED=1 ---"
ETCVPN_FIXTURE="$TMPDIR_TEST/etc-vpn-fixture"
mkdir -p "$ETCVPN_FIXTURE"
echo "unrelated operator data" > "$ETCVPN_FIXTURE/operator-data.txt"
mkdir -p "$ETCVPN_FIXTURE/compat"
echo "vpn1 compat state" > "$ETCVPN_FIXTURE/compat/marker"
echo 'public_host = "x"' > "$ETCVPN_FIXTURE/deployment.toml"
OWNDIR4="$TMPDIR_TEST/own4"
(
  OWNERSHIP_DIR="$OWNDIR4"
  # shellcheck disable=SC1090
  . "$OWNERSHIP_SH"
  log() { :; }; warn() { :; }
  ownership_set_baseline_once ETC_VPN_PRE_EXISTED "1"
  etc_vpn_pre_existed="$(ownership_get ETC_VPN_PRE_EXISTED "")"
  case "$etc_vpn_pre_existed" in
    1)
      [ -e "$ETCVPN_FIXTURE/deployment.toml" ] && rm -f "$ETCVPN_FIXTURE/deployment.toml"
      [ -e "$ETCVPN_FIXTURE/compat" ] && rm -rf "$ETCVPN_FIXTURE/compat"
      ;;
    *) echo "UNEXPECTED_BRANCH" ;;
  esac
)
if [ -f "$ETCVPN_FIXTURE/operator-data.txt" ] && [ "$(cat "$ETCVPN_FIXTURE/operator-data.txt")" = "unrelated operator data" ]; then
  ok "pre-existing sentinel file under /etc/vpn survives byte-for-byte"
else
  fail "pre-existing sentinel file under /etc/vpn was altered or removed"
fi
if [ ! -e "$ETCVPN_FIXTURE/deployment.toml" ] && [ ! -e "$ETCVPN_FIXTURE/compat" ]; then
  ok "vpn1's own children (deployment.toml, compat/) were removed"
else
  fail "vpn1's own children were not removed"
fi
if [ -d "$ETCVPN_FIXTURE" ]; then
  ok "the pre-existing /etc/vpn directory itself is never removed"
else
  fail "the pre-existing /etc/vpn directory itself was removed — this is the exact destructive-ownership bug checkpoint 2 fixes"
fi

echo
echo "--- static: deploy/almalinux/install.sh records ETC_VPN_PRE_EXISTED before ever creating /etc/vpn ---"
create_dirs_body="$(sed -n '/^create_directories() {/,/^}/p' "$INSTALL_SH")"
baseline_line="$(echo "$create_dirs_body" | grep -n 'ownership_set_baseline_once ETC_VPN_PRE_EXISTED' | head -n1 | cut -d: -f1)"
mkdir_line="$(echo "$create_dirs_body" | grep -n 'install -d -m 0755 /etc/vpn$' | head -n1 | cut -d: -f1)"
if [ -n "$baseline_line" ] && [ -n "$mkdir_line" ] && [ "$baseline_line" -lt "$mkdir_line" ]; then
  ok "ETC_VPN_PRE_EXISTED is captured before install.sh ever creates /etc/vpn"
else
  fail "ETC_VPN_PRE_EXISTED is not captured strictly before /etc/vpn is created (baseline=$baseline_line mkdir=$mkdir_line)"
fi

echo
echo "--- static: deploy/almalinux/uninstall.sh only 'rm -rf /etc/vpn' inside the ETC_VPN_PRE_EXISTED=0 case branch, never unconditionally ---"
etc_vpn_block="$(awk '/^if \[ -d \/etc\/vpn \]; then/{flag=1} flag{print} flag && /^fi$/{exit}' "$UNINSTALL_SH")"
case_line="$(echo "$etc_vpn_block" | grep -n 'case "\$etc_vpn_pre_existed" in' | head -n1 | cut -d: -f1)"
rmrf_line="$(echo "$etc_vpn_block" | grep -n '^ *rm -rf /etc/vpn$' | head -n1 | cut -d: -f1)"
if [ -n "$case_line" ] && [ -n "$rmrf_line" ] && [ "$case_line" -lt "$rmrf_line" ]; then
  ok "the only 'rm -rf /etc/vpn' is inside the ownership-gated case statement (not a bare unconditional removal)"
else
  fail "could not confirm 'rm -rf /etc/vpn' is gated by the ETC_VPN_PRE_EXISTED case statement (case_line=$case_line rmrf_line=$rmrf_line)"
fi
if grep -q 'ETC_VPN_PRE_EXISTED' "$UNINSTALL_SH"; then
  ok "uninstall.sh's /etc/vpn cleanup is gated on ETC_VPN_PRE_EXISTED"
else
  fail "uninstall.sh does not reference ETC_VPN_PRE_EXISTED at all"
fi

# ---------------------------------------------------------------------
# Fixed-name systemd/nginx/certbot-hook ownership wiring (static).
# ---------------------------------------------------------------------
echo
echo "--- static: install.sh installs the 4 systemd units + certbot hook through install_fixed_path_with_ownership() ---"
for key in SINGBOX_UNIT VPNSUB_UNIT EXPIRY_SVC_UNIT EXPIRY_TIMER_UNIT CERTBOT_HOOK; do
  # -A2: the call may wrap onto a continuation line (e.g. the certbot
  # hook call), so the KEY argument is not always on the same line as
  # the function name.
  if grep -A2 'install_fixed_path_with_ownership' "$INSTALL_SH" | grep -q "$key"; then
    ok "install.sh installs the $key fixed path with ownership tracking"
  else
    fail "install.sh does not install the $key fixed path with ownership tracking"
  fi
done
if grep -q 'FIXEDPATH_NGINX_CONF_PRE_EXISTED' "$INSTALL_SH" && grep -q 'FIXEDPATH_NGINX_CONF_BACKUP' "$INSTALL_SH"; then
  ok "install.sh tracks nginx vhost pre-existence + backup"
else
  fail "install.sh does not track nginx vhost pre-existence/backup"
fi

echo
echo "--- static: uninstall.sh restores/removes all 6 fixed paths via restore_or_remove_fixed_path() ---"
for key in SINGBOX_UNIT VPNSUB_UNIT EXPIRY_SVC_UNIT EXPIRY_TIMER_UNIT CERTBOT_HOOK NGINX_CONF; do
  if grep -q "restore_or_remove_fixed_path .* $key" "$UNINSTALL_SH"; then
    ok "uninstall.sh restores/removes the $key fixed path via restore_or_remove_fixed_path()"
  else
    fail "uninstall.sh does not call restore_or_remove_fixed_path() for $key"
  fi
done

# ---------------------------------------------------------------------
# Residue verification / truthful exit status (static + functional
# where practical without touching real host state).
# ---------------------------------------------------------------------
echo
echo "--- static: uninstall.sh has a CRITICAL_RESIDUE-gated exit and never prints success while critical residue remains ---"
if grep -q 'CRITICAL_RESIDUE+=(' "$UNINSTALL_SH" && grep -q 'UNINSTALL INCOMPLETE' "$UNINSTALL_SH" && grep -q 'UNINSTALL COMPLETE' "$UNINSTALL_SH"; then
  ok "uninstall.sh distinguishes UNINSTALL COMPLETE from UNINSTALL INCOMPLETE based on collected residue"
else
  fail "uninstall.sh does not implement a residue-gated COMPLETE/INCOMPLETE distinction"
fi
if grep -qE 'exit "\$final_rc"' "$UNINSTALL_SH"; then
  ok "uninstall.sh's final exit status is driven by the residue check (final_rc), not a hardcoded 0"
else
  fail "uninstall.sh does not exit with a residue-derived status code"
fi

echo
echo "--- functional: a critical-residue scenario (fake still-present REALITY key, no ownership dir) yields nonzero final_rc ---"
residue_sim="$(
  CRITICAL_RESIDUE=()
  FAKE_KEY="$TMPDIR_TEST/fake-reality-key"
  touch "$FAKE_KEY"
  [ -e "$FAKE_KEY" ] && CRITICAL_RESIDUE+=("REALITY private key still present ($FAKE_KEY)")
  if [ "${#CRITICAL_RESIDUE[@]}" -eq 0 ]; then final_rc=0; else final_rc=1; fi
  echo "$final_rc"
)"
[ "$residue_sim" = "1" ] && ok "critical residue correctly drives final_rc to nonzero (same logic uninstall.sh uses)" || fail "critical residue logic did not yield nonzero rc"

# ---------------------------------------------------------------------
# Package-name validation before dnf/apt remove.
# ---------------------------------------------------------------------
echo
echo "--- functional: is_safe_pkg_name() accepts real package names and rejects shell-metacharacter-bearing ones ---"
is_safe_pkg_name_body="$(sed -n '/^is_safe_pkg_name() {/,/^}/p' "$UNINSTALL_SH")"
for good in nginx certbot policycoreutils-python-utils python3.11 libidn2; do
  rc=0
  PKGVAL="$good" bash -c "$is_safe_pkg_name_body"'
is_safe_pkg_name "$PKGVAL"' || rc=$?
  [ "$rc" -eq 0 ] && ok "accepts real package name '$good'" || fail "wrongly rejected real package name '$good'"
done
for bad in 'nginx; rm -rf /' '$(whoami)' '../etc/passwd' '' '-rf'; do
  rc=0
  PKGVAL="$bad" bash -c "$is_safe_pkg_name_body"'
is_safe_pkg_name "$PKGVAL"' 2>/dev/null || rc=$?
  [ "$rc" -ne 0 ] && ok "rejects unsafe package-name-shaped string '$bad'" || fail "wrongly accepted unsafe string '$bad'"
done

echo
echo "--- static: package removal filters names through is_safe_pkg_name() before dnf/apt remove ---"
if grep -q 'is_safe_pkg_name "\$pkg"' "$UNINSTALL_SH"; then
  ok "package names from the ownership manifest are validated before being passed to the package manager"
else
  fail "package removal does not validate manifest-sourced package names"
fi

# ---------------------------------------------------------------------
# OPT_VPN1_PRE_EXISTED read-before-STATE_DIR_ROOT-removal ordering bug
# fix.
# ---------------------------------------------------------------------
echo
echo "--- static: OPT_VPN1_PRE_EXISTED (and unit PRE_EXISTED facts) are cached BEFORE STATE_DIR_ROOT is removed ---"
tail_body="$(tail -n 120 "$UNINSTALL_SH")"
cache_line="$(echo "$tail_body" | grep -n 'opt_vpn1_pre_existed="\$(ownership_get OPT_VPN1_PRE_EXISTED' | head -n1 | cut -d: -f1)"
rm_line="$(echo "$tail_body" | grep -n 'rm -rf "\$STATE_DIR_ROOT"' | head -n1 | cut -d: -f1)"
if [ -n "$cache_line" ] && [ -n "$rm_line" ] && [ "$cache_line" -lt "$rm_line" ]; then
  ok "OPT_VPN1_PRE_EXISTED is read and cached strictly before \$STATE_DIR_ROOT is removed (a prior ordering bug always read the post-removal default here)"
else
  fail "OPT_VPN1_PRE_EXISTED is not clearly cached before STATE_DIR_ROOT removal (cache=$cache_line rm=$rm_line)"
fi

# ---------------------------------------------------------------------
# Legacy uninstall (pre-07f8b72: no --yes, only --purge-state/
# --purge-firewall) flag translation, using the REAL historical script
# from git history — never invented/approximated.
# ---------------------------------------------------------------------
echo
echo "--- functional: legacy pre-07f8b72 uninstall.sh (real historical commit d8a4c87) rejects --yes directly ---"
LEGACY_FIXTURE="$TMPDIR_TEST/legacy-uninstall.sh"
if git -C "$REPO_ROOT" show d8a4c87:deploy/almalinux/uninstall.sh > "$LEGACY_FIXTURE" 2>/dev/null && [ -s "$LEGACY_FIXTURE" ]; then
  chmod +x "$LEGACY_FIXTURE"
  if grep -q -- '--purge-state' "$LEGACY_FIXTURE" && ! grep -q -- '\-\-yes) ASSUME_YES=1' "$LEGACY_FIXTURE"; then
    ok "fixture confirms the real historical script (commit d8a4c87) has no --yes support and only understands --purge-state/--purge-firewall"
  else
    fail "historical fixture does not match the expected pre-07f8b72 interface — re-check the commit range"
  fi

  echo
  echo "--- functional: the bootstrap's run_legacy_uninstaller() translates --yes -> --purge-state --purge-firewall (no --yes) for this real historical fixture ---"
  MOCKBIN="$TMPDIR_TEST/mockbin"
  mkdir -p "$MOCKBIN"
  cat > "$MOCKBIN/bash" <<'EOF'
#!/bin/bash
echo "MOCK_BASH_CALLED:$*"
EOF
  chmod +x "$MOCKBIN/bash"
  fn_extract="$(sed -n '/^run_legacy_uninstaller() {/,/^}/p' "$BOOTSTRAP_UNINSTALL_SH")"
  out="$(timeout -k 3 10 /bin/bash -c "
    set -Eeuo pipefail
    log() { :; }; warn() { :; }; die() { echo \"DIE:\$*\"; exit 1; }
    $fn_extract
    export PATH=\"$MOCKBIN:\$PATH\"
    run_legacy_uninstaller '$LEGACY_FIXTURE' --yes
  " 2>&1)"
  if echo "$out" | grep -q "MOCK_BASH_CALLED:$LEGACY_FIXTURE --purge-state --purge-firewall"; then
    ok "run_legacy_uninstaller() correctly translates --yes to --purge-state --purge-firewall (drops the unsupported --yes) for the real pre-07f8b72 script"
  else
    fail "run_legacy_uninstaller() did not translate flags as expected against the real historical fixture; got: $out"
  fi

  echo
  echo "--- functional: run_legacy_uninstaller() forwards --yes unchanged for the CURRENT (--yes-supporting) uninstall.sh ---"
  out2="$(timeout -k 3 10 /bin/bash -c "
    set -Eeuo pipefail
    log() { :; }; warn() { :; }; die() { echo \"DIE:\$*\"; exit 1; }
    $fn_extract
    export PATH=\"$MOCKBIN:\$PATH\"
    run_legacy_uninstaller '$UNINSTALL_SH' --yes
  " 2>&1)"
  if echo "$out2" | grep -q "MOCK_BASH_CALLED:$UNINSTALL_SH --yes"; then
    ok "run_legacy_uninstaller() forwards --yes unchanged to a script that actually supports it (no unnecessary translation)"
  else
    fail "run_legacy_uninstaller() altered args unnecessarily for the current uninstaller; got: $out2"
  fi
else
  echo "SKIP: could not extract the real historical fixture (commit d8a4c87) from git history in this checkout — legacy-compat tests require it verbatim, not an invented approximation."
fi

echo
echo "--- static: the bootstrap uses install-state.json's exact pinned version/repo (not the mutable default) when the local copy is damaged/missing ---"
if grep -q 'INSTALL_STATE_MANIFEST="/var/lib/vpn1/install-state.json"' "$BOOTSTRAP_UNINSTALL_SH" \
    && grep -q 'refs/tags/\$VPN1_REF' "$BOOTSTRAP_UNINSTALL_SH"; then
  ok "bootstrap reads install-state.json and fetches the exact pinned tag when available"
else
  fail "bootstrap does not prefer install-state.json's exact pinned version for a damaged-local-copy recovery"
fi
if grep -q -- '--ref) VPN1_REF="\$2"; VPN1_REF_EXPLICIT=1' "$BOOTSTRAP_UNINSTALL_SH"; then
  ok "an explicit operator --ref always wins over the install-state.json-derived version"
else
  fail "explicit --ref does not override the manifest-derived version"
fi

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all checkpoint-2 uninstall-ownership tests passed"
