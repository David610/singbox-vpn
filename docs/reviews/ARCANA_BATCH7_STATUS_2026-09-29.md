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

## Human input needed (as of 7a)

- ~~Area 6's hostname case is a real, acknowledged gap~~ — closed in 7b,
  see below.
- Area 4's "known fleet node" positive-allowlist model (vs. this batch's
  IP-range floor) needs the vpn-web contract tracing explicitly deferred
  to a separate follow-up (original Batch 7 Area "real protocol health").

---

# Batch 7b addendum — 2026-09-29 (Areas 6 close-out + 2)

Continues from `ea52484`. Two tasks: closing Area 6's hostname gap, and
auditing/extending Area 2 (revision-apply dynamic-state preservation).

## Area 6 — Own-public-IP egress gap (hostname case)

**Status: FIXED (both IP-literal `public_host` and hostname `public_host`
cases now covered) — closed as far as this remediation's own scope goes.**

Field/schema decision: added `DeploymentConfig::public_ipv4: Option<String>`
and `public_ipv6: Option<String>` (`crates/compat-config/src/deployment.rs`).
Both are optional and additive — no `DEPLOYMENT_SCHEMA_VERSION` bump, no
migration step, because `#[serde(default)]` on an `Option` already makes
every pre-existing `deployment.toml` (which has neither key) load, validate,
and render identically to before. `validate()` rejects a non-empty value
that doesn't parse as the correct IP family (IPv4 literal in `public_ipv4`,
IPv6 in `public_ipv6`).

`apply_c16_egress_policy` (`crates/compat-config/src/server.rs`) now takes
two more `Option<&str>` parameters and folds `public_ipv4`/`public_ipv6` in
alongside the existing IP-literal-`public_host` check, collapsing all
addresses that are set into one combined deny rule (so the leading-rule
count callers rely on — `c16_egress_policy_rule_count` — stays 0/4/5, never
6+, regardless of how many of the three sources are populated). This is the
"general fix" requested: it denies customer-tunnel egress to the node's own
address whether `public_host` is a hostname (the common case) or an IP
literal, as long as `public_ipv4`/`public_ipv6` is populated.

Population path at install time (`deploy/almalinux/install.sh`): the
installer already resolves the node's public IP via
`preflight_detect_public_ip` (three third-party IP-echo services), but
*only* on the auto-detect/sslip.io path taken when the operator does not
supply their own `PUBLIC_HOST`/`--domain`. That resolved `PUBLIC_IP` is now
also written to `public_ipv4` in the rendered `deployment.toml`
(`render_deployment_toml`), at no additional network cost since the lookup
was already happening. When the operator supplies their own domain (the
documented/recommended real-deployment path,
`docs/SUPPORTED_PRODUCT.md`), this installer does **not** add a new
"call an IP-detection service" step just to populate this field — that
would be a new network side-effect for a security hardening feature, not
something this remediation's stated pattern condones. In that case
`public_ipv4`/`public_ipv6` stay unset (honest: this protection is simply
not available yet for that node) unless the operator sets
`PUBLIC_IPV4=...` before running the installer, or the control plane
supplies it later via a future `APPLY_NODE_REVISION` extension (see Area 2
below — no such extension exists today; `APPLY_NODE_REVISION`'s current
payload is a users-store document only and has no way to change
`deployment.toml` at all, so "control plane sets it via revision apply" is
aspirational, not implemented, until that mechanism is extended).

Tests (`crates/compat-config/tests/relay_role_policy.rs`, all passing
locally — 71/71 in that file):
`exit_with_hostname_public_host_and_public_ipv4_field_denies_customer_tunnel`,
`exit_with_hostname_public_host_and_public_ipv6_field_denies_customer_tunnel`,
`exit_with_public_ipv4_and_public_ipv6_both_set_combines_into_one_rule`,
`deployment_toml_without_public_ipv4_ipv6_fields_still_loads_and_validates`
(backward compat), `invalid_public_ipv4_literal_is_rejected_by_validate`,
`ipv6_literal_in_public_ipv4_field_is_rejected`. The pre-existing
IP-literal-`public_host` tests from 7a are unchanged and still pass.

