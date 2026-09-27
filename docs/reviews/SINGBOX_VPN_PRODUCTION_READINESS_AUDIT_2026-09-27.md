# singbox-vpn production-readiness audit — 2026-09-27

Scope: the data plane repository `David610/singbox-vpn` at `main` =
`5cee2fa3b2` (tag `v1.1.0`), with read-only cross-checks against
`David610/vpn-web` (`6b3828e`) and `David610/tamara-next` (`a57060b`).

Method: re-run every automated suite, read the code paths that decide
security and availability, and prove the most important claims live with
the real pinned `sing-box 1.14.1` binary in a local network-namespace lab
(client → relay → exit → "internet", with a REALITY decoy, DNS server and a
simulated cloud-metadata endpoint). No remediation was made. Evidence
scripts: [`evidence/2026-09-27/`](evidence/2026-09-27/).

Evidence labels used below:

- **Verified** — reproduced in this audit (test run, lab run, or a program
  that exercises the same code path).
- **Code-verified** — read in code in this audit, with an exact file/line,
  but not executed end to end.
- **Inferred** — follows from code or docs, but depends on something this
  audit could not observe (for example a real VPS or real client).

---

## Executive Summary

The single-node product (one VPS, VLESS+REALITY + Hysteria2, `vpn-admin`,
`subscription`) is mature for its stated v1.0 scope: pinned and attested
releases, fail-closed config apply, transactional update with rollback,
SSH-safe firewall activation, privacy-first logging, and ~800 passing tests,
including real-binary interop and two-hop loopback system tests. The
v1.1.0 VPS acceptance run on a fresh AlmaLinux 9 host passed.

The fleet path that v1.1.0 now ships (provisioning agent, lease pool,
probes, vpn-web bootstrap) is **not production-ready**. Two problems are
release blockers:

1. **Exit nodes give every VPN user unrestricted egress into the node
   itself** — its loopback services, the cloud metadata endpoint
   (169.254.169.254) and private ranges. Proven live in the lab. A single
   user used this to lock every other user out of subscription delivery
   (HTTP 429) on the lab exit.
2. **By default, every fleet node restarts sing-box about three times an
   hour, forever**, which cuts every open connection of every user on the
   node. The lease pool is on by default (32 slots) and the vpn-web
   bootstrap does not turn it off. Proven by driving the real rotation code
   for 6 simulated hours (19 config changes, one every 1200 s), whether or
   not the control plane ever leases a slot.

Five more P1 issues make the fleet unsafe to run unattended: the agent can
wedge forever on one failed job report (which also stops lease expiry), timed
-out `vpn-admin` runs keep running and apply after the agent already reported
failure, the sing-box download trusts an attacker-replaceable
`checksums.txt` over the pinned hash, fleet health is liveness-only, and
every authorization change restarts sing-box for everyone.

**Verdict: NOT production-ready for a commercial multi-user fleet.**
Conditionally acceptable for the documented v1.0 scope (one self-managed
VPS, ≤10 trusted users) once SVPN-F01 and SVPN-F05 are fixed.

Confidence in this verdict: 8/10. The main limit is that no real VPS,
real device, IPv6 path or real network impairment was available to this
audit (see "Production Verification").

---

## Current Revision

| Item | Value | How it was checked |
|---|---|---|
| `main` / audit branch HEAD | `5cee2fa3b2179d1a18dad02f6b6ca3d7a306a588` = tag `v1.1.0` | `git fetch --all --prune`, `git log` |
| Working tree | clean, no staged/unstaged diff | `git status`, `git diff`, `git diff --cached` |
| Accepted RC | `v1.1.0-rc.6` = `48b82a4`; `v1.1.0` adds only the acceptance doc commit | `docs/release-acceptance/v1.1.0.md`, `git log` |
| CI on `main` | run 725: 19 jobs green (fmt, clippy, test, audit, docs, shell, os-matrix ×7, singbox-validate incl. real interop + two-hop S1–S17, release-container-smoke) | GitHub Actions API |
| VPS acceptance on `v1.1.0` | run 36304477580 (2026-09-27 07:53Z): `LIFECYCLE GATE: PASS`, 0 failing stages, 3 UNVERIFIED items | job log read in full |
| Rust toolchain | 1.94.1 (pinned) | `rust-toolchain.toml` |
| sing-box | 1.14.1, SHA-256 `12cb2816…343f` matches `deploy/lib/versions.env`; upstream has no `checksums.txt` for 1.14.1 (HTTP 404) | downloaded and hashed |
| Cargo dependency set | 265 crates, `cargo audit`: 0 vulnerabilities, 0 warnings | local `cargo audit` |

### Tests re-run in this audit

