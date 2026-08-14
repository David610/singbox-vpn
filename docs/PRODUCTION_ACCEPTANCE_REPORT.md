# Production Acceptance Report — vpn1 Hiddify-compatible deployment

Date: 2026-08-09. Companion to `docs/FINAL_PRODUCTION_AUDIT.md` (per-issue
detail lives there; this document is the pass/fail summary and the honest
answer to "is this production-ready").

## Tests executed, and how

| Check | Result | How verified |
|---|---|---|
| `cargo fmt --all -- --check` | PASS | LOCAL-EXEC, run at end of pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS (0 warnings) | LOCAL-EXEC |
| `cargo test --workspace` | PASS (all crates; 1 test `#[ignore]`d by design — `hostile_network_scenario`, requires real root+netns) | LOCAL-EXEC |
| `cargo audit` | **NOT RUN** | `cargo-audit` not preinstalled; `cargo install cargo-audit` timed out twice building from source in this sandbox |
| `bash -n` on every shipped `.sh` file | PASS | LOCAL-EXEC |
| `shellcheck -S warning` on every shipped `.sh` file | PASS (0 findings) | LOCAL-EXEC (shellcheck installed via `apt-get` mid-pass) |
| `git diff --check` | PASS | LOCAL-EXEC |
| New: filesystem ownership/permission matrix | Logic reviewed + the acceptance-test script's `sudo -u sing-box`/`sudo -u vpn-subscription` checks reviewed | STATIC (no real `sing-box`/`vpn-subscription` system users or root-owned `/etc/vpn/compat` tree exist in this container) |
| New: atomic-write ownership preservation | PASS — dedicated regression tests mutate a file 5x and assert `(mode, gid)` unchanged each time | LOCAL-EXEC, `cargo test -p compat-config` |
| New: concurrent `vpn-admin user create` | PASS — two processes race against the same state dir, both succeed, both persisted | LOCAL-EXEC, `cargo test -p admin --test cli concurrent_user_creates_do_not_lose_an_update` |
| New: REALITY rotation success path | PASS — public key changes, no leftover temp files | LOCAL-EXEC, `cargo test -p admin --test cli reality_rotate_replaces_public_key_and_succeeds` |
| New: REALITY rotation rollback on validation failure | PASS — private/public/short_id files byte-for-byte unchanged after a rejected candidate | LOCAL-EXEC, `cargo test -p admin --test cli reality_rotate_rolls_back_key_material_on_validation_failure` |
| sing-box release archive install path | STATIC only | no root/VPS to actually run `install.sh` end-to-end |
| ACME + firewall temporary port-80 open/close | STATIC only | no real firewalld/ufw + certbot + public IP in this sandbox |
| SSH port detection | Function itself exercised standalone (correctly fell back to 22 with a non-zero return code, since this sandbox has no sshd) | LOCAL-EXEC (partial — no real custom-port sshd to detect) |
| certbot deploy-hook | STATIC only | no real certbot/ACME account available |
| health-check.sh / acceptance-test.sh TLS checks | STATIC only (script logic + shellcheck), a live throwaway-TLS-listener exercise was attempted but did not reliably come up in this sandbox and is **not** claimed as a pass | see audit doc P0-13 |
| Real VPS install (any OS) | **NOT RUN** | no VM/VPS access in this environment |
| Real Hiddify/v2rayNG client import | **NOT RUN** | no devices available |

**Every "STATIC only" / "NOT RUN" line above is a genuine gap, not a
formality.** Nothing in this repository should be described as
"VPS-verified" or "device-verified" based on this pass.

## Issues found (see `docs/FINAL_PRODUCTION_AUDIT.md` for full detail)

15 P0 items, all confirmed against the real current code (not assumed from
the task brief); 12 were real, unfixed bugs, 3 were already correctly
implemented by prior sessions (transactional apply/rollback groundwork,
uninstall scoping, sudo-based acceptance-test checks) and needed no
rewrite. The two most severe, both of which would have broken **every
single fresh install**:

- **P0-1**: `sing-box` and `vpn-subscription` service users could not
  traverse each other's-group parent directories under
  `/etc/vpn/compat` — `sing-box.service` would fail to start with a
  permission error on a stock install.
