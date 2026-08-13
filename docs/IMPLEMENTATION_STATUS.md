# Implementation Status (v1.0 baseline)

Compact engineering handoff. Full boundary definition: `docs/SUPPORTED_PRODUCT.md`.
Read that file first; do not re-audit the repo from scratch.

## Supported scope (summary)

AlmaLinux 9 x86_64, <=10 users, one VPS, upstream sing-box, VLESS+REALITY
TCP/443 primary, Hysteria2 UDP/443 optional, Hiddify/sing-box-compatible
clients only, custom domain default. Native adaptive stack
(`client-daemon`, `transport-native`, `policy`, `failure-classifier`,
`network-state`, `rendezvous-client`, `telemetry`, `services/rendezvous`,
`services/relay-agent`) is a separate, out-of-scope product surface for
v1.0 — see `deploy/local/run-dev-slice.sh` for its dev-only entry point.

## Supported file/test surface (avoid full-workspace runs when only this changed)

- Installer: `install.sh` (bootstrap) -> `deploy/almalinux/install.sh`
  (real implementation, builds `-p admin -p subscription` only).
- Uninstall: `uninstall.sh` -> `deploy/almalinux/uninstall.sh`.
- Update/repair/render: `deploy/almalinux/update.sh`, `render-config.sh`,
  `health-check.sh`, `acceptance-test.sh`, `certbot-deploy-hook.sh`.
- Shared shell libs: `deploy/lib/*.sh` plus `deploy/lib/versions.env`
  (one authoritative sing-box version/checksum/arch source).
- Rust crates in the supported dependency closure: `crates/common`,
  `crates/compat-config`, `apps/admin`, `services/subscription`.
- Rust tests: `crates/compat-config/tests/*` + `src/**` unit tests
  (interop, decoy budget, deployment/store schema+migration),
  `apps/admin/tests/*`, `services/subscription/tests/*`.
- Shell tests: `deploy/lib/tests/*.sh` (18 files).
- Offline uninstall entry point: `bin/vpn1-uninstall` (persisted to
  `/opt/vpn1/bin/vpn1-uninstall`; wraps `deploy/almalinux/uninstall.sh`).
- CI gates (`.github/workflows/ci.yml`): `fmt`, `clippy`, `test`
  (workspace-wide — see Known CI scope note), `audit`, `docs`, `shell`,
  `secret-logging-check`, `license-check`, `singbox-validate` (real
  pinned `sing-box` + real REALITY/Hysteria2 handshake).
- **FAST GATE**: `bash deploy/lib/fast-gate.sh` — one canonical command
  for all of the above (supported-crate scope only).
- **DESTRUCTIVE lifecycle gate**: `deploy/almalinux/lifecycle-acceptance.sh`
  — disposable AlmaLinux 9 host only, over SSH; never run against production.

## Known CI scope note (documented, not changed)

`fmt`/`clippy --workspace`/`cargo build|test --workspace` build the whole
workspace including out-of-scope native adaptive crates. `singbox-validate`
and `deploy/lib/fast-gate.sh` already scope to the supported crates only.
Splitting the workspace-wide CI jobs would touch CI trust/release
plumbing for a runner-minutes-only win; deferred.

## Completed checkpoints

1. v1.0 boundary + supported code surface documented (`docs/SUPPORTED_PRODUCT.md`).
2. Canonical FAST GATE (`deploy/lib/fast-gate.sh`) + DESTRUCTIVE lifecycle
   gate (`deploy/almalinux/lifecycle-acceptance.sh`, SSH-only, requires
   `--i-understand-this-is-destructive`). One testability hook in
   install.sh (`VPN1_LIFECYCLE_GATE_ABORT_AFTER`, no-op by default).
3. Reproducible installs: `deploy/lib/versions.env` is the ONE source for
   sing-box version/checksums/arch. Bootstrap `install.sh` refuses (die)
   to fall back to mutable branch source when no release tag exists
   (`VPN1_CHANNEL=dev` is the only explicit opt-in).
4. Persistent state versioning/migration: `deployment.toml` gets
   `schema_version`; `users.json` moves to a versioned envelope with a
   tolerant reader (subscription service keeps working through an
   upgrade window). New `vpn-admin config validate`/`config migrate`
   (backup -> migrate -> validate -> atomic commit; idempotent;
   fail-closed on corrupted/future-schema input). `install.sh`/`update.sh`
   report FRESH/REPAIR/UPGRADE/MIGRATION_REQUIRED and auto-migrate.
