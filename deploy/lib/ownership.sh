#!/usr/bin/env bash
# vpn1 ownership manifest — the single source of truth for "did vpn1
# create/change this, or did it already exist on this host before vpn1
# touched it". Sourced by deploy/almalinux/install.sh (writes it,
# incrementally, starting at stage 1 — never only at the end) and
# deploy/almalinux/uninstall.sh (reads it to decide exactly what is safe
# to remove/restore).
#
# Design goals (see the task's uninstall/rollback requirements):
#   - Written incrementally, one fact at a time, as each mutation
#     happens — a crash at ANY stage still leaves an accurate record of
#     everything done so far, not just a snapshot taken at the very end.
#   - Every fact recorded here is either "vpn1 created/changed this and
#     therefore owns cleaning it up" or "this already existed before
#     vpn1, so uninstall must leave/restore it". Never guessed.
#   - Plain KEY="value" lines, one per line, values restricted to
#     already-validated tokens (booleans, ports, space-joined package/
#     hostname lists that have already passed preflight_validate_* or
#     are package-manager-safe names) — never raw operator/network input
#     written here unescaped.
#
# Expects log()/warn()/die() to already be defined by the caller (same
# convention as preflight.sh/perf-tuning.sh).

: "${OWNERSHIP_DIR:=/var/lib/vpn1}"
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
  val="$(grep -E "^${key}=" "$OWNERSHIP_FILE" 2>/dev/null | tail -n1 | sed -E "s/^${key}=\"//; s/\"\$//")"
  if [ -n "$val" ]; then printf '%s' "$val"; else printf '%s' "$default"; fi
}

# Boolean fact: once true, callers should never flip it back to false —
# only ever call this to RECORD "vpn1 did this", never to un-record it.
ownership_mark() { ownership_set "$1" "1"; }
ownership_is_marked() { [ "$(ownership_get "$1" "0")" = "1" ]; }

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
# before vpn1 ever touched this host", which must reflect the ORIGINAL
# state, not whatever the most recent run happened to see.
ownership_set_baseline_once() {
  local key="$1" value="$2"
  ownership_init
  if grep -qE "^${key}=" "$OWNERSHIP_FILE" 2>/dev/null; then
    return 0
  fi
  ownership_set "$key" "$value"
}
