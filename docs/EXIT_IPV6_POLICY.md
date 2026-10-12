# Exit literal IPv6 policy (introduced in v1.3.0)

This document describes the mechanism and its operation. It provides an
explicit rejection policy and diagnostics; it does not promise native iPhone
recovery. **Whether a given release has been accepted on a real device and a
disposable VPS is recorded only in** [DEVICE_ACCEPTANCE_TESTS.md](DEVICE_ACCEPTANCE_TESTS.md)
**and `release-acceptance/<tag>.md`**, never here: until those say PASS for a
release, full standalone YouTube remediation is **BLOCKED_ON_DEVICE_EVIDENCE**.
Applying a policy changes the running configuration of the host it is run on.

## Evidence and mechanism

The October 12 investigation reports six browser Shorts playing through the
standalone exit's literal IPv4 path and zero through literal IPv6, with the
same tunnel/videos. These are reported historical measurements. Ordinary IPv6
HTTPS worked; phone socket sampling did not identify the failed video's flow.
Older exits were refused even on IPv4 and required the owner-approved hairpin.

Pinned upstream source: sing-box **1.14.1**, tag commit
`1ac1a339cb1223e9c70eae14c44411c75033c02d`; the installed binary matches the
repository archive SHA256. Source anchors:

- `route/route.go:prepareMatchMetadata` derives IPVersion from Destination.
  `route/rule/rule_item_ipversion.go:Match` checks that metadata/literal;
  a domain's resolved DestinationAddresses do not make it an IPv6 literal.
- `actionSniff` fills Domain from TLS/QUIC; normal sniff preserves Destination.
  `actionResolve` operates only on a domain and fills DestinationAddresses.
  `route-options.override_address` is a configured static value, not a dynamic
  interpolation of recovered SNI. Deprecated sniff destination override is
  not a supported product mechanism. No safe general literal-to-A translation
  was established, so none is implemented.
- `routeConnection`/`routePacketConnection` return a reject error for proxy
  flows. The official `docs/configuration/route/rule_action.md` says the
  method applies to pre-sniff TUN rejection; non-TUN flows are closed.
  VLESS/Hysteria2 are proxy inbounds on this VPS. `no_drop` only disables
  rate-based escalation to drops; it does not create proven iPhone TUN ICMP.

Client-to-VPS family, server destination egress family, A/AAAA choices and
client ISP/DNS leaks are different observations. Removing the VPS hostname's
AAAA does not change a client-supplied IPv6 destination. DNS ipv4_only cannot
rewrite a literal. ECH, absent SNI and incomplete/fragmented sniff data can
evade scoped matching. No ECH or native-device recovery PASS is claimed.

## Configuration and defaults

```toml
[egress]
ipv6_literal = "reject_google"
```

| Setting | Plain exit | Configured hairpin exit |
|---|---|---|
| Section absent | Previous direct dual-stack behavior (`allow`) | Previous sniff -> Google literal-IPv6 reject -> hairpin route |
| `allow` | No new rules | Validation error: cannot disable hairpin protection |
| `reject_google` | Original literal IPv6, port 443, TLS/QUIC sniff + Google reject | Exact previous hairpin sniff/reject shape |
| `reject_all` | Reject every IPv6 literal, without sniff | Existing hairpin sniff, all-literal reject, Google hairpin route |

`reject_all` is explicitly diagnostic: IPv6-only literal destinations fail.
Non-Google literals retain IPv6 under allow/reject_google. Domain requests
remain distinct even if DNS returns only AAAA. A relay rejects the entire
`[egress]` section, including allow; typos/unknown fields and policy values
fail with operator errors. This is an optional additive deployment field;
schema versions, client contracts, versions and fixtures are unchanged.
Routine control-plane static revisions cannot change this privileged policy.

Representative **exact** plain-exit additions, after the four mandatory
C-16 rules and any leading reserved-probe exception/confinement:

