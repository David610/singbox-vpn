# Russia production investigation (2026-08-14)

## Scope, evidence labels, and baseline

This investigation analyzed `74b63cd886dd54e8601520980de578ef55c81c2b` on branch
`work`. The initial tree had one pre-existing untracked file,
`fuzz/Cargo.lock`; it was not modified or committed. There is no configured Git
remote in this checkout, so ahead/behind and remote fetch results are
**UNVERIFIED**. The host was Ubuntu 24.04.4, x86_64, root, Rust/Cargo 1.94.1,
Bash 5.2.21. `systemctl` exists, but this container is not a production systemd
host; sing-box, Docker, firewalld, SELinux tools, nft, certbot, shellcheck,
cargo-deny, and cargo-audit were unavailable. No production VPS credentials,
Russian device, Russian network, or affected Hiddify installation was supplied.

Consequently, local code/test outcomes below use **PASS** or **FAIL**, while all
claims about Germany, Russia, real devices, the live Outline deployment, or a
real VPS remain **UNVERIFIED** or **BLOCKED BY ENVIRONMENT**. “CONFIRMED” means
confirmed only at the stated layer; it never upgrades a network acceptance cell.

## Architecture and product surfaces

```text
 device
  +-- Hiddify/independent sing-box -- client-owned TUN or proxy-only, DNS,
  |                                  IPv4/IPv6 and routes
  |        | VLESS+REALITY TCP/443 or optional Hysteria2 UDP/443
  |        v
  |     sing-box listeners ----------------------> direct server outbound
  |        ^                                              |
  |        | subscription JSON/URI                        v
  | reverse proxy:8443 <-- Rust subscription service --> Internet destination
  |
  +-- native Rust client-daemon -> native relay-agent -> Internet destination

 separately managed Outline client -> existing Outline listener -> Internet
```

The native Rust policy/failure-classifier stack and the third-party-client
deployment are separate products. The former does not select transports for
Hiddify. The production-compatible renderer, subscription service, server
renderer, admin diagnostics, and lifecycle scripts are code-verified and
unit/integration tested locally. The external-client TUN/DNS/route behavior,
live reverse proxy, live listeners, direct VPS egress, and Outline are not
locally reproducible. Real-VPS, real-device, and Russian-network status for
every component is **UNVERIFIED**.

## Generated profile audit (CONFIRMED by renderer tests)

The native JSON contains VLESS, UUID, Vision flow, REALITY, public key, short
ID, SNI, chrome uTLS; Hysteria2, TLS/SNI and optional Salamander; a `urltest`
named `auto`; a manual selector; direct outbound; and `route.final: select`.
The deterministic default is REALITY; `auto` remains selectable and probes
exactly `https://www.gstatic.com/generate_204`. That is only a small generic
HTTPS latency/reachability probe, not Telegram, sustained transfer, DPI, or
Russia health.

The renderer emits no DNS block/rules, IP-family policy, inbound, TUN, mixed or
SOCKS listener, bypass/region rule, kill switch, auto/strict route, MTU, TLS
fragment/padding, mux/multiplex, packet encoding, TCP keepalive, or TCP fast
open. Thus it cannot force tunneled DNS, IPv4, full-device routing, or prevent
IPv6 escape. Hiddify owns those settings and may override/ignore imported
intent. URI lists carry individual transport credentials but have no selector
or route semantics, so URI and JSON imports are not behaviorally equivalent.
Repository output does not explain mux seen in production logs; client defaults,
a different profile, or deployed drift must be tested. Hiddify field retention
and current versions are **UNVERIFIED** because source/network research and the
affected app were unavailable.

## Executed versus unavailable experiments

| Experiment | Result | Evidence classification |
|---|---|---|
| Exact renderer shape and absence assertions | **PASS** locally | CONFIRMED at serialization layer |
| Rust workspace tests/lints and repository gates | see final test log | CONFIRMED locally only |
| Disposable AlmaLinux lifecycle | **BLOCKED BY ENVIRONMENT** | UNVERIFIED |
| sing-box schema/protocol interop | **BLOCKED BY ENVIRONMENT** when binary/network absent | UNVERIFIED for live path |
| Germany working observation | **UNVERIFIED** | ANECDOTAL |
| Russia mobile/Wi-Fi, Hiddify, raw sing-box, Outline A/B | **BLOCKED BY ENVIRONMENT** | UNVERIFIED |
| Fragmentation packet A/B | **BLOCKED BY ENVIRONMENT** | UNVERIFIED |
| REALITY mux/no-mux A/B | **BLOCKED BY ENVIRONMENT** | UNVERIFIED |
| UDP loss/block/rate/MTU tests | **BLOCKED BY ENVIRONMENT** | UNVERIFIED |

No exact measured live differences between Outline, REALITY, and Hysteria2 can
truthfully be reported. Protocol-level known differences (Shadowsocks versus
TCP REALITY versus QUIC/UDP Hysteria2) are not evidence for why this incident
occurs.

## Ranked hypothesis tree

1. **Client TUN/routing/split-route/IPv6 behavior — LIKELY, medium confidence.**
   Evidence for: “connected” with a Russian public IP is compatible with a
   proxy-only or inactive/partial TUN, and the subscription does not configure
   TUN, DNS, or IP-family routing. Evidence against: no affected-device route or
   packet capture exists, and an authenticated but stalled tunnel could create
   similar UI symptoms. Falsifier: on the same affected version/profile,
   capture device routes plus IPv4 and IPv6 public-IP probes in VPN/TUN mode;
   prove both families leave through the VPS while the symptom remains. Result:
   **UNVERIFIED**.
