# IMPLEMENTATION_AUDIT.md

Audit of the repository against the "make vpn1 production-ready and
end-user friendly" spec, written before this session's changes. Purpose:
avoid duplicating work that already exists, and give an honest baseline
for what this session adds on top of it.

## What already existed (prior sessions), verified by reading the code

The Hiddify-compatible compatibility stack (`crates/compat-config`,
`apps/admin` = `vpn-admin`, `services/subscription`,
`deploy/almalinux/`) was already substantially built and hardened by two
prior passes documented in `TASKS.md` Phase 14/15 and
`docs/PRODUCTION_HARDENING_PLAN.md`. Confirmed present and working from
reading the source (not re-derived from docs alone):

- **Canonical user model**: `CompatUser` in `crates/compat-config/src/model.rs`
  — single `users.json` store, no parallel database.
- **Secure subscription tokens**: `credentials::generate_subscription_token`
  (160-bit CSPRNG, base64url), only a SHA-256 hash is persisted
  (`subscription_token_hash_hex`), constant-time verification
  (`credentials::verify_token`). Raw token is shown exactly once, at
  `create`/`rotate-token` time — matches spec §8.
- **VLESS + Hysteria2 URI rendering and sing-box subscription JSON**:
  `crates/compat-config/src/render.rs`, tested against current sing-box
  1.13.x schema, private REALITY key never included client-side.
- **Atomic, validated config apply with rollback**:
  `crates/compat-config/src/server.rs::apply_config_atomically` — temp
  file, `fsync`, `sing-box check` validation, mode 0640, backup +
  restore on failure.
- **Real revocation**: `apps/admin/src/main.rs::regenerate_singbox_config`
  now (post Phase-15 hardening) reloads sing-box via
  `apps/admin/src/service.rs::CompatibilityServiceManager` and verifies
  the service is active after every user mutation; on reload/health
  failure it restores the previous config and reports failure rather
  than printing success. This was the single most serious gap flagged in
  `PRODUCTION_HARDENING_PLAN.md` #4, and it is fixed.
- **User lifecycle CLI**: `vpn-admin user create/list/enable/disable/
  rotate-token/rotate-vless/rotate-hysteria/rotate-credentials/remove/
  subscription`, plus `vpn-admin init` (REALITY keypair generation via
  real `sing-box generate reality-keypair`) and `render-config`.
- **Subscription HTTP service**: `services/subscription` — loopback-only,
  `GET /sub/{token}` with `singbox`/`uri`/`hiddify` formats, generic 404
  for unknown/disabled/expired token, per-IP rate limiting,
  `Cache-Control: no-store` / `Pragma: no-cache` /
  `X-Content-Type-Options: nosniff` on every response.
- **One-command installer**: `deploy/almalinux/install.sh` — 15 explicit
  numbered stages (OS check → deps → sing-box → users/groups → dirs →
  REALITY keys → Hysteria2 config → TLS presence check (fails loudly,
  no fake ACME) → nginx reverse proxy + rate limiting → systemd units →
  firewalld → validation → start → health checks), `set -Eeuo pipefail`,
  no `|| true` on a required step. Interactive + non-interactive
  (env var driven) both supported.
- **Update/rollback**: `deploy/almalinux/update.sh` — backs up, replaces
  binaries/config, reloads, health-checks, automatically restores +
  reloads the previous version on failure, only prints success after the
  post-rollback health check itself passes.
- **Uninstall**: `deploy/almalinux/uninstall.sh` — separates binary/service
  removal (default) from `--purge-state`/`--purge-firewall`.
- **Firewall**: `deploy/almalinux/firewall.sh` — firewalld, opens only
  443/tcp, 443/udp, and the subscription HTTPS port; subscription service
  itself bound to loopback only.
- **systemd hardening**: both units carry `NoNewPrivileges`,
  `ProtectSystem=strict`, `ProtectHome`, `ProtectKernelTunables`,
  `ProtectKernelModules`, `ProtectControlGroups`, dedicated non-root
  service users.
- **Filesystem permissions**: reality/hysteria secret directories and
  files fixed to be group-`sing-box`-readable (not world-readable);
  `users.json` 0640 (subscription service needs read access);
  `config.json` 0640 including its `.bak`.
- **CI**: `cargo fmt --check`, `clippy -D warnings`, `cargo test
  --workspace`, `cargo audit` (blocking, no `|| true`), a
  `singbox-validate` job that downloads pinned real sing-box and runs
  `sing-box check` against a rendered config.
- **Acceptance test**: `deploy/almalinux/acceptance-test.sh` — ownership,
  services, listeners, cert validity, reverse proxy, no-public-listener
  checks; `bash -n`/shellcheck-clean, not executed against a real host
  (documented, not claimed otherwise).
- **Docs**: `docs/CLIENT_COMPATIBILITY.md`, `docs/HIDDIFY_ANDROID.md`,
  `docs/ALMALINUX_DEPLOYMENT.md`, `docs/COMPATIBILITY_SECURITY_REVIEW.md`,
  `docs/PRODUCTION_HARDENING_PLAN.md` — all already avoid
  "production-ready"/"validated" overclaims and use explicit
  implemented-vs-verified language.