```json
[
  {"ip_version":6,"port":443,"action":"sniff","sniffer":["tls","quic"],"timeout":"100ms"},
  {"domain_suffix":["youtube.com","youtubekids.com","youtu.be","youtube-nocookie.com","googlevideo.com","ytimg.com","ggpht.com","googleusercontent.com","youtubei.googleapis.com","google.com","googleapis.com","gstatic.com"],"ip_version":6,"port":443,"action":"reject","method":"default","no_drop":true}
]
```

The existing hairpin defaults retain unconditional sniff, the same complete
domain list and literal-family reject (without the new port restriction),
then exactly one route to its dedicated REALITY outbound. No selector, extra
egress service or direct fallback is added. C-16, probe confinement, final
direct for unrelated exit traffic, relay fail-closed rules, dual-stack VPN
listeners and privacy logging remain intact. Plain mode adds no outbound.

## Operations and rollback

```sh
vpn-admin egress show
vpn-admin egress set reject_google
vpn-admin doctor --protocol --require-protocol
vpn-admin egress clear
```

These commands are for the operator after candidate review; none was run
against production here. Set/clear reuse locked render/validate/atomic swap/
reload/verify machinery, back up deployment state, and roll desired/runtime
state back on failure. Clear returns to legacy defaults; on a hairpin it
keeps its original protection. Settings survive migration, ordinary updates,
repair and the reconcile timer through the one production renderer. Backup
contains the selected policy; restore deliberately retains the current host
deployment policy (as for host/port settings), rather than applying archived
network settings silently. Use egress set/clear for an intentional recovery. Explicit offline development mode says runtime application is NOT
verified. Changing an effective config restarts sing-box and interrupts live
connections; schedule an operator window and maintain an IPv4 rescue path.

No host IPv6 shutdown or manual rendered JSON patch is needed. A separately
approved global sysctl experiment requires saved prior live per-interface
values, addresses/routes/DNS and a tested console/IPv4 rescue path. Deleting
a sysctl file does not guarantee rollback. Do not enable a deliberately
disabled relay provisioning agent. Exposed historical credentials require
owner rotation, not reuse or copying into evidence.

Doctor checks exact non-secret policy rules/order, including missing,
duplicate, shadowing and no_drop errors. Read-only aggregate sampling:

```sh
sudo python3 /opt/singbox-vpn/deploy/lib/egress_families.py --samples 6 --listen-port 443
```

Supply both listener ports if customized. Process ownership and local
listener ports distinguish incoming from outgoing sockets; mapped IPv4
peers count as IPv4. Unknown ownership stays unattributed. Output contains
counts and monotonic duration only. Repeated socket observations are not
unique flows, Google attribution, per-user evidence, playback or leak proof.
No per-flow logging is enabled. This tool requires visibility of `ss -p`.

The browser gate accepts `--literal-family 4` or `6` with a credential-free
loopback SOCKS URL; its frontend resolves the requested family on the probe
machine and preserves TLS bytes/SNI. Only TCP is measured (Chrome QUIC is
disabled). Missing A/AAAA fails; it never substitutes the other family.
Domain mode is the existing SOCKS-domain request path. Browser dependencies
and actual YouTube remain an optional owner-run gate, not CI prerequisites.

```sh
python3 deploy/lib/youtube_shorts_probe.py --proxy socks5://127.0.0.1:1080 --count 6 --literal-family 4
python3 deploy/lib/youtube_shorts_probe.py --proxy socks5://127.0.0.1:1080 --count 6 --literal-family 6
```

An endpoint URI still needs its endpoint hostname resolved. Subscription
fetch independence is not DNS independence. Never advertise IP HTTPS with
a hostname-only certificate. Restore DNS (including cached NXDOMAIN) or use
a client-supported address override preserving verified TLS hostname; a
general Hiddify override remains a follow-up, not a shipped promise.

## Local and release gates

The unprivileged real-core lab exercises IPv6 loopback with production
renderer outputs. It retains C-16 and appends **lab-only**, post-policy
destination overrides mapping public-shaped test destinations to controlled
IPv4/IPv6 TLS/banner/UDP responders; no public destination is dialed. It tests
both REALITY and Hysteria2, original domain/A/AAAA/dual-stack vs literals,
Google/non-Google SNI, absent SNI, TLS/server-first behavior, genuine QUIC
Initials and UDP payloads, bounded TCP close, C-16 loopback denial, and an
explicit simulated IPv4 retry. TLS chains and SNI are validated; the no-SNI
negative control retains chain validation. No secret is placed in argv/logs.

