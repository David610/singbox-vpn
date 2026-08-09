#!/usr/bin/env bash
# firewalld rules for the compatibility stack. Public: SSH, VLESS+REALITY
# (443/tcp), Hysteria2 (443/udp), subscription HTTPS (8443/tcp). Nothing
# else — internal Rust services (rendezvous, subscription's own loopback
# bind) stay off the public zone entirely (spec §33).
set -euo pipefail

[ "$(id -u)" -eq 0 ] || { echo "must run as root" >&2; exit 1; }

command -v firewall-cmd >/dev/null 2>&1 || { echo "firewalld not installed" >&2; exit 1; }
systemctl is-active --quiet firewalld || systemctl start firewalld

ZONE="$(firewall-cmd --get-default-zone)"

firewall-cmd --zone="$ZONE" --permanent --add-service=ssh
firewall-cmd --zone="$ZONE" --permanent --add-port=443/tcp
firewall-cmd --zone="$ZONE" --permanent --add-port=443/udp
firewall-cmd --zone="$ZONE" --permanent --add-port=8443/tcp

# Explicitly documented as NOT exposed publicly (spec §33): 1080 (any
# local SOCKS proxy), 9000/9100-class internal control-plane ports.
# firewalld default-deny already blocks these; nothing to add.

firewall-cmd --reload
echo "firewall rules applied on zone '$ZONE':"
firewall-cmd --zone="$ZONE" --list-all
