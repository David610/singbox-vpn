# deploy/almalinux/

Production deployment tooling for the Hiddify/VLESS-REALITY/Hysteria2
compatibility stack. Despite the directory name (kept for backwards
compatibility with existing links), it supports the RHEL family
(AlmaLinux 9, Rocky Linux 9, RHEL 9, Amazon Linux 2023 — the last one
covered by automated unit tests only, not a real-host verification; see
`docs/ALMALINUX_DEPLOYMENT.md`'s support matrix) and the Debian family
(Ubuntu 22.04/24.04, Debian 12/13) — see `deploy/lib/os.sh`. See
`docs/ALMALINUX_DEPLOYMENT.md` for the full runbook; this file is a
quick index.

| Script | Purpose |
|---|---|
| `install.sh` | Fresh install: packages, binaries, sing-box, secrets, systemd, firewall, SELinux context. |
| `update.sh` | Rebuild + validate + atomic swap + health-check + auto-rollback on failure. |
| `uninstall.sh` | Stop/disable/remove services and binaries; state kept unless `--purge-state`. |
| `firewall.sh` | Idempotently applies the firewalld rule set (22/tcp, 443/tcp, 443/udp, 8443/tcp only). |
| `render-config.sh` | Re-render + validate + apply sing-box config from the current user store and reload. |
| `health-check.sh` | Installed as `/usr/local/bin/vpn-health-check`; spec §42 smoke test, no secrets printed. |
| `systemd/*.service` | Hardened unit files for `sing-box` and `vpn-subscription`. |
| `templates/deployment.toml.template` | Rendered once into `/etc/vpn/deployment.toml` by `install.sh`. |

This directory is intentionally separate from `deploy/local/` (the
existing native-stack dev slice) — see spec §50 / `PLAN.md`. Nothing
here touches `deploy/local/`.

Not deployed by this directory: the native `direct-tls`/`noise-quic`
stack (`services/rendezvous`, `services/relay-agent`,
`apps/client-daemon`). That remains `deploy/local/`-only per
`docs/COMPATIBILITY_IMPLEMENTATION_PLAN.md` §14 (non-goals).
