# deploy/almalinux/

Production deployment tooling for the Hiddify/VLESS-REALITY/Hysteria2
compatibility stack. The public v1.0 target is **AlmaLinux 9 x86-64 only**.
Other distro branches in `deploy/lib/os.sh` are implementation paths, not a
support claim. See `../../docs/SUPPORTED_PRODUCT.md` for the authoritative
scope and `../../docs/ALMALINUX_DEPLOYMENT.md` for the full runbook.

| Script | Purpose |
|---|---|
| `install.sh` | Fresh install or repair: packages, binaries, sing-box, secrets, systemd, firewall and SELinux context. |
| `update.sh` | Checksum-verified transactional release update/repair with health verification and automatic rollback; `--dev-rebuild` is explicitly development-only. |
| `uninstall.sh` | Ownership-aware complete removal by default, including state and installer-owned firewall changes. |
| `firewall.sh` | Idempotently applies the firewalld rules for the detected/explicit SSH port, TCP/443, UDP/443 and the configurable subscription HTTPS port. |
| `render-config.sh` | Re-render + validate + apply sing-box config from the current user store and reload. |
| `health-check.sh` | Installed as `/usr/local/bin/vpn-health-check`; spec §42 smoke test, no secrets printed. |
| `service-watchdog.sh` | Installed as `/usr/local/bin/vpn-service-watchdog`; run periodically by `vpn-service-watchdog.timer` to recover sing-box/vpn-subscription from a parked `failed` state after they exhaust their restart budget — never touches a deliberately-stopped unit. |
| `systemd/*.service`, `systemd/*.timer` | Hardened unit files for `sing-box`, `vpn-subscription`, `vpn-expiry-reconcile` (credential-expiry reconciliation), and `vpn-service-watchdog` (failed-unit recovery safety net). |
| `templates/deployment.toml.template` | Rendered once into `/etc/vpn/deployment.toml` by `install.sh`. |

This directory is intentionally separate from `deploy/local/` (the
existing native-stack dev slice) — see spec §50 / `PLAN.md`. Nothing
here touches `deploy/local/`.

Not deployed by this directory: the native `direct-tls`/`noise-quic`
stack (`services/rendezvous`, `services/relay-agent`,
`apps/client-daemon`). That remains `deploy/local/`-only per
`docs/COMPATIBILITY_IMPLEMENTATION_PLAN.md` §14 (non-goals).
