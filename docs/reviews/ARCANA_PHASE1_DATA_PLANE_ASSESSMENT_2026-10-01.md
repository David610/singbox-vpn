# Arcana Phase 1 — data-plane implementation assessment

Date: 2026-10-01
Scope: `David610/singbox-vpn` at `b94419d` (`claude/arcana-data-plane-production-final`, 57 commits ahead of `main`, PR #123 unmerged).
Method: read the current tree. Documentation claims were re-checked against code and **not** trusted where they disagreed.

This assessment precedes every code change in this phase.

---

## 1. Baseline test state (measured, not reported)

`cargo test --locked --workspace --no-fail-fast` on Windows 11:

| Package | Result |
|---|---|
| `admin` (`vpn`, `vpn-admin` unit tests) | 79 passed / **2 failed** |
| `admin` integration (`cli.rs`, `relay_cli.rs`) | 55 passed |
| `compat-config` (unit + all integration suites) | 143 + 87 + 71 + 71 passed |
| `provisioning-contract`, `subscription`, `provisioning-agent` | all passed |

Both failures are **environment-only, pre-existing, and unrelated to this work**:
`cert_expiry_days` (`apps/admin/src/main.rs:4251`) shells out to GNU coreutils `date -u -d` to parse `openssl x509 -enddate` output, which does not exist on Windows. Both CI runners (`ubuntu-latest`) and the deployed AlmaLinux hosts have it. Not a code defect; deliberately **not** changed, because rewriting a Linux-only production path to accommodate a non-production dev OS would be unrelated redesign.

Real-sing-box interop suites **skip** locally (no `sing-box` binary on this host); they are gated in CI by `SINGBOX_VPN_REQUIRE_REAL_INTEROP=1`, which converts a skip into a hard failure, plus exact test counts.

ShellCheck `-S warning` over the CI glob: clean (only info-level `SC2086`/`SC2015` remain, per the 2026-09-27 readiness audit).

---

## 2. What the data plane actually is today

```
vpn-admin (apps/admin)                compat-config (crates)          sing-box
  ├─ user create/disable/rotate   →   render_server_config_for_  →   /etc/sing-box/config.json
  ├─ render-config (10-min timer)      deployment (role policy)       systemd: sing-box.service
  ├─ apply-revision (static)           render_singbox_server_config
  └─ lease-pool sync                  apply_config_atomically
                                      (temp → validate → rename → fsync)
```

Three facts dominate every design decision in this phase:

**(a) A credential today is one UUID and one password.** `CompatUser` (`crates/compat-config/src/model.rs:266-370`) carries exactly one `vless_uuid`, one `hysteria2_password`, and one account-level `expires_at`. There is no per-credential validity window, no `valid_from` anywhere in the repository, and no way to hold two credentials for one principal at once.

**(b) Revocation and expiry are implemented by omission.** `server.rs:660` filters users with `is_active(now)`; a disabled or expired user is simply absent from the next rendered document. sing-box has no per-user expiry field. Enforcement granularity is therefore *the render interval* — 10 minutes (`deploy/almalinux/systemd/vpn-expiry-reconcile.timer`, `OnUnitActiveSec=10min`), not the credential's own deadline.

**(c) Any change to the effective config is a full restart.** There is no Clash API, no experimental API, and no SIGHUP handler. `render_and_apply_singbox_config` (`apps/admin/src/main.rs:2053-2062`) already short-circuits when the rendered document is byte-identical, and the surrounding comment (`main.rs:2086-2111`) documents — correctly — that upstream sing-box tears down every inbound and in-flight connection on reload.

Fact (c) plus fact (b) is the crux: **the only zero-downtime mechanism that actually exists is "do not change the rendered document."** Everything below is built on that.

---

## 3. Requirement-by-requirement gap analysis

| # | Requirement | Status today | Evidence |
|---|---|---|---|
| 1 | Two credential classes | **ABSENT** | No class concept. One credential shape serves both native and third-party clients. |
| 1 | Native ~30 min, silently renewable, no reconnect | **ABSENT**, and actively harmful | The lease pool rotates the *secret* every 1800 s (`apps/provisioning-agent/src/lease_pool.rs:132-142`), which changes the config, which restarts sing-box — roughly 3×/hour for every native client. This is the "restarts ~3×/hour" blocker recorded as F02 in the 2026-09-27 readiness audit. |
| 1 | Compatibility credential, longer-lived, configurable | **PARTIAL** | `expires_at` exists but is per-*account*, not per-*credential*, and there is no class-specific policy. |
| 2 | `valid_from` | **ABSENT** | Repo-wide search: zero occurrences. `created_at` (`model.rs:276`) is written and never read. |
| 2 | `valid_until` per credential | **ABSENT** | Only whole-account `expires_at`. |
| 2 | Bounded overlap window | **ABSENT** | Rotation overwrites in place (`main.rs:3623-3625`): old credential dies the instant the new one is written. A client asleep across a rotation is locked out. |
| 2 | Deterministic generation | **MET** | Render is pure over persisted state + `now`. |
| 2 | No resurrection of expired credentials on restart | **MET** | `is_active(now)` is re-evaluated on every render; a restart cannot re-authorize. |
| 3 | Opaque principals | **PARTIAL / one live leak** | `CompatUser.id` is opaque `user_<uuidv4>` (good). But `name` stores the control plane's `user_id` verbatim (`apps/provisioning-agent/src/dispatch.rs:262-269`), and `users.json` is mode **0640**. Lease-slot users are pseudonymous; ordinary users are not. |
| 3 | No secrets in logs | **MET, strongly** | `log.level = "fatal"` is a deliberate credential-security control (`server.rs:786-806`); `SecretString` never `Debug`s its value; `redact_secrets` covers the doctor report. |
| 4 | Zero-downtime renewal | **NOT ACHIEVABLE today** | Rotation changes the UUID → document changes → restart. |
| 4 | Atomic write | **MET** | `apply_config_atomically` (`server.rs:838`): temp → `sing-box check` → rename → `fsync` file and parent; ownership preserved; 0640 enforced independent of umask (`server.rs:1307-1356`). |
| 4 | Rollback | **PRESENT, untested** | Four `std::fs::copy` restore sites; no test asserts post-rollback content or mode. |
| 5 | Logical routes vs physical nodes | **PARTIAL** | `AccessPath`/`AccessPathKind` exist (`deployment.rs`), and infrastructure endpoints are already hidden from the client catalog (`render.rs:651-675`). No named logical product (`de-privacy+`) and no stable route identifier. |
| 6 | Fast mode | **ABSENT as a named concept** | No `Fast` mode exists. Nearest analogue is `?profile=performance` (`render.rs:270-294`), which on a Privacy+ deployment is *short-circuited* by the relay arm and yields a relayed VLESS route anyway. |
| 7 | Privacy+ two-hop | **MET for sing-box** | Native `detour` chain, VLESS+REALITY on both hops, real-binary and real-two-provider device-verified (`docs/DEVICE_ACCEPTANCE_TESTS.md:539-589`). |
| 7 | Privacy+ for Xray | **ABSENT** | No Xray core in the product; `?format=xray` is a hard 400. Xray has no `detour` equivalent here. |
| 7 | **Zero silent downgrade** | **VIOLATED — live** | See §4. |
| 7 | Relay fail-closed | **MET at sing-box layer** | Terminal `reject` scoped to the document's own inbound tags (`server.rs:176-197`); empty inbound set is a hard render error. A build-failing grep (`apps/admin/tests/relay_cli.rs:155-173`) prevents production code from using the role-unaware renderer. |
| 8 | Client capability contract | **PARTIAL** | `ProvisioningDocument` carries `capabilities`, correctly *derived* from actually-configured endpoints (`contract.rs:206-209`). But it says nothing about Fast/Privacy+ composition, credential class, or minimum credential lifetime. |
| 9 | Third-party fixtures | **PARTIAL** | `fixtures/singbox-client-contract/*.json` covers single-hop; no canonical two-hop or "unsupported combination" fixture. |
| 10 | REALITY handshake-server resolution | **MET** | `resolve_hostname_with_retry`, bounded attempts, fail-closed. |
| 10 | Expired-user cleanup | **GAP** | Expired users are never removed from `users.json`; only excluded at render. State grows unboundedly at the scale this phase targets. |
| 11 | Relay egress policy | **GAP** | See §4. |
| 11 | Systemd hardening | **MET** | Verified in the readiness audit; `test-systemd-resource-protection.sh` guards it. |

---

## 4. Two findings that are security/privacy defects, not feature gaps

These are ordered ahead of the feature work because the stated priority order puts security and privacy above compatibility.

### 4.1 Silent Privacy+ → Fast downgrade on third-party share-link import (PRIVACY / NO TRAFFIC LEAKS)

`share_link_endpoints` (`render.rs:141-152`) correctly *omits* relay routes rather than emitting a link that would dial the exit directly. But it filters only on `path`, and a **direct exit peer endpoint** (`path == "direct"`) sits in the same catalog and **is** served.

Result: a user of a paired relay deployment who imports `/sub/{token}?format=uri` — the default format for Hiddify, v2rayNG, NekoBox, INCY, and every generic sing-box importer — receives **HTTP 200 and a working `vless://` link that dials the exit directly**. Their real source IP reaches the exit. Nothing in the response, the logs, or any client UI indicates the privacy path was skipped.

This is asserted as *intended* behaviour by two tests (`relay_role_policy.rs:1004-1019`, `lib.rs:2381-2395`), which is why it has survived. The fail-closed answer already exists elsewhere in the same file for every other inexpressible mode — `hiddify-pinned`, `quic-reject`, `youtube-direct`, `tcp-only` all return an explicit **400** rather than a working-but-wrong profile (`lib.rs:392-455`). Privacy+ must be handled identically.

A second, related downgrade: Hiddify rebuilds its groups from the `outbounds` array and, on a multi-route Privacy+ profile, balancer-members include the direct exits, so traffic leaves the enforced relay path unless the caller passes `?compat=hiddify-pinned`. That opt-in is not the default. `pin_to_single_route` (`render.rs:920-978`) already implements the correct behaviour; the gap is that it is opt-in rather than implied.

### 4.2 C-16 egress isolation is not applied to relay documents (SECURITY)

`apply_c16_egress_policy` is called only inside the `NodeRole::Exit` early-return (`server.rs:78-83`). The relay branch returns at `server.rs:200` without it, so a relay's `direct` outbound carries no deny set for loopback, link-local, RFC1918, CGNAT, or IPv6 ULA.

The relay-target validator does not close this: `validate_relay_target_host` (`deployment.rs:158-170`) rejects only unspecified/multicast/broadcast. A `[[peer_endpoints]]` entry with `host = "169.254.169.254"` is accepted at load and rendered as a permitted `ip_cidr` route that an authenticated relay user can reach.

Bounded (one declared host and port, authenticated only) but a genuine asymmetry: the exit refuses these destinations and the relay does not. Nothing host-level backstops it either — `nftables-egress-isolation.sh` states in its own header that it is not wired into `install.sh`/`update.sh`, and `firewall.sh` is ingress-only.

---

## 5. The central design decision

Given §2(c) — a changed document means a restart — the design question is not "how do we reload without dropping connections," which upstream makes impossible. It is:

> **How do we renew a short-lived credential without changing the rendered document?**

The answer, and the spine of this phase: **renewal must not change credential material.**

sing-box's VLESS and Hysteria2 user entries carry only `name`, `uuid`/`password`, and `flow`. There is no expiry field to update. So if a native grant is renewed *in place* — same `principal_id`, same `uuid`, same `hysteria2_password`, only `valid_until` extended in `users.json` — then:

- the rendered document is **byte-identical**,
- `target_already_matches && applied_stamp_matches` short-circuits at `main.rs:2059`,
- **sing-box is never touched**, and no connection is ever interrupted.

A 30-minute native lease renewed every 15 minutes therefore costs **zero** restarts, versus ~3/hour today. Renewal preserves the same underlying secret, which is also what makes it invisible to the client: the tunnel does not know or care that authorization was extended.

This inverts the lease-pool design, which rotated the secret precisely *because* it had no notion of a validity window to extend. Rotation is still supported and still needed — for compatibility credentials, where the third-party client must be handed a new secret — but it is now the exception, bounded by an explicit overlap window, rather than the default heartbeat.

---

## 6. Scope discipline

**In scope (data plane):** `crates/compat-config` (credential model, validity, overlap, opaque principals, render, apply), `crates/provisioning-contract` (client capability contract), `apps/admin` (`vpn-admin`, the node's credential-enforcement tool), and the two surgical privacy/compatibility fixes in `services/subscription`.

**Out of scope (business/control plane), not modified:** `apps/provisioning-agent` job dispatch, subscription business logic, Stripe/billing/entitlement concerns, `vpn-web`. `apps/provisioning-agent`'s lease-pool mode is left working; it now composes with the new model rather than being replaced, because rewriting it is fleet-plane work.

**Not done here, explicitly:** Xray two-hop. There is no Xray core in this product and Xray has no `detour` equivalent in this codebase. Xray clients are marked unsupported for Privacy+ rather than being given a downgraded profile; that is the fail-closed answer the requirements demand, and inventing an Xray renderer would be exactly the "trade privacy for easier compatibility" the brief prohibits.

---

## 7. Work plan

1. Credential grant model: `class`, `principal_id`, `valid_from`, `valid_until`, `revoked_at`, `generation`, with a legacy-path fallback so existing `users.json` renders byte-identically.
2. Bounded overlap policy with explicit validation; at most two live grants per (user, class); overlap capped.
3. Per-grant rendering; renewal proven not to change the fingerprint.
4. Privacy+ fail-closed on clients that cannot express two-hop (§4.1).
5. C-16 applied to relay documents and relay-target validation tightened (§4.2).
6. Logical route contract and client capability contract.
7. `vpn-admin` credential lifecycle commands.
8. Third-party fixtures validated against real binaries.
9. Full test suite, `fmt`, `clippy`, ShellCheck.
10. Documentation and commit.