Remaining risk: the operator-supplied-domain install path still leaves
`public_ipv4`/`public_ipv6` unset by default — this is a real residual gap
in *coverage* (not correctness: when unset, behavior is exactly the
pre-7b baseline, never worse), and closing it fully requires either an
operator-facing flag/prompt in install.sh (not added here — out of the
requested scope, which was schema + wiring) or the control-plane
revision-apply extension noted above.

## Area 2 — Revision apply must preserve dynamic state

**Status: PARTIAL — audited and proven correct for what
`APPLY_NODE_REVISION` actually does today; the task's premise that it
"re-renders the entire config, wholesale replacing whatever was on disk"
is only half true, and that distinction is the main finding.**

Read `cmd_apply_revision`/`apply_users_and_save`
(`apps/admin/src/main.rs`, ~L3670-3835) end-to-end. Findings:

- **STATIC state (protocol ports, role, hardening, `public_host`,
  certificates) lives entirely in `deployment.toml`**, loaded once at CLI
  startup (`cfg: &DeploymentConfig`) and passed through
  `cmd_apply_revision` unchanged. `cmd_apply_revision` never writes
  `deployment.toml`. There is currently **no mechanism** for
  `APPLY_NODE_REVISION` to change a static field — a protocol port or
  hardening flag cannot be changed via this command at all today. This
  contradicts the task prompt's assumption ("apply a new node revision
  (change something static, e.g. a protocol port...)") — that scenario is
  not expressible with the current `apply-revision` CLI surface, so it was
  not (and could not be) tested; flagging this gap explicitly rather than
  fabricating a static-revision-apply feature that doesn't exist in this
  codebase.
- **DYNAMIC state (customer credentials, lease-pool slot users, the
  reserved probe principal, C-16-policy-generated route rules) all lives
  in `users.json`**, and `APPLY_NODE_REVISION`'s entire payload is a
  users-store document (`store::parse_users_bytes`) that **wholesale
  replaces** the previous `users.json` via `apply_users_and_save` →
  `render_and_apply_singbox_config`. There is no merge logic on the node
  side: the reserved probe, lease users, and every existing customer
  survive a revision apply **only because and to the extent the caller
  (vpn-web) includes them in the revision document it pushes** — this is a
  control-plane responsibility outside this repo's code, not something the
  node enforces. C-16 policy and probe confinement are not stored data at
  all; they are re-derived at render time from `cfg.role`/`users` by
  `apply_c16_egress_policy`/`apply_probe_user_confinement`, so they
  "survive" a revision apply simply because the renderer always adds them
  fresh on every call, regardless of what the revision document contains.
- The existing pipeline is already fail-closed: `render_and_apply_
  singbox_config` does fingerprint-short-circuit, `sing-box check`
  validation, atomic rename, reload+verify, and rollback-from-backup on
  failure; `cmd_apply_revision` adds only revision-number staleness
  guarding and stamping on top, unchanged by this batch.

Extended test coverage (`apps/admin/tests/cli.rs`, both `#[cfg(unix)]`
alongside every other `apply-revision` test in this file):
- `apply_revision_preserves_all_dynamic_state_types_simultaneously_static_config_untouched`
  — 2 customers + reserved probe + 1 lease-pool slot user (`lease-0001`,
  recognized by `apps/admin/src/lease_pool.rs::is_lease_user`'s id-prefix
  check) applied together in one revision. Confirms probe confinement (4
  rules), C-16 policy (4 rules), all 4 users present and correctly typed
  in `users.json` after the apply, **and** that `deployment.toml` is
  byte-identical before/after — the direct proof of the static/dynamic
  separation above.
- `apply_revision_reload_failure_rolls_back_full_dynamic_state_mix` — same
  multi-type dynamic-state mix, but the new revision's reload fails.
  Confirms `config.json`, `users.json`, and the revision stamp all roll
  back to the exact previous state (never a half-old/half-new mix),
  extending the existing single-dynamic-state-type
  `apply_revision_reload_failure_rolls_back_config_users_and_stamp` test to
  the realistic multi-type scenario the task asked for.

These extend, not replace, the existing Batch 5/6
`apply_revision_preserves_reserved_probe_confinement_and_c16_policy_across_reapply`
test, which already proved the probe+C-16 case across re-renders with a
growing customer set.

