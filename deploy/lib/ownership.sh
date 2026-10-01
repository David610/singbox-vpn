#!/usr/bin/env bash
# singbox-vpn ownership manifest — the single source of truth for "did singbox-vpn
# create/change this, or did it already exist on this host before singbox-vpn
# touched it". Sourced by deploy/almalinux/install.sh (writes it,
# incrementally, starting at stage 1 — never only at the end) and
# deploy/almalinux/uninstall.sh (reads it to decide exactly what is safe
# to remove/restore).
#
# Design goals (see the task's uninstall/rollback requirements):
#   - Written incrementally, one fact at a time, as each mutation
#     happens — a crash at ANY stage still leaves an accurate record of
#     everything done so far, not just a snapshot taken at the very end.
#   - Every fact recorded here is either "singbox-vpn created/changed this and
#     therefore owns cleaning it up" or "this already existed before
#     singbox-vpn, so uninstall must leave/restore it". Never guessed.
#   - Plain KEY="value" lines, one per line, values restricted to
#     already-validated tokens (booleans, ports, space-joined package/
#     hostname lists that have already passed preflight_validate_* or
#     are package-manager-safe names) — never raw operator/network input
#     written here unescaped.
#
# Expects log()/warn()/die() to already be defined by the caller (same
# convention as preflight.sh/perf-tuning.sh).

: "${OWNERSHIP_DIR:=/var/lib/singbox-vpn}"
: "${OWNERSHIP_FILE:=$OWNERSHIP_DIR/ownership.env}"

ownership_init() {
  install -d -m 0755 "$OWNERSHIP_DIR"
  if [ ! -f "$OWNERSHIP_FILE" ]; then
    : > "$OWNERSHIP_FILE"
    chmod 0600 "$OWNERSHIP_FILE"
  fi
}

# Set KEY="value" (last write wins), atomically. $2 must already be safe
# to place inside double quotes on the right of `KEY="..."` — this
# module does not itself sanitize it.
ownership_set() {
  local key="$1" value="$2" tmp
  ownership_init
  tmp="$(mktemp "${OWNERSHIP_FILE}.tmp.XXXXXX")"
  if [ -f "$OWNERSHIP_FILE" ]; then
    grep -v -E "^${key}=" "$OWNERSHIP_FILE" > "$tmp" 2>/dev/null || true
  fi
  printf '%s="%s"\n' "$key" "$value" >> "$tmp"
  chmod 0600 "$tmp"
  mv -f "$tmp" "$OWNERSHIP_FILE"
}

ownership_get() {
  local key="$1" default="${2:-}" val
  [ -f "$OWNERSHIP_FILE" ] || { printf '%s' "$default"; return 0; }
  # An absent key is normal.  Keep grep's status out of a command
  # substitution pipeline: under `set -e -o pipefail`, grep(1)'s ordinary
  # "no match" status would otherwise terminate the installer.
  val="$(awk -v key="$key" '
    index($0, key "=\"") == 1 {
      value = substr($0, length(key) + 3)
      sub(/\"$/, "", value)
      found = value
    }
    END { if (found != "") printf "%s", found }
  ' "$OWNERSHIP_FILE" 2>/dev/null || true)"
  if [ -n "$val" ]; then printf '%s' "$val"; else printf '%s' "$default"; fi
  return 0
}

# Boolean fact: once true, callers should never flip it back to false —
# only ever call this to RECORD "singbox-vpn did this", never to un-record it.
ownership_mark() { ownership_set "$1" "1"; }
ownership_is_marked() {
  [ "$(ownership_get "$1" "0")" = "1" ] && printf '%s' "1" || printf '%s' "0"
  return 0
}

# Append a token to a space-separated list-valued key, de-duplicated.
ownership_list_add() {
  local key="$1" token="$2" current
  current="$(ownership_get "$key" "")"
  case " $current " in
    *" $token "*) return 0 ;;
  esac
  ownership_set "$key" "${current:+$current }$token"
}

ownership_list_get() {
  ownership_get "$1" ""
}

