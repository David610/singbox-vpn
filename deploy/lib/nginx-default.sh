#!/usr/bin/env bash
# Disable ONLY the unchanged Debian nginx package welcome-site symlink.
# Never remove its conffile, an admin-modified site, or an unrelated :80 site.
# Certbot uses standalone HTTP-01 and its existing temporary firewall hooks.

nginx_disable_packaged_default() {
  local available="${NGINX_DEFAULT_AVAILABLE:-/etc/nginx/sites-available/default}"
  local enabled="${NGINX_DEFAULT_ENABLED:-/etc/nginx/sites-enabled/default}"
  local expected actual target
  export NGINX_DEFAULT_CHANGED_THIS_RUN=0
  [ -L "$enabled" ] && [ -f "$available" ] || return 0
  [ "$(readlink -f "$enabled")" = "$(readlink -f "$available")" ] || return 0
  target="$(readlink "$enabled")"
  # Only known package-link forms; ownership.env never receives arbitrary text.
  case "$target" in "$available"|../sites-available/default) ;; *) return 0;; esac
  expected="$(dpkg-query -W -f='${Conffiles}\n' nginx-common 2>/dev/null | awk '$1 == "/etc/nginx/sites-available/default" {print $2}')" || return 0
  [[ "$expected" =~ ^[[:xdigit:]]{32}$ ]] || return 0
  actual="$(md5sum "$available" | awk '{print $1}')"
  if [ "$actual" != "$expected" ]; then
    warn "nginx default site is administrator-modified; retained. Review its TCP/80 listener separately."
    return 0
  fi
  ownership_set NGINX_DEFAULT_LINK_TARGET "$target"
  ownership_mark NGINX_DEFAULT_DISABLED
  rm -- "$enabled"
  export NGINX_DEFAULT_CHANGED_THIS_RUN=1
}

nginx_restore_packaged_default() {
  local available="${NGINX_DEFAULT_AVAILABLE:-/etc/nginx/sites-available/default}"
  local enabled="${NGINX_DEFAULT_ENABLED:-/etc/nginx/sites-enabled/default}"
  local target
  [ "$(ownership_get NGINX_DEFAULT_DISABLED 0)" = 1 ] || return 0
  # Preserve any new administrator-managed link/file, including a dangling link.
  [ ! -e "$enabled" ] && [ ! -L "$enabled" ] && [ -f "$available" ] || return 0
  target="$(ownership_get NGINX_DEFAULT_LINK_TARGET '')"
  case "$target" in "$available"|../sites-available/default) ;; *) return 0;; esac
  ln -s -- "$target" "$enabled"
}