**Not verifiable in this sandbox**: both new tests are `#[cfg(unix)]`
(fork fake `systemctl`/`sing-box` shell scripts, as every other
`apply-revision` test in this file does) and could not run on this
Windows sandbox. `cargo test --locked --workspace` here shows 53/53 tests
in `cli.rs` passing, but that binary, compiled for
`x86_64-pc-windows-msvc`, does not even contain the `cfg(unix)` tests —
confirmed via `cargo test -p admin --test cli -- --list`, which does not
list them. The new code was verified only by successful compilation
(rustc parses and macro-expands the whole file, including cfg-stripped
items, so there are no syntax errors) and close structural mirroring of
the adjacent passing tests it was modeled on. **Real pass/fail requires
Linux CI** (see below).

Remaining risk: the "dynamic state is preserved" property this batch
proved is entirely about the node's local pipeline not corrupting or
dropping data it's handed. It says nothing about whether vpn-web's
revision-document construction actually always includes the full current
dynamic-state set (probe, leases, all customers) before pushing a new
revision — that is a vpn-web-side contract this repo cannot verify, and
tracing it is exactly the "Area 1 protocol health / vpn-web contract"
follow-up explicitly out of scope here.

## Commits (7b)

- `e5e3e22` — fix(compat-config): close Area 6 gap for hostname
  public_host via explicit public_ipv4/ipv6 field
- `65d1bd2` — test(admin): Area 2 audit — apply-revision preserves full
  dynamic-state mix, static config untouched

## Local validation (7b, this sandbox, no sudo/root/CAP_NET_ADMIN)

- `cargo fmt --all -- --check` — clean.
- `cargo clippy --locked --workspace --all-targets -- -D warnings` —
  clean, 0 warnings.
- `cargo test --locked --workspace` — same pre-existing, unrelated,
  environment-specific failure as 7a
  (`udp_probe_tests::cert_expiry_days_reports_positive_days_for_a_freshly_issued_cert`,
  broken local OpenSSL config on this Windows box, not a code defect).
  `compat-config` 132/132 (unit) + `relay_role_policy` 71/71 (was 65,
  +6 new Area 6 tests), all other crates unchanged and green.
  `apps/admin`'s `cli.rs` — 53/53 of what compiles on Windows; the 2 new
  Area 2 tests are `#[cfg(unix)]` and did not run here (see above).

## Real CI (7b)

Pushed as `f2acacc`. `gh pr checks 123` polled to full settlement: every
job passed, including `test` (both parallel runs — this is the job that
actually executes `cargo test --workspace` on Linux, confirming the new
`#[cfg(unix)]` Area 2 tests genuinely run and pass, not just compile),
`shell` (privileged uninstall/ownership tests from earlier batches, still
green), and `singbox-validate`.

One job failed: `CodeQL`, reporting 1 high-severity "cleartext logging of
sensitive information" alert at `apps/admin/src/main.rs:2918`
(`println!("Reserved probe user {name:?} created (id {id}).")` in
`cmd_user_create_probe`). Verified via `git diff ea52484 f2acacc --
apps/admin/src/main.rs` (empty) and `git blame -L 2915,2920
apps/admin/src/main.rs` (last touched by pre-existing commit `eca658b3`,
2026-09-29, unrelated to this batch) that this file was not touched by
either 7b commit — this is a pre-existing finding on the branch that
CodeQL is attributing to "new alerts in code changed by this pull
request" because the PR's cumulative diff (not just this batch's) touches
`main.rs` elsewhere. Not investigated further or fixed here: it's outside
this batch's Area 6/Area 2 scope, and the reported severity may be
overstated (`id` here is `generate_user_id()`'s opaque probe-user id, not
a credential/secret — `vless_uuid`/`hysteria2_password` are not part of
this `println!`), but that's a judgment call for whoever owns this
finding, not a call to make while fixing something else. Flagged in
"Human input needed" below.

## Human input needed (7b)

- Area 6: decide whether install.sh should gain an explicit
  `--public-ip`/`PUBLIC_IPV4` prompt or flag for the operator-supplied-domain
  path (currently only the auto-detect/sslip.io path populates the field
  for free); until then, operator-domain deployments get this protection
  only if the operator or control plane sets it explicitly.
