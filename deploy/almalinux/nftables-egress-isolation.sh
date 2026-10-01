#!/usr/bin/env bash
# Host-layer defence-in-depth for C-16 data-plane egress isolation
# (docs/security/ARCANA_CROSS_REPO_REMEDIATION_PLAN_2026-09-27.md, cross-
# repo, and docs/reviews/BRANCH_SURVEY_2026-09-29.md's Phase 2/3 notes).
#
# The sing-box-rendered `route` policy (crates/compat-config/src/server.rs
# — apply_c16_egress_policy / apply_probe_user_confinement) is the
# PRIMARY enforcement point: it is per-user, protocol-aware, and covers
# domain-based bypass via the `resolve` route action. This script is the
# SECONDARY, host-level layer: an nftables output chain that rejects the
# same destination set for the `sing-box` OS user specifically, so a bug
# or a future code path that bypasses the rendered route policy (a new
# outbound added without going through apply_c16_egress_policy, a sing-box
# version regression in route-rule evaluation, etc.) does not silently
# reopen access to node-local services, RFC1918, or cloud metadata.
#
# Installed and enabled by the normal lifecycle. `--remove` is used by
# uninstall and rollback and deletes only this project's dedicated table.
#
# Idempotent: atomically replaces its own dedicated table
# (`inet arcana_egress_isolation`) only — never touches firewalld's
# tables/zones (deploy/almalinux/firewall.sh's inbound policy) or any
# other nftables table on the host.
set -euo pipefail

[ "$(id -u)" -eq 0 ] || { echo "must run as root" >&2; exit 1; }
command -v nft >/dev/null 2>&1 || { echo "[nftables-egress-isolation] nft not installed" >&2; exit 1; }

log() { echo "[nftables-egress-isolation] $*"; }

case "${1:-apply}" in
  apply) ;;
  --remove)
    if nft list table inet arcana_egress_isolation >/dev/null 2>&1; then
      nft delete table inet arcana_egress_isolation
      log "removed table inet arcana_egress_isolation; unrelated nftables state was not changed."
    else
      log "table inet arcana_egress_isolation is already absent."
    fi
    exit 0
    ;;
  *) echo "usage: $0 [--remove]" >&2; exit 2 ;;
esac

SING_BOX_USER="${SING_BOX_USER:-sing-box}"
id -u "$SING_BOX_USER" >/dev/null 2>&1 \
  || { echo "[nftables-egress-isolation] OS user '$SING_BOX_USER' does not exist — refusing to install a ruleset keyed to a nonexistent uid." >&2; exit 1; }
SING_BOX_UID="$(id -u "$SING_BOX_USER")"

# Same set as C16_DENY_IPV4_CIDRS / C16_DENY_IPV6_CIDRS in
# crates/compat-config/src/model.rs MINUS loopback (127.0.0.0/8, ::1/128)
# — kept in sync by hand (both are small, stable, spec-pinned lists); a
# mismatch here only WIDENS this secondary layer's coverage relative to
# the primary one, it can never narrow the primary sing-box-rendered
# policy.
#
# Loopback is deliberately excluded from this uid-scoped host rule: the
# `sing-box` process legitimately dials 127.0.0.1 for the reserved
# probe/self-test principal's confined loopback exception
# (server::render_server_config_for_deployment's relay branch), and a
# blanket uid-based host rule cannot distinguish that dial from a
# would-be customer pivot the way the application-layer auth_user rule
# does. Loopback enforcement stays at the application layer (Phase 2/4);
# this script covers every OTHER C-16 destination, where no legitimate
# sing-box-uid outbound dial ever exists.
IPV4_DENY="0.0.0.0/8, 10.0.0.0/8, 100.64.0.0/10, 169.254.0.0/16, 172.16.0.0/12, 192.0.0.0/24, 192.0.2.0/24, 192.168.0.0/16, 198.18.0.0/15, 198.51.100.0/24, 203.0.113.0/24, 224.0.0.0/4, 240.0.0.0/4"
IPV6_DENY="::ffff:0:0/96, 64:ff9b::/96, 100::/64, 2001:db8::/32, fc00::/7, fe80::/10, ff00::/8, fd00:ec2::254/128"

RULESET="$(mktemp /run/arcana-egress-isolation.XXXXXX.nft)"
trap 'rm -f "$RULESET"' EXIT

# nft processes an entire -f input as one netlink transaction. Including the
# delete in this same batch means a parse/evaluation/apply failure cannot leave
# the previously-working table deleted. A first install omits the delete.
if nft list table inet arcana_egress_isolation >/dev/null 2>&1; then
  printf '%s\n' 'delete table inet arcana_egress_isolation' >"$RULESET"
else
  : >"$RULESET"
fi
cat >>"$RULESET" <<EOF
table inet arcana_egress_isolation {
  chain output {
    type filter hook output priority filter; policy accept;

    # Only constrains the sing-box process's own outbound sockets — the
    # host's own control-plane traffic (vpn-admin, vpn-subscription,
    # provisioning-agent, certbot, sshd, the agent heartbeat) runs as
    # different OS users and is untouched by this chain.
    meta skuid != $SING_BOX_UID accept

    ip daddr { $IPV4_DENY } reject
    ip6 daddr { $IPV6_DENY } reject
    tcp dport 25 reject
  }
}
EOF

# Validate without mutation first, then apply the exact same file atomically.
nft --check -f "$RULESET"
nft -f "$RULESET"

log "installed table inet arcana_egress_isolation (output chain, uid=$SING_BOX_UID scoped)."
log "verify with: nft list table inet arcana_egress_isolation"
