#!/usr/bin/env bash
# Opt-in, reversible server-side experiment for the native-YouTube/Hiddify incident.
#
# This intentionally does NOT claim to be the root-cause fix.  It answers one
# narrow question at a layer Hiddify cannot rewrite: if the final EXIT closes
# application UDP/443 immediately, does the real device recover by falling
# back to TCP?
#
# Important semantic boundary: sing-box documents `reject` on a non-TUN
# connection as "just be closed".  The DE server receives VLESS/Hysteria2
# proxy inbounds, not a TUN inbound, so this experiment does NOT assume that
# an ICMP Port Unreachable is propagated all the way back to the phone.
# Device behavior is the measurement.
set -euo pipefail

CONFIG_PATH="${SINGBOX_VPN_CONFIG_PATH:-/etc/vpn/compat/sing-box/config.json}"
DEPLOYMENT_PATH="${SINGBOX_VPN_DEPLOYMENT_PATH:-/etc/vpn/deployment.toml}"
SING_BOX_BIN="${SINGBOX_VPN_SING_BOX_BIN:-/usr/local/bin/sing-box}"
SYSTEMCTL_BIN="${SINGBOX_VPN_SYSTEMCTL:-/usr/bin/systemctl}"
SERVICE_NAME="${SINGBOX_VPN_SERVICE_NAME:-sing-box}"
LOCK_PATH="${SINGBOX_VPN_EXPERIMENT_LOCK:-/run/lock/singbox-vpn-youtube-quic-experiment.lock}"

RULE_JSON='{"network":"udp","port":443,"action":"reject","method":"default","no_drop":true}'

usage() {
  cat <<'USAGE'
Usage:
  youtube-quic-server-experiment.sh enable
  youtube-quic-server-experiment.sh disable
  youtube-quic-server-experiment.sh status

This is an EXIT-only, opt-in diagnostic. It edits only the live generated
sing-box config and is therefore intentionally ephemeral: `vpn render-config`,
a user mutation, repair, update, or reinstall can overwrite it.

`enable` inserts exactly one top-priority application UDP/443 reject rule,
validates the candidate with `sing-box check`, atomically swaps it, and restarts
sing-box. `disable` removes only that exact rule. Both roll back if restart
fails. No credential is printed.
USAGE
}

die() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

require_file() {
  [[ -f "$1" ]] || die "required file not found: $1"
}

require_tools() {
  command -v jq >/dev/null 2>&1 || die "jq is required"
  command -v flock >/dev/null 2>&1 || die "flock is required"
  [[ -x "$SING_BOX_BIN" ]] || die "sing-box binary is not executable: $SING_BOX_BIN"
  [[ -x "$SYSTEMCTL_BIN" ]] || die "systemctl binary is not executable: $SYSTEMCTL_BIN"
}

deployment_role() {
  sed -n 's/^[[:space:]]*role[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$DEPLOYMENT_PATH" | head -n 1
}

require_exit_role() {
  require_file "$DEPLOYMENT_PATH"
  local role
  role="$(deployment_role)"
  [[ "$role" == "exit" ]] || die "this experiment is exit-only; deployment role is ${role:-unknown}"
}

rule_count() {
  jq --argjson rule "$RULE_JSON" '[.route.rules[]? | select(. == $rule)] | length' "$CONFIG_PATH"
}

status() {
  require_file "$CONFIG_PATH"
  local count
  count="$(rule_count)"
  if [[ "$count" == "1" ]]; then
    echo "ENABLED: exact server-side application UDP/443 reject rule is present"
    return 0
  fi
  if [[ "$count" == "0" ]]; then
    echo "DISABLED: server-side application UDP/443 reject rule is absent"
    return 0
  fi
  echo "INVALID: duplicate experiment rules present ($count)" >&2
  return 2
}

apply_candidate() {
  local mode="$1"
  local config_dir original candidate
  config_dir="$(dirname "$CONFIG_PATH")"
  original="$(mktemp "$config_dir/.youtube-quic-original.XXXXXX")"
  candidate="$(mktemp "$config_dir/.youtube-quic-candidate.XXXXXX")"
  cp -a "$CONFIG_PATH" "$original"

  case "$mode" in
    enable)
      if ! jq --argjson rule "$RULE_JSON" '
        .route = (.route // {}) |
        .route.rules = (.route.rules // []) |
        if any(.route.rules[]?; . == $rule)
        then .
        else .route.rules = [$rule] + .route.rules
        end
      ' "$CONFIG_PATH" >"$candidate"; then
        rm -f "$original" "$candidate"
        die "failed to build candidate config"
      fi
      ;;
    disable)
      if ! jq --argjson rule "$RULE_JSON" '
        if .route? then
          .route.rules = [(.route.rules // [])[] | select(. != $rule)] |
          if (.route.rules | length) == 0 then del(.route.rules) else . end |
          if .route == {} then del(.route) else . end
        else . end
      ' "$CONFIG_PATH" >"$candidate"; then
        rm -f "$original" "$candidate"
        die "failed to build candidate config"
      fi
      ;;
    *)
      rm -f "$original" "$candidate"
      die "internal error: unsupported mode $mode"
      ;;
  esac

  chown --reference="$CONFIG_PATH" "$candidate"
  chmod --reference="$CONFIG_PATH" "$candidate"

  if ! "$SING_BOX_BIN" check -c "$candidate" >/dev/null; then
    rm -f "$original" "$candidate"
    die "candidate rejected by sing-box; live config was not changed"
  fi

  mv -f "$candidate" "$CONFIG_PATH"

  if ! "$SYSTEMCTL_BIN" restart "$SERVICE_NAME"; then
    cp -a "$original" "$CONFIG_PATH"
    "$SYSTEMCTL_BIN" restart "$SERVICE_NAME" >/dev/null 2>&1 || true
    rm -f "$original"
    die "sing-box restart failed; previous config restored"
  fi
  if ! "$SYSTEMCTL_BIN" is-active --quiet "$SERVICE_NAME"; then
    cp -a "$original" "$CONFIG_PATH"
    "$SYSTEMCTL_BIN" restart "$SERVICE_NAME" >/dev/null 2>&1 || true
    rm -f "$original"
    die "sing-box did not become active; previous config restored"
  fi
  rm -f "$original"
}

main() {
  [[ $# -eq 1 ]] || { usage >&2; exit 2; }
  case "$1" in
    status)
      require_file "$CONFIG_PATH"
      command -v jq >/dev/null 2>&1 || die "jq is required"
      status
      ;;
    enable|disable)
      require_file "$CONFIG_PATH"
      require_exit_role
      require_tools
      mkdir -p "$(dirname "$LOCK_PATH")"
      exec 9>"$LOCK_PATH"
      flock -n 9 || die "another YouTube QUIC experiment mutation is already running"
      local_count="$(rule_count)"
      if [[ "$1" == "enable" ]]; then
        if [[ "$local_count" == "1" ]]; then
          echo "Already enabled; no change made."
          exit 0
        fi
        [[ "$local_count" == "0" ]] || die "duplicate experiment rules present ($local_count); run disable first"
        apply_candidate enable
        status
        echo "EXPERIMENT ENABLED. Test the native YouTube app now; this is not yet proof of root cause."
      else
        if [[ "$local_count" == "0" ]]; then
          echo "Already disabled; no change made."
          exit 0
        fi
        apply_candidate disable
        status
        echo "EXPERIMENT DISABLED."
      fi
      ;;
    -h|--help|help) usage ;;
    *) usage >&2; exit 2 ;;
  esac
}

main "$@"
