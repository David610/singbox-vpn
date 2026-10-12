# YouTube Shorts and IPv6 — full diagnostic and fix plan (2026-10-12)

Status: **diagnosis complete and measured; fix not yet implemented.** This
document records the root cause, every experiment behind it, what is still
unproven, every inconsistency found in the repository, and an ordered plan to
fix both the product and the docs. It supersedes nothing by itself: the
invariants in [`YOUTUBE_FIX_INVARIANTS.md`](YOUTUBE_FIX_INVARIANTS.md) stay in
force until the plan below changes them, in the same commit as their tests.

Evidence classes follow `CLAUDE.md`: *measured* (run today against the named
host, command given), *code verified* (read in this repository at commit
`d022d61`), *user reported*, *unverified hypothesis*. Nothing here is promoted
above its evidence.

No credential, subscription token, key or password appears in this file.

---

## 1. Executive summary

**Root cause.** YouTube's media server refuses Shorts to certain datacenter
egress addresses (INV doc, proven 2026-10-11). On a *new* single server
(`157.173.27.46`, Ubuntu 24.04, release v1.2.0, **no hairpin**) the server's
IPv4 address is accepted but its **IPv6 address range is refused**. The exit
sends traffic to whatever destination the client gives it. An iOS TUN client
(Hiddify) resolves DNS itself and sends **literal IP destinations**, in both
families. Literal IPv4 leaves from the accepted IPv4 address; **literal IPv6
leaves from the refused IPv6 address**. A plain exit has **no rule** that
steers or rejects literal IPv6, so a share of Shorts media requests dies, the
app shows "Video unavailable", and long videos (other hosts/paths) still work.

**Why it looked fixed, then broke.** The earlier fix (`google_egress_hairpin`)
contained a literal-IPv6 reject rule that existed *only inside the hairpin
code path*. When the exit was reinstalled as a plain single server, the
hairpin and with it the only IPv6 rule disappeared. The new server also has a
good IPv4 and a bad IPv6, a combination that the old nodes (both IPv4 refused)
never exercised, so the repository's claim "IPv6 is not the cause" was true for
those nodes and is false in general.

**Why my first tests passed.** A SOCKS client that sends *domain names* makes
sing-box resolve and dial IPv4. The failing path needs the client to send
*literal IPv6*, which a phone does and my first probe did not.

**What to do.** (1) Stopgap on this server: stop the exit from using its
IPv6 address (reversible, see §8.1). (2) Product fix in a v1.2.1 release: every
exit renders `sniff` plus a `no_drop` reject for literal-IPv6 Google/YouTube
destinations whether or not a hairpin exists, configurable, with doctor,
tests, docs and invariants updated together (§8.2). (3) Repair the 25
repository inconsistencies listed in §9. (4) Accept on the native app on both
protocols (§10).

Confidence: **high** that the IPv6 egress family decides the outcome on this
server (controlled, repeatable, §3); **medium** that this is the *only* reason
the phone still fails (the phone's own per-flow destinations were not captured,
§5).

---

## 2. Timeline of the symptom

| When | Event |
|---|---|
| 2026-10-11 | Root cause for the old nodes proven: IP refusal, not CDN peering (INV doc). Hairpin from `de1` through `ru1` fixes it; owner confirms on iPhone. v1.2.0 released (rc.5 accepted). |
| 2026-10-12 | `de1` reinstalled as a plain exit → hairpin gone → Shorts fail again (its IPv4 is refused anyway). |
| 2026-10-12 | New server `157.173.27.46` installed with v1.2.0 for the `vpn.утамары.com` name. Phone first fails with "Failed host lookup" (home router DNS cache, §7.3), then, with the profile connected, Shorts still fail. |

---

## 3. Experiments (all measured 2026-10-12 on `157.173.27.46`)

