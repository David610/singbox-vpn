#!/usr/bin/env bash
# firewalld rules for the compatibility stack. Public: SSH, VLESS+REALITY
# (443/tcp), Hysteria2 (443/udp), subscription HTTPS (SUBSCRIPTION_PORT,
# default 8443/tcp). Nothing
# else — internal Rust services (rendezvous, subscription's own loopback
# bind) stay off the public zone entirely (spec §33).
set -euo pipefail

[ "$(id -u)" -eq 0 ] || { echo "must run as root" >&2; exit 1; }

command -v firewall-cmd >/dev/null 2>&1 || { echo "firewalld not installed" >&2; exit 1; }

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
log() { echo "[firewall] $*"; }
warn() { echo "[firewall] WARNING: $*" >&2; }
die() { echo "[firewall] ERROR: $*" >&2; exit 1; }
# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/preflight.sh"

SUBSCRIPTION_PORT="${SUBSCRIPTION_PORT:-8443}"
preflight_validate_port "$SUBSCRIPTION_PORT" "SUBSCRIPTION_PORT" || die "invalid SUBSCRIPTION_PORT."

# Never assume SSH is on 22 (docs/FINAL_PRODUCTION_AUDIT.md P0-10):
# `--add-service=ssh` only covers the well-known port 22, so a custom
# sshd port must be added explicitly BEFORE this firewall goes
# effectively default-deny, or the operator's current session gets
# locked out. preflight_resolve_ssh_port() (deploy/lib/preflight.sh) is
# the single canonical implementation shared with install.sh/
# firewall-ufw.sh: it honours an explicit SSH_PORT/--ssh-port override
# and otherwise auto-detects. FAILS CLOSED (checkpoint-1 requirement):
# if detection is inconclusive and no override was supplied, this
# refuses to touch the firewall at all rather than falling back to 22.
# Resolved BEFORE firewalld is ever started below — this script is also
# callable standalone (not only via install.sh, which already activates
# firewalld SSH-safely in packages_stage), so the same ordering
# guarantee has to hold here too.
SSH_PORT="$(preflight_resolve_ssh_port)" || die "could not positively determine the real SSH port (checked sshd -T, sshd_config, and live listeners — all inconclusive), and no SSH_PORT override was supplied. Refusing to activate/reload the firewall — re-run with SSH_PORT=<port> set to sshd's real listening port (install.sh's --ssh-port does this for you)."
log "confirmed SSH port: $SSH_PORT"

FIREWALLD_WAS_INACTIVE=0
systemctl is-active --quiet firewalld || FIREWALLD_WAS_INACTIVE=1
if [ "$FIREWALLD_WAS_INACTIVE" -eq 1 ]; then
  # Do NOT start firewalld yet. Starting it before the SSH rule exists
  # anywhere in its configuration means the daemon enforces its
  # distro-default policy on this zone — default-deny for a custom SSH
  # port — for a real (if brief) window before a rule is added. Stage the
  # SSH allow into the permanent (offline) configuration via
  # firewall-offline-cmd first, which needs no running daemon, verify it
  # landed, and only then start firewalld — a freshly started daemon
  # loads its permanent configuration as the initial runtime
  # configuration, so the rule is already effective the moment it comes up.
  command -v firewall-offline-cmd >/dev/null 2>&1 \
    || die "firewall-offline-cmd not found (normally installed alongside firewall-cmd by the 'firewalld' package). Cannot safely stage the SSH allow rule before firewalld starts. Refusing to activate the firewall — nothing has been changed on this host."

  OFFLINE_ZONE="$(firewall-offline-cmd --get-default-zone)" \
    || die "firewall-offline-cmd --get-default-zone failed. Refusing to activate the firewall."
  firewall-offline-cmd --zone="$OFFLINE_ZONE" --add-service=ssh >/dev/null \
    || die "firewall-offline-cmd failed to stage the SSH service allow rule on zone '$OFFLINE_ZONE'. Refusing to start firewalld — nothing has been changed on this host."
  if [ "$SSH_PORT" != "22" ]; then
    firewall-offline-cmd --zone="$OFFLINE_ZONE" --add-port="${SSH_PORT}/tcp" >/dev/null \
      || die "firewall-offline-cmd failed to stage the ${SSH_PORT}/tcp allow rule on zone '$OFFLINE_ZONE'. Refusing to start firewalld — nothing has been changed on this host."
  fi
  firewall-offline-cmd --zone="$OFFLINE_ZONE" --query-service=ssh >/dev/null 2>&1 \
    || die "staged the SSH service rule but firewall-offline-cmd does not report it present on zone '$OFFLINE_ZONE' afterward. Refusing to start firewalld — nothing has been changed on this host."
  if [ "$SSH_PORT" != "22" ]; then
    firewall-offline-cmd --zone="$OFFLINE_ZONE" --query-port="${SSH_PORT}/tcp" >/dev/null 2>&1 \
      || die "staged the ${SSH_PORT}/tcp rule but firewall-offline-cmd does not report it present on zone '$OFFLINE_ZONE' afterward. Refusing to start firewalld — nothing has been changed on this host."
  fi
  log "permanent SSH allow rule for port ${SSH_PORT}/tcp positively verified on zone '$OFFLINE_ZONE' — starting firewalld now."

  systemctl start firewalld \
    || die "firewalld failed to start after the SSH allow rule was staged and verified. Host firewall state is unchanged (firewalld inactive); investigate before retrying."

  firewall-cmd --zone="$OFFLINE_ZONE" --query-service=ssh >/dev/null 2>&1 \
    || die "firewalld is now active but the SSH service rule is NOT present in its effective runtime configuration on zone '$OFFLINE_ZONE'. This means SSH may be blocked. firewalld is running — do not disconnect this session; investigate immediately before assuming safety."
  if [ "$SSH_PORT" != "22" ]; then
    firewall-cmd --zone="$OFFLINE_ZONE" --query-port="${SSH_PORT}/tcp" >/dev/null 2>&1 \
      || die "firewalld is now active but ${SSH_PORT}/tcp is NOT present in its effective runtime configuration on zone '$OFFLINE_ZONE'. This means SSH on that port may be blocked. firewalld is running — do not disconnect this session; investigate immediately before assuming safety."
  fi
  log "firewalld active on zone '$OFFLINE_ZONE'; SSH port ${SSH_PORT}/tcp positively confirmed allowed (permanent and runtime) before any further firewall changes."
