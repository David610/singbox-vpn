#!/usr/bin/env bash
# Complete, one-command removal of everything vpn1 (deploy/almalinux/
# install.sh) created or changed on this host. Default behavior removes
# EVERYTHING vpn1 owns — secrets, users, REALITY/Hysteria2 material,
# source tree, generated configs, binaries, the sing-box binary/LICENSE
# (when vpn1 installed it), /opt/vpn1, /etc/vpn, vpn1 state under
# /var/lib/vpn1, systemd units/timers, nginx config, the certbot
# renewal hook, vpn1-created certificates, firewall rules, and any
# packages/services/kernel tuning vpn1 introduced. No follow-up flags,
# manual rm, or separate purge command are needed — this is the ONE
# uninstall command (see docs/ALMALINUX_DEPLOYMENT.md).
#
# Ownership-aware by design (deploy/lib/ownership.sh): anything that
# already existed on this host BEFORE vpn1 touched it — nginx, certbot,
# firewalld/ufw, a Rust toolchain, a pre-existing sing-box binary, system
# users/groups, SELinux booleans, kernel sysctls, firewall rules — is
# preserved or restored to its prior state instead of blindly deleted.
# Cloud-provider Security Groups cannot be modified by this script; see
# the printed checklist at the end.
#
# Idempotent: safe to run more than once. Re-running after a successful
# uninstall exits 0 and reports nothing left to remove.
set -uo pipefail

[ "$(id -u)" -eq 0 ] || { echo "must run as root" >&2; exit 1; }

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
log() { echo "[uninstall] $*"; }
warn() { echo "[uninstall] WARNING: $*" >&2; }
die() { echo "[uninstall] ERROR: $*" >&2; exit 1; }

# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/perf-tuning.sh"
# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/ownership.sh"
# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/preflight.sh"

# This script (and everything it execs, e.g. vpn-admin) runs as root
# and is normally reached via the stable /opt/vpn1/bin/vpn1-uninstall
# entry point — refuse to trust a copy that is not itself root-controlled.
# This is a local-tampering defense-in-depth check, not a full integrity
# guarantee (a fully malicious replacement of this very file could
# remove the check too) — it catches the more likely failure mode of a
# loosened directory permission (e.g. a misconfigured umask at install
# time) rather than deliberate compromise.
assert_root_controlled_path() {
  local path="$1" label="$2" owner mode group_digit other_digit
  [ -e "$path" ] || return 0
  owner="$(stat -c '%u' "$path" 2>/dev/null || echo -1)"
  mode="$(stat -c '%a' "$path" 2>/dev/null || echo 000)"
  [ "$owner" = "0" ] || die "refusing to run: $label ($path) is not owned by root (uid $owner) — this must be root-controlled before running an uninstaller that deletes state as root."
  # Check the write bit (value 2) of the GROUP and OTHER octal digits
  # independently — a trailing-character-only check (e.g. `*[2367]`)
  # misses modes like 775 (group write set, other=5 not matching), which
  # is a real, common mode for a shared group, not just a hypothetical.
  group_digit="${mode: -2:1}"
  other_digit="${mode: -1:1}"
  if [ $(( group_digit & 2 )) -ne 0 ] || [ $(( other_digit & 2 )) -ne 0 ]; then
    die "refusing to run: $label ($path) is group- or world-writable (mode $mode) — this must not be writable by anyone but root before running an uninstaller that deletes state as root."
  fi
}
# Only enforced for the canonical persistent-install location this check
# exists to protect (reached via the untrusted "look for /opt/vpn1"
# fallback in bin/vpn1-uninstall / the online bootstrap) — NOT for an
# operator/CI explicitly invoking this exact script from a git checkout,
# dev clone, or test sandbox. Those are already an explicit, trusted
# invocation with no path-discovery ambiguity to defend against, and
# routinely are not root-owned (e.g. a CI runner checks out the repo as
# an unprivileged user, then re-invokes this script via sudo).
if [ "$REPO_ROOT" = "/opt/vpn1" ]; then
  assert_root_controlled_path "$REPO_ROOT" "vpn1 install directory"
  assert_root_controlled_path "${BASH_SOURCE[0]}" "this uninstaller script"
fi

STATE_DIR_ROOT="/var/lib/vpn1"
FIREWALL_OWNERSHIP="$STATE_DIR_ROOT/firewall-owned.env"

# ---------------------------------------------------------------------
# Truthful completion (checkpoint 2): cleanup below keeps going after a
# non-fatal step fails (this script deliberately has no `-e`) instead of
# aborting on the first warning — but the final banner must never claim
# "complete" while security/runtime-relevant vpn1 state is still on this
# host. CRITICAL_RESIDUE items make the final exit status nonzero and
# print "UNINSTALL INCOMPLETE"; NONCRITICAL_RESIDUE items (a package the
# package manager refused to remove, an ambiguous pre-existing fixed
# path left untouched, etc.) are reported but do not fail the run.
# ---------------------------------------------------------------------
CRITICAL_RESIDUE=()
NONCRITICAL_RESIDUE=()

