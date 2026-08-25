#!/usr/bin/env bash
# Regression tests for the privacy/logging hardening pass:
#   - vpn-subscription.service defaults to RUST_LOG=warn (not info), so
#     normal successful-request metadata is not retained by default.
#   - the nginx vhost template restricts /healthz to loopback/local
#     diagnostics and sets server_tokens off, scoped to this vhost only.
#   - every /healthz probe this repo ships (install.sh, health-check.sh,
#     acceptance-test.sh) that goes through the nginx vhost connects via
#     loopback (--resolve HOST:PORT:127.0.0.1), so the new restriction
#     cannot break them.
#   - sing-box.service and vpn-subscription.service both carry per-unit
#     journal rate limiting (LogRateLimitIntervalSec/LogRateLimitBurst).
#   - no global journald policy file is touched by any singbox-vpn shell script.
#
# Static/source-inspection only — does not require a real nginx/systemd
# host. See services/subscription/src/lib.rs's own `#[test]`s for the
# behavioral (not just textual) proof that the per-request success log
# line is filtered out at the production default level and that no
# secret value ever appears in a log line at any verbosity.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
NGINX_TEMPLATE="$REPO_ROOT/deploy/almalinux/templates/nginx-vpn-subscription.conf.template"
SUBSCRIPTION_UNIT="$REPO_ROOT/deploy/almalinux/systemd/vpn-subscription.service"
SINGBOX_UNIT="$REPO_ROOT/deploy/almalinux/systemd/sing-box.service"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

[ -f "$NGINX_TEMPLATE" ] || fail "nginx template is missing: $NGINX_TEMPLATE"
[ -f "$SUBSCRIPTION_UNIT" ] || fail "vpn-subscription.service is missing: $SUBSCRIPTION_UNIT"
[ -f "$SINGBOX_UNIT" ] || fail "sing-box.service is missing: $SINGBOX_UNIT"

echo "--- vpn-subscription.service: production default log level ---"
if grep -qE '^Environment=RUST_LOG=warn$' "$SUBSCRIPTION_UNIT"; then
  ok "RUST_LOG defaults to warn (successful subscription fetches are not retained by default)"
else
  fail "RUST_LOG is not set to warn in vpn-subscription.service"
fi
if grep -qE '^Environment=RUST_LOG=info$' "$SUBSCRIPTION_UNIT"; then
  fail "regression: RUST_LOG=info is back in vpn-subscription.service"
else
  ok "RUST_LOG=info is not present"
fi

echo
echo "--- per-service journal rate limiting (systemd-native, not global journald.conf) ---"
for unit_label_path in "vpn-subscription.service:$SUBSCRIPTION_UNIT" "sing-box.service:$SINGBOX_UNIT"; do
  label="${unit_label_path%%:*}"
  path="${unit_label_path#*:}"
  if grep -qE '^LogRateLimitIntervalSec=' "$path" && grep -qE '^LogRateLimitBurst=' "$path"; then
    ok "$label sets LogRateLimitIntervalSec and LogRateLimitBurst"
  else
    fail "$label is missing LogRateLimitIntervalSec/LogRateLimitBurst"
  fi
done

