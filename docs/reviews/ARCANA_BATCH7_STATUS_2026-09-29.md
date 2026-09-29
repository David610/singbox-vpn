# Arcana Batch 7a Status — 2026-09-29

Scope: a deliberately narrowed slice of Batch 7 (three self-contained areas
that don't require tracing vpn-web's contracts first). Branch
`claude/arcana-data-plane-production-final`, starting commit `7fea0f5`.
PR #123 was **not** merged.

Areas 1 (real protocol health / vpn-web contract tracing), 2 (revision-apply
dynamic-state preservation), and 5 (host C-16 firewall lifecycle wiring):
**NOT ATTEMPTED THIS BATCH** — explicitly out of scope for 7a, left for
separate follow-up batches.

## Area 3 — Safe uninstall ownership (F13)

**Status: FIXED (pre-existing, verified + one new regression guard added)**

Reading the current `deploy/almalinux/install.sh` / `uninstall.sh` /
`deploy/lib/ownership.sh` before writing anything (per instructions) found
that F13 had already been substantially closed by a prior batch:

- `deploy/lib/ownership.sh` implements a real ownership manifest
  (`ownership_init/_set/_get/_mark/_is_marked/_list_add/_list_get/
  _set_baseline_once/_path_is_safe`), incrementally written from install.sh
  stage 1 onward, not only at the end.
- `uninstall.sh` gates the sing-box binary removal on
  `ownership_get SINGBOX_BIN_PRE_EXISTED` (line ~223) and the
  provisioning-agent binary removal on `AGENT_BINARY_INSTALLED` +
  `AGENT_BINARY_PRE_EXISTED` (line ~211) — a pre-existing binary at either
  path is left untouched.
- `ownership_path_is_safe()` refuses empty/relative/`..`-containing paths
  before any manifest-sourced path reaches a destructive `rm`.