- **P0-2**: every atomic config/user-store write after install silently
  reset file ownership to `root:root` (POSIX `rename()` does not preserve
  the destination's prior owner), breaking the very first `vpn user
  create`/`disable`/`rotate-*` after a working install.

## Issues fixed in this pass

All 15 P0 items (filesystem permission matrix + setgid dirs, atomic-write
ownership preservation, process-level `flock` locking, coordinated REALITY
rotation with rollback, release archive/installer contract + CI check,
single-version resolution (never mixing main source with a different
release's binaries), pinned sing-box checksum fallback, ACME/firewall
ordering with a temporary port-80 rule, SSH-port-aware firewall rules,
certbot renewal deploy-hook, nginx `access_log off` for `/sub/` + a
secret-logging grep guard, real-hostname/real-TLS health checks, truthful
independently-tracked installer summary, Rust-native QR onboarding output)
plus several P1/P2 items (install-state manifest, hostname/port input
validation, backup archive `umask 077` from creation, nginx-config-
ownership doc fix, `LICENSE` file, `rust-toolchain.toml`, `SECURITY.md`,
CI shell-script/secret-logging gates, sing-box checksum verification in
CI too).

## Important architectural changes

- New `crates/compat-config/src/ownership.rs`: a small, reusable
  "preserve owner/group across atomic rename" helper, now used by both
  `store.rs` (users.json) and `server.rs` (sing-box config.json), and by
  the new REALITY key-rotation code path.
- New `apps/admin/src/lock.rs`: `flock`-based system-wide state lock
  (`/run/lock/vpn1.lock` in production), acquired for the full duration
  of every state-mutating `vpn-admin` command.
- New shared `vpn-compat` system group + setgid directories in
  `install.sh`'s ownership matrix, replacing the previous single-group-
  per-directory scheme that made cross-service traversal impossible.
- New coordinated `cmd_reality_rotate` transaction in `apps/admin/src/
  main.rs`: backup → generate candidate → render+validate with the real
  `sing-box` binary → atomically install key material → apply config →
  reload `sing-box` → restart `vpn-subscription` → verify both → commit,
  with full rollback (and an explicit "rollback also failed" hard-error
  path) on any failure.
- Separate installer-level lock (`/run/lock/vpn1-installer.lock`) from
  `vpn-admin`'s own state lock, specifically to avoid a self-deadlock
  when `install.sh`/`update.sh` shell out to `vpn-admin`.

## Tests that could not be executed, and why

- `cargo audit` — tool unavailable, build timed out twice in this sandbox.
- Any real VPS/VM install on any of the 6 target OSes — no VM/VPS
  provisioning available in this environment.
- Any real device import test (Hiddify Android/iOS/Windows/macOS/Linux/
  MagicOS, v2rayNG) — no devices available.
- Real ACME/certbot issuance and renewal against a real public IP/domain.
- Real firewalld/ufw rule application (this container has neither running
  as a real firewall manager).
- A live "does `curl` without `-k` actually reject an untrusted cert
  through this exact script" exercise — attempted, did not reliably
  produce a running throwaway TLS listener in the time available, not
  counted as a pass.

## Exact remaining blockers to a truthful "production-ready" claim

1. **No real-VPS installation has ever been run against these changes.**
   The single highest-value next step is: provision a real Ubuntu 24.04
   or AlmaLinux 9 VPS, run the exact one-command installer below, and
   fix whatever the first real run surfaces that this sandbox could not
   catch (SELinux contexts, real firewalld/ufw behavior, real certbot
   HTTP-01 against a real public IP, real systemd unit startup ordering).
2. **No real client has ever imported a subscription URL from this
   stack.** Hiddify (any platform) and v2rayNG import/connect/rotate/
   revoke behavior is entirely unverified against the current code.
3. **`update.sh` is still source-rebuild-based**, not the
   verified-prebuilt-release / versioned-directory / full-rollback design
   the brief asks for — this is the largest remaining structural gap and
   was explicitly deferred (see audit doc P1 table) rather than attempted
   partially and left half-working.
4. **`cargo audit` has never actually been run** against this dependency
   tree in this pass — a real dependency-vulnerability gate is missing
   until someone runs it (ideally in CI, which now has a job wired up
   and ready — it just couldn't execute here).
5. ~~Backup/restore does not yet reject symlink entries during
   extraction~~ — fixed: `vpn-admin restore` now walks the extracted
   archive and refuses to restore if any entry is a symlink, before
   reading/copying any of it (see `reject_symlinks` in
   `apps/admin/src/main.rs`, covered by
   `restore_rejects_archive_containing_a_symlink` in
   `apps/admin/tests/cli.rs`).

## Can this repository truthfully be called "production-ready"?

**No, not yet — but it is substantially closer, and the changes in this
pass fix defects that would have broken literally every fresh install.**
Specifically: before this pass, a stock `curl | sudo bash` install would
have produced a `sing-box.service` that could not start at all (P0-1),
and even if that were somehow worked around, the very first `vpn user
create` afterward would have broken the server a second time (P0-2). Those
are now fixed and covered by regression tests. What's missing for a
truthful "production-ready" is almost entirely **verification**, not
unimplemented logic: nothing in this pass has been confirmed against a
real VPS, a real domain, a real certificate authority, or a real client
device. Until at least one real end-to-end run happens (item 1 above),
"production-ready" would be an overclaim.

## Exact one-command installation (as implemented now)

```
curl -fsSL https://raw.githubusercontent.com/David610/vpn1/main/install.sh | sudo bash
```

Resolves the latest stable tagged release if one exists (source + binaries
from the same tag, never mixed). If no release has been tagged yet, the
default (stable) channel refuses to run rather than falling back to `main`
— see `docs/IMPLEMENTATION_STATUS.md` and the top-level README for the
current no-release-yet behavior and the explicit `VPN1_CHANNEL=dev`
development-only escape hatch. This paragraph described an earlier,
now-superseded behavior; corrected to match the current fail-closed
bootstrap. Version-pinned form:

```
curl -fsSL https://raw.githubusercontent.com/David610/vpn1/main/install.sh | sudo bash -s -- --version vX.Y.Z
```

## Exact user onboarding flow for Hiddify (as implemented now)

1. Run the one-command installer above on a fresh supported VPS.
2. The installer prints a "vpn1 installation complete" summary with each
   component's independently-confirmed status, followed directly by the
   default user's subscription URL and a terminal QR code (rendered by
   `vpn-admin` itself via the `qrcode` crate — no `qrencode` package
   dependency).
3. Install Hiddify on the target device (iOS/Android/MagicOS/Linux/
   Windows/macOS).
4. In Hiddify: Add profile → Scan QR (or paste the printed subscription
   URL) → Connect.
5. Both VLESS+REALITY and Hysteria2 endpoints are included in the single
   subscription; Hiddify selects between them automatically.

Steps 3-5 are **not device-verified** in this pass (see above) — this is
the documented flow based on the subscription format the code produces,
not a confirmed real-device walkthrough.