2. **Hiddify integration/version defect — LIKELY, low-to-medium confidence.**
   Evidence for: the client owns routing/import behavior and can report app
   state independently of system routing. Evidence against: raw upstream
   sing-box has not succeeded on the same Russian path. Falsifier: one-variable
   A/B using the minimal raw client and Hiddify, identical credentials/network/
   destination/time. Result: **UNVERIFIED**.
3. **Post-handshake interference or PMTU/path failure — LIKELY, medium
   confidence.** Evidence for: reported brief success followed by stalls.
   Evidence against: no repeatable byte threshold, retransmission trace, RST,
   ICMP PTB, or synchronized log exists. Falsifier: three exact-size repetitions
   per transport with synchronized bounded capture, varying only MTU in a second
   phase. Result: **UNVERIFIED**.
4. **Stale/mismatched REALITY state — POSSIBLE, low confidence.** Evidence for:
   intermittent invalid-connection logs can accompany a stale profile. Evidence
   against: that message can also describe unauthenticated probes and repository
   doctor already checks stored/live fingerprints. Falsifier: correlate a
   client attempt timestamp/profile fingerprint with server log and a successful
   minimal-client authentication using the same material. Result:
   **UNVERIFIED**.
5. **UDP blocking/throttling specific to Hysteria2 — POSSIBLE, medium confidence
   for Hysteria2 only.** Evidence for: Hysteria2 requires UDP. Evidence against:
   it cannot explain REALITY/TCP or a Russian IP after claimed TUN activation.
   Falsifier: simultaneous TCP/443 and UDP/443 captures on the same path, then a
   controlled UDP-blocked network. Result: **UNVERIFIED**.
6. **Generic VPS, DNS, TCP/443, or single bad SNI — SPECULATIVE, low
   confidence.** Outline on the reported same VPS and Germany observation argue
   against a total outage, but are anecdotal. Dual-family target validation and
   synchronized A/B are the falsifiers. Result: **UNVERIFIED**.

EOF/`processed invalid connection` alone must never be classified as DPI or bad
credentials. Suggested diagnostic classes after evidence collection are
`TCP_REACHABLE_BUT_POST_HANDSHAKE_STALL`, `CENSORSHIP_SUSPECTED`,
`PMTU_OR_MTU_FAILURE`, `CLIENT_CORE_BUG`, `CLIENT_TUN_NOT_ACTIVE`,
`DNS_FAILURE`, `UDP_BLOCKED`, and `UNKNOWN`; default to `UNKNOWN` when the
required discriminating evidence is missing.

## Implemented changes, compatibility, rollback

The renderer now has an exact regression contract for required REALITY fields,
the URLTest URL, and absence of client-owned/experimental fields. This changes
no emitted profile bytes and has no compatibility impact. A new read-only
investigation helper validates a target by DNS, IPv4/IPv6 TCP+TLS+HTTP timing,
certificate hostname, optional ICMP loss, performs a root/operator-triggered
bounded capture filtered to one client and ports 443, and summarizes packet
metadata without payload. It has no install/service/firewall integration, so
rollback is deletion of the helper and test. Its validation test covers bounds
and injection-resistant inputs. Real-device testing remains required.

## Architecture choices and recommendation

* **Option A — minimal change:** retain REALITY primary and optional Hysteria2;
  make independent raw sing-box the control client and require explicit TUN and
  dual-family public-IP acceptance. Lowest risk, but one VPS/IP remains a single
  censorship and outage domain.
* **Option B — genuinely different fallback:** after the controlled experiment,
  keep the existing Outline deployment separately managed or add a separately
  ported maintained Shadowsocks implementation. Do not touch or import Outline
  credentials/lifecycle. This is preferred only if synchronized Russian A/B
  proves a Shadowsocks-family advantage and port preflight/coexistence tests pass.
* **Option C — Russia-first:** multiple providers/ASNs/endpoints plus at least two
  measured wire families, client-specific onboarding, and continuous Russian
  mobile/fixed acceptance. This changes v1 product scope and operations.

Recommendation today: **Option A while collecting evidence**. Hiddify should be
listed as provisional, not universally recommended. REALITY may remain the
deterministic primary only as an unverified default, Hysteria2 should remain an
optional UDP path, and no new Shadowsocks service should be added until the
Outline control experiment identifies a reproducible advantage. vpn1 cannot
truthfully be called Russia-ready: status is **UNVERIFIED**.

## Exact operator procedure

On the VPS, first validate the configured target without printing credentials:

```bash
sudo /opt/vpn1/deploy/lib/vpn-investigate.sh target www.google.com 443
sudo vpn-admin doctor --protocol
sudo vpn-admin doctor --report --report-output /root/vpn-doctor.txt
```

For each Russian phone network (mobile, then Wi-Fi), record UTC time, phone/app/
core versions, profile fingerprint, region, VPN/TUN versus Proxy Only, and
selected transport. Disable mux and fragmentation for baseline. Verify both
`https://api4.ipify.org` and `https://api6.ipify.org` (an unavailable family is
recorded, not silently passed), then DNS, browser, Telegram text/media/call, and
three repetitions each of 1, 4, 8, 16, 20, 32, 64, 256 KiB, 1 MiB, and 10 MiB
from an operator-controlled exact-size endpoint. Record TTFB, completion,
received bytes and error. Repeat, changing only client: Hiddify, minimal raw
upstream sing-box (one loopback SOCKS inbound, one REALITY outbound, no DNS,
selector, urltest, mux, or routes), independent client, and Outline. Run each
stable path for ten minutes, screen-off idle, and Wi-Fi/cellular handover.

At the synchronized UTC start, restrict capture to the phone's observed source
IP (never a broad public capture):