- Area 2: the `APPLY_NODE_REVISION` → static-config-change gap is real —
  if the product requirement is "vpn-web can push a port/hardening change
  to a node," that needs a new mechanism (either extending the revision
  payload to optionally include a `deployment.toml` delta, or a separate
  command); nothing in this batch invents that, since it wasn't asked for
  and would be a meaningful new attack surface to design carefully, not a
  narrow fix.
- Areas 1 (protocol health) and 5 (nftables lifecycle) remain untouched,
  as instructed.
- Real CI: a pre-existing (not introduced by this batch — confirmed via
  `git diff`/`git blame`) `CodeQL` high-severity "cleartext logging"
  finding at `apps/admin/src/main.rs:2918` needs triage/owner decision:
  fix the `println!`, or dismiss as a false positive (opaque probe id,
  not a secret) in GitHub's code-scanning UI. Left as-is since it's
  outside this batch's scope and changing it without that decision risks
  masking or misjudging a real finding.
- Confirm real CI status on PR #123 once it settles.

---

# Batch 7c — 2026-09-29 (CodeQL #61 + static-revision-apply scoping)

## Task 1 — CodeQL alert #61 (`rust/cleartext-logging`)

**Status: RESOLVED as false positive (dismissed via GitHub API with
evidence), no code change needed.**

Queried `gh api repos/David610/singbox-vpn/code-scanning/alerts/61` fresh
rather than trusting the 7b addendum's line-2918 note (which turned out to
describe a *different*, still-open alert — #38, `main.rs:2924`, the probe
`println!`, not #61). Alert #61's own current location data:
`apps/admin/src/main.rs:5763`, flow message "This operation writes
`cert_expiry_days(...)` to a log file," analyzed on `refs/heads/main` at
`927d9e6e`.

Traced the real data flow: `cert_expiry_days()` (`main.rs:4228`) returns
`Option<Result<i64, String>>` — a plain integer count of days until a
locally-generated TLS cert's `notAfter` expiry. All 4 call sites
(`main.rs:3966`, `4689`, `5767`, `5782`, confirmed via `grep -n`) feed only
that integer into `writeln!(out, "... expires in {days} day(s)")`-style
diagnostic output for `vpn-doctor`/perf reporting. No password, VLESS UUID,
Hysteria2 password, private key, bearer token, connection URI, or command
output reaches any of these sinks — verified by reading every call site,
not by assumption. This is a demonstrable CodeQL false positive (the
taint tracker over-attributes sensitivity to any value derived from a
path containing `cert`/`key`).

Corroborating evidence: PR #123's own `codeql-rust` check (pull_request
analysis of this branch, not the `refs/heads/main` baseline scan alert #61
is filed against) is currently **green** at head `1ecd2dd` — this exact
code, present on this branch too, does not trip the PR-diff CodeQL gate.

Action taken: dismissed alert #61 via
`gh api repos/David610/singbox-vpn/code-scanning/alerts/61 -X PATCH
-f state=dismissed -f dismissed_reason="false positive" -f
dismissed_comment=...`. API response confirms `"state":"dismissed"`,
`"dismissed_reason":"false positive"`, `"dismissed_by":"David610"` (the
authenticated `gh` identity), `"dismissed_at":"2026-09-29T14:27:11Z"`. No
regression test added — there is no real vulnerability to regress against;
a synthetic-secret test through this path would only prove a negative that
grep already proves.

**Not touched**: alerts #33–60 (17 more `rust/cleartext-logging` findings)
and #22–29 (`rust/hard-coded-cryptographic-value`, mostly in
`hysteria2_interop.rs` test fixtures) exist on the same `refs/heads/main`
baseline scan. Task 1's scope was #61 only. #38 (`main.rs:2924`, the probe
`println!` the 7b addendum actually described) remains open and is a
real, still-undecided triage item — flagged again below, not fixed here
since it's a different alert than what was assigned.

## Task 2 — Static-config revision apply

**Superseded — implemented in Batch 7d (see the "Batch 7d" section at the
end of this document).** The original 7c note is kept below for history.