# Restores or removes a FIXED-name system path vpn1 may have installed
# over (a systemd unit, the certbot hook, the nginx vhost — anything
# install.sh wrote via install_fixed_path_with_ownership(), or the
# equivalent inline nginx tracking). $1 = the actual path on disk, $2 =
# the ownership-record KEY used at install time. Restores the exact
# pre-vpn1 backup when the path is known to have pre-existed vpn1;
# removes it when vpn1 is known to have created it; and — for a
# pre-checkpoint-2 install with no ownership record for this key at all
# — defaults to LEAVING the path in place untouched rather than
# guessing, since it might predate vpn1 and there is no way to prove
# otherwise on such a host.
restore_or_remove_fixed_path() {
  local path="$1" key="$2"
  [ -e "$path" ] || return 0
  local pre_existed
  pre_existed="$(ownership_get "FIXEDPATH_${key}_PRE_EXISTED" "")"
  case "$pre_existed" in
    0)
      rm -f "$path"
      note_removed
      ;;
    1)
      local backup
      backup="$(ownership_get "FIXEDPATH_${key}_BACKUP" "")"
      if [ -n "$backup" ] && ownership_path_is_safe "$backup" && [ -f "$backup" ]; then
        cp -a "$backup" "$path"
        rm -f "$backup"
        log "restored pre-existing $path to its state before vpn1 touched it."
      else
        warn "$path pre-existed vpn1 but no valid backup was recorded — leaving the current (vpn1-written) file in place rather than deleting something that predates vpn1. Inspect manually."
        NONCRITICAL_RESIDUE+=("$path (pre-existing file could not be restored to its original content — vpn1's version left in place)")
      fi
      note_removed
      ;;
    *)
      warn "cannot determine whether $path pre-existed vpn1 (no ownership record — likely a pre-checkpoint-2 install) — leaving it in place rather than guessing. Remove manually if you know it is vpn1's."
      NONCRITICAL_RESIDUE+=("$path (ambiguous ownership — no record predates checkpoint 2 — left in place)")
      ;;
  esac
}

# Package names loaded from the ownership manifest must never reach the
# package manager unvalidated (checkpoint-2 requirement): a corrupted or
# hand-edited ownership.env must not be able to smuggle something
# unexpected into `dnf remove`/`apt-get remove`'s argument list.
is_safe_pkg_name() {
  [[ "$1" =~ ^[A-Za-z0-9][A-Za-z0-9._+-]*$ ]]
}

ASSUME_YES=0
for arg in "$@"; do
  case "$arg" in
    --yes) ASSUME_YES=1 ;;
    --purge-state|--purge-firewall)
      log "'$arg' is no longer needed — complete removal (including state and firewall rules) is now the default. Ignoring."
      ;;
    -h|--help)
      cat <<'USAGE'
vpn1 uninstaller — removes EVERYTHING vpn1 created, completely, by
default. No other flags are required for a full removal.

  sudo /opt/vpn1/bin/vpn1-uninstall --yes

Options:
  --yes   skip the interactive confirmation prompt (required when no
          terminal is attached, e.g. non-interactive automation).

Online fallback (only if the local copy at /opt/vpn1 is missing):
  curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/uninstall.sh | sudo bash
USAGE
      exit 0 ;;
    *) die "unknown flag: $arg" ;;
  esac
done

# Irreversible and deletes user/credential state — require explicit
# confirmation. --yes skips the prompt for scripted/automated use; an
# interactive run without it must positively confirm on /dev/tty rather
# than silently proceeding (this is a deletion of live user credentials
# and REALITY/Hysteria2 secrets, not a reversible operation).
if [ "$ASSUME_YES" -ne 1 ]; then
  if [ -r /dev/tty ] && [ -w /dev/tty ]; then
    reply=""
    printf '\nThis will completely remove vpn1 from this host: all users, credentials,\nREALITY/Hysteria2 secrets, generated config, and vpn1-owned firewall/\npackage/certificate state. This cannot be undone.\n' >/dev/tty
    printf 'Continue? [y/N] ' >/dev/tty
    IFS= read -r reply </dev/tty || reply=""
    case "$reply" in
      y|Y|yes|YES|Yes) ;;
      *) die "aborted — nothing was changed. Re-run with --yes to skip this prompt." ;;
    esac
  else
    die "no terminal attached to confirm this irreversible removal, and --yes was not given. Re-run with --yes (sudo /opt/vpn1/bin/vpn1-uninstall --yes) to proceed non-interactively."
  fi
fi

REMOVED_ANYTHING=0
note_removed() { REMOVED_ANYTHING=1; }

log "stopping and disabling vpn1 services..."
for unit in sing-box.service vpn-subscription.service vpn-expiry-reconcile.timer vpn-expiry-reconcile.service; do
  if systemctl is-enabled --quiet "$unit" 2>/dev/null || systemctl is-active --quiet "$unit" 2>/dev/null; then
    systemctl disable --now "$unit" >/dev/null 2>&1 || true
    note_removed
  fi
done

log "removing/restoring vpn1 systemd units..."
restore_or_remove_fixed_path /etc/systemd/system/sing-box.service SINGBOX_UNIT
restore_or_remove_fixed_path /etc/systemd/system/vpn-subscription.service VPNSUB_UNIT
restore_or_remove_fixed_path /etc/systemd/system/vpn-expiry-reconcile.service EXPIRY_SVC_UNIT
restore_or_remove_fixed_path /etc/systemd/system/vpn-expiry-reconcile.timer EXPIRY_TIMER_UNIT
systemctl daemon-reload
systemctl reset-failed sing-box.service vpn-subscription.service vpn-expiry-reconcile.timer vpn-expiry-reconcile.service >/dev/null 2>&1 || true