fi

ZONE="$(firewall-cmd --get-default-zone)"
OWNERSHIP_STATE="/var/lib/singbox-vpn/firewall-owned.env"
owned_443_tcp=0
owned_443_udp=0
owned_subscription_tcp=0
if [ -f "$OWNERSHIP_STATE" ]; then
  # Installer-owned root-only file containing only numeric flags/validated
  # backend metadata written below.
  # shellcheck disable=SC1090
  . "$OWNERSHIP_STATE"
  if [ "${firewall_backend:-}" != "firewalld" ] || [ "${firewall_zone:-}" != "$ZONE" ]; then
    owned_443_tcp=0; owned_443_udp=0; owned_subscription_tcp=0
  fi
fi

# When firewalld was freshly activated above, the SSH rule is already
# staged, verified, and active on this zone. When firewalld was already
# active before this script ran (pre-existing installation, checkpoint-1:
# never re-activate or flush it), the rule may still be missing — add and
# positively verify it here too, in both the runtime and permanent
# configuration. `firewall-cmd --add-*` on a rule that already exists is
# idempotent (exit 0), so re-running these against an already-safe zone
# is harmless — this is not a fail-open `|| true`: a real failure here
# aborts the script under `set -e`, before any further firewall mutation.
firewall-cmd --zone="$ZONE" --add-service=ssh >/dev/null
if [ "$SSH_PORT" != "22" ]; then
  firewall-cmd --zone="$ZONE" --add-port="${SSH_PORT}/tcp" >/dev/null
fi
firewall-cmd --zone="$ZONE" --permanent --add-service=ssh >/dev/null
if [ "$SSH_PORT" != "22" ]; then
  firewall-cmd --zone="$ZONE" --permanent --add-port="${SSH_PORT}/tcp" >/dev/null
fi
firewall-cmd --zone="$ZONE" --query-service=ssh >/dev/null 2>&1 \
  || die "SSH service rule is not present in the effective runtime configuration on zone '$ZONE' after adding it. Refusing to proceed to VLESS/Hysteria2/subscription firewall rules."
if [ "$SSH_PORT" != "22" ]; then
  firewall-cmd --zone="$ZONE" --query-port="${SSH_PORT}/tcp" >/dev/null 2>&1 \
    || die "${SSH_PORT}/tcp is not present in the effective runtime configuration on zone '$ZONE' after adding it. Refusing to proceed to VLESS/Hysteria2/subscription firewall rules."
fi
firewall-cmd --zone="$ZONE" --permanent --query-port=443/tcp >/dev/null 2>&1 \
  || { firewall-cmd --zone="$ZONE" --permanent --add-port=443/tcp; owned_443_tcp=1; }
firewall-cmd --zone="$ZONE" --permanent --query-port=443/udp >/dev/null 2>&1 \
  || { firewall-cmd --zone="$ZONE" --permanent --add-port=443/udp; owned_443_udp=1; }
firewall-cmd --zone="$ZONE" --permanent --query-port="${SUBSCRIPTION_PORT}/tcp" >/dev/null 2>&1 \
  || { firewall-cmd --zone="$ZONE" --permanent --add-port="${SUBSCRIPTION_PORT}/tcp"; owned_subscription_tcp=1; }

install -d -m 0755 /var/lib/singbox-vpn
umask 077
cat >"$OWNERSHIP_STATE.tmp" <<EOF
firewall_backend=firewalld
firewall_zone=$ZONE
subscription_port=$SUBSCRIPTION_PORT
owned_443_tcp=$owned_443_tcp
owned_443_udp=$owned_443_udp
owned_subscription_tcp=$owned_subscription_tcp
EOF
mv -f "$OWNERSHIP_STATE.tmp" "$OWNERSHIP_STATE"

# Explicitly documented as NOT exposed publicly (spec §33): 1080 (any
# local SOCKS proxy), 9000/9100-class internal control-plane ports.
# firewalld default-deny already blocks these; nothing to add.

firewall-cmd --reload
echo "firewall rules applied on zone '$ZONE':"
firewall-cmd --zone="$ZONE" --list-all