```bash
sudo /opt/vpn1/deploy/lib/vpn-investigate.sh capture CLIENT_PUBLIC_IP /root/run.pcap 120
sudo journalctl -u sing-box --since '2026-08-14 12:00:00 UTC' \
  --until '2026-08-14 12:02:30 UTC' --output short-iso > /root/run-singbox.log
sudo /opt/vpn1/deploy/lib/vpn-investigate.sh summarize /root/run.pcap > /root/run-summary.txt
sudo chmod 600 /root/run.pcap /root/run-singbox.log /root/run-summary.txt
```

Redact source IPs and inspect logs for secrets before sharing. Compare same VPS,
phone, network, destination, duration, and timestamps. Then perform one-variable
A/B for mux; fragmentation on/off requires a client-side capture and ClientHello
packet-length/timing comparison. MTU 1280/1360/1400/1500 comes only after the
baseline and never becomes a global default from one run. Test system DNS,
8.8.8.8/1.1.1.1 UDP, DoH/DoT, raw IP/hostname, IPv4/IPv6/dual stack, private and
split DNS separately. Record every unexecuted cell **UNVERIFIED**.

## Known limitations and next gate

The helper cannot infer authentication from packets, inspect phone TUN state,
create an exact-size HTTP server, or test censorship. Captures truncated to 160
bytes intentionally minimize payload exposure, so detailed TLS fingerprinting
needs a separately approved client/server capture procedure. Hiddify release/
issue research was **BLOCKED BY ENVIRONMENT** by unavailable authenticated web
access and must be repeated against upstream source/release artifacts. Live
Outline measurements, certificate/key coherence, resource/load stress,
AlmaLinux lifecycle/reboot/SELinux/firewalld, and Russian acceptance remain
**UNVERIFIED**. The next architecture decision gate is a complete synchronized
Outline/REALITY/Hysteria2/raw-sing-box matrix—not another speculative protocol
setting change.

## 2026-08-19 addendum: repo re-audit, Aug-15 diff, Xray-core A/B path

**Status: implemented, server-tested, awaiting Russian verification. Nothing
in this addendum claims the connectivity problem is fixed.**

### Symptom recap

Production (Ubuntu 24.04.4, upstream sing-box VLESS+REALITY on TCP/443,
Hysteria2 on UDP/443) passes server-side self-tests (`vpn-admin doctor
--protocol` and a throwaway upstream sing-box client completing a full
REALITY handshake locally against the live config), but real Russian
Hiddify clients repeatedly get `inbound/vless[vless-reality-in]: process
connection from <IP>:<port>: TLS handshake: REALITY: processed invalid
connection`, including from a freshly reinstalled profile (rules out a
stale-credentials theory). Outline behavior (MagicOS partial, iPhone/
Android failing on Wi-Fi and mobile data) is tracked as a probably-separate
network-layer issue — no shared root cause is assumed.

### What `processed invalid connection` means (from source/behavior
knowledge, not a live capture)

