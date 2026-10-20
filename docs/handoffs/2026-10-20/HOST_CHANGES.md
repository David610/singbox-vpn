# Persistent host changes on 62.238.46.190 (staging) — with rollback

| UTC | Change | Rollback |
|---|---|---|
| 2026-10-20 | rustup + Rust 1.94.1 (`~/.rustup`, `~/.cargo`) | `rustup self uninstall` |
| 2026-10-20 | apt: python3.14-venv build-essential pkg-config libssl-dev shellcheck jq | `apt remove` the same set |
| 2026-10-20 | `/root/venv-probe` (playwright), `/root/pw-browsers` (Chromium) | `rm -rf` both |
| 2026-10-20 | `/root/singbox-vpn` clone; `/root/evidence`, `/root/host-changes` | `rm -rf` |
No service, firewall, sshd, DNS or credential change has been made.