| Suite | Result | Notes |
|---|---|---|
| `cargo test --workspace --locked` | 798 passed, 0 failed | Without a sing-box binary, the interop and two-hop suites pass by **skipping** (only CI's `SINGBOX_VPN_REQUIRE_REAL_INTEROP=1` turns skips into failures). |
| Real-binary interop + two-hop (`SINGBOX_VPN_REQUIRE_REAL_INTEROP=1`) | 92 run; 90 passed; 2 failed | Both failures bind `[::]` and this audit kernel has no IPv6 (`EAFNOSUPPORT`). Environment, not product. A first run also failed 18 tests because sing-box resolves the REALITY decoy name through its own DNS and the sandbox resolver returns NXDOMAIN for `localhost`; fixed by running a local resolver. |
| `shellcheck -S warning` (CI scope) | clean | info-level: 4× SC2086 (unquoted port in URLs), 16× SC2015 |
| `deploy/lib/tests/*.sh` (51 scripts, as root) | 50 passed, 1 failed | `test-uninstall-idempotency.sh` failed because the real uninstaller **deleted `/usr/local/bin/sing-box` that this audit had installed independently** — see SVPN-F13. |
| `cargo audit` | clean | |

### What the v1.1.0 acceptance record does and does not prove

The record (`docs/release-acceptance/v1.1.0.md`) and run 36304477580 prove,
on one fresh AlmaLinux 9 x86_64 Hetzner host: bootstrap install, reboot,
idempotent re-run, interrupted install cleanup, a real REALITY handshake and
a real Hysteria2 transfer (72 Mbit/s single run), SIGKILL recovery,
`StartLimitBurst` exhaustion + watchdog recovery, update + injected-failure
rollback, backup/restore, `certbot renew --dry-run` with firewall hooks, and
residue-free uninstall. Device acceptance is maintainer-reported (iPhone,
Hiddify).

It does **not** prove anything about: the provisioning agent in operation,
the lease pool, protocol probes, fleet bootstrap through vpn-web, two-hop on
real hosts, other distributions, IPv6-only or dual-stack client behaviour,
external reachability (the run itself marks this UNVERIFIED), load, or
long-running behaviour. The doc does not overclaim these; readers should not
either.

---

## Supported Product

What the code actually ships (not what the docs say):

| Area | Actual state at `v1.1.0` |
|---|---|
| Transports | VLESS + REALITY (TCP/443, Vision flow, mux accepted), Hysteria2 (UDP/443, Salamander obfs on by default after `init`, optional Brutal) |
| Roles | `exit` (unrestricted `direct` egress, no route rules) and `relay` (fail-closed allow-list to declared exits) |
| Binaries | `vpn-admin`/`vpn`, `vpn-subscription-svc`, `vpn-provisioning-agent` (new in 1.1.0, installed by `install.sh` and refreshed by `update.sh`) |
| Client contract | `GET /v1/provision/{token}` schema v1 (Tamara), `GET /sub/{token}` share links / sing-box JSON (Hiddify etc.) |
| Fleet | vpn-web cloud-init → enroll → install pinned release → agent. Agent: jobs (7 imperative + `APPLY_NODE_REVISION`), heartbeat, traffic (only if Clash API configured), lease pool (default ON, 32 slots), protocol probes (only if configured) |
| OS | AlmaLinux 9 x86_64: real-host verified (v1.1.0 run). Everything else: container/fixture only. |

Drift between docs and code (details in SVPN-F08):

- `docs/SUPPORTED_PRODUCT.md` still says "≤10 trusted users", "No v1.0
  multi-node control plane / fleet manager", "Binaries built: `-p admin -p
  subscription` only", and that a fresh AlmaLinux install of HEAD is
  UNVERIFIED. All four are now false or stale.
- `docs/THREAT_MODEL.md` does not mention the provisioning agent, vpn-web,
  enrollment, leases, probes or the node API key at all.
- `docs/TRAFFIC_ACCOUNTING.md` tells operators to add
  `experimental.clash_api` to the sing-box config by hand, but
  `vpn-admin` regenerates that file on every change and never renders a
  Clash API (`crates/compat-config/src/server.rs`), so the edit is lost on
  the next user change.

---

## Threat Model

The existing threat model is sound for a single self-managed VPS. It is
incomplete for what v1.1.0 ships. Missing actors and assets:

| Actor / asset | Why it matters | Covered today? |
|---|---|---|
| Authenticated VPN customer as an attacker | Every paying user holds a credential that lets them send arbitrary traffic from inside the node. | No. The model treats authenticated users as trusted. SVPN-F01, F06 exploit this. |
| Leaked single credential (lease slot, probe credential, shared device) | Same as above, for anyone who gets one credential. | Partly (probe user is confined; customers and lease users are not). |
| Compromised control plane (vpn-web / Supabase) | Can push any user set (`APPLY_NODE_REVISION`), choose probe targets, and makes every node run jobs as root. | No. |
| Compromised node | Holds its permanent agent API key; can read its own revision documents, lease secrets, probe targets (other nodes' probe credentials). | No. |
| Node agent API key | Bearer credential for all `/api/agent/*` endpoints. | No. |
| Revision documents (full user secrets) | vpn-web stores every revision's full `config` forever (`node_revisions`). | No (vpn-web side). |
| Cloud metadata service | Reachable from the exit's network namespace; reachable by customers because of F01. | No. |

Recommendation: extend `docs/THREAT_MODEL.md` with a "fleet" section before
any further fleet feature work, and treat "authenticated customer" as an
untrusted principal for everything except its own tunnel.

---

## Host Security

### Listening sockets (from the real v1.1.0 acceptance host + code)

| Socket | Bind | Proto | Authentication | TLS | Runs as | Rate limit | Intended caller |
|---|---|---|---|---|---|---|---|
| sing-box VLESS+REALITY | `*:443` (`::` dual-stack) | TCP | VLESS UUID + REALITY x25519/short_id | REALITY TLS 1.3 (decoy SNI) | `sing-box`, only `CAP_NET_BIND_SERVICE` | none | clients, relays |
| sing-box Hysteria2 | `*:443` | UDP | per-user password (+ Salamander) | TLS 1.3, Let's Encrypt cert | `sing-box` | none (BBR, or Brutal if set) | clients |
| nginx subscription vhost | `0.0.0.0:8443`, `[::]:8443` | TCP | bearer token in URL path | TLS 1.2/1.3, LE cert | nginx | `limit_req` 20 r/s, burst 100 per client IP | clients, Tamara |
| nginx distro default server | `0.0.0.0:80`, `[::]:80` | TCP | none | none | nginx | none | nobody (acceptance flags it as unexpected; host firewall blocks it) |
| subscription backend | `127.0.0.1:9100` | TCP | bearer token | none (plain HTTP) | `vpn-subscription`, `IPAddressAllow` loopback | one global bucket (1000, 250/s) | nginx — **and any VPN user, see F01/F06** |
| sshd | `0.0.0.0:22`, `[::]:22` | TCP | distro default | SSH | root | none from this repo | operator |
| certbot standalone | `:80` during renewal only | TCP | ACME | none | root | — | Let's Encrypt |
| probe sing-box client (agent) | `127.0.0.1:<ephemeral>` during a probe | TCP | **none** (`mixed` SOCKS/HTTP inbound) | — | **root** | none | the agent |
| Clash API | not rendered → no socket | — | — | — | — | — | — |
| provisioning agent | no listener; outbound HTTPS to vpn-web | — | node API key (Bearer) | rustls + webpki roots | root | — | — |

Firewall (firewalld public zone on the acceptance host): 443/tcp, 443/udp,
8443/tcp opened by singbox-vpn, plus `ssh`. IPv4 and IPv6 share the zone.
External scanning from another host was **not** possible in this audit
(no VPS access); the acceptance run also marks external reachability as
UNVERIFIED.

### systemd units

`sing-box.service` is well hardened: dedicated user, only
`CAP_NET_BIND_SERVICE`, `ProtectSystem=strict`, `ReadOnlyPaths=/etc/vpn/compat`,
`SystemCallFilter=@system-service`, `RestrictAddressFamilies`,
`PrivateDevices`, `ProtectProc=invisible`, memory/tasks limits,
`StartLimitBurst=8/300s` + a 5-minute watchdog timer. Reasonable gaps:
no `MemoryDenyWriteExecute` (fine for Go, but untested), `LimitNOFILE=65535`
(~32k proxied connections, since each uses two fds), `Restart=on-failure`
(a clean exit 0 is not restarted).

`vpn-subscription.service` is well hardened and network-restricted to
loopback.

`vpn-provisioning-agent.service` (shipped here and duplicated verbatim in
vpn-web's bootstrap) runs **as root** with only `NoNewPrivileges`,
`PrivateTmp`, `ProtectHome`, `ProtectKernel*`, `ProtectControlGroups`. No
`ProtectSystem`, no `CapabilityBoundingSet`, no `RestrictAddressFamilies`, no
`SystemCallFilter`. It is the only network-facing, JSON-parsing,
control-plane-driven process on the node, and it spawns sing-box clients as
root (SVPN-F09). The unit comment already says dropping root needs a
narrower privilege model for `vpn-admin`; that work is not done.

`vpn-expiry-reconcile.service` / `vpn-service-watchdog.service`: root
oneshots with light hardening; acceptable.

### File ownership

- `/etc/vpn/compat` `root:vpn-compat 02750`, reality `02750`, users
  `root:vpn-subscription 02750`, hysteria `root:sing-box 02750`; config
  files written 0640 with fsync; lease table 0600 under a 0700 state dir.
  Good.
- `/etc/vpn/compat/sing-box` is created `sing-box:sing-box 02750`
  (`deploy/almalinux/install.sh` `create_directories`). Root later writes
  `config.json.tmp.<pid>` there with `O_CREAT|O_TRUNC` and no
  `O_NOFOLLOW`. The systemd sandbox stops the running service from
  planting a symlink, so this is defense-in-depth only (SVPN-F25,
  Inferred).

---

## Installer

Strong points (Code-verified, many also tested by the shell suite and the
VPS run): stable channel refuses unpinned source; source archive checked
against `SHA256SUMS` **and** a Sigstore provenance bundle bound to
`release.yml@refs/tags/<version>`; cosign itself is pinned by hash; version
inside the archive must match the tag; bounded curl retries; SSH port
positively determined before any firewall activation; ownership manifest
for everything touched; install lock; signal and fatal-error rollback;
dry-run.

Weak points:

- **sing-box verification order is inverted** (SVPN-F05): an upstream
  `checksums.txt` is preferred over the pinned hash, so whoever can publish
  release assets for SagerNet/sing-box can make the installer skip the pin.
- `install_singbox` skips download if `sing-box version` output merely
  **contains** the pinned version string (`grep -q "$SINGBOX_VERSION"`);
  `1.14.1` also matches `1.14.10`. Low impact; noted.
- If `cosign` is already on `PATH`, it is used without verification.
  Root-only PATH, low impact.
- `SINGBOX_VPN_VERSION` is not format-checked in `install.sh` (it is in
  `update.sh`). Operator-supplied, low impact.
- Interrupted installs: covered by `test-installer-signal-rollback.sh`
  (passes) and the VPS stage "interrupted-install cleanup" (passes). SIGKILL
  of the installer itself cannot run traps; not tested here (no VPS).

Tar handling: archives are verified before extraction; the source tree is
normalized to root ownership before it is trusted (fixed in rc.6). No path
traversal risk found in the verified-archive path.

---

## Update/Rollback

`deploy/almalinux/update.sh` is the strongest script in the repo:
version-format check, downgrade refusal unless `--allow-downgrade`,
download + checksum + provenance **before** any live change, target-release
schema pre-check, relay-enforcement capability check (refuses to switch a
relay to a build that would render it as an exit), `.update-new` + `mv`
swaps, transaction marker, rollback of binaries/units/source tree/
`deployment.toml`, post-switch health check and `doctor --protocol`. The
VPS run exercised a real rollback.

Gaps:

- Same sing-box checksum inversion as the installer (`update.sh:759`).
- The update holds `/run/lock/singbox-vpn.lock` through switch, render,
  health check and protocol check. Any agent job during that window blocks
  on the same lock with no timeout; the agent gives up after 60 s but the
  blocked `vpn-admin` keeps waiting and applies later (SVPN-F04).
- The agent binary is refreshed after commit and is outside the rollback
  transaction (deliberate). On hosts updated from a pre-1.1 install this
  also places `/usr/local/bin/vpn-provisioning-agent` without an ownership
  record, so uninstall leaves it behind (residue, P3).
- Fleet nodes have **no in-place update path**: no job type triggers
  `update.sh`, and the bootstrap is "no human, no SSH". Security patches for
  sing-box or the agent mean SSH to each node or node replacement; the
  replace workflow was not verifiable here (SVPN-F16).

Not tested here (no VPS): disk full, read-only FS, reboot mid-update,
wrong architecture on a real host. The shell suite covers transactional
rollback, signal rollback and conditional restart with fixtures.

---

## Uninstall

Mostly careful: ownership-aware restore/remove for fixed paths, refuses to
run from a group/world-writable `/opt/singbox-vpn`, validates package names
from the manifest, restores sysctls/firewall, reports residue truthfully.

Problems (SVPN-F13, **Verified**):

- On a host with **no** ownership record, `SINGBOX_BIN_PRE_EXISTED`
  defaults to `0`, so `/usr/local/bin/sing-box` and its LICENSE are
  deleted (`deploy/almalinux/uninstall.sh:227-231`). The log text of the
  other branch even says "(or ownership could not be determined) —
  leaving it in place", which is the opposite of what the code does. This
  audit's own sing-box binary was deleted by the shell test suite this way.
- The same run stops and disables any `sing-box.service` it finds
  (`uninstall.sh:191-197`) regardless of who installed it, and always
  deletes `/usr/local/bin/vpn`, `vpn-admin`, `vpn-subscription-svc`,
  `vpn-health-check`, `vpn-benchmark*`, `vpn-service-watchdog` without an
  ownership check.
- The online fallback (`uninstall.sh` top level) downloads and runs code
  from mutable `main` (or a codeload tag tarball) as root with no checksum
  or provenance check, unlike `install.sh`.
- `test-uninstall-idempotency.sh` runs the real uninstaller against real
  system paths on the machine running the test suite. It checks that
  `/etc/vpn` etc. do not exist, but not `/usr/local/bin/sing-box`, so it is
  not hermetic.

Repeated uninstall: second run is a clean no-op (Verified in the test log).

---

## Provisioning Agent

Files: `apps/provisioning-agent/src/*.rs`, unit file, and vpn-web's
`functions/lib/node-bootstrap.js` which writes its config.

What is good: job types are a closed set; payload values are passed to
`vpn-admin` as argv (no shell); clap rejects flag-shaped positional values
(Verified: `user disable --config=...` is rejected); revision/lease inputs go
through 0700 dirs and 0600 files; secrets are redacted from `Debug`; errors
never carry `vpn-admin` stdout/stderr; lease-pool state is persisted before
apply; expired leases rotate with the control plane down (repo's own B2
evidence).

Problems:

1. **Wedge on job reporting** (SVPN-F03). `report_complete_until_ack` and
   `report_fail_until_ack` (`main.rs:185-225`) retry forever on any
   non-2xx. Everything else — heartbeat, traffic, lease-pool tick (expiry
   enforcement), next job — runs in the same loop, so it all stops. vpn-web
   returns 404 for a job that no longer exists; `provisioning_jobs.vpn_account_id`
   is `ON DELETE CASCADE`, so deleting a device/account while its job is
   claimed turns into a permanent wedge. Any deterministic 400/500 from
   `complete.js` (for example a failing `vpn_accounts` upsert) does the
   same. systemd does not restart it because the process is alive.
2. **Timeouts do not stop the child** (SVPN-F04). Every `vpn-admin` call is
   `tokio::time::timeout(60 s, command.output())` without `kill_on_drop`.
   Verified with a minimal program of the same shape: the agent sees a
   timeout and would report FAILED, and the child still writes its result
   3 s later. `vpn-admin` waits on the state lock with a blocking `flock`
   and no timeout (`apps/admin/src/lock.rs`). Retries (3 per job) stack
   more waiting children. When the lock frees, all of them apply, possibly
   out of order (e.g. DISABLE then an older ENABLE), after vpn-web was told
   they failed.
3. **Retries of non-idempotent jobs.** `run_job` retries any failure 3×,
   including `CREATE_USER` after a timeout. Combined with (2) this can
   create two node users for one request; vpn-web records one. The second
   is a live credential no one tracks or revokes (Inferred).
4. **Runs as root and spawns sing-box clients as root** toward
   control-plane-chosen hosts (`protocol_probe.rs:630`). A sing-box client
   bug reachable from a malicious "peer" becomes root on the node
   (SVPN-F09).
5. **No bounds on control-plane responses**: claim, revision, lease sync
   and probe-target bodies are read fully into memory (`reqwest …json()`).
   The 30 s client timeout is the only bound (P3).
6. **`worker_url` is not required to be https** in the agent; vpn-web's
   bootstrap does enforce `https:` (P3).
7. **Config typos are silent**: no `deny_unknown_fields`. A misspelled
   `lease_pool_size = 0` leaves the default 32-slot pool on (P3).
8. Heartbeat reports the agent's own version as `vpn_version`, not the
   installed `vpn-admin` version (P3).

Compromised control plane: can already set any users on the node via
`APPLY_NODE_REVISION` or imperative jobs (by design), choose which hosts
every node probes (SSRF-like: nodes will dial arbitrary host:port with a
sing-box client as root), and push arbitrary strings into `vpn-admin` user
names (not validated; a newline was accepted in a test). It cannot run a
shell command directly — Code-verified. It cannot write arbitrary paths:
revision content goes to a fresh temp dir and is parsed as a user store.

Compromised node: holds its API key; can fetch its own revisions (all its
users' secrets), report fake health, and fetch probe targets (other nodes'
probe credentials, which are confined to three destinations). Blast radius
is bounded by vpn-web's per-node scoping (Code-verified in
`functions/api/agent/revision/[revision].js`).

---

## Provisioning Contract

`crates/provisioning-contract` (leaf crate, schema v1): 29 tests pass,
including adversarial and embedded-config cross-validation (rules A–E,
detour required for relayed endpoints, no private material, no
client-owned DNS/TUN/MTU fields). Tamara's adapter
(`lib/infrastructure/provisioning/singbox_vpn_contract_adapter.dart`)
matches it: rejects `schema_version != 1`, checks `server.product`,
bounds endpoint count (≤64) and string lengths, requires a detour first
hop for Privacy+, refuses to downgrade a relay-only catalog to direct.

No schema drift found between the three repos for: provisioning document,
heartbeat fields (all agent fields are parsed by vpn-web `heartbeat.js`),
traffic sample, lease sync (`as_of`, `min_remaining_seconds`, `extend_to`,
`urgent`, `obfs_stored`, `MAX_SLOTS=1024`, slot range `[0,4096)`, 2 h cap)
and probe report.

Contract risks:

- `APPLY_NODE_REVISION` means "replace the whole user store"
  (`apps/admin/src/main.rs:3647`). The lease users (`lease-NNNN`) and the
  probe user are part of that store, and the agent's lease state still says
  `applied: true` afterwards, so it will not re-apply until the next
  rotation or restart. vpn-web's `createNodeRevision` is "not yet wired
  into a live caller", so this is latent today (SVPN-F14).
- vpn-web keeps every revision document (full user secrets) forever in
  `node_revisions` (vpn-web issue, noted for the cross-repo plan).

---

## Subscription/Auth

- Tokens: 160-bit from `OsRng`, stored as SHA-256, compared in constant
  time over every user; generic 404 for unknown/disabled/expired;
  responses are `no-store`; token length capped at 128. Good.
- `users.json` is read and parsed on **every request** (O(users) CPU and
  I/O per request; fine for tens of users, worth caching for fleet nodes
  with hundreds of users plus lease slots). P3.
- Rate limiting (SVPN-F06, **Verified**): the backend has one
  deployment-wide token bucket because every request arrives from nginx at
  127.0.0.1. Any caller who reaches the backend directly — any VPN user,
  because of F01 — or anyone who can rotate source addresses through nginx
  (an IPv6 /64 gives unlimited per-IP buckets) can drain it. In the lab, a
  single REALITY user sent ~144k requests in 6 s through the tunnel to the
  exit's `127.0.0.1:9100`; a legitimate fetch returned **429** three times
  in a row during the flood and 200 before/after.
- Revocation semantics (Code-verified + repo B2 evidence): disabling a user
  re-renders and restarts sing-box at once (fail-closed order: sing-box
  first, then `users.json`). Expiry is applied by the 10-minute
  `vpn-expiry-reconcile.timer`, so an expired user keeps working up to
  ~10 minutes. Lease slots are enforced by the agent within one poll + apply
  (≈3 s, B2 evidence) — **unless the agent is wedged (F03)**. Open
  connections are cut by the restart itself, so revocation does not leave
  old sessions open; the cost is that everyone else is cut too (F11).
- Subscription-token rotation does not revoke transport credentials; the
  CLI says so. Correct.
- Not tested here: clock skew, concurrent renewal, DB corruption (vpn-web
  side), replay at the vpn-web authorize endpoint.

---

## Network Privacy

Lab capture (Verified, IPv4 only):

| Observer | Sees | Does not see |
|---|---|---|
| Network between client and exit (1-hop) | client IP, exit IP, TLS ClientHello with the **decoy SNI** (`decoy.test`) | destination domains (0 occurrences of `example.test`/`target.test` in the capture) |
| Exit's DNS resolver | every destination domain, from the exit's IP, plaintext UDP (A and AAAA) | client IP |
| Exit | client IP, credential, destinations (in memory; not logged at `fatal`) | — |
| Relay (2-hop) | client IP, exit IP:443, relay user credential A | destinations (inner REALITY to the exit), credential B |
| Exit (2-hop) | relay IP only as source (9/9 TCP flows from the relay in capture), credential B, destinations | client IP — **unless the same user ever uses the Direct route (F10)** |
| Relay's DNS resolver | only the decoy name | destinations |

DNS: the server config has no `dns` section; the exit uses the host's
system resolver for every tunnelled domain, in plaintext. That is normal
for a VPN exit but means the provider's resolver (or whatever
`/etc/resolv.conf` points to) holds a full per-exit browsing log by domain.
Recommendation: point exits at a local caching resolver with DoT/DoH
upstream, and document it.

IPv6: not testable here (kernel has no IPv6). Server inbounds listen on
`::`; exits use sing-box defaults for AAAA. Client-side IPv6 leak
behaviour is client-owned by contract. The maintainer-reported iPhone test
saw only the server's IPv4/IPv6 egress. **UNVERIFIED in this audit.**

---

## 1-Hop

Verified in the lab with configs rendered by `vpn-admin` from this commit:

- REALITY and Hysteria2 tunnels carry traffic; Hysteria2 with Salamander.
- **Every authenticated user can reach the exit's loopback services and
  anything the exit can route to** (SVPN-F01): `127.0.0.1:8081`
  (a loopback-only service on the exit) over REALITY and over Hysteria2;
  the simulated metadata service at `169.254.169.254` (with the client's
  own direct path blocked, so the tunnel was the only way). Only
  `localhost` by name failed (sing-box resolves the name through DNS).
  Private ranges are reachable by construction (no rules); the lab could not
  prove this independently because its "internet" is itself RFC1918.
- REALITY needs the exit to reach and resolve the decoy for every new
  connection: with the decoy stopped, or with the exit's resolver
  stopped, new REALITY connections failed; Hysteria2 kept working
  (SVPN-F12).

---

## 2-Hop

Verified in the lab (real relay rendered by `vpn-admin`, real provisioning
document fetched from the relay's subscription service):

- Relay route = allow `127.0.0.1:<subscription port>` (self-test) + allow
  `10.0.3.2:443/tcp` (declared exit) + reject everything on all inbounds.
- Client → relay → exit → target works; the exit only sees the relay's IP.
- Exit stopped → the Privacy+ route fails (curl rc 97) and **0 packets**
  reach the target's HTTP port; no fallback. Fail-closed.
- The client document's `auto` group contains only the relayed route and
  the selector defaults to it; Direct is an explicit choice (defect D3 fix
  holds).
- CI's S1–S17 already cover wrong/stale exit, wrong credentials, unpaired
  relay, crash/restart, reload and repair; re-run green here except the
  two IPv6-binding tests.

Privacy weakness (SVPN-F10, Verified from the served document): the Direct
and "via relay" routes use **the same exit credential B** (`credential_ref`
reuse, documented in `ALMALINUX_DEPLOYMENT.md`). Tamara offers both as
"Privacy+" and "Fast". Any time a user picks Fast, the exit sees credential
B from the user's real IP; afterwards it can tie every Privacy+ session
with B to that person. The "exit never learns the client IP" property is
therefore per-session, not per-user.

Not verified: real two-provider relay/exit, reachability from restricted
networks, real device behaviour when the exit dies (only lab behaviour).

---

## Firewall

- firewalld path activates SSH first, offline, verifies it in permanent and
  runtime config, then opens 443/tcp, 443/udp, subscription port, and
  records ownership. UFW path is equivalent (tests pass).
- IPv4/IPv6 parity: firewalld zones cover both families; UFW relies on the
  distro default `IPV6=yes` (not checked by the installer).
- No connection-rate limits or SYN-flood protection beyond kernel
  defaults; no egress policy at all (SVPN-F15: exits will relay SMTP/25,
  scans and other abuse from any customer, which is what gets VPS IPs
  blocklisted or accounts suspended).
- Certbot hooks: port 80 is opened **runtime-only** and removed in the
  post-hook; a failed removal is logged, and a reboot or reload clears it.
  nginx is stopped during renewal (standalone HTTP-01), so the
  subscription endpoint is down for the renewal window. If the post-hook
  cannot restart nginx, subscription delivery stays down until someone
  notices; `vpn-health-check` would show it, but fleet health would not
  (F07).
- Hook failure simulation: covered by `test-certbot-firewall-hooks.sh` and
  `test-certbot-renewal-recovery.sh` (both pass). Not run on a real host
  here.

---

## Certificates

- Let's Encrypt via certbot standalone; deploy hook reloads consumers;
  `health-check.sh` fails at <14 days; the probe report carries
  `hysteria2_cert_days_remaining`.
- The VPS run passed `certbot renew --dry-run` with hooks.
- Expired-certificate behaviour: Hysteria2 clients verifying the cert
  fail; REALITY is unaffected (uses the decoy's cert). Subscription HTTPS
  fails. Not tested live here.
- Probe clients verify Hysteria2 certificates in production
  (`tls_insecure_for_tests` defaults false and is reported upstream if on).

---

## Health

Three different things are called "health" and they are often confused:

| Check | What it proves | Where |
|---|---|---|
| `vpn-health-check` | services active, config parses, listeners bound, firewall ports listed, local TLS trust, cert not expiring | install, update, bootstrap HEALTH stage |
| `vpn-admin doctor --protocol` | a real REALITY handshake against `127.0.0.1:443` returns application bytes | install/update acceptance only |
| Agent `probe_ok` | sing-box's **`direct` outbound** can fetch `gstatic.com` (Clash API delay test) — egress only, not the inbound handshake | heartbeat, **only if Clash API configured** |
| Agent `protocol_probe` | real REALITY/Hysteria2 handshakes to peers (and self), DNS, IPv4/IPv6 egress, egress-IP match | heartbeat, **only if `[protocol_probe]` configured** |

SVPN-F07 (Code-verified in both repos): vpn-web's bootstrap writes an agent
config with neither `clash_api_url` nor `[protocol_probe]`
(`vpn-web/functions/lib/node-bootstrap.js` `write_agent_config`), and the
renderer never emits a Clash API. So on a bootstrapped fleet node:
`probe_ok` is absent, `protocol_probe` is absent, and vpn-web's automated
lifecycle (`node-health-transition.js`) can only react to **silence**. A
node whose sing-box is parked `failed`, whose REALITY decoy is unreachable,
whose certificate expired, or whose firewall is wrong keeps heartbeating
and stays READY. The protocol-probe code itself is good (real handshakes,
IPv6/DNS dimensions, confined probe user); it is just not turned on.

2-hop health: no probe tests relay → exit → internet end to end. A relay's
probe user is confined to the three probe URLs via the relay's own
`direct` outbound, so a probe of a relay proves the first hop, not the
Privacy+ path.

---

## Fleet Lifecycle

| Stage | State |
|---|---|
| create / bootstrap / enroll | Code-verified in vpn-web: one-time enrollment token in cloud-init, permanent key generated on the node, only its SHA-256 sent, idempotent stages, retries by a systemd oneshot. Not run live here. |
| READY | reached after local `vpn-health-check`; afterwards liveness-only (F07). |
| serve users | imperative jobs work; every change restarts sing-box (F11). |
| drain | vpn-web lifecycle state; nothing node-side (the node keeps serving whatever users it has). |
| update | no remote path (F16). |
| replace / retire | vpn-web Phase 12 code exists (`node-auto-replace.js`); not verifiable here. |
| uninstall | works on the acceptance host; unsafe on hosts without an ownership record (F13). |

Interruption at every stage was not testable without VPS access.

---

## Reliability

Measured or code-verified behaviours that hurt availability:

- **Restart = full outage for the node's users.** sing-box 1.14.1 cannot
  change users at runtime; every apply that changes the rendered config does
  `systemctl reload-or-restart sing-box` (the unit has no `ExecReload`, so
  it restarts), and the repo's own B2 run measured that SIGHUP also cuts
  every open connection. Triggers: user create, enable/disable, rotate,
  expiry reconcile (every 10 min, whenever any user crossed its expiry),
  lease-pool rotation (F02), revision apply. On a fleet node with many
  users this becomes frequent, node-wide disconnects (SVPN-F11).
- Agent wedge (F03) and stale-child mutations (F04).
- REALITY depends at runtime on decoy + DNS (F12).
- SIGKILL of sing-box: systemd restarts within 30 s; burst exhaustion is
  recovered by the watchdog within ~5 min (VPS run). Good.
- Chaos items not testable here (no VPS/systemd PID 1/netem): reboot,
  interface restart, disk full, read-only disk, memory/CPU pressure,
  clock jump, high latency/loss to control plane.

---

## Control-plane outage behaviour

Intended (Code-verified): the data plane keeps serving; the agent keeps
enforcing lease expiry locally; jobs, heartbeat, traffic and lease sync
simply fail and retry; the probe loop keeps running. B2 evidence shows
expiry enforced with the control plane down.

Actual gap: if the outage starts after a job was claimed and before its
completion was acknowledged, the agent loops in `report_complete_until_ack`
and **stops enforcing lease expiry** until the control plane is back (F03).
Leased credentials then outlive `valid_until` for the length of the outage.

Offline window: no documented maximum. Suggest documenting: tunnels for
existing users continue indefinitely; imperative user expiry is enforced
locally by the timer; lease slots are enforced locally only while the agent
is healthy; new customers cannot be provisioned.

---

## Performance

Lab only: 4 vCPU VM, veth links, no added latency/loss, IPv4, one client,
one connection. These numbers show CPU cost and relative overhead, not real
Internet speed.

| Path | Single-flow download | New-connection latency (p50 / p95, 300 sequential) |
|---|---|---|
| no tunnel | 13,018 Mbit/s | — |
| 1-hop REALITY (Vision) | 6,039 Mbit/s | 3.7 / 4.5 ms (264 conn/s) |
| 1-hop Hysteria2 (Salamander) | **809 Mbit/s** | 1.4 / 1.8 ms (QUIC streams reuse one connection) |
| 2-hop REALITY via relay | 5,906 Mbit/s | 6.7 / 7.6 ms |

Observations:

- Hysteria2 is CPU-bound far below REALITY on the same host. On 1–2 vCPU
  VPS plans this can cap at a few hundred Mbit/s per node; measure on the
  real plan before selling Hysteria2 as the "fast" option (SVPN-F20).
- Each new REALITY connection costs a full handshake **plus a server-side
  dial to the decoy**. With real RTTs this is the per-connection cost the
  YouTube Shorts investigation hit; the server already accepts mux, but only
  Vision-off clients can use it.
- 2-hop overhead in the lab is small in bandwidth and ~2× in connection
  setup; real overhead is dominated by the extra RTT, which was not
  measurable here.
- The periodic restarts (F02, F11) are the largest real-world performance
  problem: they break long downloads, video and calls regardless of
  bandwidth.
- Not measured: 10/50/100+ users, UDP throughput, jitter/loss, QUIC-heavy
  workloads, real apps (YouTube/TikTok/Telegram). `deploy/lib/vpn-benchmark.sh`
  exists and should be run on the real VPS plan.

---

## MTU / Fragmentation / PMTUD

Not testable here (no netem, no IPv6, no real path). The server renders no
MTU settings; Hysteria2 relies on quic-go's defaults; client MTU is
client-owned by contract. The Shorts/TikTok investigations explicitly
parked MTU as low-likelihood and never tested it on a real mobile path.
Recommendation: one controlled test per transport with a 1280-byte and a
1400-byte path MTU (and ICMP blocked) on a disposable VPS pair before
calling MTU "fine".

---

## Resource Limits

- File descriptors: `LimitNOFILE=65535` for sing-box.
- Tasks/memory: `TasksMax=512`, `MemoryHigh=75%`, `MemoryMax=90%`.
- conntrack: 65,536 entries on the acceptance host, not tuned by
  `perf-tuning.sh` (which only sets `rmem_max`/`wmem_max`/BBR/fq). Each
  proxied flow uses two conntrack entries; a busy node with UDP/QUIC can hit
  this (P3, SVPN-F21).
- No per-user connection or bandwidth limits exist in sing-box route rules;
  one user can take a node's capacity.
- Subscription backend: see F06.
- DoS tests against a real node (half-open floods, auth floods) were not
  run (no disposable VPS).

---

## Traffic Accounting

- What is collected: per-node cumulative bytes and open-connection count
  from Clash API `/connections`; per-user accounting is impossible with the
  official build (documented, correct).
- Privacy: only counts leave the node; destinations and source IPs from
  `/connections` are parsed and dropped. Good.
- **In practice it is off on managed nodes**: the renderer never emits a
  Clash API and vpn-web's bootstrap does not configure one (F07/F08). The
  documented manual enablement is overwritten on the next user change.
- Counter resets on sing-box restart are handled by vpn-web; with F02/F11
  that will be several resets per hour.

---

## Logging

- sing-box: `level: fatal`. No credentials, client IPs or destinations
  reach the journal (tested in CI and here). Trade-off: the only signal for
  REALITY rejections ("processed invalid connection"), which both REALITY
  incidents relied on, is gone. Suggest privacy-safe counters instead of
  log lines (SVPN-F18, P3).
- nginx: `access_log off` for token paths; error log at `crit` to a
  dedicated file. Good. The distro default server on :80 keeps its access
  log (firewalled).
- subscription: `RUST_LOG=warn`; served events not logged; test proves no
  secret at any verbosity.
- Agent: `RUST_LOG=info`; logs job id/type, never payloads or vpn-admin
  output. During a control-plane outage it logs a poll error every 5 s and a
  lease-tick warning every 5 s (~35k lines/day); journald rate limits and
  rotation apply. No journald size policy is set by the installer (distro
  defaults).
- `check-no-secret-logging.sh` CI gate exists and passes.

---

## Supply Chain

Good: all GitHub Actions pinned by commit SHA; `pull_request_target` not
used; top-level `contents: read`; only the publish job gets
`contents: write`, only build gets `id-token`/`attestations`; stable tags
cannot auto-publish and require the acceptance record; releases are built
in a pinned AlmaLinux 8.10 container with a glibc ≤2.28 gate and executed
in AL8/AL9 containers before publish; SBOM; Sigstore provenance verified on
install/update; Dependabot for Cargo and Actions; CodeQL for Rust and
Actions; `cargo audit` blocking and clean.

Problems:

- **SVPN-F05**: sing-box download prefers a same-origin `checksums.txt`
  over the pinned hash (install, update, CI).
- `vpn-web`'s bootstrap fetches `install.sh` from
  `raw.githubusercontent.com/<repo>/<tag>/install.sh`. Tag protection is
  explicitly "not assumed enabled" in `SECURITY.md`. A moved tag changes
  what every new node runs before attestation checks start (those check
  release assets, not this first script). Enable tag protection, or pin the
  bootstrap to a commit SHA and verify `install.sh` by hash.
- The release `build` job restores a `Swatinem/rust-cache` keyed
  `release-x86_64-…` into the release build. Low practical risk (tag-scoped
  caches), but release builds should not consume caches at all (P3).
- The official sing-box build includes many optional features (tailscale,
  cloudflared, openvpn, openconnect, naive, ccm/ocm…). Unused ones are not
  reachable from the rendered config, but every sing-box CVE triage has to
  consider the whole binary.
- Branch/tag protection cannot be verified from inside the repo
  (`SECURITY.md` says so). Not verified here either.

---

## Distro Support

| Distro | Real evidence |
|---|---|
| AlmaLinux 9 x86_64 | real-host lifecycle PASS on v1.1.0 (Hetzner) |
| Amazon Linux 2023, Rocky 9, Ubuntu 22.04/24.04, Debian 12/13 | container package-install matrix + fixtures only |
| RHEL 9, CentOS Stream 9 | code path only |
| arm64 | source-build fallback only; no prebuilt binary |

Do not claim generic Linux support. `SUPPORTED_PRODUCT.md`'s table is
honest except that the AlmaLinux row is now stale in the other direction
(it says HEAD is unverified; v1.1.0 was verified).

---

## Cross-Repo Contract

Checked read-only (vpn-web `6b3828e`, tamara-next `a57060b`):

- Provisioning document, heartbeat, traffic, lease sync, probe report and
  revision fetch fields all match.
- Job-completion contract: vpn-web says "on 5xx retry `/complete`, never
  `/fail`". The agent follows that, but also retries 4xx forever, and a
  cascade-deleted job returns 404 forever (F03).
- Health contract: vpn-web's Phase 8 logic expects `probe_ok` and
  `protocol_probe`; its own bootstrap never enables either (F07).
- Lease contract: vpn-web implements ADR-0003 (`/v1/vpn/authorize`,
  `/api/agent/leases/sync`); the node default (32 slots) is on whether or
  not the product uses leases (F02). ADR-0003 lives in vpn-web; this repo
  references it from code and docs but does not contain it.
- Revision contract: "full user-store snapshot" on this side; vpn-web does
  not add lease/probe users and does not call `createNodeRevision` yet
  (F14).
- Relay semantics: same credential for Direct and via-relay routes on both
  sides (F10).
- `HEARTBEAT_INTERVAL_MS = 60_000` in vpn-web must match the agent's clamp
  (10–60 s, default 60). OK today; coupled by comment only.

---

## Dead Code

- `docs/archive/*` and the `archive/native-adaptive-stack-2026` branch hold
  the removed native stack; `main` is clean of it (Cargo members match).
- ~45 remote branches, many merged (`claude/*`, `chatgpt/*`, `fix/*`,
  `experiment/server-side-quic-reject`, `release/*`). Candidates for
  deletion after checking they are merged.
- `docs/superpowers/*` plans/specs are working notes, several superseded
  (e.g. two-hop real acceptance plan, backend prerequisites). Move to
  `docs/archive/` once their decisions live in ADRs/docs.
- `docs/b2-evidence/*`, `docs/health-probe-evidence/*` (incl. an 89 KB
  `events.log`) are evidence, not docs; keep, but under an `evidence/`
  path.
- Compatibility modes still served on every subscription:
  `compat=tcp-only`, `vision-off`, `hiddify-pinned`, `quic-reject`,
  `youtube-direct`, plus `google_egress_hairpin` on the server. Each was an
  incident workaround. Keep only the ones with a current device-verified
  need; each one widens the test and support matrix.
- `TASKS.md` (21 KB) and `docs/IMPLEMENTATION_STATUS.md` (57 KB) overlap
  with release notes and plans; one of them should be the status source.
- `git grep` finds no `TODO`/`FIXME`/`HACK`/`XXX` markers outside `docs/`.

---

## Production Verification

What this audit could and could not do:

| Required by the brief | Done? | Why not / substitute |
|---|---|---|
| Disposable VPS acceptance | **No** — no SSH key or provider API in this container; the previous disposable hosts are not reachable from here | Read run 36304477580 (v1.1.0, today) in full |
| `ss`, `nft`, `systemctl`, `ps` on a fresh node | No | Used the listener inventory recorded by that run |
| External port scan | No | — |
| Real client (device) | No | Real sing-box clients in network namespaces |
| IPv6 | No (no IPv6 in the audit kernel) | — |
| Latency / loss / MTU | No (no `netem`) | — |
| Reboot / systemd semantics | No (no systemd PID 1) | VPS run covers reboot + SIGKILL |
| 1-hop and 2-hop real protocol paths | **Yes, in a lab** | REALITY, Hysteria2, relay→exit with configs from this commit |
| Packet captures | Yes (lab) | client link, relay link, exit link |
| Kill B (exit) and observe | Yes (lab) | Fail-closed, no fallback |

No production host was touched. No destructive action was taken outside
this audit container.

---

## Findings

Priority: **P0** release blocker; **P1** fix before running the fleet
unattended; **P2** fix soon; **P3** hygiene. Difficulty: S (≤1 day),
M (days), L (a week or more / design change). Confidence 1–10.

### SVPN-F01 — Exit nodes let VPN users reach the node's loopback, cloud metadata and private networks

- **Priority**: P0
- **Evidence**: Lab (`evidence/2026-09-27/netlab.sh`, `services.sh`):
  through REALITY and through Hysteria2 the client read a
  `127.0.0.1:8081`-only service on the exit; with the client's own route to
  `169.254.169.254` dropped by nftables, the client still read the simulated
  metadata service through the tunnel. The control request without the
  tunnel failed in both cases. Private-range reachability is inferred from
  the missing rules (the lab "internet" itself uses RFC1918 addresses, so
  reaching `10.0.4.2` is not an independent proof).
- **File**: `crates/compat-config/src/server.rs:70-79` (exit returns with no
  `route` rules) and `:461-463` (single `direct` outbound).
- **Reproduction**: render an exit with `vpn-admin`, start it; from a client
  run `curl --socks5-hostname <socks> http://127.0.0.1:<port>/` and
  `http://169.254.169.254/` under `env -i` (see evidence README about
  `no_proxy`).
- **Impact**: any customer, or anyone with one leaked credential, can: hit
  loopback-only services (subscription backend → F06; sshd from
  "localhost"; the nginx default server; any future local admin API such as
  a Clash API); read cloud metadata (Hetzner user-data with the node's
  bootstrap env and spent enrollment token; on AWS with an instance role,
  IAM credentials — IMDSv2 does not stop an on-host proxy); reach provider
  private networks and other nodes' private addresses.
- **Fix**: render exit route rules that `reject` destinations in
  `127.0.0.0/8`, `::1/128`, `169.254.0.0/16`, `fe80::/10`, `0.0.0.0/8`,
  RFC1918, `100.64.0.0/10`, `fc00::/7`, multicast, and the node's own
  public IPs on non-443 ports; keep the relay self-test exception only on
  relays. Add a real-binary test that proves loopback/metadata/private are
  refused for customers, lease users and the hairpin path. Consider a
  host-level nftables owner match for the `sing-box` user as a second layer.
- **Difficulty**: S–M
- **Confidence**: 9
- **Status**: Verified

### SVPN-F02 — Default lease pool restarts sing-box every ~20 minutes on every fleet node

- **Priority**: P0
- **Evidence**: (a) `lease_pool_size` defaults to 32
  (`apps/provisioning-agent/src/config.rs:88-90`); vpn-web's bootstrap does
  not set it. (b) Driving the real `plan`/`slots_to_rotate` functions for 6
  simulated hours with default lifetime/grid gives **19 config-changing
  applies, one every 1200 s**, both when the control plane never answers
  lease sync and when it reports every slot unleased
  (`evidence/2026-09-27/lease-pool-cadence-sim.rs.txt`). (c) Each apply is
  `vpn-admin lease-pool sync` → `render_and_apply_singbox_config` →
  `systemctl reload-or-restart sing-box` (no `ExecReload`, so a restart).
  (d) The repo's own `docs/B2_EPHEMERAL_AUTH_EVIDENCE.md` §3 and §5 measured
  that each such apply (and SIGHUP) cuts every open connection.
- **File**: `apps/provisioning-agent/src/lease_pool.rs:157-221`,
  `config.rs:88-90`; `vpn-web/functions/lib/node-bootstrap.js`
  `write_agent_config`.
- **Reproduction**: append the sim module to `lease_pool.rs` in a scratch
  copy and run `cargo test -p provisioning-agent audit_sim -- --nocapture`.
- **Impact**: every user on every bootstrapped node loses all connections
  ~3×/hour (calls, video, downloads, gaming), plus a sing-box traffic
  counter reset each time. Not observed on a live fleet node in this audit.
- **Fix**: default `lease_pool_size = 0`; enable leases only on nodes that
  serve ADR-0003 traffic; do not rotate unleased slots that nobody holds
  (let them expire unused and mint replacements only when the leasable
  pool runs low); long term, isolate lease users into a separate sing-box
  process/port or move to a data plane that supports runtime user changes.
- **Difficulty**: S (default) / L (isolation)
- **Confidence**: 8
- **Status**: Verified (simulation + code + repo measurement); not observed
  live

### SVPN-F03 — One undeliverable job report wedges the agent forever and stops lease expiry

- **Priority**: P1
- **Evidence**: `report_complete_until_ack` / `report_fail_until_ack` loop
  until 2xx with no exit (`apps/provisioning-agent/src/main.rs:185-225`);
  lease tick, heartbeat and traffic run in the same loop (`main.rs:69-119`).
  vpn-web `complete.js`/`fail.js` return 404 for a missing job;
  `provisioning_jobs.vpn_account_id … ON DELETE CASCADE`
  (`vpn-web/supabase/migrations/20260921000000_initial_schema.sql:156`).
- **Reproduction**: mock control plane that returns 404 on
  `/api/agent/jobs/1/complete`; queue one job; observe no heartbeats and no
  lease rotations after expiry.
- **Impact**: node looks silent → vpn-web marks it FAILED (good) but the
  node keeps serving users with no expiry enforcement for leases, no
  revocations, no new users, until someone restarts the agent. Same during
  any control-plane outage that begins between claim and ack.
- **Fix**: treat 4xx (except 401/429) on complete/fail as terminal; cap
  retries and persist "unacked result" locally; run heartbeat, lease tick
  and job processing as independent tasks with their own timers.
- **Difficulty**: S
- **Confidence**: 8
- **Status**: Code-verified (both repos)

### SVPN-F04 — Timed-out vpn-admin runs are not killed and apply later, after the agent reported failure

- **Priority**: P1
- **Evidence**: all agent calls use `tokio::time::timeout(60 s,
  Command::output())` without `kill_on_drop` (no occurrence in the repo);
  `vpn-admin` takes a blocking `flock(LOCK_EX)` with no timeout
  (`apps/admin/src/lock.rs`); `update.sh` holds that lock through switch,
  health check and protocol check. A minimal program of the same shape
  (`evidence/2026-09-27/timeout-does-not-kill-child.rs.txt`) printed "timed
  out -> reports job FAILED" and its child still wrote its marker afterwards.
- **File**: `apps/provisioning-agent/src/dispatch.rs` (every job fn),
  `lease_pool.rs:542-551`, `protocol_probe.rs:723`.
- **Impact**: control plane and node disagree about users; queued stale
  mutations can apply out of order (e.g. DISABLE then older ENABLE);
  `CREATE_USER` retries can create untracked live credentials.
- **Fix**: `kill_on_drop(true)` and explicit kill+wait on timeout; give
  `vpn-admin` a bounded lock wait (`LOCK_NB` + retry until a deadline) and
  a distinct exit code for "busy"; make `CREATE_USER` idempotent by passing
  the vpn-web job/idempotency key and having `vpn-admin` return the existing
  user.
- **Difficulty**: S–M
- **Confidence**: 8
- **Status**: Verified (mechanism); scenario Inferred

### SVPN-F05 — sing-box download trusts a same-origin checksums.txt over the pinned hash

- **Priority**: P1
- **Evidence**: `deploy/almalinux/install.sh:1752-1766`,
  `deploy/almalinux/update.sh:759-770`, `.github/workflows/ci.yml:321-331`:
  if `sing-box_<v>_checksums.txt` downloads, `sha256sum --ignore-missing -c`
  is the only check. Verified locally: a `checksums.txt` that lists the
  attacker's tarball hash returns `OK`/exit 0. For 1.14.1 the file does not
  exist today (404), so the pin is used — until someone uploads one.
- **Impact**: whoever can modify SagerNet's release assets (upstream
  compromise) can bypass this project's pin on every fresh install and
  update. This is exactly the case the pin exists for.
- **Fix**: always compare against `SINGBOX_SHA256_*`; optionally also check
  upstream sums when present, requiring both to agree.
- **Difficulty**: S
- **Confidence**: 9
- **Status**: Verified

### SVPN-F06 — One VPN user can block subscription/provisioning delivery for the whole node

- **Priority**: P2 (drops to P3 once F01 is fixed, except the IPv6 path)
- **Evidence**: global bucket 1000 / 250 per second for all loopback
  callers (`services/subscription/src/lib.rs:47-83`, `main.rs:212`). Lab:
  `subscription-flood-through-tunnel.py` through REALITY to the exit's
  `127.0.0.1:9100`; legitimate fetch: 200 → 429, 429, 429 → 200.
- **Impact**: new devices cannot import, Tamara/Hiddify refreshes fail;
  existing tunnels keep working. Without F01, the same is possible through
  nginx from many source addresses (an IPv6 /64 is enough), because nginx's
  limit is per address. Inferred for the IPv6 path.
- **Fix**: fix F01; bind the backend to a Unix socket readable only by nginx;
  key nginx `limit_req` for IPv6 on the /64; add a global `limit_req` zone
  in nginx; keep the backend bucket as a last resort.
- **Difficulty**: S
- **Confidence**: 9 (tunnel path), 6 (IPv6 path)
- **Status**: Verified (tunnel path)

### SVPN-F07 — Fleet node health is liveness-only; a broken data plane stays READY

- **Priority**: P1
- **Evidence**: vpn-web `write_agent_config` writes no `clash_api_url` and no
  `[protocol_probe]`; the renderer never emits `experimental.clash_api`
  (grep: only test code); `health_probe.rs` returns `(None, None)` without
  a Clash URL; `node-health-transition.js` only acts on silence when
  `probe_ok` is absent. Where `probe_ok` exists it tests the `direct`
  outbound to gstatic, not the inbound handshake, and nothing tests
  relay → exit end to end.
- **Impact**: nodes with a failed sing-box, dead decoy, expired cert or
  broken firewall keep getting customers.
- **Fix**: enable `[protocol_probe]` in the bootstrap config with
  `self_probe = true` at minimum; make vpn-web require a recent passing
  protocol probe for READY; add a relay→exit probe; render a loopback-only
  Clash API with a random secret if node-level probes/traffic are wanted.
- **Difficulty**: M
- **Confidence**: 8
- **Status**: Code-verified (both repos)

### SVPN-F08 — Product scope, threat model and docs do not describe what v1.1.0 ships

- **Priority**: P2
- **Evidence**: `docs/SUPPORTED_PRODUCT.md` (≤10 users, no fleet, binaries
  `-p admin -p subscription` only, AlmaLinux HEAD unverified);
  `docs/THREAT_MODEL.md` (no agent/control plane/customer-as-attacker);
  `docs/TRAFFIC_ACCOUNTING.md` (manual Clash API edit that is overwritten).
- **Impact**: reviewers and operators reason about the wrong system; fleet
  risks (F01–F04, F07, F09) fall outside every documented boundary.
- **Fix**: update scope and threat model first; make docs state that
  traffic accounting is off on managed nodes until the renderer supports it.
- **Difficulty**: S
- **Confidence**: 9
- **Status**: Verified

### SVPN-F09 — Agent runs as root, lightly sandboxed, and runs sing-box clients as root toward control-plane-chosen hosts

- **Priority**: P2
- **Evidence**: `apps/provisioning-agent/vpn-provisioning-agent.service`
  (`User=root`, no `ProtectSystem`/`CapabilityBoundingSet`/
  `RestrictAddressFamilies`/`SystemCallFilter`); `protocol_probe.rs:630`
  spawns `sing-box run` with the agent's uid; targets come from
  `/api/agent/probe-targets`; probe SOCKS inbound on `127.0.0.1` has no auth.
- **Impact**: a sing-box client bug triggered by a malicious "peer" (or a
  compromised control plane choosing targets) is root on the node.
- **Fix**: run probe clients as a dedicated unprivileged user with
  `systemd-run --scope` or `setuid`/seccomp; add `ProtectSystem=strict` with
  explicit `ReadWritePaths` for `/etc/vpn`, `/var/lib/vpn-provisioning-agent`,
  `/run/lock`; add `RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX`; plan
  the non-root agent + narrow sudo/polkit for `vpn-admin`.
- **Difficulty**: M
- **Confidence**: 8
- **Status**: Code-verified

### SVPN-F10 — Privacy+ sessions can be linked to the user's real IP by the exit

- **Priority**: P2
- **Evidence**: relay provisioning document (lab): `de1-direct` and
  `de1-via` carry the same UUID; `credential_ref` reuse is the documented
  design; Tamara exposes both ("Fast" and "Privacy+").
- **Impact**: one Direct use by a user ties all their past and future
  Privacy+ traffic at that exit to their IP. Weakens the two-hop privacy
  claim from per-user to per-session.
- **Fix**: issue a separate exit credential for the via route (exit
  accepts it only from declared relays, enforced by an `auth_user` +
  `source_ip_cidr` rule on the exit), or do not serve the Direct route in
  documents that carry Privacy+ for that user.
- **Difficulty**: M
- **Confidence**: 8
- **Status**: Verified (document + code)

### SVPN-F11 — Every authorization change restarts sing-box and cuts every user on the node

- **Priority**: P1
- **Evidence**: `apps/admin/src/main.rs` `apply_users_and_save` →
  `render_and_apply_singbox_config` → `service.rs:195`
  `reload-or-restart`; sing-box unit has no `ExecReload`;
  `vpn-expiry-reconcile.timer` every 10 min; B2 evidence §5 (SIGHUP also
  cuts connections).
- **Impact**: on a fleet node with many customers, signups, expiries,
  rotations and revokes each cause node-wide disconnects; revocation is
  immediate but expensive; expiry is up to 10 min late.
- **Fix**: coalesce changes into scheduled apply windows (the revision path
  is built for this), keep revocations immediate, and evaluate a data
  plane with runtime user management (e.g. Xray-core's HandlerService API,
  or an upstream sing-box feature) behind the existing
  `CompatibilityBackend` trait.
- **Difficulty**: M (batching) / L (backend)
- **Confidence**: 9
- **Status**: Code-verified + repo measurement

### SVPN-F12 — REALITY stops accepting new connections when the exit cannot reach or resolve the decoy

- **Priority**: P2
- **Evidence**: lab: decoy stopped → REALITY fails (rc 97), Hysteria2
  works; exit resolver stopped → REALITY fails. Also the first interop run
  in this audit failed 18 tests for the same reason (decoy name not
  resolvable through sing-box DNS).
- **Impact**: a decoy outage, a decoy blocking the provider's IP range, or a
  resolver outage takes REALITY down on that node; no health signal covers
  it (F07).
- **Fix**: health-probe the decoy from the node; pick decoys with high
  availability from the node's network; consider pinning the decoy address
  via a hosts-style DNS rule in the server config; alert when the decoy's
  certificate chain changes size (known REALITY failure class).
- **Difficulty**: S–M
- **Confidence**: 9
- **Status**: Verified

### SVPN-F13 — Uninstaller removes/stops sing-box it did not install

- **Priority**: P2
- **Evidence**: the shell suite's real uninstall run deleted this audit's
  independently installed `/usr/local/bin/sing-box`; code
  `deploy/almalinux/uninstall.sh:191-197`, `:220-231`; online fallback
  runs unverified code from `main`.
- **Impact**: running the uninstaller on the wrong host (or a host with a
  different sing-box deployment) stops that service and deletes its binary.
- **Fix**: default to "leave" when there is no ownership record (as the log
  text already claims); require ownership for units and every binary; make
  the online fallback download a verified release like `install.sh`; make
  the test hermetic (skip when `/usr/local/bin/sing-box` exists).
- **Difficulty**: S
- **Confidence**: 10
- **Status**: Verified

### SVPN-F14 — Revision apply replaces the whole user store, including lease and probe users (latent)

- **Priority**: P2
- **Evidence**: `apps/admin/src/main.rs:3647-3708`; agent keeps
  `applied: true`; vpn-web `createNodeRevision` has no live caller yet.
- **Impact**: the first real revision would drop all leased sessions and the
  probe user until the next rotation/agent restart; probes then fail.
- **Fix**: make revision apply preserve reserved users (`lease-*`,
  `arcana-probe`) or make the agent re-apply after any revision; add a test.
- **Difficulty**: S
- **Confidence**: 8
- **Status**: Code-verified (latent)

### SVPN-F15 — No egress abuse controls on exits

- **Priority**: P2
- **Evidence**: exit route has no rules; lab showed arbitrary ports are
  attempted (SMTP/25 connection refused by the target, not by policy).
- **Impact**: spam/scanning from customers gets fleet IPs blocklisted and
  provider accounts suspended.
- **Fix**: block TCP/25 (and optionally 465/587 to non-allowlisted hosts),
  consider rate limits; document an abuse-handling process.
- **Difficulty**: S
- **Confidence**: 7
- **Status**: Verified (no policy) / Inferred (abuse impact)

### SVPN-F16 — No remote update path for fleet nodes

- **Priority**: P2
- **Evidence**: job types in `dispatch.rs`; bootstrap "no human, no SSH".
- **Impact**: sing-box/agent security fixes need SSH to every node or node
  replacement, which was not verifiable here.
- **Fix**: either an `UPDATE_NODE` job that runs `update.sh --version <tag>`
  with canarying, or a tested "replace instead of update" runbook.
- **Difficulty**: M
- **Confidence**: 7
- **Status**: Code-verified

### SVPN-F17 — Agent config and control-plane input are loosely bounded

- **Priority**: P3
- **Evidence**: no `deny_unknown_fields` (`config.rs`); unbounded JSON
  bodies; `worker_url` scheme not checked; user names not validated
  (newline accepted by `vpn-admin user create`).
- **Fix**: `deny_unknown_fields`, body size caps, https check, name
  allowlist (`[A-Za-z0-9._@-]{1,128}`).
- **Difficulty**: S
- **Confidence**: 9
- **Status**: Verified (name), Code-verified (rest)

### SVPN-F18 — Fatal-only sing-box logging removes the REALITY rejection signal

- **Priority**: P3
- **Fix**: privacy-safe counters (rejections per minute, no IP/credential),
  exported through the heartbeat.
- **Confidence**: 8. **Status**: Code-verified.

### SVPN-F19 — Subscription service re-reads users.json on every request

- **Priority**: P3. **File**: `services/subscription/src/lib.rs` `get_subscription`.
- **Fix**: cache with an mtime/inode check. **Confidence**: 9. **Status**: Code-verified.

### SVPN-F20 — Hysteria2 is CPU-bound well below REALITY

- **Priority**: P3. **Evidence**: 809 vs 6,039 Mbit/s single flow, same lab host.
- **Fix**: benchmark on the real VPS plan; set expectations/capacity per plan.
- **Confidence**: 7 (lab only). **Status**: Verified (lab).

### SVPN-F21 — conntrack and fd limits untuned for a busy node

- **Priority**: P3. **Evidence**: conntrack 65,536 on the acceptance host;
  `perf-tuning.sh` does not touch it; `LimitNOFILE=65535`.
- **Fix**: size `nf_conntrack_max`, consider `NOTRACK` for 443 inbound, raise
  fd limit per plan. **Confidence**: 6. **Status**: Code-verified.

### SVPN-F22 — Heartbeat `vpn_version` is the agent's version

- **Priority**: P3. **File**: `telemetry.rs:41-46`. **Fix**: read `vpn-admin
  --version`. **Confidence**: 10. **Status**: Code-verified.

### SVPN-F23 — Tests depend on the host environment

- **Priority**: P3. **Evidence**: decoy test needs sing-box to resolve
  `localhost` through the host resolver; two two-hop tests need IPv6;
  `test-uninstall-idempotency.sh` touches real system binaries.
- **Fix**: use an IP decoy or an explicit DNS rule in test configs; skip
  IPv6 binds when unsupported; make the uninstall test hermetic.
- **Confidence**: 10. **Status**: Verified.

### SVPN-F24 — Release build restores a build cache

- **Priority**: P3. **File**: `.github/workflows/release.yml:266`.
- **Fix**: drop `rust-cache` from the release build job. **Confidence**: 7. **Status**: Code-verified.

### SVPN-F25 — sing-box-owned config directory written by root without O_NOFOLLOW

- **Priority**: P3. **File**: `install.sh` `create_directories`;
  `crates/compat-config/src/server.rs` `write_config_file_mode_0640`.
- **Fix**: `root:sing-box 0750` directory; `O_NOFOLLOW|O_EXCL` temp file.
- **Confidence**: 5. **Status**: Inferred (systemd sandbox blocks the obvious path).

### SVPN-F26 — Google egress hairpin changes every user's egress path and turns on sniffing for all flows

- **Priority**: P3 (product/privacy decision)
- **File**: `server.rs` `apply_google_egress_hairpin`.
- **Impact**: when set, all users' Google/YouTube traffic leaves from the
  relay's country/IP with no user-visible indication; the exit reads SNI of
  every flow (in memory).
- **Fix**: document it in the privacy policy and product UI, or make it
  per-profile. **Confidence**: 8. **Status**: Code-verified.

### SVPN-F27 — Bootstrap fetches install.sh by mutable tag

- **Priority**: P2 (supply chain, cross-repo)
- **File**: `vpn-web/functions/lib/node-bootstrap.js` `stage_install`.
- **Fix**: enable tag protection and verify; better, pin a commit SHA and a
  SHA-256 of `install.sh` in the bootstrap. **Confidence**: 7. **Status**: Code-verified.

---

## Blockers

**P0 (must fix before selling the fleet):**

- SVPN-F01 — customer egress into loopback / metadata / private networks.
- SVPN-F02 — default lease pool restarts sing-box ~3×/hour on every node.

**P1 (must fix before running the fleet unattended):**

- SVPN-F03 — agent wedge on job reporting (also stops lease expiry).
- SVPN-F04 — timed-out vpn-admin children apply after reported failure.
- SVPN-F05 — sing-box pin bypass via same-origin checksums.txt.
- SVPN-F07 — liveness-only fleet health.
- SVPN-F11 — every authorization change is a node-wide disconnect.

---

## Remediation

In order (each step is small enough for one reviewed PR):

1. **F02 (S)**: `lease_pool_size` default → 0; vpn-web bootstrap writes it
   explicitly; stop rotating unleased slots that nobody holds.
2. **F01 (S–M)**: exit egress deny-list rendered by `server.rs` for every
   user class; real-binary regression test (loopback, metadata, RFC1918,
   IPv6 equivalents); host nftables owner-match second layer.
3. **F05 (S)**: always enforce the pinned sing-box hash (install, update, CI).
4. **F03 + F04 (S–M)**: terminal 4xx handling, independent task loops,
   `kill_on_drop`, bounded lock wait, idempotent `CREATE_USER`.
5. **F13 + F23 (S)**: safe uninstall defaults, verified online fallback,
   hermetic tests.
6. **F07 (M)**: enable protocol probes in the bootstrap, require probe
   evidence for READY, add relay→exit probing.
7. **F06 (S)**: Unix-socket backend or nginx-only access; IPv6 /64 keying;
   global nginx limit.
8. **F11 (M→L)**: batch non-urgent changes into apply windows; start the
   runtime-user-management evaluation behind `CompatibilityBackend`.
9. **F08 (S)**: update `SUPPORTED_PRODUCT.md`, `THREAT_MODEL.md`,
   `TRAFFIC_ACCOUNTING.md`.
10. **F09, F10, F14, F15, F16, F27 (M)**: agent sandboxing/non-root probes,
    per-route exit credentials, reserved-user-safe revisions, egress abuse
    policy, fleet update/replace path, pinned bootstrap.
11. **P3 items** as hygiene.

Then run, on two disposable VPSs (ideally different providers): the
existing lifecycle gate on both, a real relay→exit acceptance using the
S1–S17 list, packet captures on each hop, IPv6 and DNS leak checks with a
real device, `vpn-benchmark.sh` at 1/10/50 concurrent users, a 1280-MTU
and ICMP-blocked path test, and a 24-hour soak that counts sing-box
restarts and dropped long-lived connections.

---

## Final Readiness Statement

- **Single self-managed VPS, ≤10 trusted users, 1-hop** (the documented
  v1.0 product): acceptable after SVPN-F01 and SVPN-F05. Evidence is strong
  (CI + fresh-host lifecycle PASS on v1.1.0 + device report).
- **Commercial fleet via vpn-web (provisioning agent, leases, probes)**:
  **not ready.** Two P0 and five P1 findings, and no real-host evidence yet
  for the agent, the lease pool, probes, two-hop, IPv6, load or long-running
  behaviour.
- **Two-hop Privacy+**: mechanism is correct and fail-closed in the lab;
  privacy claim needs F10 fixed and real multi-provider evidence before it
  is marketed.

Overall confidence: 8/10.