log "removing installed binaries..."
for f in vpn-admin vpn vpn-subscription-svc vpn-health-check vpn-benchmark vpn-benchmark-lib.sh; do
  if [ -e "/usr/local/bin/$f" ]; then
    rm -f "/usr/local/bin/$f"
    note_removed
  fi
done
if [ "$(ownership_get SINGBOX_BIN_PRE_EXISTED "0")" = "1" ]; then
  log "sing-box binary pre-dated vpn1 (or ownership could not be determined) — leaving /usr/local/bin/sing-box in place."
else
  if [ -e /usr/local/bin/sing-box ]; then rm -f /usr/local/bin/sing-box; note_removed; fi
  if [ -e /usr/local/bin/sing-box.LICENSE ]; then rm -f /usr/local/bin/sing-box.LICENSE; note_removed; fi
fi

# Leaving the deploy hook installed breaks EVERY future `certbot renew`
# on this host even after the rest of vpn1 is gone: its own guards would
# still pass, so it would run and then fail trying to reload a unit this
# script just deleted, making certbot report the whole renewal as failed
# — including for certificates that have nothing to do with vpn1. In
# every realistic case this fixed, vpn1-specific filename was created by
# vpn1 (FIXEDPATH_CERTBOT_HOOK_PRE_EXISTED=0) and is simply removed; the
# ownership-aware helper only preserves it in the (essentially
# theoretical) case where something else already occupied this exact
# path before vpn1 ever ran.
restore_or_remove_fixed_path /etc/letsencrypt/renewal-hooks/deploy/vpn1-hysteria.sh CERTBOT_HOOK

log "removing vpn1-issued certificates..."
CERT_LINEAGES="$(ownership_list_get CERT_LINEAGES_CREATED_BY_VPN1)"
if [ -n "$CERT_LINEAGES" ]; then
  for host in $CERT_LINEAGES; do
    # Re-validate before using a manifest-sourced value destructively —
    # a corrupted/hand-edited ownership.env must never turn into broad
    # rm -rf behavior (requirement: refuse obviously unsafe entries).
    if ! preflight_validate_hostname "$host" "CERT_LINEAGES_CREATED_BY_VPN1 entry" >/dev/null 2>&1; then
      warn "skipping certificate-lineage removal for a manifest entry that is not a valid hostname ('$host') — leaving it untouched. This indicates a corrupted ownership record; the certificate must be checked/removed manually if it is actually vpn1's."
      continue
    fi
    if [ -d "/etc/letsencrypt/live/$host" ] || [ -f "/etc/letsencrypt/renewal/$host.conf" ]; then
      if command -v certbot >/dev/null 2>&1; then
        certbot delete --cert-name "$host" --non-interactive >/dev/null 2>&1 \
          || warn "could not cleanly 'certbot delete' the lineage for $host — removing its files directly."
      fi
      rm -rf "/etc/letsencrypt/live/$host" "/etc/letsencrypt/archive/$host"
      rm -f "/etc/letsencrypt/renewal/$host.conf"
      note_removed
      log "removed vpn1-issued certificate lineage for $host."
    fi
  done
else
  log "no vpn1-issued certificate lineages recorded (either none were issued, or this host predates ownership tracking — pre-existing/unrelated certificates are always left untouched)."
fi

# Restore certbot to its pre-vpn1 state: uninstalled if vpn1 installed
# it on an otherwise-clean host, or its prior timer-enablement state if
# it already existed.
if command -v certbot >/dev/null 2>&1; then
  timer_unit="$(ownership_get CERTBOT_TIMER_UNIT "")"
  timer_pre_enabled="$(ownership_get CERTBOT_TIMER_PRE_ENABLED "1")"
  if [ -n "$timer_unit" ] && [ "$timer_pre_enabled" = "0" ]; then
    systemctl disable --now "$timer_unit" >/dev/null 2>&1 || true
    log "disabled $timer_unit (was not enabled before vpn1)."
    note_removed
  fi
fi

log "removing/restoring vpn1 nginx configuration..."
if [ -e /etc/nginx/conf.d/vpn-subscription.conf ]; then
  restore_or_remove_fixed_path /etc/nginx/conf.d/vpn-subscription.conf NGINX_CONF
  rm -f /etc/nginx/conf.d/vpn-subscription.conf.install-bak /etc/nginx/conf.d/vpn-subscription.conf.tmp
  if command -v nginx >/dev/null 2>&1 && nginx -t >/dev/null 2>&1; then
    systemctl reload nginx 2>/dev/null || true
  fi
fi
if command -v nginx >/dev/null 2>&1 && [ "$(ownership_get NGINX_PRE_INSTALLED "1")" = "0" ]; then
  # vpn1 installed nginx on an otherwise-clean host — restore its prior
  # enablement state (also "0", since it did not exist at all) by
  # stopping/disabling it; the package itself is removed in the package
  # cleanup step below.
  systemctl disable --now nginx >/dev/null 2>&1 || true
elif command -v nginx >/dev/null 2>&1 && [ "$(ownership_get NGINX_PRE_ENABLED "1")" = "0" ]; then
  systemctl disable nginx >/dev/null 2>&1 || true
  log "restored nginx to its pre-vpn1 (not enabled) state."
