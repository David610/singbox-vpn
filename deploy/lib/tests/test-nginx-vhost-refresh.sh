#!/usr/bin/env bash
# I-45: update.sh must refresh the nginx subscription vhost from the template
# (otherwise security fixes such as HSTS never reach deployed nodes), as a
# recoverable transaction: no-op when identical, `nginx -t` + reload when it
# changes, exact restore of the previous file if either fails, never create a
# vhost that does not exist. Stubs only; no host change.
set -Euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
LIB="$REPO_ROOT/deploy/lib/nginx-vhost.sh"
TPL="$REPO_ROOT/deploy/almalinux/templates/nginx-vpn-subscription.conf.template"
UPDATE_SH="$REPO_ROOT/deploy/almalinux/update.sh"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"
failures=0
ok() { echo "ok: $*"; }
bad() { echo "FAIL: $*"; failures=$((failures + 1)); }
[ -f "$LIB" ] || { bad "deploy/lib/nginx-vhost.sh missing"; exit 1; }
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
mkdir -p "$T/bin" "$T/conf"; : >"$T/calls"
cat >"$T/bin/nginx" <<EOF
#!/bin/sh
echo "nginx \$*" >>"$T/calls"
[ -e "$T/NGINX_T_FAILS" ] && [ "\$1" = "-t" ] && { echo "nginx: [emerg] boom" >&2; exit 1; }
exit 0
EOF
cat >"$T/bin/systemctl" <<EOF
#!/bin/sh
echo "systemctl \$*" >>"$T/calls"
[ -e "$T/RELOAD_FAILS" ] && [ "\$1" = "reload" ] && exit 1
exit 0
EOF
chmod +x "$T/bin/nginx" "$T/bin/systemctl"
export PATH="$T/bin:$PATH"
CONF="$T/conf/vpn-subscription.conf"
# shellcheck source=/dev/null
. "$LIB"
log() { :; }; warn() { echo "WARN: $*" >&2; }
args=("$TPL" "$CONF" fin1.example.test fin1.example.test 8443 9100)

# 1. never create a vhost that is not there
refresh_nginx_vhost "${args[@]}"; rc=$?
{ [ $rc -eq 0 ] && [ ! -e "$CONF" ] && [ ! -s "$T/calls" ]; } && ok "no existing vhost: nothing created, nothing reloaded" || bad "created/reloaded without an existing vhost"

# 2. identical -> no-op
render_nginx_vhost "$TPL" fin1.example.test fin1.example.test 8443 9100 >"$CONF"
: >"$T/calls"; refresh_nginx_vhost "${args[@]}"; rc=$?
{ [ $rc -eq 0 ] && [ ! -s "$T/calls" ]; } && ok "identical vhost: no nginx -t, no reload" || bad "touched nginx for an identical vhost: $(cat "$T/calls")"

# 3. old vhost without the HSTS fix -> replaced, validated, reloaded
grep -v 'Referrer-Policy' "$CONF" >"$T/old.conf"; sed -i '/location \/sub\//,/^    }/{/Strict-Transport-Security/d}' "$T/old.conf"; cp "$T/old.conf" "$CONF"
: >"$T/calls"; refresh_nginx_vhost "${args[@]}"; rc=$?
if [ $rc -eq 0 ] && grep -q 'nginx -t' "$T/calls" && grep -q 'systemctl reload nginx' "$T/calls" && cmp -s "$CONF" <(render_nginx_vhost "$TPL" fin1.example.test fin1.example.test 8443 9100); then
  ok "stale vhost replaced by the current template, validated, reloaded"
else bad "stale vhost not refreshed (rc=$rc calls: $(cat "$T/calls"))"; fi
ls "$T/conf"/*.update-bak >/dev/null 2>&1 && bad "backup left behind after success" || ok "no backup left behind after success"

# 4. nginx -t fails -> exact previous file restored, nonzero
cp "$T/old.conf" "$CONF"; : >"$T/NGINX_T_FAILS"; : >"$T/calls"
refresh_nginx_vhost "${args[@]}" 2>/dev/null; rc=$?
{ [ $rc -ne 0 ] && cmp -s "$CONF" "$T/old.conf"; } && ok "nginx -t failure: previous vhost restored byte-identically, nonzero" || bad "not restored after nginx -t failure (rc=$rc)"
rm -f "$T/NGINX_T_FAILS"

# 5. reload fails -> restored
cp "$T/old.conf" "$CONF"; : >"$T/RELOAD_FAILS"
refresh_nginx_vhost "${args[@]}" 2>/dev/null; rc=$?
{ [ $rc -ne 0 ] && cmp -s "$CONF" "$T/old.conf"; } && ok "reload failure: previous vhost restored, nonzero" || bad "not restored after reload failure (rc=$rc)"
rm -f "$T/RELOAD_FAILS"

# 6. wiring: both scripts use the shared renderer; update refreshes on both paths
grep -q 'deploy/lib/nginx-vhost.sh' "$INSTALL_SH" && grep -q 'render_nginx_vhost' "$INSTALL_SH" && ok "install.sh renders through the shared function (no drift)" || bad "install.sh does not use the shared renderer"
n="$(grep -c '^ *update_refresh_nginx_vhost$' "$UPDATE_SH")"
[ "$n" -ge 2 ] && ok "update.sh refreshes the vhost on both update paths ($n call sites)" || bad "update.sh calls update_refresh_nginx_vhost $n time(s)"

[ "$failures" -eq 0 ] || { echo "$failures failure(s)"; exit 1; }; echo "all passed"
