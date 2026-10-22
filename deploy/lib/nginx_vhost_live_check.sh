#!/usr/bin/env bash
# I-45 on a REAL nginx (staging only; the live vhost is changed transiently and always restored):
#   1. a refreshed vhost that nginx -t REJECTS must roll back to the exact previous bytes, nginx stays valid and serving;
#   2. a valid but different vhost is applied (reloaded) and a second refresh is a no-op;
#   3. custom state is not lost: an operator line outside the rendered template is NOT preserved (documented) —
#      so the check only asserts what the contract promises (exact restore on failure, no-op when identical).
set -Eeuo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CONF=/etc/nginx/conf.d/vpn-subscription.conf
TPL="$REPO_ROOT/deploy/almalinux/templates/nginx-vpn-subscription.conf.template"
[ -f "$CONF" ] || { echo "SKIP-NOT-PASS: no live vhost on this host"; exit 3; }
log() { echo "  $*"; }; warn() { echo "  WARN $*"; }
# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/nginx-vhost.sh"
HOST="$(awk -F'"' '/^subscription_host/{print $2;exit}' /etc/vpn/deployment.toml)"; PUB="$(awk -F'"' '/^public_host/{print $2;exit}' /etc/vpn/deployment.toml)"
PORT="$(awk '/^\[subscription\]/{s=1;next} /^\[/{s=0} s&&/public_port/{gsub(/[^0-9]/,"");print;exit}' /etc/vpn/deployment.toml)"
BACK="$(awk '/^\[subscription\]/{s=1;next} /^\[/{s=0} s&&/listen_port/{gsub(/[^0-9]/,"");print;exit}' /etc/vpn/deployment.toml)"
before="$(sha256sum <"$CONF")"; fails=0
serving() { curl -s -m8 -o /dev/null -w '%{http_code}' "https://$HOST:$PORT/healthz" --resolve "$HOST:$PORT:127.0.0.1" 2>/dev/null; }
TMPD="$(mktemp -d)"; trap 'cp -p "$TMPD/orig.conf" "$CONF" 2>/dev/null; nginx -t >/dev/null 2>&1 && systemctl reload nginx; rm -rf "$TMPD"' EXIT
cp -p "$CONF" "$TMPD/orig.conf"
# 1. broken template: nginx -t must fail, the exact previous file must come back
sed 's/server_tokens off;/server_tokens off;\n    this_is_not_a_directive on;/' "$TPL" >"$TMPD/broken.template"
set +e; refresh_nginx_vhost "$TMPD/broken.template" "$CONF" "$HOST" "$PUB" "$PORT" "$BACK" >"$TMPD/out1" 2>&1; rc=$?; set -e
after="$(sha256sum <"$CONF")"
if [ "$rc" -ne 0 ] && [ "$before" = "$after" ] && nginx -t >/dev/null 2>&1; then echo "PASS: nginx -t rejected the refreshed vhost: previous bytes restored (rc=$rc), nginx valid"; else echo "FAIL: broken refresh rc=$rc restored=$([ "$before" = "$after" ] && echo yes || echo NO)"; fails=$((fails+1)); fi
[ "$(serving)" = 200 ] && echo "PASS: vhost still serving /healthz after the failed refresh" || { echo "FAIL: not serving (http $(serving))"; fails=$((fails+1)); }
[ ! -e "$CONF.update-bak" ] && [ ! -e "$CONF.update-new" ] && echo "PASS: no backup/new files left behind" || { echo "FAIL: leftovers"; fails=$((fails+1)); }
# 2. a valid, different template is applied; applying it again is a no-op
sed 's/server_tokens off;/server_tokens off;\n    # live-check marker/' "$TPL" >"$TMPD/ok.template"
refresh_nginx_vhost "$TMPD/ok.template" "$CONF" "$HOST" "$PUB" "$PORT" "$BACK" >"$TMPD/out2" 2>&1 && grep -q 'live-check marker' "$CONF" && echo "PASS: a valid changed template is applied and reloaded" || { echo "FAIL: valid refresh not applied"; fails=$((fails+1)); }
h2="$(sha256sum <"$CONF")"; refresh_nginx_vhost "$TMPD/ok.template" "$CONF" "$HOST" "$PUB" "$PORT" "$BACK" >/dev/null 2>&1; [ "$h2" = "$(sha256sum <"$CONF")" ] && echo "PASS: identical refresh is a no-op" || { echo "FAIL: not idempotent"; fails=$((fails+1)); }
[ "$(serving)" = 200 ] && echo "PASS: serving after the valid refresh" || { echo "FAIL: not serving after valid refresh"; fails=$((fails+1)); }
# restore (trap does it too) and verify
cp -p "$TMPD/orig.conf" "$CONF"; nginx -t >/dev/null 2>&1 && systemctl reload nginx
[ "$before" = "$(sha256sum <"$CONF")" ] && echo "PASS: live vhost restored byte-identically" || { echo "FAIL: live vhost not restored"; fails=$((fails+1)); }
[ "$fails" -eq 0 ] && echo "RESULT: PASS" || { echo "RESULT: FAIL ($fails)"; exit 1; }