fi

# ---------------------------------------------------------------------
# Kernel network tuning: use the SAME rollback machinery install.sh used
# to apply it (deploy/lib/perf-tuning.sh) — restores the captured
# pre-vpn1 live sysctl values without a reboot, then removes vpn1's own
# sysctl drop-ins.
# ---------------------------------------------------------------------
log "restoring kernel network tuning to its pre-vpn1 state..."
if [ -f "$PERF_BASELINE_FILE" ]; then
  if perf_tuning_rollback; then
    log "kernel network values restored to their pre-vpn1 baseline."
  else
    warn "kernel network rollback reported one or more mismatches — see warnings above."
  fi
  rm -f "$PERF_SYSCTL_DROPIN" "$PERF_ROLLBACK_DROPIN" "$PERF_BASELINE_FILE"
  note_removed
else
  log "no perf-tuning baseline found — kernel tuning was never applied, or already rolled back."
fi
# tcp_bbr is left loaded even if vpn1 caused it to load: unloading a
# congestion-control module out from under whatever else on the host may
# now be using it (any connection that negotiated bbr while it was
# available) is not something that can be done safely without knowing
# every current user of it, so this is a deliberate, documented no-op —
# not a gap. It costs nothing to leave loaded (a few KB of kernel
# memory) and 'modprobe -r tcp_bbr' would fail harmlessly anyway if
# anything still references it.

# ---------------------------------------------------------------------
# Firewall: remove exactly the rules vpn1 added (443/tcp+udp,
# subscription port, and — for firewalld — reload takes care of any
# runtime-only leftover ACME TCP/80 rule automatically since that rule
# was never --permanent; ufw's ACME 80/tcp rule IS persistent and is
# handled explicitly below).
# ---------------------------------------------------------------------
log "removing vpn1 firewall rules..."
if [ -f "$FIREWALL_OWNERSHIP" ]; then
  firewall_backend=""
  firewall_zone=""
  subscription_port=""
  owned_443_tcp=0
  owned_443_udp=0
  owned_subscription_tcp=0
  firewall_record_valid=1
  # shellcheck disable=SC1090
  . "$FIREWALL_OWNERSHIP"
  case "$owned_443_tcp:$owned_443_udp:$owned_subscription_tcp" in
    0:0:0|0:0:1|0:1:0|0:1:1|1:0:0|1:0:1|1:1:0|1:1:1) ;;
    *)
      warn "invalid vpn1 firewall ownership record at $FIREWALL_OWNERSHIP — cannot safely determine which rules are vpn1's. Leaving the record and firewall untouched rather than guessing; continuing with the rest of cleanup."
      firewall_record_valid=0
      ;;
  esac
  if [ "$firewall_record_valid" -eq 1 ] && [ "$owned_subscription_tcp" -eq 1 ] && ! [[ "$subscription_port" =~ ^[0-9]+$ ]]; then
    warn "invalid subscription port in vpn1 firewall ownership record — cannot safely remove that rule. Leaving the record and firewall untouched rather than guessing; continuing with the rest of cleanup."
    firewall_record_valid=0
  fi
  if [ "$firewall_record_valid" -ne 1 ]; then
    CRITICAL_RESIDUE+=("vpn1 firewall ownership record at $FIREWALL_OWNERSHIP is corrupted/ambiguous — vpn1-owned firewall rules (if any) could not be safely identified for removal; inspect $FIREWALL_OWNERSHIP and the host firewall manually.")
  elif [ "${firewall_backend:-}" = "firewalld" ] && command -v firewall-cmd >/dev/null 2>&1; then
    if [[ "$firewall_zone" =~ ^[A-Za-z0-9_.-]+$ ]]; then
      reload_needed=0
      if [ "${owned_443_tcp:-0}" -eq 1 ]; then firewall-cmd --zone="$firewall_zone" --permanent --remove-port=443/tcp >/dev/null 2>&1 && reload_needed=1; fi
      if [ "${owned_443_udp:-0}" -eq 1 ]; then firewall-cmd --zone="$firewall_zone" --permanent --remove-port=443/udp >/dev/null 2>&1 && reload_needed=1; fi
      if [ "${owned_subscription_tcp:-0}" -eq 1 ]; then firewall-cmd --zone="$firewall_zone" --permanent --remove-port="${subscription_port}/tcp" >/dev/null 2>&1 && reload_needed=1; fi
      # A killed install can leave a runtime-only (non-permanent) TCP/80
      # ACME rule behind; it is not in the permanent ruleset so it never
      # survives the reload below regardless, but remove it explicitly
      # too in case it somehow persisted.
      firewall-cmd --zone="$firewall_zone" --remove-port=80/tcp >/dev/null 2>&1 || true
      if [ "$reload_needed" -eq 1 ] && ! firewall-cmd --reload >/dev/null 2>&1; then
        warn "firewall-cmd --reload failed after removing vpn1's rules — rules may not be fully applied until the next reload/reboot."
      fi
      # Verify the removal actually took effect (checkpoint-2 residue
      # check: exposure vpn1 introduced must actually be gone, not just
      # "a removal command was attempted").
      if [ "${owned_443_tcp:-0}" -eq 1 ] && firewall-cmd --zone="$firewall_zone" --query-port=443/tcp >/dev/null 2>&1; then
        CRITICAL_RESIDUE+=("firewalld still allows 443/tcp (vpn1-owned rule) after removal was attempted")
      fi
      if [ "${owned_443_udp:-0}" -eq 1 ] && firewall-cmd --zone="$firewall_zone" --query-port=443/udp >/dev/null 2>&1; then
        CRITICAL_RESIDUE+=("firewalld still allows 443/udp (vpn1-owned rule) after removal was attempted")
      fi
      if [ "${owned_subscription_tcp:-0}" -eq 1 ] && firewall-cmd --zone="$firewall_zone" --query-port="${subscription_port}/tcp" >/dev/null 2>&1; then
        CRITICAL_RESIDUE+=("firewalld still allows ${subscription_port}/tcp (vpn1-owned subscription rule) after removal was attempted")
      fi
      note_removed
    else
      warn "invalid firewalld zone in ownership record — skipping firewalld rule removal."
      CRITICAL_RESIDUE+=("vpn1 firewall ownership record names an invalid firewalld zone — vpn1-owned rules (if any) could not be removed; inspect the host firewall manually.")
    fi
  elif [ "${firewall_backend:-}" = "ufw" ] && command -v ufw >/dev/null 2>&1; then
    [ "${owned_443_tcp:-0}" -eq 1 ] && { ufw delete allow 443/tcp >/dev/null 2>&1 || true; }
    [ "${owned_443_udp:-0}" -eq 1 ] && { ufw delete allow 443/udp >/dev/null 2>&1 || true; }
    [ "${owned_subscription_tcp:-0}" -eq 1 ] && { ufw delete allow "${subscription_port}/tcp" >/dev/null 2>&1 || true; }
    # ufw's temporary ACME TCP/80 rule (install.sh's
    # firewall_open_port_80_temp) IS persistent, unlike firewalld's — a
    # `kill -9`'d install can leave `80/tcp ALLOW` behind permanently.
    # Only remove it if it's currently present AND vpn1 does not also
    # need it for anything else (it never does outside the brief ACME
    # window), so this is always safe to attempt.
    ufw status 2>/dev/null | grep -Eq '^80/tcp[[:space:]]+ALLOW' \
      && { ufw delete allow 80/tcp >/dev/null 2>&1 || true; log "removed a leftover ACME TCP/80 rule."; }
    if [ "${owned_443_tcp:-0}" -eq 1 ] && ufw status 2>/dev/null | grep -Eq '^443/tcp[[:space:]]+ALLOW'; then
      CRITICAL_RESIDUE+=("ufw still allows 443/tcp (vpn1-owned rule) after removal was attempted")
    fi
    if [ "${owned_443_udp:-0}" -eq 1 ] && ufw status 2>/dev/null | grep -Eq '^443/udp[[:space:]]+ALLOW'; then
      CRITICAL_RESIDUE+=("ufw still allows 443/udp (vpn1-owned rule) after removal was attempted")
    fi
    if [ "${owned_subscription_tcp:-0}" -eq 1 ] && ufw status 2>/dev/null | grep -Eq "^${subscription_port}/tcp[[:space:]]+ALLOW"; then
      CRITICAL_RESIDUE+=("ufw still allows ${subscription_port}/tcp (vpn1-owned subscription rule) after removal was attempted")
    fi
    note_removed
  fi
  [ "$firewall_record_valid" -eq 1 ] && rm -f "$FIREWALL_OWNERSHIP"
