#!/usr/bin/env bash
# Node identity (node_id) and role (exit|relay) helpers shared by
# deploy/almalinux/install.sh and deploy/almalinux/update.sh. Sourced, not
# executed. Never calls die(): callers own the recovery message.
#
# The rules here are the shell half of
# crates/compat-config/src/deployment.rs (`default_node_id_for_host`,
# `validate_node_id`, `NodeRole::parse`). deploy/lib/tests/test-node-identity.sh
# and the Rust test `default_node_id_rule_is_documented_and_deterministic`
# read the same fixture table so the two implementations cannot drift.

NODE_ID_MAX_LEN=63
# Printed by every `vpn-admin config validate` that renders relays
# fail-closed (RELAY_ENFORCEMENT_CAPABILITY in deployment.rs).
RELAY_ENFORCEMENT_CAPABILITY="capability: relay-fail-closed-forwarding"

# True if `vpn-admin config validate` output ($1) comes from a build that
# enforces relay forwarding restrictions.
admin_output_declares_relay_enforcement() {
  printf '%s\n' "${1:-}" | grep -qF "$RELAY_ENFORCEMENT_CAPABILITY"
}

# Strict role parser: prints the role and returns 0 for exactly "exit" or
# "relay"; anything else returns 1.
node_role_is_valid() {
  case "${1:-}" in
    exit|relay) return 0 ;;
    *) return 1 ;;
  esac
}

# 1..63 ASCII letters/digits/'-'/'_'/'.', starting with a letter or digit.
node_id_is_valid() {
  local id="${1:-}"
  [ -n "$id" ] || return 1
  [ "${#id}" -le "$NODE_ID_MAX_LEN" ] || return 1
  case "$id" in
    [A-Za-z0-9]*) ;;
    *) return 1 ;;
  esac
  case "$id" in
    *[!A-Za-z0-9._-]*) return 1 ;;
  esac
  return 0
}

# Documented default node identity for a host: the first DNS label of
# PUBLIC_HOST, unsupported characters replaced by '-', leading/trailing
# non-alphanumerics trimmed, truncated to 63 characters, trailing '-'/'_'
# trimmed again; "legacy-node" if nothing usable remains.
default_node_id_for_host() {
  local host="${1:-}" label id
  label="${host%%.*}"
  id="$(printf '%s' "$label" \
    | LC_ALL=C sed -E 's/[^A-Za-z0-9_-]/-/g; s/^[^A-Za-z0-9]+//; s/[^A-Za-z0-9]+$//' \
    | cut -c1-"$NODE_ID_MAX_LEN" \
    | LC_ALL=C sed -E 's/[-_]+$//')"
  [ -n "$id" ] || id="legacy-node"
  printf '%s\n' "$id"
}

# Value of a TOP-LEVEL string/bare key in a deployment.toml (everything
# before the first [table] header), without surrounding quotes. Prints
# nothing when absent. Keys of the same name inside a table are ignored,
# exactly like the Rust migration.
deployment_top_level_value() {
  local file="$1" key="$2"
  [ -f "$file" ] || return 0
  awk -v key="$key" '
    /^[[:space:]]*\[/ { exit }
    /^[[:space:]]*#/ { next }
    {
      line = $0
      sub(/^[[:space:]]+/, "", line)
      eq = index(line, "=")
      if (eq == 0) next
      name = substr(line, 1, eq - 1)
      gsub(/[[:space:]]+$/, "", name)
      if (name != key) next
      value = substr(line, eq + 1)
      sub(/^[[:space:]]+/, "", value)
      sub(/[[:space:]]+#.*$/, "", value)
      gsub(/[[:space:]]+$/, "", value)
      gsub(/^"|"$/, "", value)
      print value
      exit
    }
  ' "$file"
}

# Effective role of an existing deployment: an absent role predates roles
# and was an ordinary exit.
deployment_effective_role() {
  local role
  role="$(deployment_top_level_value "$1" role)"
  printf '%s\n' "${role:-exit}"
}

# Guard used around every mutation of an EXISTING deployment (repair,
# update, migration): the role must be identical, and a node_id that
# existed before must still be identical. $1=file, $2=role before,
# $3=node_id before. Prints the reason and returns 1 on any change.
deployment_identity_unchanged() {
  local file="$1" role_before="$2" node_before="$3" role_after node_after
  role_after="$(deployment_effective_role "$file")"
  node_after="$(deployment_top_level_value "$file" node_id)"
  if [ "$role_after" != "$role_before" ]; then
    echo "deployment role changed from '$role_before' to '$role_after'"
    return 1
  fi
  if [ -n "$node_before" ] && [ "$node_after" != "$node_before" ]; then
    echo "deployment node_id changed from '$node_before' to '$node_after'"
    return 1
  fi
  return 0
}
