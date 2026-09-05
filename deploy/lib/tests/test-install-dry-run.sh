#!/usr/bin/env bash
# Regression test for install.sh --dry-run (docs/INSTALLATION.md,
# SUPPORTED_PRODUCT completion program Phase 2): a dry run must report a
# PASS/WARN/FAIL preflight summary and make ZERO persistent changes to
# the host, regardless of whether the checks it reports pass or fail.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

echo "--- static: --dry-run is parsed and short-circuits before any mutating stage ---"
if grep -q -- '--dry-run) DRY_RUN=1; shift ;;' "$INSTALL_SH"; then
  ok "--dry-run is a recognized CLI flag"
else
  fail "--dry-run flag parsing not found in install.sh"
fi
main_body="$(sed -n '/^main() {/,/^}/p' "$INSTALL_SH")"
if echo "$main_body" | grep -q 'if \[ "\$DRY_RUN" -eq 1 \]; then'; then
  dry_run_line="$(echo "$main_body" | grep -n 'DRY_RUN.*-eq 1' | head -n1 | cut -d: -f1)"
  preflight_line="$(echo "$main_body" | grep -n '^\s*preflight_stage$' | head -n1 | cut -d: -f1)"
  if [ -n "$dry_run_line" ] && [ -n "$preflight_line" ] && [ "$dry_run_line" -lt "$preflight_line" ]; then
    ok "main() checks DRY_RUN before calling preflight_stage (the first mutating stage)"
  else
    fail "DRY_RUN check in main() does not run strictly before preflight_stage"
  fi
else
  fail "main() has no DRY_RUN branch"
fi

echo
echo "--- static: dry_run_report() never calls preflight_stage() (which mutates: lock file, persisted source tree, IDN package install, install-state manifest) ---"
dry_run_body="$(sed -n '/^dry_run_report() {/,/^}/p' "$INSTALL_SH")"
if [ -z "$dry_run_body" ]; then
  fail "dry_run_report() function not found"
elif echo "$dry_run_body" | grep -qE 'preflight_stage|packages_stage|persist_source_tree|install_idn_support|write_install_state_manifest'; then
  fail "dry_run_report() calls a mutating stage/helper — this would make --dry-run not a dry run"
else
  ok "dry_run_report() only calls read-only detection/validation helpers"
fi

echo
echo "--- functional: --dry-run makes zero persistent host changes (even when checks FAIL) ---"
TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT

# An invalid hostname is a deterministic, network-independent way to
# force a FAIL line without depending on live reachability to a
# third-party host in CI.
out="$TMPDIR_TEST/dry-run.out"
rc=0
bash "$INSTALL_SH" --dry-run --domain 'not a valid host' --reality-handshake-server 'also not valid' \
  > "$out" 2>&1 || rc=$?

if [ "$rc" -eq 1 ]; then
  ok "--dry-run with invalid input exits 1 (fail-closed)"
else
  fail "--dry-run with invalid --domain/--reality-handshake-server exited $rc, expected 1"
fi

if grep -q "No changes made\." "$out"; then
  ok "report explicitly states no changes were made"
else
  fail "dry-run output did not include the 'No changes made.' statement"
fi

if grep -qE 'NOT READY|READY TO INSTALL' "$out"; then
  ok "report prints a final readiness verdict"
else
  fail "dry-run output did not print a READY TO INSTALL / NOT READY verdict"
fi

if grep -q 'FAIL' "$out"; then
  ok "invalid input is reported as a FAIL line, not silently ignored"
else
  fail "expected at least one FAIL line for invalid --domain/--reality-handshake-server input"
fi

if grep -qiE 'rolling back|on_fatal_error|installation failed' "$out"; then
  fail "--dry-run triggered the fatal-error/rollback path — a report-only run must never look like a failed real install"
else
  ok "--dry-run did not trigger the fatal-error/rollback path"
fi

for path in /opt/singbox-vpn /var/lib/singbox-vpn /etc/vpn /run/lock/singbox-vpn-installer.lock; do
  if [ -e "$path" ]; then
    fail "--dry-run left $path on disk — this is not a dry run"
  else
    ok "$path was not created by --dry-run"
  fi
done

echo
if [ "$failures" -eq 0 ]; then
  echo "install --dry-run tests: PASS"
  exit 0
else
  echo "install --dry-run tests: FAIL ($failures failing check(s))"
  exit 1
fi