Server: Ubuntu 24.04.5, sing-box 1.14.1, `vpn-admin 1.2.0`, role `exit`, no
`[google_egress_hairpin]`, public IPv6 `2a12:bec4:19a5:924::/64`, IPv6 default
route present, `net.ipv6.conf.all.disable_ipv6 = 0`. A temporary sing-box
*client* on the server dialed its own REALITY inbound on `127.0.0.1:443`
(the same path as `vpn doctor`'s protocol self-test) and exposed a local SOCKS
port. It was removed afterwards.

### E1 — Which family does each destination *form* leave from?

`icanhazip.com` has both an A and an AAAA record. The destination is requested
three ways through the real REALITY inbound; the echoed address is the egress
address the destination sees.

| Destination form sent to the exit | Egress address seen |
|---|---|
| domain name (`socks5h`) ×3 | `157.173.27.46` (IPv4) |
| literal IPv4 | `157.173.27.46` (IPv4) |
| literal IPv6 | **`2a12:bec4:19a5:924::` (IPv6)** |

Reading: sing-box dials IPv4 for domain flows even though AAAA exists, but it
obeys a literal IPv6 destination and uses the host's IPv6.

### E2 — Is each egress address accepted by YouTube for Shorts?

`deploy/lib/youtube_shorts_probe.py` (Chrome, iPhone emulation, six consecutive
Shorts; "playing" means the `<video>` clock advances, not `playabilityStatus`).

| Path | Result |
|---|---|
| Plain TCP over SSH (sshd resolves, prefers IPv6) → egress IPv6 | **0/6** "Video unavailable" |
| Plain TCP over SSH, IPv4 forced | **6/6** |
| Real REALITY stack, domain destinations | **6/6** |
| Real REALITY stack, **literal IPv4** destinations | **6/6** |
| Real REALITY stack, **literal IPv6** destinations | **0/6** |

The last two rows are the controlled experiment: same server, same tunnel, same
browser, same Shorts; the only variable is the family of the literal address
the client hands the exit. (Harness: a local SOCKS front that resolves every
domain itself and forwards a literal address of the chosen family into the
server's socks inbound, i.e. it imitates an iOS TUN client.)

### E3 — What did the real phone do?

While the phone was connected (`sing-box` log level is `fatal`, so no per-flow
log exists) a socket sampler on the server saw, simultaneously:

* inbound clients on tcp/443: one carrier-looking client with hundreds of
  sessions, plus scanners;
* outbound tcp/443 to **IPv4** Google/Cloudflare/Apple addresses
  (`172.217.118.4`, `173.194.160.131`, `104.16.124.96`, `17.253.73.208`, …);
* outbound tcp/443 to **IPv6** addresses, including Google
  (`2a00:1450:4005:9::4`) and one Apple/other range (`2001:1438:1:4::f`).

My first sampler also counted sing-box's own `::ffff:` inbound sockets as
"IPv6"; the figure "138 IPv6 destinations" in that output is **wrong** and is
withdrawn. Only the explicitly routed `2001:`/`2a00:` addresses are real
outbound IPv6.

This shows the phone *does* send literal IPv6 destinations to the exit while
also sending IPv4. It does not prove which flow carried the Short's media.

### E4 — IPv6 health of the host (to exclude a broken IPv6 stack)

10/10 `curl -6 https://www.google.com/generate_204` = 204; 10/10 over IPv4;
IPv6 neighbours `STALE` (normal), MTU 1500, `ping6 -M do -s 1400` loses 0/2.
The IPv6 path works; it is the *address range* that YouTube refuses.

### E5 — What does the rendered exit actually contain?

`/etc/vpn/compat/sing-box/config.json` (read-only): inbounds `vless` and
`hysteria2` on `::`:443; one `direct` outbound; route rules = probe confinement
(4 rules, `auth_user`-scoped), C-16 `resolve`, C-16 IPv4 deny, C-16 IPv6 deny,
tcp/25 reject; `final: direct`. **No `sniff`, no IPv6 reject, no Google rule.**

### E6 — Other facts collected

* One Let's Encrypt certificate (`vpn.xn--80aa9argf5d.com`) serves nginx on
  8443; Hysteria2 uses a copy at `/etc/vpn/compat/hysteria/cert.pem`.
  `certbot renew --dry-run` passes (hooks reopen/close TCP/80 via ufw).
* `vpn doctor --protocol --require-protocol` passes; it **warns** that nginx
  listens on `0.0.0.0:80` and `[::]:80` (Ubuntu's `default` site is enabled)
  and on `127.0.0.53:53` (systemd-resolved) — "not part of expected public
  surface". ufw blocks :80 from outside, so this is cosmetic but noisy (§9, D-17).
* `vpn-expiry-reconcile.timer` and `vpn-service-watchdog.timer` are active.

---

## 4. Mechanism (end to end)

1. Hiddify (iOS) runs a TUN. The phone's resolver (inside Hiddify) returns A and
   AAAA records for `*.googlevideo.com`, `*.youtube.com`, etc. Apps connect by
   address. (INV-4: iOS TUN clients "resolve DNS themselves and send bare IPs".)
2. Whatever address the app picks is sent over VLESS+REALITY or Hysteria2 to the
   exit as the *destination*. Nothing in the subscription expresses IPv4/IPv6
   preference (`CLIENT_PROTOCOL_BEHAVIOR.md`, "IPv4 / IPv6").
3. The exit (`route.final = direct`, no sniff, no IPv6 rule) dials that exact
   address from the host: IPv4 → `157.173.27.46`, IPv6 → `2a12:bec4:…`.
4. YouTube's media front-end serves Shorts to the first and answers the second
   with the 155-byte control-only reply (INV doc), which the player renders as
   "Video unavailable". Long videos, other Google endpoints and most sites
   still work, so the VPN "mostly works".
5. When the hairpin exists, the exit *also* sniffs the SNI and (a) rejects
   literal IPv6 for the Google set with ICMP unreachable so the client falls
   back to IPv4, (b) routes the Google set to the accepted node. Without the
   hairpin none of that runs.

Why `no_drop`/ICMP matters: a silent drop makes the client wait for its own
timeout (measured 0/20 QUIC handshakes in the earlier hairpin work). A reject
makes the app retry IPv4 immediately.

---

## 5. What is proven and what is not

| Claim | State |
|---|---|
| The server's IPv6 egress range is refused by YouTube; its IPv4 is accepted | **Measured** (E2) |
| A literal IPv6 destination leaves from the refused range; domain/literal-IPv4 do not | **Measured** (E1) |
| Literal IPv6 destinations make Shorts fail, literal IPv4 make them play, same tunnel | **Measured** (E2, controlled) |
| A plain exit has no rule that prevents this | **Code verified + measured** (E5, `server.rs`) |
| The phone sends literal IPv6 to the exit at the same time as IPv4 | **Measured** (E3), not attributed to Shorts media |
| Disabling/rejecting IPv6 egress will make Shorts play **on the phone** | **Unverified hypothesis** — to be confirmed first (§8.1, step 2) |
| Hysteria2/QUIC literal-IPv6 behaves like TCP | **Unverified** (QUIC not driven in E2) |
| Cellular/Wi-Fi IPv6 differences, long session, reconnect | **Unverified** |

If, after IPv6 egress is stopped, Shorts still fail on the phone, the next
suspects, in order: (a) sniff-less Google flows to *other* refused families
(none seen), (b) the phone is not actually using this profile (DNS/profile
state, §7.3), (c) a Hiddify-side IPv6/DNS setting. The plan keeps a stop-gate for
that case (§8.1).

---

## 6. IPv6 issues in this project (catalogue)

| ID | Issue | Evidence |
|---|---|---|
| I-1 | Exit IPv6 egress range may be refused by YouTube while IPv4 is accepted; no product control exists on a plain exit | E1–E2, E5 |
| I-2 | The only literal-IPv6 reject lives inside `apply_google_egress_hairpin` and vanishes with the hairpin | `server.rs:728–735` |
| I-3 | The hairpin target is IPv4-only; this was the original reason for the reject, so the rule's *documented* purpose is narrower than the real need | `server.rs:684–690`, INV-5 |
| I-4 | Documentation says the server config is "deliberately silent on IPv4-vs-IPv6 preference" and describes only the client↔server leg, never the exit↔internet leg | `CLIENT_PROTOCOL_BEHAVIOR.md` "IPv4 / IPv6" |
| I-5 | `CLAUDE.md` and INV-2 forbid "IPv6 changes" as a Shorts fix. True for refused-IPv4 nodes, false for IPv4-good/IPv6-bad nodes | `CLAUDE.md` hard rules, INV-2 |
| I-6 | Hetzner acceptance host lost IPv6 neighbour discovery after reboot (`fe80::1` failed), breaking the AAAA doctor check | rc.3–rc.5 acceptance history; environmental, AAAA record removed |
| I-7 | Installer only *tests* IPv6 reachability for diagnostics; it never decides about it | `install.sh:149–170` |
| I-8 | IPv6 egress is also a privacy surface (client IPv6 literals reach the exit's v6) with no documented policy | docs silent |
| I-9 | The hairpin's IPv6 reject depends on `sniff` recovering the SNI; ECH or SNI-less flows would bypass it | design (§8.2 risk R-2) |
| I-10 | No supported way to observe which families real clients use (log level `fatal`; diagnosis needed ad-hoc `ss` sampling) | E3 |

---

## 7. Other findings that are *not* the root cause but caused confusion

### 7.1 The old nodes
`de1` `91.244.71.165` (Evolus) and Hetzner `62.238.46.190`: both IPv4 and IPv6
refused. Nothing in this document changes that; for them only the hairpin helps.

### 7.2 The IPv6 experiment ordering mistake (mine)
My first browser probe used an SSH SOCKS that let `sshd` resolve names, which
prefers IPv6 → 0/6; the second forced IPv4 → 6/6. I initially read that as
"the VPN is fine", then found that sing-box itself prefers IPv4 for *domains*.
Only the literal-address harness (E1/E2) reproduces what a phone does.

### 7.3 "Failed host lookup" on the phone
The new hostname did not exist when the home router (`fritz.box`) first
looked it up, so the router cached NXDOMAIN. Public resolvers (1.1.1.1,
8.8.8.8, 9.9.9.9) answered correctly (measured). It is independent of Shorts
and goes away on mobile data, with a public DNS, or after the cache expires.
It deserves a troubleshooting entry (§9, D-10).

---

## 8. Fix plan

### 8.1 Phase A — stopgap on `157.173.27.46` (no release, reversible, needs owner approval)

The auto-mode classifier refused me this change, so the **owner** runs it on
the server console as root:

```bash
cat > /etc/sysctl.d/99-singbox-vpn-no-ipv6-egress.conf <<'EOF'
net.ipv6.conf.all.disable_ipv6 = 1
net.ipv6.conf.default.disable_ipv6 = 1
EOF
sysctl --system
systemctl restart sing-box && systemctl is-active sing-box nginx vpn-subscription
curl -4 -s https://ifconfig.me ; echo
curl -6 -s -m 5 https://ifconfig.me || echo "ipv6 off (expected)"
vpn doctor --protocol --require-protocol
```

Steps and gates:

1. Verify `sing-box`, `nginx` (`listen [::]:…`) and the subscription still start
   with IPv6 disabled (a `bind` on `::` should still succeed with the sysctl
   method; **if `systemctl restart sing-box` fails, run the undo immediately**).
2. Re-run the E2 controlled test (literal IPv6 must now fail fast, not stall)
   and, decisively, the **native iPhone test** on REALITY then Hysteria2: Shorts
   play, normal video plays, 5 consecutive Shorts.
3. **Gate:** if Shorts still fail, stop. Collect the sampler output (§10.4) and
   reconsider §5 before any code change.
4. Undo: delete the sysctl file, `sysctl --system`, restart `sing-box`.

Why a stopgap and not the final fix: it also disables the host's own IPv6
(apt, certbot, updates use IPv4, fine) and does not tell the client why a flow
failed. ICMP behaviour for UDP/QUIC without a route is unmeasured; the product
rule in §8.2 gives a deterministic ICMP reject.

Alternative stopgap (surgical, not recommended by hand): an nftables rule in a
*new* table scoped to the `sing-box` uid with `ct state new` and
`reject with icmpv6 addr-unreachable`, leaving host IPv6 alone and established
replies to IPv6 clients untouched. This duplicates the product's own
`nftables-egress-isolation.sh` and should come in through the release instead.

### 8.2 Phase B — product fix (release v1.2.1, full RC cycle)

**Goal:** a plain exit and a hairpin exit both prevent literal-IPv6 flows to
the Google/YouTube domain set from leaving through an IPv6 address, with a fast,
visible failure so the client falls back to IPv4. Transparent to the client
(INV-3).

**Design (recommended).** New deployment setting:

```toml
[egress]
# What the exit does with a client-supplied LITERAL IPv6 destination.
#   reject_google (default)  reject for the Google/YouTube domain set (needs sniff)
#   reject_all               reject every literal IPv6 destination (no sniff needed)
#   allow                    previous behaviour; documented as Shorts-unsafe on
#                            hosts whose IPv6 range YouTube refuses
ipv6_literal = "reject_google"
```

Rendered rules (`crates/compat-config/src/server.rs`), exit role only:

```
{ "action": "sniff" }                                        // reject_google only
{ "domain_suffix": GOOGLE_EGRESS_DOMAINS, "ip_version": 6,
  "action": "reject", "method": "default", "no_drop": true }  // reject_google
{ "ip_version": 6, "action": "reject",
  "method": "default", "no_drop": true }                     // reject_all (no sniff)
```

* Factor the sniff+reject out of `apply_google_egress_hairpin` into a shared
  `apply_exit_ipv6_policy(config, policy)`; the hairpin keeps only the
  outbound and the route rule. One implementation, one set of tests.
* Ordering: after the four C-16 rules (as today), before the hairpin route.
* Domain flows are unaffected: `ip_version` matches only literal destinations
  (INV-5; verified in sing-box `route/route.go`). Do **not** use `ip_cidr: ::/0`.
* Default is `reject_google` because it matches the proven hairpin semantics and
  changes least. `reject_all` is the escape hatch when SNI cannot be recovered
  (R-2). **Owner decision D-A: default `reject_google` vs `reject_all`.**
* Update/repair re-render the config, so existing exits pick the rule up on
  `update.sh`/`vpn repair`; the 10-minute reconcile timer keeps it.
* `vpn doctor` must accept exactly these shapes (plain/hairpin × policy) and
  **fail** an exit whose policy is `reject_google` but lacks the rules
  (extends today's hairpin-only check in `apps/admin/src/main.rs`).
* Optional but valuable (I-10): `vpn doctor --egress-families`, a privacy-safe
  sampler that prints only counts of outbound tcp/udp 443 by family, no
  addresses. Removes the need for ad-hoc `ss` loops.

**Tests (write first, TDD):**

| Layer | Test |
|---|---|
| Renderer unit (`relay_role_policy.rs`) | plain exit renders sniff + reject (policy default); `reject_all` renders one rule and no sniff; `allow` renders neither; hairpin exit keeps `[sniff, reject, route]`; no `selector`/`urltest`; order after C-16 |
| Renderer negative | policy value validated; `[egress]` on a relay is refused; domain flows never matched (documented via `ip_version` semantics) |
| Contract | rendered config passes `sing-box check` with the pinned version (existing `singbox-validate` job) |
| Behaviour (loopback, real sing-box; same harness as `TWO_HOP_SYSTEM_TESTS.md`) | a TLS ClientHello with SNI `www.youtube.com` to a literal IPv6 is rejected in <2 s; the same to a non-Google SNI is *not* rejected (reaches the dial and times out/fails differently); QUIC variant if feasible |
| Doctor | doctor passes each valid shape, fails a missing-rule shape (`deploy/lib/tests/test-doctor-*.sh`) |
| Acceptance (`lifecycle-acceptance.sh`) | new stage asserts the rendered rules on a fresh exit; if the disposable host has IPv6, asserts a literal-IPv6 Google flow is rejected fast |
| Update path (`update.sh`) | A→B update from 1.2.0 renders the new rules and the post-update doctor passes (reuse the rc.4→rc.5 harness) |
| Docs guard (`youtube_invariants_doc.rs`) | extend to require INV-13 and the new enforcing tests |

**Tooling to add to the repo:** extend `deploy/lib/youtube_shorts_probe.py` with
`--literal-family {4,6}` and commit the literal-address SOCKS front used in E2
as `deploy/lib/literal_ip_socks_front.py`, with the "domain / literal v4 /
literal v6" matrix documented as the standing IP-acceptance probe. (My scratch
versions exist; they need cleanup and tests, and no credentials.)

**Invariant changes (same commit as the tests, say why in the message):**

* INV-2: remove "disabling IPv6" from the list of things that "do not change the
  outcome"; replace with: *the exit's IPv6 egress family is a separate IP that
  YouTube may refuse independently of its IPv4; the correct control is the
  literal-IPv6 reject (INV-5/INV-13), not host-level IPv6 changes, except as a
  documented stopgap.*
* INV-5: generalise from "hairpin" to "every exit": the reject applies with or
  without a hairpin; keep `no_drop`.
* New **INV-13 — Exit egress addresses are verified per family.** Before an IP
  is used as an egress address, run the probe for both the domain path and the
  literal-IPv4 and literal-IPv6 paths; a refused family needs the reject.
* INV-7/INV-8 and the "two already operated nodes" text: update to the real node
  inventory (§9, D-5) once the owner states it.
* `CLAUDE.md` hard rules: replace "Do not try to fix Shorts with … IPv6 …" by the
  scoped statement above.

**Release procedure** (`docs/RELEASE.md`): branch → PR → CI green → merge →
`v1.2.1-rc.1` tag → release gates → VPS acceptance (lifecycle + A→B update from
1.2.0 with rollback proof) → device acceptance (§10) → docs-only stable commit
(acceptance record + device ledger) → `v1.2.1` tag → manual dispatch with
`confirm_stable_release=true` (needs the owner; I cannot dispatch).

**Risks.**

| ID | Risk | Mitigation |
|---|---|---|
| R-1 | `reject_google` default changes behaviour for IPv6-literal Google traffic | That is the intent; clients fall back to IPv4; covered by behaviour test |
| R-2 | `sniff` fails to recover SNI (ECH, SNI-less QUIC, fragmented Initial) → flow reaches refused IPv6 | `reject_all` option; probe matrix; doctor documents the limit; ECH is not used by Google endpoints today (unverified) |
| R-3 | Sniffing adds a wait on server-first protocols (default sniff timeout) | Rule only matches after sniff; measure latency on non-TLS flows in acceptance; only tcp/25 is server-first and is rejected already |
| R-4 | `reject_all` breaks IPv6-only sites for TUN clients | Opt-in; documented |
| R-5 | New RC reveals unrelated harness flakiness (seen in rc.1–rc.5) | Keep the existing quiesce/start-limit fixes; do not widen scope |
| R-6 | Plain exits on refused-IPv4 nodes still fail Shorts | Out of scope: owner decision (INV-7/INV-8) |

### 8.3 Phase C — repository consistency sweep (§9), same release where it touches
code/docs, a docs-only follow-up where it touches only history.

### 8.4 Phase D — operations cleanup and security (§11)

---

## 9. Inconsistencies found in the repository

Severity: **H** misleads operators/agents into a wrong action; **M** wrong or
stale statement; **L** wording/housekeeping. "Fix" is the proposed change.

### 9.1 Contradictions with the new finding

| ID | Where | Inconsistency | Sev | Fix |
|---|---|---|---|---|
| D-1 | `CLAUDE.md` hard rules | Says IPv6 changes are "not the cause" of Shorts failures | H | Scope the rule (§8.2) |
| D-2 | `YOUTUBE_FIX_INVARIANTS.md` INV-2 | Lists "disabling IPv6" as measured-useless; true only where IPv4 is also refused | H | Rewrite as in §8.2 |
| D-3 | INV doc "Root cause" | "IPv6 … are not the cause" stated as a general consequence | H | Add the per-family measurement and the new node's result |
| D-4 | INV-5 / `server.rs:684–690` / `server.rs:728` | Reject exists only with a hairpin and is justified only by "paired node is IPv4-only" | H | Generalise (§8.2) |
| D-5 | INV-7, INV-8, "Known gaps", `CLAUDE.md` | Name two operated nodes (`de1`, `ru1`) and the hairpin target as fixed; reality now has a third node, `de1` reinstalled plain | H | Update inventory with the owner; mark `de1` state |
| D-6 | INV "Known gaps" bullet 1 | Says the exit runs v1.1.0 binaries with a hand patch and the reconcile timer stopped — long false (rc.5 deployed, then reinstalled) | M | Rewrite or delete |
| D-7 | INV "Status" table | "Hysteria2 … not tested" although the owner reported REACTIVE+Hysteria2 working, and the acceptance record says PASS | M | Reconcile with `DEVICE_ACCEPTANCE_TESTS.md` and `docs/release-acceptance/v1.2.0.md` |
| D-8 | `CLIENT_PROTOCOL_BEHAVIOR.md` "IPv4 / IPv6" | "deliberately silent on IPv4-vs-IPv6 preference … does not force one, does not disable one" — false for an exit with the reject rule; also omits the exit→internet leg | H | Rewrite section; document `[egress].ipv6_literal` |
| D-9 | `CLIENT_PROTOCOL_BEHAVIOR.md:~190` (`youtube-direct`) | "YouTube's Shorts playability decision rejects sessions that egress from a hosting/datacenter IP, and only egress from the client's own line is demonstrated to work" — disproved (Selectel, and the new node's IPv4, play) | M | Correct to "some datacenter IPs/ranges" |

### 9.2 Stale code comments and CLI help (user-visible)

| ID | Where | Inconsistency | Sev | Fix |
|---|---|---|---|---|
| D-10 | `deployment.rs:347` | Comment still says "peering to Google's CDN" | M | Rewrite to the IP-acceptance cause |
| D-11 | `apps/admin/src/main.rs:604–617` (`user google-egress-hairpin` help) | Cites superseded `YOUTUBE_FINAL_ROOT_CAUSE.md §16`; says "RELAY ROLE ONLY … never for a real end user" while the real deployment used an **exit-role** dedicated user (the sibling `set` help even says that works) | M | Make both helps agree; cite INV doc |
| D-12 | `render.rs`, `model.rs`, `server.rs:898`, `apps/admin/src/main.rs:2521–2876` | Many comments/help strings cite `YOUTUBE_FINAL_ROOT_CAUSE.md §13–§16`, whose theories the doc itself marks wrong | L | Point to INV doc / §18 |
| D-13 | `YOUTUBE_FINAL_ROOT_CAUSE.md` header | Banner says superseded, yet the second paragraph still opens with "Status (2026-09-20): FIXED and device-path-verified … peering" | L | Keep history but strike/qualify the status line |
| D-14 | `DEVICE_ACCEPTANCE_TESTS.md` "Superseded" entry (~l.129) | Says the discriminator is "per-CDN-edge latency … not IP class" — flatly opposite of the proven cause | M | Mark "disproved", link INV doc |

### 9.3 Docs that contradict recorded evidence

| ID | Where | Inconsistency | Sev | Fix |
|---|---|---|---|---|
| D-15 | `DEVICE_ACCEPTANCE_TESTS.md` "Matrix" table | Every client/protocol cell reads "not yet tested", but the file's own 2026-09-27 entry (iPhone/iOS 18.7, REALITY+Hysteria2, DEVICE-VERIFIED) and the v1.1.0/v1.2.0 acceptance records say otherwise | H | Update cells with dated references |
| D-16 | same file, "Open: YouTube native-app acceptance" | "no YouTube test has passed on any device" vs the 2026-10-11 user-reported PASS entry | M | Rewrite the section header |
| D-17 | same file, "Canonical evidence ledger" | "Current as of 2026-08-22 for commit 5370a30" at HEAD d022d61 (releases 1.0–1.2 since) | M | Refresh or mark historical |

### 9.4 Installer / operator experience

| ID | Where | Inconsistency | Sev | Fix |
|---|---|---|---|---|
| D-18 | `install.sh` (7, 663, 672, 675, 778, 786), `deploy/lib/os.sh:66`, `deploy/almalinux/README.md`, `SUPPORTED_PRODUCT.md:59` | Says "supported **v1.0** target" in a v1.2.0 product; Ubuntu is "tested" (`implementation_coverage=tested`) yet warned as "outside supported target" while README lists it "Recognized / best-effort" | L | Unify wording ("supported target: AlmaLinux 9"; tiers named identically in installer, `os.sh`, README, SUPPORTED_PRODUCT) |
| D-19 | Ubuntu install | Installer leaves nginx's `default` site on `:80`/`[::]:80`; `vpn doctor` then WARNs about an "unexpected listener" it caused, while INSTALLATION says TCP/80 is only opened during renewals | M | Disable/replace the default site on Debian-family installs, or whitelist the nginx :80 vhost in doctor; keep the parity test (`test-doctor-public-surface-parity.sh`) |
| D-20 | `INSTALLATION.md` "TLS / Certbot" | Table implies two hostnames/two certs (`vpn.` and `sub.`); a default install uses one name and one certificate (nginx) plus a copy for Hysteria2; `subscription_host` is a separate optional field | L | Describe the single-name default and when the second differs |
| D-21 | `INSTALLATION.md` Troubleshooting | No entry for "Failed host lookup" right after creating the DNS record (negative caching in router/ISP) | M | Add entry (§7.3) |
| D-22 | `vpn-admin user` CLI | `remove` prints "REALITY handshake self-test … INCONCLUSIVE — does NOT confirm" on a host where the probe principal exists and `doctor` passes; also no `delete` verb (`remove` only), while ad-hoc docs/scripts used `delete` | L | Make the self-test message accurate or silent for non-probe removals; keep verbs consistent in docs |
| D-23 | `vpn-admin user links` / subscription | Out-of-band URI emits the **hostname**, so a DNS failure also breaks the "DNS-independent" recovery path the help text describes | M | Offer `--use-ip` (or print both) |
| D-24 | `sing-box` log level `fatal` by design | Good for privacy, but leaves no supported per-flow diagnostic; diagnosis needed ad-hoc sampling | L | `vpn doctor --egress-families` (§8.2) |
| D-25 | `README.md` / `INSTALLATION.md` (my 2026-10-12 edit, PR #149) | Says "both certificates and the subscription use that single name" — accurate for the name, but the same section elsewhere still describes two; and the new Shorts section says a plain install "has no workaround", which becomes false once §8.2 ships | L | Re-edit in the same release |

(There are no remaining inconsistencies between `Cargo.toml`/`Cargo.lock`
(1.2.0), `fixtures/arcana-node/node-capabilities-v1.json` (`node_release` 1.2.0)
and the published `v1.2.0` assets as far as checked; the CI
`workspace-version-consistency-check` passes. Not every historical doc under
`docs/reviews`, `docs/archive` and `TASKS.md` was audited; they are labelled
historical and are excluded from this list.)

---

## 10. Verification plan

1. **Config shape:** unit/behaviour/doctor tests in §8.2 green in CI.
2. **VPS acceptance (disposable AlmaLinux 9):** lifecycle + A→B 1.2.0→1.2.1 with
   rollback proof; record run URLs in `docs/release-acceptance/v1.2.1.md`.
3. **IP/family probe matrix (browser, per exit IP):** domain / literal-IPv4 /
   literal-IPv6 each 6/6 (or literal-IPv6 *rejected fast* when the family is
   refused). Commit results as evidence, class "browser verified".
4. **Egress-family sampler:** `vpn doctor --egress-families` (or the `ss`
   sampler) during a phone session shows no outbound IPv6 to Google ranges on
   an exit with `reject_google`/`reject_all`.
5. **Native device (the only acceptance, INV-12):** iPhone, Hiddify, on
   REALITY **and** Hysteria2: profile name "Arcana VPN"; Shorts `sAcElROnYIE`
   plus five consecutive; a long video for ≥ 1 minute; Wi-Fi **and** cellular;
   disconnect/reconnect; 15-minute session; record device, iOS, Hiddify version,
   network, protocol, date, commit in `DEVICE_ACCEPTANCE_TESTS.md`.
6. **Regression guard:** `youtube_invariants_doc.rs` covers the new invariant;
   the matrix probe is documented as the gate before choosing any new egress IP.

Pass criteria: every row above passes; any literal-IPv6 Google flow is rejected
(not stalled) when its family is refused; no regression on the old
`google_egress_hairpin` shape (exit with hairpin still renders
`sniff → reject → route` and passes doctor).

---

## 11. Operations and security items (not code)

| Item | Action |
|---|---|
| Root passwords for `91.244.71.165`, `135.106.178.167`, `62.238.46.190` and `157.173.27.46`, a Cloudflare API token and several subscription links/UUIDs were pasted into chat | **Rotate all of them**; use SSH keys; revoke the token |
| `157.173.27.46` | Owner approves/applies §8.1; after v1.2.1, remove the sysctl stopgap; consider AlmaLinux 9 for the supported tier |
| `de1` `91.244.71.165` | Plain exit with a refused IP: do not point users at it for Shorts, or re-add the hairpin (owner decision on INV-7/INV-8) |
| `ru1` `135.106.178.167` | Still on an older release; provisioning agent must stay disabled; `hairpin-de1` user exists; remove `ios-test` |
| Hetzner `62.238.46.190` | Disposable acceptance host; its AAAA record is deleted; clean leftovers |
| Leftovers on old nodes | `/root/arcana-backup-*`, `/root/config.json.pre-*`, `/root/rc*-updater`, `/root/warp-test` (inert WARP registration — delete with owner approval) |
| DNS | `vpn.utamary.com` (IDN `vpn.xn--80aa9argf5d.com`) points at `157.173.27.46`; stale records for dropped nodes should be removed; keep one A, DNS-only, no AAAA unless needed |
| `update.sh` cosign (141 MB) download sticks on slow flows | Add a download timeout + retry; separate small fix (new RC) |
| Memory/notes | Update the project memory when the plan changes state |

---

## 12. Decisions needed from the owner

| ID | Decision | Recommendation |
|---|---|---|
| D-A | Default `ipv6_literal`: `reject_google` or `reject_all` | `reject_google` (least change); document `reject_all` as the escape hatch |
| D-B | Apply the Phase A stopgap on `157.173.27.46` now | Yes; it is reversible and gives the native-app answer before coding |
| D-C | Is `de1` to be reconfigured with the hairpin to `ru1`, or retired, or left as is? | Owner's call under INV-7/INV-8; document the outcome |
| D-D | Release train: v1.2.1 (patch) with the full RC procedure | Yes; this changes rendered runtime config |
| D-E | Authorise the manual public release dispatch for v1.2.1 when acceptance is done | Required each time (I cannot dispatch) |

---

## 13. Reproduction notes

* Server-side matrix: start a temporary sing-box **client** on the node that
  dials `127.0.0.1:<reality-port>` with a throwaway user and exposes a local
  SOCKS port; run `curl -x socks5h://…` (domain), `curl -x socks5://…
  --resolve host:443:<A>` (literal v4) and `--resolve host:443:[<AAAA>]`
  (literal v6) against a dual-stack echo host. Stop the client and delete the
  throwaway user afterwards.
* Browser matrix: `deploy/lib/youtube_shorts_probe.py --proxy socks5://127.0.0.1:<port>`
  against each of the three paths; keep the harness out of any production
  process list.
* Never run the matrix with a customer's credential, and never leave a
  throwaway user or temporary process behind (all of mine were removed).