**Status (7c): NOT ATTEMPTED this batch — scoped out honestly rather than
producing a partial/unverified implementation.**

Per this batch's own stated discipline ("if you cannot finish both with
real verification, finish one completely and document honest partial
status on the other"): Task 1 is now genuinely closed with real evidence.
Task 2 as specified — an atomic, versioned, allowlisted static-config
revision-apply pipeline spanning `crates/compat-config`, `apps/admin`,
`apps/provisioning-agent`'s job dispatcher, four required test classes
(dynamic-preservation, failed-apply rollback, idempotency, stale-revision
rejection), plus a cross-repo `vpn-web` contract trace and a
correspondingly-scoped separate branch/PR there — is architecturally
equivalent in size to Batches 7a+7b combined (which took a full dedicated
pass each for a single narrower area). Starting it without the budget to
finish the atomic-apply/rollback/idempotency state machine and its Linux
`#[cfg(unix)]` test coverage for real would risk exactly what 7b's Area 2
audit already flagged as the live gap: landing code that *claims* to
preserve dynamic state and fail closed without the fork/exec-based tests
that actually exercise `sing-box check`, reload failure, and rollback
paths (this sandbox has no sudo/CAP_NET_ADMIN and those tests don't
compile on `x86_64-pc-windows-msvc` — see 7b's Area 2 section above for
the exact same constraint already hit once).

What is already known and load-bearing for whoever picks this up next
(from 7b's Area 2 audit, still accurate — re-confirmed by re-reading
`cmd_apply_revision` in `apps/admin/src/main.rs` this session): today's
`APPLY_NODE_REVISION` payload is a `users.json`-only document
(`store::parse_users_bytes`); `deployment.toml` (all STATIC fields —
ports, role, hardening, `public_host`/`public_ipv4`/`public_ipv6`,
certificates) is loaded once at CLI startup and never written by this
command. There is currently **no mechanism at all** in this repo for a
remote static-config change — Task 2 would be new functionality, not a
bugfix, which is why it needs its own fully-budgeted pass with real
Linux-CI-verified tests rather than a same-session addendum after Task 1.

Default per the task's own instruction stands and is **not contradicted**
by anything found while re-reading `cmd_apply_revision`,
`apply_c16_egress_policy`, or `apply_probe_user_confinement` this session:
role must remain NOT remotely mutable via any future routine-revision
path — C-16 policy and probe confinement are both derived directly from
`cfg.role` at render time, so a role flip would silently rewrite security
policy underneath any customer/lease/probe state a static revision was
never meant to touch.

`vpn-web` checkout: not inspected this session (Task 2 not started, so
there was nothing to trace against it yet).

## Commits (7c)

- (none for code — Task 1 was resolved via the GitHub code-scanning API,
  not a repository change; no commit was needed or created)

## Real CI

Unchanged from 7b's last polled state: `gh pr checks 123` at head
`1ecd2dd` — all jobs pass, including `codeql-rust`, `secret-logging-check`,
`test` (both parallel runs), `shell`, `singbox-validate`. Re-verified this
session (see checks list above, polled once, already settled — no
in-flight jobs to wait on since no new commit was pushed).

## Human input needed (7c)

- Task 2 (static-config revision apply) needs a dedicated, fully-budgeted
  batch of its own — not attempted here, see above for exactly what's
  already known (payload shape today, the role-immutability default, the
  test-infrastructure constraint) so the next pass doesn't have to
  re-derive it.
- Alert #38 (`main.rs:2924`, probe-creation `println!` of an opaque
  user id) is a separate, still-open, still-undecided CodeQL finding from
  7b — needs the same fix-or-dismiss triage call Task 1 just resolved for
  #61, but is a different alert and was not in scope for this batch's
  Task 1 assignment.
- Areas 1 (protocol health) and 5 (nftables lifecycle) remain untouched,
  as instructed — still the two remaining areas before real two-VPS
  acceptance.


---

# Batch 7d — 2026-09-29 (static-config revision apply: IMPLEMENTED)

**Status: FIXED (singbox-vpn side, Linux-CI-verified) + vpn-web contract
side implemented on a separate branch (not merged, no PR opened).**

## Architecture traced (re-verified this session)