echo
echo "--- global journald policy is never touched ---"
for f in "$REPO_ROOT/deploy/almalinux/install.sh" "$REPO_ROOT/deploy/almalinux/update.sh" \
         "$REPO_ROOT/deploy/almalinux/uninstall.sh" "$REPO_ROOT/deploy/lib"/*.sh; do
  [ -f "$f" ] || continue
  if grep -q '/etc/systemd/journald.conf' "$f"; then
    fail "$f touches /etc/systemd/journald.conf — this must stay a per-unit-only change"
  fi
done
ok "no shell script under deploy/ touches /etc/systemd/journald.conf"

echo
echo "--- nginx vhost template: server_tokens scoped to this vhost only ---"
if grep -qE '^\s*server_tokens off;' "$NGINX_TEMPLATE"; then
  ok "server_tokens off is present"
else
  fail "server_tokens off is missing from the nginx template"
fi
if grep -q 'http {' "$NGINX_TEMPLATE"; then
  fail "the template defines its own http{} block — server_tokens must stay scoped to the singbox-vpn server{} block, not become a global change"
else
  ok "the template has no http{} block (server_tokens applies only inside singbox-vpn's own server{} block)"
fi

echo
echo "--- nginx vhost template: /healthz restricted to loopback/local diagnostics ---"
healthz_block="$(awk '/location \/healthz \{/,/^    \}/' "$NGINX_TEMPLATE")"
if [ -z "$healthz_block" ]; then
  fail "could not locate the /healthz location block in the template"
else
  if printf '%s' "$healthz_block" | grep -q 'allow 127.0.0.1;' \
    && printf '%s' "$healthz_block" | grep -q 'allow ::1;' \
    && printf '%s' "$healthz_block" | grep -q 'deny all;'; then
    ok "/healthz allows only 127.0.0.1/::1 and denies everything else"
  else
    fail "/healthz is not restricted to loopback (allow 127.0.0.1; allow ::1; deny all;)"
  fi
  # Ordering matters for nginx's access module: allow/deny rules must
  # come before proxy_pass has a chance to matter, but more importantly
  # here, they must both be present in the same location so this can
  # never regress into a location that forgot the `deny all;` catch-all.
  if printf '%s' "$healthz_block" | grep -q 'proxy_pass'; then
    ok "/healthz still proxies to the loopback backend (local diagnostics keep working)"
  else
    fail "/healthz no longer proxies to the backend — local health checks would break"
  fi
fi

echo
echo "--- nginx vhost template: /sub/ token-safety directives are untouched by this change ---"
sub_block="$(awk '/location \/sub\/ \{/,/^    \}/' "$NGINX_TEMPLATE")"
if printf '%s' "$sub_block" | grep -q 'access_log off;' \
  && printf '%s' "$sub_block" | grep -q 'error_log .* crit;'; then
  ok "/sub/ still disables full access logging and caps error_log at crit (bearer token never logged)"
else
  fail "/sub/'s token-safety logging directives regressed"
fi

echo
echo "--- every shipped /healthz probe through the nginx vhost uses loopback (--resolve ...:127.0.0.1) ---"
# Find every /healthz probe that goes through the PUBLIC vhost (i.e. uses
# a curl invocation containing "/healthz" together with an https:// URL,
# as opposed to the direct http://127.0.0.1:<backend-port>/healthz probes
# which never touch nginx at all and are unaffected by this change).
found_public_healthz_probe=0
for f in "$REPO_ROOT/deploy/almalinux/install.sh" "$REPO_ROOT/deploy/almalinux/health-check.sh" \
         "$REPO_ROOT/deploy/almalinux/acceptance-test.sh"; do
  [ -f "$f" ] || continue
  # A public-vhost probe is a shell statement (possibly spanning several
  # backslash-continued lines) containing both "https://" and "/healthz".
  # Match on a small surrounding window rather than one exact line, since
  # health-check.sh puts --resolve and the URL on separate continuation
  # lines of the same curl invocation.
  while IFS= read -r matchline; do
    lineno="${matchline%%:*}"
    start=$((lineno - 3))
    [ "$start" -lt 1 ] && start=1
    context="$(sed -n "${start},$((lineno + 1))p" "$f")"
    if printf '%s' "$context" | grep -q 'https://'; then
      found_public_healthz_probe=1
      if printf '%s' "$context" | grep -q -- '--resolve' && printf '%s' "$context" | grep -q '127\.0\.0\.1'; then
        ok "$(basename "$f"): public-vhost /healthz probe (line $lineno) uses --resolve ...:127.0.0.1 (survives the loopback restriction)"
      else
        fail "$(basename "$f"): public-vhost /healthz probe (line $lineno) does not resolve to loopback — it would now get 403:
$context"
      fi
    fi
  done < <(grep -n '/healthz' "$f" 2>/dev/null || true)
done
if [ "$found_public_healthz_probe" -eq 0 ]; then
  fail "expected at least one public-vhost (https://.../healthz) probe across install.sh/health-check.sh/acceptance-test.sh — found none; this test may no longer be checking what it claims to"
fi

echo
if [ "$failures" -eq 0 ]; then
  echo "PASS: test-privacy-logging-hardening.sh"
  exit 0
else
  echo "FAIL: test-privacy-logging-hardening.sh ($failures failure(s))"
  exit 1
fi