This is a strong, already largely-complete implementation of spec
sections 3, 4, 7, 8, 9, 10, 11, 12, 13 (partially — see below), 14, 15,
21, 22, 23, 25, 26. Re-implementing any of the above would be pure churn;
none of it is touched by this session except where a genuine gap is
listed below.

## Gaps found by this audit (what this session adds)

1. **QR codes (spec §6)** — no QR generation existed anywhere. Added:
   `vpn-admin user qr NAME` (mints a fresh subscription token — the raw
   token is never persisted, so QR-ing an *existing* subscription without
   rotating it is not possible by the existing security design; this is
   documented at the call site, not silently worked around) plus `--qr`
   on `create`/`rotate-token` printing a terminal QR code inline. PNG
   file output (`--output FILE.png`) was **not** implemented — marked
   "Not implemented" below to keep the new dependency footprint to the
   `qrcode` crate alone (terminal/unicode rendering only), per the task's
   "keep dependencies minimal" instruction.
2. **`vpn version` (spec §19)** — did not exist. Added as `vpn-admin
   version`, printing the crate version and, if the configured sing-box
   binary is present, its reported version.
3. **`vpn status` / `vpn doctor` (spec §16/§17)** — did not exist. Added
   `vpn-admin status` (service active/inactive, active/disabled user
   counts, sing-box config presence) and `vpn-admin doctor` (numbered
   `[OK]`/`[WARN]`/`[FAIL]` checks: sing-box binary + `sing-box check`,
   REALITY/Hysteria2 material present with non-world-readable
   permissions, users store parses, systemd unit state, firewalld state
   if present, Hysteria2 cert expiry via `openssl x509`). Checks that
   need a tool not present on this box (e.g. `openssl`, `firewall-cmd`,
   `ss`) report `[WARN] <tool> not available — check skipped`, not a
   faked pass. `doctor` exits non-zero on any `[FAIL]`.
4. **`vpn backup` / `vpn restore` (spec §20)** — did not exist. Added:
   `vpn-admin backup` (tar of `users.json`, the deployment config, the
   REALITY key material, and the Hysteria2 TLS material, written
   mode 0600) and `vpn-admin restore` (extracts to a temp dir, validates
   the users file parses and the REALITY private key is present *before*
   touching live state, then applies via the same
   `regenerate_singbox_config` path used by every other mutating command
   — so a bad backup cannot be restored over a working deployment without
   `sing-box check` catching it first).
5. **Machine-readable output (spec §28)** — `user create --json` did not
   exist. Added: `{"id", "name", "enabled", "subscription_url"}` — no
   server private keys, matching the spec's explicit constraint. (The
   subscription URL itself necessarily contains the one-time token, same
   as the human-readable path.)
6. **Ergonomic `vpn` command name (spec §29)** — only `vpn-admin` existed.
   Added a second `[[bin]]` target `vpn` in `apps/admin/Cargo.toml`
   pointing at the same `main.rs` (same clap parser, so all of the above
   is reachable as both `vpn-admin ...` and `vpn ...`). No wrapper script,
   no symlink-at-install-time hack — the existing `vpn-admin` name keeps
   working unchanged for any script/doc that already references it.
7. **Client onboarding docs (spec §27)** — only `docs/HIDDIFY_ANDROID.md`
   existed. Added `docs/clients/HIDDIFY_IOS.md`,
   `docs/clients/HIDDIFY_MAGICOS.md`, `docs/clients/HIDDIFY_LINUX.md`,
   `docs/clients/V2RAYNG_ANDROID.md` (the existing Android doc is left in
   place, referenced from the new client index rather than duplicated).
8. **`docs/DEVICE_ACCEPTANCE_TESTS.md` (spec §35)** — did not exist.
   Added the explicit platform × protocol matrix, all cells currently
   "not yet manually tested" (honest — no real device/VPS available in
   this sandbox).

## Explicitly not attempted this session (documented, not faked)

- **Real VPS / AlmaLinux 9 host execution** of `install.sh`,
  `update.sh`, `acceptance-test.sh`, or `doctor`/`status`'s
  systemd/firewalld/openssl-dependent checks — this sandbox has no
  systemd, no firewalld, no real network-facing host. Every such check
  degrades to an explicit `[WARN] ... not available` rather than a
  fabricated pass.
- **Real Hiddify/v2rayNG/MagicOS device import** — no such device
  available; `docs/DEVICE_ACCEPTANCE_TESTS.md` matrix is left unchecked.
- **PNG QR output**, **DNS-over-installer automatic hostname
  verification beyond what already existed**, **a web
  dashboard/multi-tenant control plane** — out of scope per spec §38 /
  not attempted.
- **Native adaptive client (`client-daemon`/`transport-native`)** — out
  of scope for this pass; unchanged. It remains the clearly-separated
  "experimental/native adaptive mode" per `docs/ARCHITECTURE.md`, not
  touched or presented as the recommended end-user path.

## What was preserved deliberately

Per the task's explicit instruction not to replace working architecture:
no rewrite of `crates/compat-config`, `services/subscription`, or the
installer's stage structure. All additions in this session are new,
additive CLI subcommands and new documentation files; no existing
command's behavior, output format, or file layout changed except where
noted above (all additive, none removed).