- singbox-vpn: `APPLY_NODE_REVISION` job payload is `{"revision": N}`; the
  agent (`apps/provisioning-agent/src/dispatch.rs::apply_node_revision`)
  fetches `GET /api/agent/revision/N` (node-scoped bearer auth), writes the
  response's `config` value to a 0600 temp file and runs
  `vpn-admin apply-revision --revision N --input <file>`. Before this batch,
  `cmd_apply_revision` only accepted a `users.json`-shaped document and
  never wrote `deployment.toml`.
- vpn-web (`D:\David610\vpn-web`, branch `claude/arcana-control-plane-remediation`
  @ `1b8f52b`): `node_revisions` table (immutable, `(node_id, revision)`
  unique, `config jsonb`), `functions/lib/node-revisions.js::createNodeRevision`
  (insert row → `nodes.desired_revision` → coalesced `APPLY_NODE_REVISION`
  job with idempotency key `apply-revision:<node>:<rev>`),
  `POST /api/admin/nodes/:id/revisions` (admin, non-readonly, audited),
  `GET /api/agent/revision/:revision` (returns `{revision, config}` scoped to
  the authenticated node), `observed_revision` written only from the node's
  heartbeat. vpn-web had **no static-config concept**: `config` was any JSON
  object, validated only as "is an object".
- Design decision: reuse that exact wire path (no new job type, no new
  endpoint). A static revision is just a different `config` document shape,
  sharing the same per-node monotonic revision number.
- Re-verified: C-16 egress policy and reserved-probe confinement are still
  derived from `cfg.role` (+ `public_host`/`public_ipv4`/`public_ipv6`) at
  render time (`crates/compat-config/src/server.rs`), so role/public identity
  stay non-revisable here.

## Wire format / schema

```json
{ "revision_schema": 1,
  "static_config": {
    "reality":   { "handshake_server": "www.example.org" },
    "hysteria2": { "up_mbps": 200, "down_mbps": 200 },
    "udp_probe": { "ipv4_resolvers": ["1.1.1.1"], "ipv6_resolvers": [],
                   "retries": 2, "timeout_ms": 2000, "delay_ms": 250 } } }
```

- Only `revision_schema` + `static_config` at top level; any other key
  (including `users`) rejects. Static and dynamic changes are separate
  revisions — a static revision never carries or touches users.
- `revision_schema` must be exactly integer `1`; anything else rejects.
- Present field = desired value; absent field = unchanged.
  `hysteria2.up_mbps`/`down_mbps` must be sent together (both numbers, or
  both `null` to clear).
- Max document size 16 KiB; at least one field required.
- Backward safety: an older `vpn-admin` fails to parse this document as a
  users store (no `schema_version`/`users` array) and rejects the revision,
  rather than silently "succeeding" with no static change.
- Routing is by presence of `revision_schema` or `static_config`; the whole
  document is parsed before routing, so an oversized document carrying
  `static_config` can never fall through to the users path.

## Field classification

**Allowed (routine-revisable, schema 1):**
`reality.handshake_server`; `hysteria2.up_mbps`, `hysteria2.down_mbps`;
`udp_probe.ipv4_resolvers`, `udp_probe.ipv6_resolvers`, `udp_probe.retries`,
`udp_probe.timeout_ms`, `udp_probe.delay_ms`.

Value validation: bandwidth 1..=100000; retries 1..=10; timeout_ms
100..=30000; delay_ms 0..=10000; IPv4 resolvers 1..=8, IPv6 0..=8, each a
public unicast literal of the right family, no duplicates (loopback,
RFC1918, link-local incl. 169.254.169.254, CGNAT, ULA, multicast,
documentation, IPv4-mapped-private all refused). `handshake_server` must be
a lowercase public DNS name — no IP literals, single labels or
local/reserved suffixes (`.internal`, `.local`, `localhost`, …) — because
REALITY forwards every unauthenticated connection to it.

**Forbidden (privileged reprovision only — out of scope):** `schema_version`,
`node_id`, `role`, `public_host`, `subscription_host`, `public_ipv4`,
`public_ipv6`, `state_dir`, `singbox_binary`, `access_paths`,
`peer_endpoints`, `google_egress_hairpin`, `subscription`. `role` is
therefore NOT remotely mutable: no exit↔relay conversion via this path.