Per `docs/INCIDENT_2026-08-10_REALITY_HANDSHAKE_TIMEOUT.md` §2 and
`docs/INCIDENT_2026-08-11_REALITY_HANDSHAKE_INCONCLUSIVE.md` §2, this line
is logged by the SERVER's REALITY layer (`github.com/metacubex/utls`'s
`reality.go`, vendored into sing-box) when it cannot authenticate a
connection's REALITY handshake — the connection is not dropped, it is
transparently proxied through to the real decoy `handshake_server` instead
(REALITY's core design property: a rejected connection is indistinguishable
from an ordinary TLS session with the decoy). It does NOT by itself
distinguish: an unauthenticated probe/scanner, a client presenting a
mismatched/stale public key or short_id, active DPI interference, or a
client-side TLS/REALITY implementation difference (e.g. a different core
engine's ClientHello shape). Treating it alone as proof of DPI or bad
credentials would be exactly the overclaim both incident documents warn
against. The new `vpn-investigate.sh client <IP>` command (below) surfaces
the host-wide count of these events alongside accepted/reset counts and
recent per-IP journal lines, explicitly labeled FACT/INFERENCE/UNKNOWN, as
the safe minimal diagnostic for narrowing this — no sing-box fork, no
protocol change.

### Exact versions

- sing-box: pinned `1.13.19` (`deploy/lib/versions.env`; matches
  `docs/COMPATIBILITY_VERSIONS.md`'s 2026-08-17 entry). Bumped from
  `1.13.18` in commit `f1dd376` ("Bump pinned sing-box 1.13.18 -> 1.13.19
  (stable patch release)") — a routine patch bump, not a protocol change;
  no changelog evidence was available in this session to say whether it
  touched REALITY handling.
- Hysteria2: sing-box's bundled inbound (not the standalone
  `apernet/hysteria` binary) — unchanged.
- Xray-core: not used server-side; see "Why sing-box, not Xray-core" in
  `docs/COMPATIBILITY_VERSIONS.md` and `docs/ADR/0002-transport-portfolio.md`.
  No concrete server-side incompatibility with Xray-core has ever been
  found — the seam (`CompatibilityBackend`) exists but was never exercised.
- Hiddify client version/build: **UNVERIFIED** — not supplied this session.

### Hiddify `core=xray` / `xvless://` support — UNVERIFIED, not assumed

This session had no web access to check current Hiddify release notes or
source for whether it exposes a distinct `core=xray` import path, an
`xvless://` URI scheme, or any other Xray-specific import syntax. Per the
task's own instruction, that syntax was **not invented**. Instead, Step 5
below ships the already-standard `vless://` share-link shape (which both
sing-box-core and Xray-core already parse identically — see
`crates/compat-config/src/render.rs`'s existing
`render_vless_reality_uri`) under an explicit, separately-labeled link, so
a Russian tester can deliberately import "the other one" as a control, and
the operator can tell from the endpoint label (never sent over the wire —
purely an operator/tester bookkeeping aid, e.g. in
`vpn-investigate.sh client`'s output) which one was used. If real Russian
testing later shows Hiddify needs different import syntax to force its
Xray-core engine specifically, that must be verified against real Hiddify
source/docs before being added — not guessed.

**"identically" above needs a caveat added later and cross-referenced
here, not silently left standing**: the two cores parse the *same
share-link syntax* identically (same fields accepted, same encoding) —
that is what was actually verified. Their **runtime behavior** for
`xtls-rprx-vision` is NOT identical: `docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md`
§9.5a/§13.3 (a later pass, source-verified against sing-box v1.13.19 and
Xray-core) found Xray-core's `xtls-rprx-vision` actively intercepts and
rejects application UDP/443, while sing-box's does not. This A/B link
therefore isolates more than "which core parsed the link" — it also
isolates that specific behavioral difference, which is directly relevant
if a Russian YouTube-app test ever uses this `?format=xray` link. See
§13.3 there for why a Russia-side Vision/UDP result must not be
attributed to "Vision blocks QUIC" without confirming which core was
actually active for that specific test.

### Aug-15 (last known-working) vs current: exact REALITY-path diff

The task named a prior-repo commit hash that does not exist in this repo's
history (a mistyped/concatenated hash across the `vpn1`→`singbox-vpn`
migration). The actual migration point is commit `7392990` ("Migrate
repository to singbox-vpn", 2026-08-15), preceded by `29e1e5c` (2026-08-14,
"Merge pull request #25 ... perform-production-hardening-pass") — the last
`vpn1`-era commit before the rebrand. `29e1e5c..HEAD` is therefore the
honest "Aug-15 baseline vs current" diff window.

**`crates/compat-config/src/model.rs`, `deployment.rs`, `credentials.rs`:
zero diff.** REALITY key generation/validation (base64url-no-pad X25519
keys — see `credentials.rs`'s `validate_reality_keypair`/
`validate_reality_public_key_shape`), the `RealityServerParams`/
`PublicParameters::Reality` shapes, and server-side inbound rendering are
byte-identical to the Aug-15 state.

**`crates/compat-config/src/render.rs`: one material change, additive and
opt-in.** `CompatibilityMode`/`render_singbox_client_subscription_with_options`
were added (commit `e958e79`, iOS YouTube TCP-only compat mode) — but
`render.rs`'s own test `normal_mode_via_with_options_is_identical_to_existing_renderer`
proves `CompatibilityMode::Normal` (the default; unrelated to REALITY
authentication) reproduces the pre-existing subscription byte-for-byte.
Every REALITY-relevant field — `encryption=none`, `security=reality`,
`sni`, `fp` (uTLS fingerprint), `pbk` (public key), `sid` (short_id),
`type=tcp`, `flow=xtls-rprx-vision`, and the native JSON's
`tls.reality`/`tls.utls` blocks — is unchanged in both value and encoding.
The REALITY decoy `handshake_server` default (`www.google.com`) predates
`29e1e5c` (commit `2d4575f`, before the diff window).

**`services/subscription/src/lib.rs`, `apps/admin/src/main.rs`:
substantial diffs, none touching REALITY handshake material.** Grepping
the diff for REALITY-related additions/removals surfaces only cosmetic
label text (e.g. `"VLESS+REALITY"` display strings) and reload-messaging
changes around `reality-rotate`; no change to key generation, validation,
encoding, or the fields sent to sing-box.

**Conclusion: no material regression was found in the REALITY rendering/
credential path between the Aug-15 baseline and current HEAD.** This does
NOT prove there is no regression anywhere (client-side behavior, a
sing-box upstream behavior change between whatever version was live around
Aug-15 and `1.13.19`, or a deployment/config-file drift on the actual
production VPS are all still open and unverified) — it narrows the search:
this repository's own generated-profile code is not the differentiator.

### Generated client profile audit (field-by-field, confirms
`docs/RUSSIA_PRODUCTION_INVESTIGATION.md`'s existing "Generated profile
audit" section)

Re-verified directly against `crates/compat-config/src/render.rs`:
- Share-link (`render_vless_reality_uri`, line ~26): `vless://{uuid}@{host}:{port}?encryption=none&security=reality&sni={sni}&fp={fp}&pbk={pbk}&sid={sid}&type=tcp&flow=xtls-rprx-vision#{label}`.
- Native JSON (`render_singbox_client_subscription_with_options`, line
  ~242): `type: vless`, `uuid`, `flow: xtls-rprx-vision`,
  `tls.enabled: true`, `tls.server_name`, `tls.utls.enabled: true`,
  `tls.utls.fingerprint`, `tls.reality.enabled: true`,
  `tls.reality.public_key`, `tls.reality.short_id`.
- Encoding: `public_key_hex`/`short_id` are misleadingly named (`_hex`
  suffix) but are actually base64url-no-padding
  (`credentials.rs::URL_SAFE_NO_PAD`, matching sing-box's
  `base64.RawURLEncoding` expectation for REALITY public keys) — verified
  correct despite the naming; no encoding bug found. No trailing
  whitespace, no case-folding, no padding characters (`=`) are introduced
  anywhere in the render path.
- Fingerprint (`fp=`/`utls.fingerprint`): hardcoded `"chrome"`
  (`render.rs::standard_endpoints`). No concrete evidence was found in
  this session that `chrome` is incompatible with any client/network —
  left unchanged per the task's own instruction not to change it without
  evidence. If a future Russian A/B test shows a specific fingerprint
  value correlates with success/failure, that is the trigger to revisit
  this, not a guess made now.

### What was implemented this session

1. **`?format=xray` subscription variant** (`services/subscription/src/
   lib.rs`, new `"xray"` arm in `get_subscription`;
   `crates/compat-config/src/render.rs`'s new
   `render_vless_reality_uri_xray_labeled`/`render_xray_uri_list`). Reuses
   the exact same UUID/pbk/sid/SNI/port/fingerprint/flow as the existing
   `?format=uri`/`?format=hiddify` share links — only the VLESS+REALITY
   line's label gets an explicit `" (Xray)"` suffix (percent-encoded as
   `%28Xray%29` in the URI fragment). Hysteria2 lines are unchanged/
   unlabeled (Hysteria2's URI syntax is not core-specific). The default
   `?format=singbox` (native JSON) and existing `?format=uri`/`hiddify`
   outputs are provably byte-for-byte unchanged (regression test below).
   `vpn-admin user create` now also prints/JSON-emits this link
   (`subscription_url_xray`, additive field in `--json` output) alongside
   the existing Hiddify/sing-box links — nothing existing was removed or
   reordered.
2. **`vpn-investigate.sh client <IP>`** (`deploy/lib/vpn-investigate.sh`):
   gathers, from THIS server only: `vpn-admin doctor --protocol` result;
   TCP/443 and UDP/443 listener presence (`ss`); recent (2h) sing-box
   journal lines mentioning the IP; host-wide counts of REALITY
   `processed invalid connection` / accepted / `connection reset` events;
   a suggested-but-not-run bounded `capture`/`summarize` command reusing
   the existing helpers; read-only firewall state
   (firewalld/nftables/iptables, whichever is present). Every line is
   labeled FACT/INFERENCE/UNKNOWN; no secret (REALITY key material, VLESS
   UUID, Hysteria2 password) is ever printed; nothing is mutated, captured,
   or invoked automatically.
3. **Fixed a real defect in `vpn-benchmark.sh`'s VLESS+REALITY/Hysteria2
   protocol-overhead section** (`deploy/lib/vpn-benchmark.sh`,
   `deploy/lib/vpn-benchmark-lib.sh`): the benchmark's throwaway-user
   subscription URL always carries an explicit `?format=...` query string
   (`apps/admin/src/main.rs::subscription_url`), but the section extracted
   its "token" with a bare `${sub_url##*/}`, which strips only up to the
   *last* `/` — leaving the query string attached to the token (e.g.
   `AbC123token?format=hiddify`). The section then re-appended its own
   `?format=singbox`, producing a doubled query string
   (`.../sub/TOKEN?format=hiddify?format=singbox`) that
   `services/subscription`'s `Query<SubQuery>` extractor parses as one
   literal `format` value matching none of `singbox`/`uri`/`hiddify`/
   `xray` — a guaranteed 400 that `curl -f` silently turned into "could
   not reach the local subscription backend", permanently and silently
   SKIPPING the entire section on every run. This is a strong plausible
   contributor to the reported `jq: parse error: Invalid numeric literal`
   symptom in that section: a malformed/empty response body reaching an
   un-redirected `jq` call downstream of a future change to this path
   would surface exactly that class of error; regardless of the precise
   trigger, the token-extraction bug itself was real and is fixed. New
   `vpn_benchmark_extract_token()` helper
   (`deploy/lib/vpn-benchmark-lib.sh`) correctly strips both the `/sub/`
   prefix and the `?...` suffix, with a regression test in
   `deploy/lib/tests/test-vpn-benchmark-lib.sh`.

### Tests added and results

- `crates/compat-config/src/render.rs`: 6 new unit tests for
  `render_vless_reality_uri_xray_labeled`/`render_xray_uri_list` (exact
  field contents, no-private-key, wrong-transport rejection, label-only
  diff from the default URI list). `cargo test -p compat-config --lib`:
  **all 100 tests pass** (94 pre-existing + 6 new). `cargo test -p
  compat-config` (including the `tests/*.rs` integration suites —
  `reality_interop`, `reality_decoy_budget`, `hysteria2_interop`): all
  pass.
- `services/subscription/src/lib.rs`: 6 new tests for `?format=xray`
  (200 + exact fields + no private key, identical-credentials-to-`uri`,
  `compat=tcp-only` rejection, and an explicit
  `default_and_existing_format_outputs_are_byte_for_byte_unchanged`
  regression test). `cargo test -p subscription --lib`: **all 26 tests
  pass** (20 pre-existing + 6 new); the crate's `tests/startup_validation.rs`
  integration suite also passes unchanged.
- `deploy/lib/tests/test-vpn-benchmark-lib.sh`: 5 new assertions for
  `vpn_benchmark_extract_token` (with/without/multiple query params, no
  `/sub/` segment, empty token) — **all pass**.
- `deploy/lib/tests/test-vpn-investigate.sh`: extended for the new
  `client` subcommand (input validation, FACT/INFERENCE labeling present,
  no secret-shaped output) — **all pass**.
- `cargo test -p admin --test cli`: 5 pre-existing failures, **not caused
  by this session's changes** — `apply_restored_file_policy()` performs a
  real `getgrnam("vpn-subscription")` lookup when running as root (this
  sandbox is root), and that group only exists on a real installed host;
  already documented as a "Blockers" entry in
  `docs/IMPLEMENTATION_STATUS.md` before this session. The 4
  `user_create_*` tests most relevant to this session's
  `apps/admin/src/main.rs` changes pass. `cargo test -p admin --test cli
  user_create` and the full targeted suite were re-run after this
  session's edits specifically to confirm no new failure was introduced —
  none was.
- `cargo fmt --check`: clean. `git diff --check`: clean (no whitespace
  errors).

### Unresolved risks / what remains UNVERIFIED

- Whether the Xray-core-oriented A/B link actually changes real-device
  behavior — **UNVERIFIED**, requires a real Russian device test (see the
  A/B/C/D template below).
- Whether Hiddify exposes any Xray-specific import syntax beyond the
  standard `vless://` shape — **UNVERIFIED**, no web access this session.
- Whether the `vpn-benchmark.sh` token-extraction fix is the actual root
  cause of the previously-reported `jq: parse error: Invalid numeric
  literal` (as opposed to a different, still-latent jq call) —
  **UNVERIFIED**; the fix removes a confirmed real defect on this code
  path regardless, but was not reproduced against a live host in this
  session (no disposable AlmaLinux 9 host / production VPS available).
- The core hypothesis tree in this document's earlier sections (client
  TUN/routing, Hiddify integration/version defect, post-handshake
  interference, stale/mismatched REALITY state, UDP blocking) remains
  **UNVERIFIED** exactly as before — this addendum narrows "is it the
  server-side renderer/credential path" to NO (see the Aug-15 diff above)
  without resolving which of those remaining hypotheses is correct.
- Outline's inconsistent behavior remains untouched and unexplained by
  this addendum — tracked separately per the task's own framing.

### Exact VPS deployment commands for this revision

Safe: does not rotate any key/UUID/password, does not touch MTU/BBR/
sysctls/firewall/DNS/SSH/TLS/certs/ports, does not remove REALITY or
Hysteria2, does not wipe users or config. This is a normal
release-to-release update — see `docs/IMPLEMENTATION_STATUS.md`
"Checkpoint 3" for the full transactional update contract.

```bash
# On the VPS, as the existing production install:
sudo /opt/vpn1/deploy/almalinux/update.sh --dev-rebuild   # dev/unreleased checkout
# or, once a tagged release containing this branch's commit exists:
sudo /opt/vpn1/deploy/almalinux/update.sh --version vX.Y.Z

# Verify nothing regressed:
sudo vpn-admin doctor --protocol --require-protocol
sudo deploy/lib/vpn-benchmark.sh --skip-tunnel=0   # confirm the jq fix: no more
                                                    # "could not reach the local
                                                    # subscription backend" SKIP
```

### Exact commands to generate each profile variant

```bash
# Normal REALITY + Hysteria2 (unchanged default — sing-box-core-oriented):
vpn-admin user create --name "russia-test-A"
# prints both the Hiddify (?format=hiddify) and native (?format=singbox)
# links, plus the new Xray A/B link (?format=xray), for one user/one set
# of credentials — see subscription_url_xray in apps/admin/src/main.rs.

# Fetch each format directly (TOKEN from the created user's printed URLs):
curl -s "https://<host>:<port>/sub/<TOKEN>?format=hiddify"   # share-link list (unchanged)
curl -s "https://<host>:<port>/sub/<TOKEN>?format=singbox"   # native JSON (unchanged)
curl -s "https://<host>:<port>/sub/<TOKEN>?format=xray"      # NEW: Xray-labeled A/B share-link list

# Hysteria2 only (already part of every format above; no separate command
# needed — Hysteria2 is not part of this session's A/B change).
```

### Russian A/B/C/D test sequence template

One row per attempt. Fill every column; use `UNVERIFIED`/`N/A` explicitly
rather than leaving a cell blank. A = Hiddify default link (sing-box
core), B = Hiddify Xray A/B link (`?format=xray`), C = raw upstream
sing-box (minimal client, no Hiddify), D = Outline (separate credential/
listener, tracked for comparison only, not expected to share a root
cause).

| Variant | Device | App version | Core (sing-box/Xray/unknown) | Network (Wi-Fi/mobile) | Carrier | Server log result (accepted / processed invalid connection / no attempt seen) | Public IP before | Public IP after connect | Sustained >10MB download (Y/N, MB, duration) |
|---|---|---|---|---|---|---|---|---|---|
| A |  |  |  |  |  |  |  |  |  |
| B |  |  |  |  |  |  |  |  |  |
| C |  |  |  |  |  |  |  |  |  |
| D |  |  |  |  |  |  |  |  |  |

Run `sudo vpn-investigate.sh client <observed-IP>` on the VPS immediately
after each attempt (within the same UTC minute if possible) and attach its
FACT-labeled output (event counts, listener presence, recent log lines) to
the corresponding row instead of paraphrasing it.

### Step 11 — explicitly deferred, not implemented

New transport protocols (XHTTP, WSS, or any other REALITY/Hysteria2
alternative) are NOT implemented in this session. This is gated on real
Russian testing of the Xray-core A/B path (above) also failing — only
then does the evidence justify the cost of a new transport. This is a
documented next step, not a decision made now.

## 2026-08-22 addendum: failure-domain decision, not a transport proposal

### Scope and research limitation

This addendum audits commit `f90b64adc97c38b111392e67b0f75219fd6a8a82`.
The checkout had no Git remote and the only pre-existing working-tree item was
untracked `fuzz/Cargo.lock`. No Russian network, device, second VPS, or
repository-settings access was available. Attempts to query the configured web
research service and to fetch OONI, sing-box, and Project X documentation were
rejected by the execution environment (HTTP 401 from the research service and
HTTP 403 from the outbound proxy). Therefore this pass does **not** claim a
fresh, comprehensive measurement of Russian filtering in 2026. Existing links
below are a research queue, not evidence that their current contents were
verified in this pass.

That limitation is decision-relevant: there is insufficient current field or
measurement evidence to make HTTPUpgrade, HTTP, gRPC, WebSocket, or xHTTP a
production or experimental server transport. No transport was implemented.

### Facts in the current repository

* **FACT / CODE-VERIFIED:** the server renderer has exactly two compatibility
  inbounds: VLESS+REALITY on the configured TCP port and Hysteria2 on the
  configured UDP port, with a direct outbound. The supported installer assigns
  both port 443. Both endpoints produced by `standard_endpoints` use one
  `public_host`.
* **FACT / CODE-VERIFIED:** `CompatTransport` can represent only
  `VlessReality` and `Hysteria2`. `standard_endpoints` produces exactly one of
  each. A second host is therefore not currently expressible through the
  standard deployment/subscription path merely by installing another VPS.
* **FACT / CODE-VERIFIED:** subscription delivery can use a distinct
  `subscription_host`, but the standard installer still provisions nginx and
  the subscription service as part of the same deployment. A distinct DNS name
  alone is not a distinct failure domain when it resolves to the same host.
* **FACT:** the first-party client is outside this repository's compatibility
  domain. This repository cannot prove that it imports multiple compatibility
  endpoints, performs cross-host failover, preserves TUN state during handover,
  or supports a newly proposed transport.
* **UNVERIFIED:** whether either current transport works on any named Russian
  ISP/carrier today; whether a particular ISP fingerprints Vision/REALITY,
  throttles long-lived sessions, blocks QUIC/UDP, enforces an allow-list, or
  blocks the current VPS IP/subnet/ASN; and whether native YouTube or TikTok
  works through either transport.

### Failure-domain map

| Observable/failure | VLESS+REALITY | Hysteria2 | Correlation and classification |
|---|---|---|---|
| Server public IP, subnet, provider and ASN | Same | Same | One IP/subnet/provider action can kill both — **FACT** about topology; filtering is **HYPOTHESIS** |
| Server host, route, power and account | Same | Same | One operational failure kills both — **FACT** |
| L4 | TCP/443 | UDP/443 (QUIC-based) | UDP-only impairment can isolate Hysteria2 — **FACT** at protocol layer; Russian incidence **UNVERIFIED** |
| Authentication/handshake family | VLESS + REALITY, Vision by default | Hysteria2 TLS, optional Salamander | A protocol-specific classifier could isolate one — **HYPOTHESIS** without a controlled trace |
| Subscription/control plane | Common service; commonly same host, TCP/8443 | Common service; commonly same host, TCP/8443 | Loss prevents refresh/onboarding but does not invalidate already downloaded tunnel credentials — **FACT** |
| Exit IP/ASN and destination peering | Same | Same | YouTube/TikTok treatment or peering problems can affect both independent of ingress transport — **STRONG INFERENCE** from topology, application cause **UNVERIFIED** |

Consequently, adding a third protocol on the same IP changes a wire behavior but
leaves the largest shared outage and censorship domains intact. It may still be
useful after a trace identifies protocol-specific interference; it is not the
highest-information first experiment now.

### Current-source research register

| Source to verify | Relevant question | Status in this pass | Confidence allowed |
|---|---|---|---|
| [OONI Russia research](https://ooni.org/country/RU) and dated OONI reports | What was measured, on which Russian networks, and when? | Network access blocked; no result imported | None; do not make a current Russia claim |
| [sing-box V2Ray transport documentation](https://sing-box.sagernet.org/configuration/shared/v2ray-transport/) | Current HTTP, HTTPUpgrade, WebSocket and gRPC schema/support | Fetch blocked; repository does not configure these | Candidate support remains **UNVERIFIED** here |
| [Project X transport documentation](https://xtls.github.io/en/config/transports/) | Current xHTTP behavior and Xray-core requirements | Fetch blocked; this deployment uses sing-box | xHTTP remains **REQUIRES DIFFERENT CORE / UNVERIFIED** |
| [sing-box VLESS documentation](https://sing-box.sagernet.org/configuration/outbound/vless/) | Current REALITY/Vision constraints | Fetch blocked; shipped fields are established by local code/tests | Current shipped shape is CODE-VERIFIED; current upstream claims are not |
| [Hysteria 2 protocol documentation](https://v2.hysteria.network/docs/developers/Protocol/) | Observable QUIC/TLS/obfuscation properties | Fetch blocked | Protocol family is known locally; Russian behavior is UNVERIFIED |

Before a transport PR, repeat this research with working access and record each
measurement's publication date, observation dates, ISP/carrier, method, sample
size, and limitations. Community posts may nominate an experiment but cannot
make a candidate a default.

### Candidate comparison and client classification

Scores are architectural assessments, not Russian field results.

| Candidate | Russia evidence in this pass | Failure-domain diversity | Complexity/performance/observability | Client classification | Decision |
|---|---|---|---|---|---|
| Keep VLESS+REALITY | None current; shipped and tested locally | TCP alternative to Hysteria2, but same host/IP/ASN | Lowest change; good no-censorship baseline; REALITY/Vision-specific interference remains possible | **CURRENT FIRST-PARTY CLIENT COMPATIBLE** only to the extent of the existing profile contract | Keep as control/default; no claim of Russia availability |
| VLESS + HTTPUpgrade | None | Changes wire shape, not IP/provider | Moderate server/client config; performance and fingerprint benefit unmeasured | **STANDARD SING-BOX CONFIG BUT CLIENT CHANGE REQUIRED**, pending upstream-version verification | Do not implement |
| VLESS + HTTP transport | None | Changes wire shape, not IP/provider | Moderate; HTTP-shaped is not automatically indistinguishable from ordinary browsing | **STANDARD SING-BOX CONFIG BUT CLIENT CHANGE REQUIRED**, pending upstream-version verification | Do not implement |
| VLESS gRPC | None | Changes framing, not IP/provider | HTTP/2 operational surface; distinguishability/blocking and mobile benefit unmeasured | **STANDARD SING-BOX CONFIG BUT CLIENT CHANGE REQUIRED**, pending upstream-version verification | Do not implement |
| VLESS WebSocket | None | Changes framing, not IP/provider | Widely understood but extra proxy/config overhead; no measured advantage | **STANDARD SING-BOX CONFIG BUT CLIENT CHANGE REQUIRED**, pending upstream-version verification | Do not implement |
| xHTTP | None | Changes wire behavior, not IP/provider | Adds Xray-core and a second configuration/security lifecycle | **REQUIRES DIFFERENT CORE** | Do not implement or replace sing-box |
| Second provider/ASN/IP, same current transport | Not yet run | Changes IP, subnet, ASN, provider, host and route | One additional small server; clearest one-variable A/B if transport/config are held constant | Existing profile syntax; **first-party multi-endpoint/failover compatibility UNVERIFIED** | **Selected experiment** |
| Separate subscription host | No Russia field result | Diversifies refresh/onboarding only | Modest if an independently hosted TLS origin is used; a DNS alias to same VPS is insufficient | Subscription URL behavior unchanged | Useful follow-up, not data-plane resilience |
| CDN/reverse proxy path | None, and service/contract constraints unreviewed | Can diversify ingress only when technically supported and legitimately operated | High semantic/TLS/provider-policy risk; REALITY and Hysteria2 cannot simply be assumed CDN-proxyable | Client and core changes likely | Reject without a concrete supported design |
| Active probing/measurement only | N/A | Does not add availability | Low risk; essential to distinguish IP, UDP, handshake and client failures | No client contract change | Required alongside selected experiment |

“CURRENT FIRST-PARTY CLIENT COMPATIBLE” above describes only the already shipped
profile contract; it is not a device-verification claim. This repository cannot
upgrade compatibility for any new or multi-endpoint behavior without the
separate client's acceptance evidence.

### Single highest-value experiment: second-provider active/standby A/B

**Hypothesis:** when the primary fails on a Russian access network, an otherwise
identical endpoint on a different provider, ASN and subnet succeeds often enough
to show that address/provider correlation dominates a protocol-specific failure.

**Control:** current server A and current VLESS+REALITY profile, pinned server
commit/core version, one Russian device/client version, one access network and a
bounded time window.

**Treatment:** server B in the same broad geography with a different provider,
ASN and subnet; same singbox-vpn release, transport settings and test-user UUID /
Hysteria2 password. Use a test-only identity, not a production user's secrets.
Keep the two subscription tokens independent because subscription authentication
and revocation are per deployment.

**Expected signal:** repeated A-fail/B-pass or A-pass/B-fail outcomes correlated
with the endpoint, while the selected transport, client, device, application and
network are held constant. Alternating both transports on both servers then
separates endpoint correlation from UDP- or handshake-family correlation.

**Success criterion:** on at least two materially different Russian networks
(one fixed/Wi-Fi and one mobile), three or more timestamped repetitions show B
restores tunnel egress and the required application/transfer checks when A fails,
with server logs showing an attempt and public-IP proof showing the selected
server. This justifies a small active/standby endpoint feature; it does not prove
all-Russia availability.

**Failure criterion:** both servers fail identically, or outcomes follow the
transport/client rather than the endpoint. Stop the second-endpoint feature and
use the captured distinction to nominate one subsequent transport or client
experiment.

No automatic failover should be tested first: manual selection makes the active
variable observable. If the experiment succeeds, the next PR should extend the
server-owned endpoint model/subscription representation for two explicitly
labeled sites, define independent health display, and keep manual selection as
the initial behavior. Credential rotation/revocation must update both servers as
one operator transaction or explicitly report partial failure; until that is
implemented, use a dedicated short-lived test user and remove it from both.
For no more than ten trusted users, two independent installations and a written
runbook are preferable to a dynamic control plane.

### Control-plane separation

The model already distinguishes `subscription_host` from `public_host`, so URLs
and TLS naming can be different. Deployment work still remains to put the
subscription reverse proxy/service (or a carefully designed publisher) on an
independent host, issue and renew its certificate there, expose only the intended
subscription path, transfer no server-private REALITY/TLS keys, and make token
rotation/revocation consistent. That improves onboarding and refresh during a
data-plane IP outage. It does **not** route tunnel traffic around a blocked
`public_host`, revive existing connections, or provide client failover.

### YouTube and TikTok remain unresolved

A native-app failure can originate in the VPN ingress transport, client TUN or
routes, application UDP/QUIC relay/fallback, DNS, IPv6, path MTU, exit ASN/CDN
handling, account/region policy, or application-specific endpoints. A second
provider experiment can isolate endpoint/ASN/peering correlation; it cannot by
itself identify client routing or application behavior. A server-side probe or
successful web playback is not native-app evidence. Until the separate
first-party client completes dated device tests, server experiments also cannot
exclude client defects. Both applications remain **UNVERIFIED** in the canonical
ledger.

### Exact real-device record for this experiment

Use one row per attempt; never merge Wi-Fi and cellular or infer a missing cell.
Store sensitive IPs/logs in an access-controlled evidence bundle and put only a
redacted reference in the ledger.

| UTC date/time | City/region | ISP/carrier | Wi-Fi/cellular | Device | OS | Client + version/commit | Server commit/release | VPS provider + ASN | A/B | Transport | Public IPv4 | IPv6 result | DNS result | YouTube web/native | TikTok web/native | Telegram media/calls | Idle/reconnect | Wi-Fi↔cellular | Throughput/latency | Exact failure | Evidence ref |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| UNVERIFIED |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |

Run order: A/REALITY, B/REALITY, A/Hysteria2, B/Hysteria2, then repeat the
sequence three times in alternating order. For every attempt manually select the
transport/endpoint, record the pre-connect and tunneled public IP, confirm the
matching server log timestamp, perform a controlled 10 MiB transfer, then test
both app web/native surfaces, Telegram media/calls, ten-minute idle/reconnect,
and handover. A handover cell is valid only when the selected client is observed
before and after the transition; a reconnect is not silently called seamless.

### Recommendation

Do not ship a transport in this PR. The recommended next PR is a narrowly scoped
**experimental two-endpoint representation and operator runbook**, but only after
(1) current external research access is restored, (2) a disposable second
provider/ASN is available, and (3) the first-party client team confirms how it
will represent manual site selection without silent fallback. Default production
behavior must remain the current single endpoint until the above controlled
matrix produces evidence.