else
  log "no vpn1 firewall ownership record found — either firewall rules were never added, or this uninstall already ran."
fi

# Restore firewalld/ufw's own prior enabled/active state if vpn1 was the
# one that turned it on for a previously-clean host.
if command -v firewall-cmd >/dev/null 2>&1; then
  if [ "$(ownership_get FIREWALLD_PRE_INSTALLED "1")" = "0" ] || [ "$(ownership_get FIREWALLD_PRE_ENABLED "1")" = "0" ]; then
    systemctl disable --now firewalld >/dev/null 2>&1 || true
    log "restored firewalld to its pre-vpn1 (not enabled) state."
    note_removed
  fi
fi
if command -v ufw >/dev/null 2>&1 && [ "$(ownership_get UFW_PRE_ENABLED "1")" = "0" ]; then
  ufw --force disable >/dev/null 2>&1 || true
  log "restored ufw to its pre-vpn1 (not enabled) state."
  note_removed
fi

# ---------------------------------------------------------------------
# SELinux: restore the httpd_can_network_connect boolean to its captured
# pre-vpn1 value, and drop the fcontext rules vpn1 added (these name
# only vpn1's own paths, so they are always vpn1-owned when present).
# ---------------------------------------------------------------------
if command -v semanage >/dev/null 2>&1; then
  if [ "$(ownership_is_marked SELINUX_FCONTEXT_RULES_ADDED)" = "1" ]; then
    semanage fcontext -d "/usr/local/bin/sing-box" 2>/dev/null || true
    semanage fcontext -d "/etc/vpn/compat/sing-box(/.*)?" 2>/dev/null || true
    semanage fcontext -d "/etc/vpn/compat/hysteria(/.*)?" 2>/dev/null || true
    note_removed
  fi
  ports_added="$(ownership_list_get SELINUX_PORT_LABELS_ADDED)"
  for portspec in $ports_added; do
    port="${portspec%%/*}" proto="${portspec##*/}"
    semanage port -d -t http_port_t -p "$proto" "$port" 2>/dev/null || true
    note_removed
  done