- `/etc/vpn` removal is scope-gated (checkpoint-2: never blind `rm -rf`
  when singbox-vpn didn't create the whole tree) — see
  `deploy/lib/tests/test-uninstall-ownership-checkpoint2.sh`.
- Existing root-requiring test coverage in `deploy/lib/tests/`:
  `test-ownership-manifest.sh`, `test-uninstall-ownership-parity.sh`,
  `test-uninstall-ownership-checkpoint2.sh`, `test-uninstall-idempotency.sh`,
  `test-uninstall-hardening.sh`, `test-ambiguous-preexisting-residue.sh`,
  `test-install-manifest-idempotency.sh`. CI's `shell` job runs these under
  sudo (real privileged uninstall behavior); this sandbox has no sudo, so
  they were verified by reading, not by execution here.

Added: `deploy/lib/tests/test-f13-binary-ownership-invariant.sh` — a
static, no-root, source-level regression guard asserting (1) the sing-box
binary `rm` is textually inside the `SINGBOX_BIN_PRE_EXISTED`-gated branch,
(2) exactly one such removal site exists (no second, ungated copy), (3) the
provisioning-agent binary removal checks both ownership facts, (4) the
binary path is a literal constant, never built from a manifest string
(path-substitution defense-in-depth). Ran locally in this sandbox: **4/4
checks pass**.

**Remaining risk**: the symlink-attack/path-substitution and
tampered-manifest scenarios rely on `rm -f` never following a symlink to
its target (true by POSIX semantics) plus `ownership_path_is_safe()`'s
traversal checks — not independently re-verified against a real symlink on
a live filesystem in this sandbox (no root). The root-requiring test
scripts listed above are the ones that exercise that for real, in CI.

## Area 4 — Probe-target SSRF

**Status: FIXED for the core SSRF vector (destination-IP validation +
DNS-rebind pinning); PARTIAL on probe-privilege reduction**

`apps/provisioning-agent/src/protocol_probe.rs`'s `fetch_targets` pulls
probe destinations from the control plane (`GET /api/agent/probe-targets`)
— per the module's own doc comment, "The Worker control plane cannot
speak UDP..." implies it is not assumed fully trustworthy — and feeds the
resulting host straight into a sing-box client's `server` field with no
validation. That is the actual SSRF vector: a compromised/buggy control
plane returning e.g. `127.0.0.1`, `169.254.169.254`, or an internal
RFC1918 host as a "peer target" would previously be dialed by the root
provisioning agent's spawned sing-box process.

Fix: new `apps/provisioning-agent/src/ssrf_guard.rs`, wired into
`protocol_probe::probe_target` ahead of every probe (both `static_targets`
and `fetch_targets` origins — defense in depth, since the operator's own
config could also be a rendered artifact from a compromised control plane
in the fleet-deploy path). `resolve_and_validate(host, port)`:
- IP literal: validated directly.
- Hostname: resolved exactly once via `std::net::ToSocketAddrs`; **every**
  resolved address is checked (not just the first), and the endpoint is
  rewritten to the validated IP literal before the sing-box client config
  is built — so the actual TCP connect targets that pinned IP, not a
  second resolution of the hostname, closing the DNS-rebinding window.
- Denies: loopback (v4/v6), RFC1918, CGNAT (100.64.0.0/10), IPv4/IPv6
  link-local, IPv6 ULA (fc00::/7), the metadata address
  169.254.169.254, IPv4-mapped IPv6 wrapping any denied v4 address,
  multicast/broadcast/unspecified, RFC 5737/3849 documentation ranges,
  RFC 2544 benchmarking (198.18.0.0/15).
- A denied destination is skipped with a `tracing::warn!` (host, reason,
  node_id) and simply not probed — the round continues for other targets.

**Not attempted / partial**:
- The "allowed probe model" (self-test target / known fleet node / approved
  public endpoint, validated against trusted node/fleet identity data) is
  only partially satisfied: this fix adds IP-range validation as a
  necessary floor for *every* target regardless of origin, but does not
  add positive allowlisting against a fleet-identity source — that would
  require tracing how `fetch_targets`' control-plane response is
  authenticated/scoped, which is vpn-web-contract territory explicitly out
  of scope for 7a (Area 1).
- Redirect-following protection: not applicable — the prober never follows
  HTTP redirects across origins; the sing-box client is configured with a
  single fixed outbound per probe.
- Unix-socket-style URL tricks: `Endpoint::parse` only accepts `vless`/
  `hysteria2`/`hy2` schemes and rejects everything else already (existing
  code, unit-tested); no change needed.
- Probe privilege drop: reviewed, not changed. No systemd unit for
  `vpn-provisioning-agent` exists under `deploy/almalinux/systemd/` in this
  repo (the binary/service references in `install.sh`/`uninstall.sh` target
  a unit that must come from elsewhere in the fleet-bootstrap path, not
  checked into this tree). The probe child itself is a SOCKS/mixed sing-box
  client with no TUN device, no raw sockets, no privileged bind — it does
  not need root or any Linux capability to do what it does. Documented here
  rather than changed, since there is no in-repo unit file to harden.

15 unit tests in `ssrf_guard.rs` (all local, no root): explicit deny for
127.0.0.1/127.0.0.2/::1/10.0.0.1/172.16.0.1/192.168.1.1/169.254.169.254/
fc00::1/fe80::1/IPv4-mapped-private; explicit allow for public literals
(1.1.1.1, 8.8.8.8, 2606:4700:...); CGNAT boundary tests.

## Area 6 — Own-public-IP egress gap

**Status: PARTIAL — closed for the IP-literal case, not the hostname case**

`crates/compat-config/src/server.rs`'s `apply_c16_egress_policy` previously
had an explicit, documented decision to never reject the node's own public
IP, reasoning that rendering has no way to learn it "without an extra
network call." That reasoning does not hold when `DeploymentConfig::
public_host` (the same trusted value already used to render share links —
see `crates/compat-config/src/render.rs`) is itself an IP literal, which is
a supported and real deployment shape (the field doc says "Public
hostname/IP").

Fix: `apply_c16_egress_policy` now takes `Option<&str>` for the deployment's
`public_host`; when it parses as an `IpAddr`, a 5th C-16 rule
(`ip_cidr: [ip/32 or ip/128]`, reject) is appended, denying a customer
tunnel destination equal to the node's own public address, exactly like
the other C-16 ranges. `c16_egress_policy_rule_count` updated accordingly.
This only affects the VLESS/Hysteria2 listener route tables (customer
tunnel traffic); it does not touch host-originated traffic, host firewall
policy, or anything binding decisions — a host's own use of its address is
unaffected.

**Not closed**: when `public_host` is a hostname (the common/default case
in every existing test fixture and, per the field's own doc comment, the
typical production shape), no rule is added — turning an arbitrary
hostname into a safe IP-based deny would require either (a) a DNS
resolution step inside config rendering (explicitly what the task said to
avoid — "prefer using that existing trusted value... rather than adding a
new network call into config rendering") or (b) persisting a separately
resolved/operator-declared public-IP field that does not currently exist
in `DeploymentConfig`. Adding such a field, its migration, its population
path (bootstrap? `vpn-admin` command? both?), and CLI/doctor plumbing is a
real schema change beyond this narrowed batch's budget — flagged as the
concrete next step for whoever picks up Area 6 fully. The task's own test
list ("node public IPv4 X... deny", "node public IPv6 Y... deny") is
satisfied for the IP-literal case tested below.

Three new tests in `crates/compat-config/tests/relay_role_policy.rs`:
`exit_denies_customer_tunnel_to_its_own_public_ipv4`,
`exit_denies_customer_tunnel_to_its_own_public_ipv6`,
`exit_with_hostname_public_host_adds_no_extra_own_ip_rule` (documents the
gap above as expected/tested behavior, not a silent miss).

## Commits

- `4601cd6` — test(deploy): add static F13 regression guard for
  binary-ownership gating (Area 3)
- `e593dbe` — fix(provisioning-agent): SSRF guard for protocol-probe
  destinations (Area 4)
- `689c12f` — fix(compat-config): deny customer tunnel access to node's own
  public IP (Area 6)

All pushed to `claude/arcana-data-plane-production-final`
(`7fea0f5` → `689c12f`).

## Local validation (this sandbox, no sudo/root/CAP_NET_ADMIN)

- `cargo fmt --all -- --check` — clean.
- `cargo clippy --locked --workspace --all-targets -- -D warnings` — clean,
  0 warnings.
- `cargo test --locked --workspace` — **80 passed / 1 failed** in the
  `admin` crate's `udp_probe_tests::cert_expiry_days_reports_positive_days_
  for_a_freshly_issued_cert`: pre-existing, unrelated to this batch's
  changes — fails because `openssl` on this Windows sandbox can't find its
  config file (`Can't open "D:\Program Files\PostgreSQL\psqlODBC\etc\
  openssl.cnf"` — a stray `OPENSSL_CONF` pointing at a PostgreSQL-bundled
  OpenSSL on this machine), not a code defect. All other workspace crates'
  tests, including every test touched by this batch
  (`compat-config` 62+3=65/65, `provisioning-agent` 87/87, all new
  `ssrf_guard` and Area 6 tests), passed.
- `deploy/lib/tests/test-f13-binary-ownership-invariant.sh` — 4/4 checks
  pass locally (no root needed).

## Real CI

Pushed to `689c12f`; `gh pr checks 123` was polled but had not settled by
report time — **CI result not yet available; do not assume pass or fail.**
Whoever reads this next should re-run `gh pr checks 123` for the real
outcome, in particular the `shell` job (which is the one that would
actually exercise Area 3's root-requiring uninstall tests against real
privileged behavior — not verifiable in this sandbox).

## Human input needed

- Area 6's hostname case is a real, acknowledged gap: decide whether to
  add a persisted `public_ipv4`/`public_ipv6` field to `DeploymentConfig`
  (schema bump) as the proper fix, or accept IP-literal-only `public_host`
  deployments as the supported path for this protection.
- Area 4's "known fleet node" positive-allowlist model (vs. this batch's
  IP-range floor) needs the vpn-web contract tracing explicitly deferred
  to a separate follow-up (original Batch 7 Area "real protocol health").
- Confirm real CI status on PR #123 once it settles.
