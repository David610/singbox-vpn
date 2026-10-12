#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
warn() { :; }
# shellcheck source=/dev/null
. "$ROOT/deploy/lib/ownership.sh"
# shellcheck source=/dev/null
. "$ROOT/deploy/lib/nginx-default.sh"
export OWNERSHIP_DIR="$TMP/owned" OWNERSHIP_FILE="$TMP/owned/ownership.env"
export NGINX_DEFAULT_AVAILABLE="$TMP/sites-available/default" NGINX_DEFAULT_ENABLED="$TMP/sites-enabled/default"
mkdir -p "$TMP/sites-available" "$TMP/sites-enabled"
echo 'stock welcome site' > "$NGINX_DEFAULT_AVAILABLE"
STOCK_HASH="$(md5sum "$NGINX_DEFAULT_AVAILABLE" | awk '{print $1}')"
dpkg-query() { printf ' /etc/nginx/sites-available/default %s\n' "$STOCK_HASH"; }
ln -s ../sites-available/default "$NGINX_DEFAULT_ENABLED"
nginx_disable_packaged_default
[ ! -L "$NGINX_DEFAULT_ENABLED" ]
[ -f "$NGINX_DEFAULT_AVAILABLE" ]
[ "$NGINX_DEFAULT_CHANGED_THIS_RUN" = 1 ]
nginx_disable_packaged_default
[ "$NGINX_DEFAULT_CHANGED_THIS_RUN" = 0 ]
nginx_restore_packaged_default
[ "$(readlink "$NGINX_DEFAULT_ENABLED")" = ../sites-available/default ]
echo 'admin changes' >> "$NGINX_DEFAULT_AVAILABLE"
nginx_disable_packaged_default
[ -L "$NGINX_DEFAULT_ENABLED" ]
rm "$NGINX_DEFAULT_ENABLED"
ln -s /admin/unrelated "$NGINX_DEFAULT_ENABLED"
nginx_restore_packaged_default
[ "$(readlink "$NGINX_DEFAULT_ENABLED")" = /admin/unrelated ]
echo 'nginx default ownership, idempotence, rollback/uninstall restore and administrator preservation: PASS'