# Record a fact only the FIRST time it is observed on this host (i.e. a
# baseline) — never overwritten by a later run, exactly like
# perf_capture_baseline in perf-tuning.sh. Use for "was X already true
# before singbox-vpn ever touched this host", which must reflect the ORIGINAL
# state, not whatever the most recent run happened to see.
ownership_set_baseline_once() {
  local key="$1" value="$2"
  ownership_init
  if grep -qE "^${key}=" "$OWNERSHIP_FILE" 2>/dev/null; then
    return 0
  fi
  ownership_set "$key" "$value"
}

# Install a fixed-name path while preserving the first pre-singbox-vpn
# occupant byte-for-byte. Shared by install and update so a unit introduced by
# a later release has exactly the same uninstall/restore semantics as one
# present on the first installation.
install_fixed_path_with_ownership() {
  local src="$1" dest="$2" key="$3" mode="${4:-0644}"
  local backup_dir="${OWNERSHIP_DIR}/preexisting-backups"
  local backup="$backup_dir/$key"
  if [ -e "$dest" ]; then
    ownership_set_baseline_once "FIXEDPATH_${key}_PRE_EXISTED" "1"
    if [ "$(ownership_get "FIXEDPATH_${key}_BACKED_UP" "0")" != "1" ]; then
      install -d -m 0700 "$backup_dir"
      cp -a "$dest" "$backup"
      ownership_set "FIXEDPATH_${key}_BACKUP" "$backup"
      ownership_mark "FIXEDPATH_${key}_BACKED_UP"
    fi
  else
    ownership_set_baseline_once "FIXEDPATH_${key}_PRE_EXISTED" "0"
  fi
  install -m "$mode" "$src" "$dest"
}

ownership_capture_systemd_baseline_once() {
  local key="$1" unit="$2" active=0 enabled=0
  systemctl is-active --quiet "$unit" 2>/dev/null && active=1
  systemctl is-enabled --quiet "$unit" 2>/dev/null && enabled=1
  ownership_set_baseline_once "SYSTEMD_${key}_PRE_ACTIVE" "$active"
  ownership_set_baseline_once "SYSTEMD_${key}_PRE_ENABLED" "$enabled"
}

# Capture the original fixed-name nftables table before the first managed
# mutation. This is an uninstall baseline (lifetime of the installation), not
# the per-update rollback snapshot maintained by update.sh.
ownership_capture_egress_table_baseline_once() {
  ownership_init
  if grep -q '^EGRESS_TABLE_PRE_EXISTED=' "$OWNERSHIP_FILE" 2>/dev/null; then
    return 0
  fi
  command -v nft >/dev/null 2>&1 || return 1
  if nft list table inet arcana_egress_isolation >/dev/null 2>&1; then
    local backup_dir="${OWNERSHIP_DIR}/preexisting-backups"
    local backup="$backup_dir/EGRESS_TABLE.nft"
    install -d -m 0700 "$backup_dir"
    nft list table inet arcana_egress_isolation >"$backup" || return 1
    chmod 0600 "$backup"
    ownership_set EGRESS_TABLE_BACKUP "$backup"
    ownership_set EGRESS_TABLE_BACKED_UP "1"
    ownership_set EGRESS_TABLE_PRE_EXISTED "1"
  else
    # Distinguish a genuinely absent table from nft/kernel/permission failure;
    # never record "we created it" on an ambiguous observation.
    nft list tables >/dev/null 2>&1 || return 1
    ownership_set EGRESS_TABLE_PRE_EXISTED "0"
  fi
}

# Refuse to treat a manifest-sourced value as a safe destructive-cleanup
# path (uninstall.sh's own use of e.g. RUSTUP_HOME_DIR) unless it is a
# non-empty absolute path, is not "/" itself, and contains no ".."
# component. This is deliberately conservative — it is NOT a general
# canonicalization/symlink-safety check, only a defense against a
# corrupted or hand-edited ownership.env turning a bounded, suffixed
# `rm -rf "$value/some-subdir"` into something unbounded. Callers still
# suffix a fixed subdirectory themselves; this only validates the
# manifest-supplied prefix.
ownership_path_is_safe() {
  local path="$1"
  [ -n "$path" ] || return 1
  case "$path" in
    /) return 1 ;;
    /*) ;;
    *) return 1 ;;
  esac
  case "/$path/" in
    */../*) return 1 ;;
  esac
  return 0
}