```sh
python3 -m venv /tmp/egress-lab-venv
/tmp/egress-lab-venv/bin/pip install -r deploy/lib/tests/requirements-egress-lab.txt
YOUTUBE_TEST_PYTHON=/tmp/egress-lab-venv/bin/python SING_BOX_BIN=/path/to/pinned/sing-box SINGBOX_VPN_REQUIRE_REAL_INTEROP=1 \
  cargo test --locked -p compat-config --test egress_family_interop -- --nocapture
python3 deploy/lib/tests/test-egress-tools.py
```

Final local lab: 88 assertions passed, including IPv4-mapped address controls. UDP reject produced response absence,
not demonstrated client-visible ICMP; rejected IPv6 did not steer to IPv4.
An explicit simulated retry passed. Timing output records non-Google TLS,
Google TLS and server-first durations; non-443 traffic has no sniff rule.
Measured scoped server-first port-443 latency was 107–108 ms on this local
run, versus 1.5–4.5 ms outside port 443; successful non-Google TLS was 5–7 ms.
These are fixture timings, not production benchmarks. Actual Google mapping/production egress, fragmented/ECH behavior and native
client recovery remain separate acceptance work. A missing IPv6 loopback or
aioquic is a failing prerequisite when real interop is required, not a PASS.

The existing disposable lifecycle harness accepts `--verify-egress-policy`
after candidate installation/update, switches each mode, runs required doctor
and idempotent reconcile, and restores the fresh allow baseline. Its existing
injected-update rollback and TLS-renewal gates still apply. Example **owner
execution only**, on an already authorized disposable AlmaLinux host:

```sh
bash deploy/almalinux/lifecycle-acceptance.sh --host USER@DISPOSABLE_HOST \
  --domain DISPOSABLE_DOMAIN --i-understand-this-is-destructive \
  --version v1.2.0 --update-to-version CANDIDATE_RC --verify-egress-policy
```

Also run a fresh exact-candidate lifecycle with `--version CANDIDATE_RC` and
the same policy flag. Preserve the documented special updater path when
coming from v1.1.0. The release acceptance workflow
(`.github/workflows/vps-acceptance.yml`) requests this stage by default and
refuses a release run that omits it, skips the reboot or lacks a verified SSH
host key; the harness fails if a requested stage did not run. The live VPS
runs, exact-RC CI, TLS renewal and the
[native checklist](NATIVE_IPHONE_IPV6_ACCEPTANCE.md) are recorded per release
(see the top of this document). These docs authorise no tag, release dispatch,
purchase, deployment or merge.

## Downgrade and rollback safety

A deployment that declares `[egress]` must never silently lose the policy.
v1.2.0 and earlier ignore an unknown `[egress]` table without any error, so:

- `vpn-admin config validate` of a build that understands the policy prints
  `capability: exit-egress-policy`;
- `update.sh` (update, repair or downgrade) checks the **staged target's**
  `config validate` before any live change and refuses to switch a deployment
  that declares `[egress]` to a build that does not print the marker;
- to leave the policy on purpose, run `vpn-admin egress clear` first, then
  downgrade; the deployment schema version is deliberately not bumped, because
  that would break every rollback to v1.2.0 even for hosts with no policy.

Policy commands never change user credentials, subscription tokens or key
material (covered by an automated test); clients do not need to re-import after
`egress set`/`clear`.

## Release identity

`vpn-admin doctor` (L1 line "release identity") compares the installed
`vpn-admin`, `vpn` and `vpn-subscription-svc` with the SHA-256 recorded by the
installer/updater in `install-state.json` and with the recorded release tag:
published stable, release candidate, development channel, inconsistent
(manual copy or partial update) or UNKNOWN (installed before hashes were
recorded). Repair a reported inconsistency with `update.sh --repair`; do not
copy executables by hand.