**Recognized but deferred (rejected in schema 1):** `reality.listen_port`,
`reality.handshake_port`, `hysteria2.listen_port`. A listener port move also
needs host-firewall changes, relay-peer forwarding changes (relay nft rules
target the exit's port) and client-profile rollout; this node-local
operation cannot apply or verify those atomically. Needs its own design.

**Dynamic state (never touched by this path):** customer credentials, lease
users, reserved probe principal, temporary authorization, route/mode
credentials — all in `users.json`, which the static path reads but never
writes; usage/accounting state is not in scope of the files touched.

Unknown section / unknown field / forbidden field → the WHOLE revision is
rejected before any disk write (fail closed, never partially applied).

## Atomic apply flow (`apps/admin/src/static_apply.rs`)

1. staleness guard (shared stamp; see below) → 2. parse + validate schema,
size, allowlist, values → 3. recover an interrupted earlier static apply if
its fixed-name backup exists (restore toml, re-render/reload, restart
subscription) → 4. plan candidate `deployment.toml` in memory
(`plan_static_revision`: reparse + `validate()` + structural check that the
normalized config differs ONLY in allowlisted fields) → 5. load
`users.json` (dynamic snapshot, read-only) → 6. pre-flight: render current
and candidate sing-box documents with the same users/time and refuse unless
`route`, `outbounds` and `dns` are identical (C-16, probe confinement,
relay forwarding policy cannot change) → 7. write fixed-name backup
`deployment.toml.static-revision.bak` (0600), atomically write candidate
(original mode/ownership preserved) → 8. `render_and_apply_singbox_config`
(render → `sing-box check` → atomic swap → reload + verify → REALITY
self-test, with its own config.json rollback) → 9. if Hysteria2
bandwidth/handshake server changed, restart `vpn-subscription` (it caches
endpoints at startup) → 10. commit revision stamp, delete backup.

**Rollback:** any failure in 8–10 restores the original `deployment.toml`,
re-renders and reloads sing-box from the old static config with the
unchanged users, restarts `vpn-subscription` again if it had been
restarted, and leaves the stamp unchanged. Security policy is never weaker
at any point: the pre-flight guarantees the candidate's route/outbound
policy is identical to the current one, and `role`/identity cannot change.
If rollback itself fails, the backup is kept and the next static revision
restores from it first; the error says so explicitly.

## Idempotency / stale handling

- Shared monotonic stamp (`users/applied_revision.json`) with users
  revisions. Revision `<=` applied → no-op with a "not newer than the
  already-applied revision" message, exit 0 (existing semantics; the job
  completes, `observed_revision` stays at the newer number). Corrupt stamp →
  fail closed (existing).
- New revision number whose values are already in effect → stamp only, no
  file rewrite, no reload, no restart.
- No timestamped backups; the single fixed-name backup exists only during
  an apply. No credential rotation happens on this path at all.

## Tests

| Required test | Test | Where verified |
|---|---|---|
| 1 dynamic-state preservation | `static_revision_preserves_all_dynamic_state_and_applies_new_static_values` | `#[cfg(unix)]` — Linux CI + WSL |
| 2 failed apply (`sing-box check`) | `static_revision_rejected_by_singbox_check_leaves_everything_unchanged` | unix — CI + WSL |
| 2 failed apply (service restart) | `static_revision_service_failure_rolls_back_static_and_runtime_config` | unix — CI + WSL |
| 2 invalid port / protocol setting | `static_revision_unknown_field_or_invalid_value_rejects_entire_revision` | cross-platform — Windows + CI |
| 3 idempotency | `static_revision_is_idempotent` | unix — CI + WSL |
| 4 stale revision | `static_revision_stale_revision_is_not_applied` | unix — CI + WSL |
| 5 forbidden field (role etc.) | `static_revision_forbidden_role_change_is_rejected_fail_closed` | cross-platform — Windows + CI |
| 6 allowlist enforcement | `static_revision_unknown_field_or_invalid_value_rejects_entire_revision` | cross-platform — Windows + CI |
| crash recovery | `static_revision_recovers_an_interrupted_apply_first` | unix — CI + WSL |
| schema unit tests | `compat_config::static_revision::tests` (11) | Windows + CI |
| cross-repo contract | `crates/compat-config/tests/static_revision_contract.rs` (shared fixture, 6 valid / 53 invalid docs) | Windows + CI |
| agent pass-through | `apply_node_revision_passes_a_static_revision_document_through_verbatim` | unix — CI + WSL |

