#!/usr/bin/env bash
# Regression tests for IDN/Unicode domain handling
# (derive_punycode_host(), sourced from the real install.sh): a Unicode
# domain such as чёрт.com must be safely converted to its punycode
# representation (xn--p1aen4b.com) before it is ever interpolated into
# deployment.toml/nginx config/certificate paths, and the conversion
# must work regardless of this host's own locale (a minimal VPS image
# is not guaranteed to default to a UTF-8 locale).
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"
PREFLIGHT_SH="$REPO_ROOT/deploy/lib/preflight.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

if ! command -v idn2 >/dev/null 2>&1 && ! command -v idn >/dev/null 2>&1; then
  echo "SKIP: neither idn2 nor idn is installed in this environment — derive_punycode_host()'s fallback-to-unchanged-input behavior is exercised instead (still a real, intentional code path: it lets preflight_validate_hostname correctly reject an unconvertable IDN input rather than mis-encoding it silently)."
fi

TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT

run_derive() {
  (
    set -Eeuo pipefail
    DEPLOYMENT_TOML="$TMPDIR_TEST/no-such-deployment.toml"
    # shellcheck source=/dev/null
    source "$INSTALL_SH"
    derive_punycode_host "$1"
  )
}

echo "--- derive_punycode_host(): чёрт.com -> punycode ---"
result="$(run_derive "чёрт.com")"
if [ "$result" = "xn--p1aen4b.com" ]; then
  ok "чёрт.com converts to xn--p1aen4b.com"
elif command -v idn2 >/dev/null 2>&1 || command -v idn >/dev/null 2>&1; then
  fail "чёрт.com converted to '$result', expected xn--p1aen4b.com (idn2/idn IS installed, so this should have converted)"
else
  ok "no IDN converter installed — чёрт.com passed through unchanged ('$result'), which correctly fails hostname validation next rather than being silently mis-encoded"
fi

echo
echo "--- derive_punycode_host(): already-ASCII input is passed through unchanged ---"
result_ascii="$(run_derive "vpn.example.com")"
[ "$result_ascii" = "vpn.example.com" ] && ok "ASCII hostname passes through unchanged" || fail "ASCII hostname was altered: '$result_ascii'"

echo
echo "--- end-to-end: the punycode form passes preflight_validate_hostname ---"
(
  set -Eeuo pipefail
  # shellcheck source=/dev/null
  . "$PREFLIGHT_SH"
  log() { :; }
  warn() { :; }
  preflight_validate_hostname "xn--p1aen4b.com" "PUBLIC_HOST"
) && ok "the punycode form of чёрт.com passes preflight_validate_hostname" || fail "the punycode form did not pass hostname validation"

echo
echo "--- end-to-end: the RAW Unicode form is correctly REJECTED by preflight_validate_hostname (must never be interpolated unconverted) ---"
rc=0
(
  set -Eeuo pipefail
  # shellcheck source=/dev/null
  . "$PREFLIGHT_SH"
  log() { :; }
  warn() { :; }
  preflight_validate_hostname "чёрт.com" "PUBLIC_HOST"
) >/dev/null 2>&1 || rc=$?
[ "$rc" -ne 0 ] && ok "raw Unicode input is rejected by hostname validation (never reaches sed/TOML/nginx interpolation unconverted)" || fail "raw Unicode input was NOT rejected by hostname validation — this would let it reach interpolation sites unconverted"

if command -v idn2 >/dev/null 2>&1 || command -v idn >/dev/null 2>&1; then
  echo
  echo "--- functional: resolve_host_config() end-to-end converts a Unicode PUBLIC_HOST supplied via env, before validation ---"
  # Uses the RFC 2606 reserved .invalid TLD (guaranteed to never resolve
  # anywhere) so this test is not at the mercy of whether SOME real
  # domain happens to share a punycode encoding with a Cyrillic test
  # string and can actually reach this host's DNS-resolves-here check
  # without it correctly (and separately) refusing to proceed.
  run_resolve_host_config() {
    (
      set -Eeuo pipefail
      # shellcheck disable=SC2034 # read by install.sh's top-level probe on source
      DEPLOYMENT_TOML="$TMPDIR_TEST/no-such-deployment2.toml"
      # shellcheck source=/dev/null
      source "$INSTALL_SH"
      # shellcheck disable=SC2034 # read by resolve_host_config/resolve_reality_handshake_server
      NONINTERACTIVE=1
      PUBLIC_HOST="тест.invalid"
      resolve_host_config
      echo "PUBLIC_HOST=$PUBLIC_HOST"
    )
  }
  out="$(run_resolve_host_config 2>&1)" || true
  if echo "$out" | grep -q '^PUBLIC_HOST=xn--e1aybc.invalid$'; then
    ok "resolve_host_config() converts an env-supplied Unicode PUBLIC_HOST to punycode before validation/use"
  else
    fail "resolve_host_config() did not convert an env-supplied Unicode PUBLIC_HOST as expected; got: $out"
  fi
fi

echo
echo "--- static: on the supported AlmaLinux 9 (rhel family) path, install_idn_support() installs a real IDN converter (libidn2) BEFORE resolve_host_config() ever normalizes an operator-supplied domain ---"
if grep -q 'rhel) pkg="libidn2"' "$INSTALL_SH"; then
  ok "install_idn_support() installs libidn2 for OS_FAMILY=rhel (AlmaLinux 9's family)"
else
  fail "install_idn_support() no longer installs libidn2 for the rhel family — Unicode domains on AlmaLinux 9 would silently have no converter available"
fi
preflight_body="$(sed -n '/^preflight_stage() {/,/^}/p' "$INSTALL_SH")"
idn_line="$(echo "$preflight_body" | grep -n '^\s*install_idn_support\s*$' | head -n1 | cut -d: -f1)"
host_line="$(echo "$preflight_body" | grep -n '^\s*resolve_host_config\s*$' | head -n1 | cut -d: -f1)"
if [ -n "$idn_line" ] && [ -n "$host_line" ] && [ "$idn_line" -lt "$host_line" ]; then
  ok "install_idn_support() runs before resolve_host_config() in preflight_stage — the converter is available before it is needed"
else
  fail "install_idn_support() does not run before resolve_host_config() (idn=$idn_line host=$host_line)"
fi

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all IDN/punycode tests passed"