5. **Installer hardening audit** (this checkpoint): read every supported
   installer stage against 13 hardening requirements (fresh/repair/
   upgrade/incompatible-state classification, non-destructive preflight,
   ownership capture, idempotency, rollback, SSH preservation, loopback-
   only subscription backend, custom-domain default, REALITY handshake
   validation, ACME/firewall restoration, sing-box render+check+real
   protocol test, success-messaging honesty, systemd hardening). **11 of
   13 were already correctly implemented** (VERIFIED-CODE by reading —
   SSH port detection, ACME temp-port-80 ownership+restore+interrupt-safe
   cleanup, systemd hardening directives, loopback `IPAddressAllow`,
   REALITY handshake required-no-default, rollback-gated-on-fresh-install,
   REALITY key idempotency via `vpn-admin init`'s rotate-gate, etc.) —
   confirmed via new tests, not new code. **Found and fixed 2 real gaps**:
   - Custom-domain-default (requirement 8) was NOT enforced: a
     non-interactive install with no `--domain` and no TTY silently fell
     back to the IP-derived sslip.io hostname with no opt-in required,
     contradicting `docs/SUPPORTED_PRODUCT.md`'s own stated policy.
     Fixed: `resolve_host_config()` now `die()`s with clear trade-off
     messaging unless `--allow-ip-hostname`/`VPN1_ALLOW_IP_HOSTNAME=1` is
     passed explicitly; an interactive prompt (pressing Enter to skip)
     still counts as the explicit opt-in, so the documented human
     Quickstart flow is unaffected. `deploy/almalinux/lifecycle-acceptance.sh`
     updated to pass the new flag (it also had a latent, never-exercised
     bug: its `--non-interactive` invocations never set
     `REALITY_HANDSHAKE_SERVER`, which is mandatory in that mode — fixed
     too, found only because this flag change forced re-reading that
     code path).
   - `preflight_detect_ssh_port()` had no fixture-testable seam
     (hardcoded `/etc/ssh/sshd_config`); added
     `SSHD_CONFIG_FILE="${SSHD_CONFIG_FILE:-/etc/ssh/sshd_config}"`
     (same pattern as `os.sh`'s `OS_RELEASE_FILE`) — no behavior change,
     makes non-default-SSH-port detection actually testable.
   - Added a regression test proving `vpn-admin init` re-run (no
     `--rotate`) never regenerates the REALITY keypair (idempotency),
     using the existing `fake_singbox` fixture's deliberate
     second-call-returns-a-different-key behavior as the trap.
   - New test: `deploy/lib/tests/test-installer-hardening.sh` (18 checks:
     SSH port fixture detection + fallback, firewall non-default-port
     rules, firewall ownership-of-pre-existing-state ordering, ACME
     temp-port-80 open/close/interrupt/nginx-restore, custom-domain-
     default enforcement functional tests, REALITY init idempotency).
6. **Uninstall hardening** (this checkpoint): new stable, offline entry
   point `bin/vpn1-uninstall` (persisted to `/opt/vpn1/bin/vpn1-uninstall`
   by `persist_source_tree`, no network access needed on a normal host).
   `deploy/almalinux/uninstall.sh` now: requires `--yes` or an interactive
   `/dev/tty` confirmation before this irreversible action (bootstrap
   `uninstall.sh` forwards `--yes`/other flags through, and now prefers
   the local `bin/vpn1-uninstall` over its old direct-`deploy/almalinux`
   path, itself only a network-download fallback); refuses to run from a
   directory/file it does not itself control (not root-owned, or
   group/world-writable — defense-in-depth); re-validates manifest-
   sourced values before destructive use (`CERT_LINEAGES_CREATED_BY_VPN1`
   hostnames via `preflight_validate_hostname`, `RUSTUP_HOME_DIR` via new
   `ownership_path_is_safe()`) so a corrupted `ownership.env` is reported
   and left alone, never turned into broad `rm -rf`. `install.sh` now
   explicitly `chown root:root`/`chmod 0755`s `/opt/vpn1` instead of
   relying on umask. Confirmed via code reading (VERIFIED-CODE, no
   change needed): SSH firewall state is never touched anywhere in
   uninstall.sh; services/timers are stopped before state removal;
   ownership-aware preserve-vs-remove logic for nginx/certbot/firewall/
   packages/Rust toolchain was already correct. Found+fixed a real bug
   in my OWN new safety check while testing it: the group/world-writable
   mode check only inspected the trailing octal digit (missed mode 775 —
   group-writable, common for shared groups); fixed to check the group
   and other digits' write bit independently. New tests:
   `deploy/lib/tests/test-uninstall-hardening.sh` (14 checks) +
   `ownership_path_is_safe()` cases added to `test-ownership-manifest.sh`.
   PR #20's first real CI run (not reproducible in the sandbox — the
   sandbox's checkout happens to be root-owned) caught 2 real bugs the
   FAST GATE could not: the new root-controlled-execution check firing
   on any non-`/opt/vpn1` checkout (fixed: scoped to `/opt/vpn1` only),
   and a pre-existing (checkpoint 3) `ci.yml` bug appending
   `versions.env`'s comment header raw to `$GITHUB_ENV` (fixed: filter
   comments/blanks; regression test added). Lesson: FAST GATE running as
   root in this sandbox cannot catch non-root-checkout-specific bugs —
   real CI is still load-bearing for this class of issue.
7. **Client protocol behavior audit** (this checkpoint): read the
   subscription renderer, server config generator, and every client doc
   against 11 client-correctness requirements — nearly all were already
   correctly implemented and honestly documented (per-transport
   `SelectionProfile`, deterministic REALITY default with `auto` always
   available, disabled/revoked-user 404 + no-store + no-token-logging,
   HIDDIFY_IOS.md's "four different claims" framework already
   distinguishing server/client/OS-controlled behavior). Ran the real
   sing-box interop suite for the first time in this session with a real
   network-fetched pinned binary — all 9 tests (REALITY + Hysteria2,
   matched/mismatched credentials, obfuscation, Brutal bandwidth) PASS —
   upgrading `docs/CLIENT_COMPATIBILITY.md`'s sing-box row from
   "schema-checked" to "verified" and correcting its stale "no network
   capability in this sandbox" framing. New
   `docs/CLIENT_PROTOCOL_BEHAVIOR.md`: the single protocol-level
   reference for what vpn1 controls vs. client-controlled vs.
   unguaranteed — full-device tunneling (config has no `inbounds`/TUN
   directive at all), DNS (no `dns` block, no leak-prevention claim),
   IPv4/IPv6 (dual-stack `"listen": "::"` server-side, no client route
   preference expressed), MTU (no override, none needed), UDP/TCP
   (Hysteria2 UDP not assumed to work everywhere; REALITY stays
   selectable if it doesn't), failover honesty (`urltest`/`auto` is
   latency-only, never called censorship-aware). Two new regression
   tests lock the honesty claims in code: the client subscription must
   never gain a `dns` or `inbounds` block, and both server inbounds must
   keep binding the dual-stack wildcard. Enriched
   `docs/DEVICE_ACCEPTANCE_TESTS.md`'s per-row template with the exact
   fields this task's REAL CLIENT ACCEPTANCE section asks for (observed
   public IP, DNS leak result, IPv6 result, tunnel-drop behavior,
   required client-side setting) plus a non-benchmark performance
   sanity-check template. No real device/client test was run — none
   available this session; see Remaining UNVERIFIED in the final report
   for this checkpoint.