fi
if command -v setsebool >/dev/null 2>&1; then
  prior="$(ownership_get SELINUX_HTTPD_NETCONNECT_PRE "")"
  case "$prior" in
    off) setsebool -P httpd_can_network_connect 0 2>/dev/null && log "restored SELinux boolean httpd_can_network_connect to its pre-vpn1 value (off)." && note_removed ;;
    on) ;; # was already on before vpn1 — leave it
    ""|unknown) ;; # never recorded — do not guess, leave as-is
  esac
fi

# ---------------------------------------------------------------------
# Service users/groups: delete only what vpn1 created. If they
# pre-existed, leave them (and their vpn-compat group membership) alone
# — removing a pre-existing operator account would be destructive far
# beyond vpn1's own footprint.
# ---------------------------------------------------------------------
log "removing vpn1-created service accounts..."
if [ "$(ownership_is_marked USER_SINGBOX_CREATED)" = "1" ] && id sing-box >/dev/null 2>&1; then
  if ! userdel sing-box >/dev/null 2>&1; then
    warn "could not remove user 'sing-box'."
    NONCRITICAL_RESIDUE+=("system user 'sing-box' (vpn1-created) could not be removed — likely still has a running process; retry after 'pkill -u sing-box'")
  fi
  note_removed
fi
if [ "$(ownership_is_marked USER_VPNSUB_CREATED)" = "1" ] && id vpn-subscription >/dev/null 2>&1; then
  if ! userdel vpn-subscription >/dev/null 2>&1; then
    warn "could not remove user 'vpn-subscription'."
    NONCRITICAL_RESIDUE+=("system user 'vpn-subscription' (vpn1-created) could not be removed — likely still has a running process; retry after 'pkill -u vpn-subscription'")
  fi
  note_removed
fi
if [ "$(ownership_is_marked GROUP_VPNCOMPAT_CREATED)" = "1" ] && getent group vpn-compat >/dev/null 2>&1; then
  if ! groupdel vpn-compat >/dev/null 2>&1; then
    warn "could not remove group 'vpn-compat' (a pre-existing user may still be a member)."
    NONCRITICAL_RESIDUE+=("group 'vpn-compat' (vpn1-created) could not be removed")
  fi
  note_removed
fi

# ---------------------------------------------------------------------
# Rust toolchain: remove ONLY if vpn1 installed it (i.e. no toolchain
# was already present before install.sh ran rustup-init).
# ---------------------------------------------------------------------
if [ "$(ownership_is_marked RUSTUP_INSTALLED_BY_VPN1)" = "1" ]; then
  rustup_home="$(ownership_get RUSTUP_HOME_DIR "/root")"
  if ! ownership_path_is_safe "$rustup_home"; then
    warn "RUSTUP_HOME_DIR in the ownership record ('$rustup_home') is not a safe absolute path — skipping Rust toolchain removal. This indicates a corrupted ownership record; check/remove it manually if it is actually vpn1's."
  elif [ -d "$rustup_home/.rustup" ] || [ -d "$rustup_home/.cargo" ]; then
    log "removing the Rust toolchain vpn1 installed ($rustup_home/.rustup, $rustup_home/.cargo)..."
    rm -rf "$rustup_home/.rustup" "$rustup_home/.cargo"
    note_removed
  fi
else
  log "Rust toolchain was pre-existing (or never installed by vpn1) — leaving it in place."
fi

# ---------------------------------------------------------------------
# Packages: remove only packages that were ABSENT before vpn1 installed
# them. Best-effort — if a package is still required by something else
# the package manager will report that and this continues rather than
# forcing removal.
# ---------------------------------------------------------------------
pkgs_owned_raw="$(ownership_list_get PKGS_INSTALLED_BY_VPN1)"
pkgs_owned=""
for pkg in $pkgs_owned_raw; do
  if is_safe_pkg_name "$pkg"; then
    pkgs_owned="${pkgs_owned:+$pkgs_owned }$pkg"
  else
    warn "skipping invalid/unsafe package name in ownership record: '$pkg' — this indicates a corrupted ownership.env; check/remove it manually if it is actually vpn1's."
    NONCRITICAL_RESIDUE+=("ownership record contained an invalid package name ('$pkg') — skipped, not passed to the package manager")
  fi
done
if [ -n "$pkgs_owned" ]; then
  log "removing packages vpn1 installed that were not present before: $pkgs_owned"
  if command -v dnf >/dev/null 2>&1; then
    # shellcheck disable=SC2086
    if ! dnf remove -y $pkgs_owned >/dev/null 2>&1; then
      warn "some vpn1-installed packages could not be removed automatically (likely still depended on by something else) — leaving them; check with: dnf list installed $pkgs_owned"
      NONCRITICAL_RESIDUE+=("package(s) vpn1 installed could not be removed (dnf refused, likely a dependency of unrelated software): $pkgs_owned")
    fi
  elif command -v apt-get >/dev/null 2>&1; then
    export DEBIAN_FRONTEND=noninteractive
    # shellcheck disable=SC2086
    if ! apt-get remove -y $pkgs_owned >/dev/null 2>&1; then
      warn "some vpn1-installed packages could not be removed automatically — leaving them; check with: dpkg -l $pkgs_owned"
      NONCRITICAL_RESIDUE+=("package(s) vpn1 installed could not be removed (apt-get refused, likely a dependency of unrelated software): $pkgs_owned")
    fi
  fi
  note_removed
