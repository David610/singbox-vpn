#!/usr/bin/env bash
# ufw rules for the compatibility stack (Debian/Ubuntu). Mirrors
# firewall.sh (firewalld/RHEL family): SSH, VLESS+REALITY (443/tcp),
# Hysteria2 (443/udp), subscription HTTPS (8443/tcp). Nothing else —
# internal Rust services stay off the public interface entirely
# (spec §33). Never runs `ufw --force reset` or otherwise flushes
# pre-existing rules; only adds vpn1's own allow rules.
set -euo pipefail

[ "$(id -u)" -eq 0 ] || { echo "must run as root" >&2; exit 1; }

command -v ufw >/dev/null 2>&1 || { echo "ufw not installed" >&2; exit 1; }

# Never let this script be the reason SSH access is lost.
ufw allow OpenSSH >/dev/null 2>&1 || ufw allow 22/tcp >/dev/null
ufw allow 443/tcp >/dev/null
ufw allow 443/udp >/dev/null
ufw allow 8443/tcp >/dev/null

# `ufw enable` is only safe to run non-interactively once SSH is
# explicitly allowed above; -y skips ufw's "will disrupt existing ssh
# connections" confirmation prompt.
ufw --force enable >/dev/null

echo "firewall rules applied (ufw):"
ufw status verbose