## vpn-web dependency

Branch `claude/node-static-revision-contract` in `David610/vpn-web`
(based on `claude/arcana-control-plane-remediation` @ `1b8f52b`), commit
`7192cce`, pushed; **no PR opened, nothing merged**. Adds
`functions/lib/static-revision.js` (same allowlist/forbidden/deferred
lists and value rules), makes `POST /api/admin/nodes/:id/revisions` return
400 for an invalid static revision (no row, no desired_revision bump, no
job, no audit), audits accepted ones with `kind: "static"`, and asserts the
byte-identical fixture `functions/lib/__tests__/fixtures/static-revision-v1-contract.json`.
Existing vpn-web semantics already satisfy the rest: desired state is
stored as an immutable revision row + `desired_revision`; the agent job is
authenticated and coalesced; `observed_revision` only advances from the
node's heartbeat after the node stamps a successful apply (never at job
creation); job success/failure flows through the existing
`/api/agent/jobs/:id/complete|fail`; a stale job cannot regress the node
because the node-side stamp is monotonic.

Cross-repo dependency: the singbox-vpn node side is safe without the
vpn-web branch (it validates independently and a static apply can only be
triggered by a document of this exact shape); the vpn-web branch is needed
so invalid static revisions are refused at creation rather than becoming
unconvergeable desired state. Both fixture copies must change together.
Known residual on the vpn-web side (not changed): `heartbeat.js` writes
whatever `observed_revision` the node reports; a delayed out-of-order
heartbeat HTTP request could briefly show an older observed value until the
next heartbeat (display only; the node never regresses).

Local vpn-web validation: `npx vitest run` 102 files / 1079 tests pass;
`npm run lint` 0 errors (11 pre-existing warnings, none in changed files);
`npm run build` OK.

## Local validation (7d)

- Windows: `cargo fmt --all -- --check` clean; `cargo clippy --locked
  --workspace --all-targets -- -D warnings` clean; `cargo audit` clean
  (265 deps); `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`
  clean; cross-platform static-revision tests + compat-config unit/contract
  tests pass.
- WSL Ubuntu 24.04 (real Linux, no CAP_NET_ADMIN): all 8 new admin CLI
  static-revision tests pass (incl. the 6 `#[cfg(unix)]` ones);
  `cargo fmt --check` + clippy clean; full workspace run green except the
  pre-existing `two_hop_system` suite, which needs passwordless
  `sudo ip link add` (environmental, unrelated — CI runs it).
- No shell scripts touched.

## Real CI (7d)

First push (`a1aa5eb`): every job passed on both parallel runs — including
`test` (Linux, executes the `#[cfg(unix)]` tests), `clippy`, `docs`,
`audit`, `shell`, `singbox-validate`, `codeql-rust`, `secret-logging-check`.
The `CodeQL` summary check reported 2 "new" alerts: one genuinely mine
(`apps/admin/tests/cli.rs`, a test-fixture UUID interpolated into an assert
message) — fixed in the follow-up commit by naming the principal instead;
the other is the pre-existing alert #38 (`main.rs` probe-creation
`println!`, same code, only moved down by this batch's inserted lines),
still awaiting the fix-or-dismiss decision flagged in 7b/7c.

## Human input needed (7d)

- Merge/PR decision for vpn-web `claude/node-static-revision-contract`
  (pushed, no PR opened).
- Listener-port revisions (`reality.listen_port`, `hysteria2.listen_port`,
  `reality.handshake_port`) are deliberately deferred: they need a
  coordinated firewall + relay-peer + client rollout design.
- `reality.handshake_server` residual: syntactic public-DNS check only; a
  control-plane-chosen public name could still resolve to an internal
  address on the operator's fixed handshake port. Decide whether to add
  apply-time resolution checks or remove it from the allowlist.
- CodeQL alert #38 triage (pre-existing) still open.