else
  log "no vpn1-installed packages recorded (either none were needed, or this host predates ownership tracking)."
fi

# ---------------------------------------------------------------------
# vpn1 state, config and secrets. Complete removal is the default — no
# --purge-state flag needed (a deliberate change: destroying REALITY/
# Hysteria2 secrets and user credentials is the whole point of a
# COMPLETE uninstall, not an opt-in extra).
# ---------------------------------------------------------------------
# /etc/vpn is a SHARED PARENT, not a resource vpn1 can prove it created
# by its mere existence — checkpoint-2 requirement: never rm -rf a
# shared parent without proven ownership. ETC_VPN_PRE_EXISTED (recorded
# once, before install.sh's create_directories() ever touched it) is the
# only basis for the decision:
#   0 (vpn1 created the whole tree)   -> rm -rf the entire directory.
#   1 (something else already had it) -> remove only vpn1's own known
#                                          children (deployment.toml,
#                                          the compat/ state tree),
#                                          leaving everything else
#                                          untouched.
#   unset (pre-checkpoint-2 install,
#          no record either way)      -> default to preservation: same
#                                          vpn1-only-children removal as
#                                          the pre-existing case, never
#                                          the blind rm -rf.
if [ -d /etc/vpn ]; then
  etc_vpn_pre_existed="$(ownership_get ETC_VPN_PRE_EXISTED "")"
  case "$etc_vpn_pre_existed" in
    0)
      log "removing /etc/vpn (vpn1 created this directory; contains the REALITY private key, Hysteria2 material, all user credentials, deployment.toml)..."
      rm -rf /etc/vpn
      note_removed
      ;;
    1)
      log "/etc/vpn pre-existed before vpn1 — removing only vpn1's own entries (deployment.toml, compat/), preserving everything else..."
      [ -e /etc/vpn/deployment.toml ] && { rm -f /etc/vpn/deployment.toml; note_removed; }
      [ -e /etc/vpn/compat ] && { rm -rf /etc/vpn/compat; note_removed; }
      if [ -n "$(ls -A /etc/vpn 2>/dev/null)" ]; then
        log "/etc/vpn still contains non-vpn1 content that pre-dated vpn1 — left in place, as intended."
      fi
      ;;
    *)
      warn "cannot determine whether /etc/vpn pre-existed vpn1 (no ownership record — this host predates checkpoint 2's ownership tracking) — defaulting to preservation: removing only vpn1's known entries (deployment.toml, compat/), never the directory itself."
      [ -e /etc/vpn/deployment.toml ] && { rm -f /etc/vpn/deployment.toml; note_removed; }
      [ -e /etc/vpn/compat ] && { rm -rf /etc/vpn/compat; note_removed; }
      if [ -n "$(ls -A /etc/vpn 2>/dev/null)" ]; then
        NONCRITICAL_RESIDUE+=("/etc/vpn still exists with ambiguous ownership (pre-checkpoint-2 install) — preserved rather than guessed at; review manually")
      fi
      ;;
  esac
fi
# Read every ownership fact the rest of this script (including the
# residue check below) still needs BEFORE the manifest itself (under
# $STATE_DIR_ROOT) is removed a few lines down — reading any of these
# AFTER deleting $STATE_DIR_ROOT would silently get back only
# ownership_get's default value (the file is gone), which previously
# made /opt/vpn1 ALWAYS look "not pre-existing" (and so always get
# deleted, even on the rare host where it genuinely pre-existed vpn1 —
# e.g. an operator's own clone placed there before ever running
# install.sh) and would make a correctly-RESTORED pre-existing fixed
# path (systemd unit/nginx conf) look like unexplained residue instead
# of the intended, successfully-restored end state. Cache everything
# needed first instead.
opt_vpn1_pre_existed="$(ownership_get OPT_VPN1_PRE_EXISTED "0")"
declare -A vpn1_unit_keys=(
  [/etc/systemd/system/sing-box.service]=SINGBOX_UNIT
  [/etc/systemd/system/vpn-subscription.service]=VPNSUB_UNIT
  [/etc/systemd/system/vpn-expiry-reconcile.service]=EXPIRY_SVC_UNIT
  [/etc/systemd/system/vpn-expiry-reconcile.timer]=EXPIRY_TIMER_UNIT
)
declare -A vpn1_unit_pre_existed
for unit_path in "${!vpn1_unit_keys[@]}"; do
  vpn1_unit_pre_existed[$unit_path]="$(ownership_get "FIXEDPATH_${vpn1_unit_keys[$unit_path]}_PRE_EXISTED" "0")"
done

if [ -d "$STATE_DIR_ROOT" ]; then
  log "removing vpn1 state under $STATE_DIR_ROOT..."
  rm -rf "$STATE_DIR_ROOT"
  note_removed
fi
rm -f /tmp/vpn1-sysctl-system.out /tmp/vpn1-sysctl-rollback.out /run/lock/vpn1-installer.lock 2>/dev/null || true

