#!/usr/bin/env bash
# certbot deploy-hook: keeps the Hysteria2 TLS cert/key vpn1 reads from
# /etc/vpn/compat/hysteria/{cert,key}.pem in sync with certbot's renewed
# certificate. Installed by install.sh into
# /etc/letsencrypt/renewal-hooks/deploy/vpn1-hysteria.sh, which certbot
# runs automatically after ANY successful renewal (via `certbot renew`,
# normally driven by the distro's certbot-renew.timer).
#
# Why this exists (docs/FINAL_PRODUCTION_AUDIT.md P0-11): install.sh only
# ever copies the cert/key ONCE, at install time. sing-box never reads
# /etc/letsencrypt directly (it isn't in sing-box's ReadOnlyPaths, and the
# vpn1 state dir has its own explicit ownership matrix), so without this
# hook the copied files silently go stale ~90 days after install and
# Hysteria2 starts failing TLS handshakes with no visible warning.
#
# certbot exports $RENEWED_LINEAGE (the live cert directory that was just
# renewed) and $RENEWED_DOMAINS when it runs deploy hooks; this script
# only acts if $RENEWED_LINEAGE matches vpn1's configured PUBLIC_HOST, so
# it never touches unrelated certificates on a host running certbot for
# other purposes too.
set -euo pipefail

STATE_DIR="/etc/vpn/compat"
DEPLOYMENT_TOML="/etc/vpn/deployment.toml"
SINGBOX_BIN="/usr/local/bin/sing-box"

log() { echo "[certbot-deploy-hook] $*"; }
warn() { echo "[certbot-deploy-hook] WARNING: $*" >&2; }
die() { echo "[certbot-deploy-hook] ERROR: $*" >&2; exit 1; }

[ -n "${RENEWED_LINEAGE:-}" ] || die "expected to be run by certbot as a deploy hook (RENEWED_LINEAGE not set)."
[ -f "$DEPLOYMENT_TOML" ] || { log "no $DEPLOYMENT_TOML — vpn1 not installed on this host, nothing to do."; exit 0; }

public_host="$(grep -E '^public_host' "$DEPLOYMENT_TOML" | sed -E 's/^public_host *= *"([^"]*)".*/\1/')"
renewed_host="$(basename "$RENEWED_LINEAGE")"
if [ "$renewed_host" != "$public_host" ]; then
  log "renewed certificate is for '$renewed_host', vpn1's PUBLIC_HOST is '$public_host' — not this deployment's cert, skipping."
  exit 0
fi

cert="$STATE_DIR/hysteria/cert.pem"
key="$STATE_DIR/hysteria/key.pem"

[ -s "$RENEWED_LINEAGE/fullchain.pem" ] || die "renewed fullchain.pem missing/empty at $RENEWED_LINEAGE"
[ -s "$RENEWED_LINEAGE/privkey.pem" ] || die "renewed privkey.pem missing/empty at $RENEWED_LINEAGE"

# Verify the renewed cert before installing it: correct hostname/SAN, not
# expired. A corrupt/mismatched renewal must never silently replace a
# working certificate.
openssl x509 -in "$RENEWED_LINEAGE/fullchain.pem" -noout -checkend 0 >/dev/null 2>&1 \
  || die "renewed certificate at $RENEWED_LINEAGE is not currently valid — refusing to install it."
if ! openssl x509 -in "$RENEWED_LINEAGE/fullchain.pem" -noout -text 2>/dev/null \
    | grep -qE "DNS:${public_host//./\\.}(,|$| )"; then
  die "renewed certificate at $RENEWED_LINEAGE does not have a SAN matching $public_host — refusing to install it."
fi

install -d -m 02750 -o root -g sing-box "$STATE_DIR/hysteria"
tmp_cert="$cert.renew.tmp"
tmp_key="$key.renew.tmp"
install -m 0640 -o root -g sing-box "$RENEWED_LINEAGE/fullchain.pem" "$tmp_cert"
install -m 0640 -o root -g sing-box "$RENEWED_LINEAGE/privkey.pem" "$tmp_key"
mv -f "$tmp_cert" "$cert"
mv -f "$tmp_key" "$key"
log "installed renewed certificate into $cert / $key"

if [ -x "$SINGBOX_BIN" ] && [ -f "$STATE_DIR/sing-box/config.json" ]; then
  "$SINGBOX_BIN" check -c "$STATE_DIR/sing-box/config.json" \
    || die "sing-box check failed against the current config after cert renewal — NOT reloading sing-box with a config that fails validation. Investigate immediately: Hysteria2 will fail once the old cert expires."
  systemctl reload-or-restart sing-box \
    || die "failed to reload sing-box after certificate renewal."
  sleep 1
  systemctl is-active --quiet sing-box \
    || die "sing-box is not active after reload following certificate renewal."
  log "sing-box reloaded and verified active with the renewed certificate."
else
  warn "sing-box binary or config not found — skipping reload (unexpected on a real vpn1 install)."
fi

# The subscription vhost (nginx) reads /etc/letsencrypt/live directly via
# its own certbot-managed symlink, so it needs no copy step — only a
# config test + reload so nginx picks up the renewed files instead of
# keeping the old ones cached in its worker processes.
if command -v nginx >/dev/null 2>&1 && systemctl is-active --quiet nginx 2>/dev/null; then
  nginx -t >/dev/null 2>&1 || die "nginx -t failed after certificate renewal — NOT reloading nginx with a broken config."
  systemctl reload nginx || die "failed to reload nginx after certificate renewal."
  log "nginx reloaded with the renewed certificate."
fi

log "certificate renewal deploy hook completed successfully for $public_host."
