#!/usr/bin/env bash
# Shared nginx subscription-vhost rendering and transactional refresh
# (I-45). Sourced by install.sh (render) and update.sh (render + refresh) so a
# template-level fix reaches deployed nodes and the two paths cannot drift.
# Requires log and warn from the caller. Sourced, never executed.

# render_nginx_vhost TEMPLATE SUBSCRIPTION_HOST PUBLIC_HOST SUBSCRIPTION_PORT BACKEND_PORT
render_nginx_vhost() {
  local template="$1" host="$2" public_host="$3" port="$4" backend="$5"
  sed -e "s/{{SUBSCRIPTION_HOST}}/$host/" -e "s/{{PUBLIC_HOST}}/${public_host:-$host}/" \
      -e "s/{{SUBSCRIPTION_PORT}}/$port/" \
      -e "s/{{SUBSCRIPTION_BACKEND_PORT}}/$backend/" \
    "$template"
}

# refresh_nginx_vhost TEMPLATE CONF SUBSCRIPTION_HOST PUBLIC_HOST PORT BACKEND_PORT
# Only ever replaces an EXISTING vhost. No-op when the rendered text is
# identical. Otherwise: keep the exact previous file, install the new one,
# `nginx -t`, reload; if either fails restore the previous file (and reload it)
# and return nonzero. Does not touch SELinux labels, ports, or certificates.
refresh_nginx_vhost() {
  local template="$1" conf="$2" host="$3" public_host="$4" port="$5" backend="$6"
  [ -f "$conf" ] || return 0
  local new="$conf.update-new" bak="$conf.update-bak"
  render_nginx_vhost "$template" "$host" "$public_host" "$port" "$backend" >"$new" || { rm -f "$new"; return 1; }
  if cmp -s "$new" "$conf"; then rm -f "$new"; return 0; fi
  cp -a "$conf" "$bak"
  cat "$new" >"$conf" && rm -f "$new"
  if nginx -t >/dev/null 2>&1 && systemctl reload nginx >/dev/null 2>&1; then
    rm -f "$bak"
    log "nginx subscription vhost refreshed from the release template."
    return 0
  fi
  warn "nginx rejected the refreshed subscription vhost; restoring the previous one."
  cat "$bak" >"$conf" && rm -f "$bak"
  nginx -t >/dev/null 2>&1 && systemctl reload nginx >/dev/null 2>&1 || true
  return 1
}