# The persistent source tree — remove it last, since this script itself
# very likely lives inside it (/opt/vpn1/deploy/almalinux/uninstall.sh).
# bash has already read this whole file into memory before executing
# any of it, so removing the directory out from under the running
# process is safe; nothing below this point reads from $REPO_ROOT again.
if [ -d /opt/vpn1 ]; then
  if [ "$opt_vpn1_pre_existed" = "1" ]; then
    log "/opt/vpn1 pre-existed before vpn1 (unexpected — leaving it in place; inspect it manually if this is unexpected)."
  else
    log "removing /opt/vpn1 (persistent vpn1 source tree)..."
    rm -rf /opt/vpn1
    note_removed
  fi
fi

# ---------------------------------------------------------------------
# Residue verification (checkpoint 2): the single authoritative check
# that decides whether this run may honestly claim "complete". Runs
# AFTER all cleanup above has been attempted (this script never aborts
# on the first non-fatal failure — see CRITICAL_RESIDUE/
# NONCRITICAL_RESIDUE above) so one early problem never hides the rest
# of the report. Only security/runtime-relevant vpn1 state (active
# services, live secrets/credentials, vpn1 binaries, vpn1-owned
# firewall exposure, a vpn1-created /opt/vpn1 that should be gone) makes
# the run print INCOMPLETE and exit nonzero; everything else already
# collected above (a package the package manager refused to remove, an
# ambiguous pre-existing fixed path left alone, a userdel that failed)
# is reported as non-critical and does not change the exit status.
# ---------------------------------------------------------------------
for unit in sing-box.service vpn-subscription.service vpn-expiry-reconcile.timer vpn-expiry-reconcile.service; do
  systemctl is-active --quiet "$unit" 2>/dev/null && CRITICAL_RESIDUE+=("$unit is still active")
done
for unit_path in "${!vpn1_unit_keys[@]}"; do
  if [ -e "$unit_path" ] && [ "${vpn1_unit_pre_existed[$unit_path]}" = "0" ]; then
    CRITICAL_RESIDUE+=("$unit_path still present (vpn1-created)")
  fi
done
pgrep -x sing-box >/dev/null 2>&1 && CRITICAL_RESIDUE+=("a sing-box process is still running")
pgrep -f 'vpn-subscription-svc' >/dev/null 2>&1 && CRITICAL_RESIDUE+=("a vpn-subscription-svc process is still running")
for f in vpn-admin vpn vpn-subscription-svc; do
  [ -e "/usr/local/bin/$f" ] && CRITICAL_RESIDUE+=("/usr/local/bin/$f still present")
done
[ -e /etc/vpn/deployment.toml ] && CRITICAL_RESIDUE+=("/etc/vpn/deployment.toml still present")
[ -e /etc/vpn/compat/reality/private.key ] && CRITICAL_RESIDUE+=("REALITY private key still present (/etc/vpn/compat/reality/private.key)")
[ -e /etc/vpn/compat/reality/hysteria_obfs_password.txt ] && CRITICAL_RESIDUE+=("Hysteria2 obfuscation credential still present")
[ -d /etc/vpn/compat/users ] && [ -n "$(ls -A /etc/vpn/compat/users 2>/dev/null)" ] && CRITICAL_RESIDUE+=("/etc/vpn/compat/users still contains user credential files")
[ -e "$FIREWALL_OWNERSHIP" ] && CRITICAL_RESIDUE+=("vpn1 firewall ownership record still present at $FIREWALL_OWNERSHIP")
if [ -d /opt/vpn1 ] && [ "$opt_vpn1_pre_existed" != "1" ]; then
  CRITICAL_RESIDUE+=("/opt/vpn1 still present (vpn1-created)")
fi

if [ "${#CRITICAL_RESIDUE[@]}" -eq 0 ]; then
  if [ "${#NONCRITICAL_RESIDUE[@]}" -gt 0 ]; then
    log "UNINSTALL COMPLETE WITH SAFE RETAINED DEPENDENCIES/AMBIGUOUS ITEMS — critical vpn1 state is gone; see retained items below."
  elif [ "$REMOVED_ANYTHING" -eq 1 ]; then
    log "UNINSTALL COMPLETE — vpn1 and everything it created have been removed."
  else
    log "UNINSTALL COMPLETE — nothing to remove (vpn1 is not installed, or was already fully uninstalled)."
  fi
  final_rc=0
else
  echo "[uninstall] UNINSTALL INCOMPLETE — manual cleanup required. Critical vpn1 state remains:" >&2
  for item in "${CRITICAL_RESIDUE[@]}"; do
    echo "[uninstall]   - $item" >&2
  done
  final_rc=1
fi
if [ "${#NONCRITICAL_RESIDUE[@]}" -gt 0 ]; then
  log "Non-critical items left behind (safe to ignore, or clean up manually):"
  for item in "${NONCRITICAL_RESIDUE[@]}"; do
    log "  - $item"
  done
fi

cat <<'BANNER'

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 vpn1 uninstall: manual checklist
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
This script cannot see or modify your CLOUD PROVIDER's network-level
firewall (AWS/EC2 Security Groups, GCP firewall rules, Azure NSGs,
etc.) — only this host's own firewall (firewalld/ufw), which it already
restored above. If you opened inbound rules there specifically for
vpn1 (443/tcp, 443/udp, the subscription port), remove them in that
provider's console/CLI yourself.
BANNER

exit "$final_rc"