## Blockers

- `cargo test -p admin --test cli` has 5 failures **only when run as
  root** (this sandbox is root): `apply_restored_file_policy()` only does
  a real `getgrnam("vpn-subscription")` lookup when `geteuid()==0`, and
  that group doesn't exist outside a real installed host. Pre-existing
  (reproduced via `git stash` on prior commits); does not fail in CI
  (non-root runner). `deploy/lib/fast-gate.sh` therefore reports FAIL
  when run as root; PASS expected in CI/any non-root dev environment.

## Important UNVERIFIED items (require real VPS/network/client testing)

- Public reachability, provider firewall correctness.
- Real VLESS+REALITY / Hysteria2 connectivity from an actual Hiddify
  client on a real device.
- Full clean-VPS install -> update -> migrate -> rollback -> uninstall
  lifecycle, including SSH-preservation, on real AlmaLinux 9 (no
  disposable host available this session — `lifecycle-acceptance.sh` not
  executed; migration correctness above is fixture/unit-test-verified
  only, not exercised against a real upgraded deployment).
- DNS leak prevention, IPv6 correctness, WebRTC, kill-switch behavior,
  censorship resistance against a real adversary.

## Canonical verification commands (supported product)

```bash
# FAST GATE — one command, run after every change (see Blockers re: root)
bash deploy/lib/fast-gate.sh

# DESTRUCTIVE lifecycle gate — disposable AlmaLinux 9 host ONLY, over SSH
./deploy/almalinux/lifecycle-acceptance.sh \
  --host root@DISPOSABLE-HOST --i-understand-this-is-destructive

# state schema validate/migrate
vpn-admin --config /etc/vpn/deployment.toml config validate
vpn-admin --config /etc/vpn/deployment.toml config migrate

# offline uninstall (new this checkpoint) — no network access needed
sudo /opt/vpn1/bin/vpn1-uninstall --yes
```

## Next logical checkpoint

Prompt 8: push the first real `vX.Y.Z` release tag, then run
`deploy/almalinux/lifecycle-acceptance.sh` against a real disposable
AlmaLinux 9 host AND, on that same host, work through
`docs/DEVICE_ACCEPTANCE_TESTS.md`'s matrix with at least one real Hiddify
device — this is now the single highest-value remaining gap across
*every* prior checkpoint: SSH preservation, rollback, idempotency, ACME
restoration, config migration, offline uninstall, AND real client
connectivity/DNS/IPv6 behavior are all code-read/unit/fixture verified
only, never exercised end-to-end against a real host + real device.
Until then, pick one concrete supported-product defect/gap and fix it
with minimum churn, running `deploy/lib/fast-gate.sh` after every
change.
