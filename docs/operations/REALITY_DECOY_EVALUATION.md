# Should the REALITY decoy move onto the node? — evaluation (2026-10-20)

**Decision: NOT changed, and not to be changed on latency alone.** Measurements
(`REALITY_DECOY_LATENCY.md`): a remote decoy adds ~81 ms to every new REALITY connection; a local decoy ~10 ms.
Latency is one input. The change is service-affecting and alters what a censor or scanner sees, so it needs the
owner's decision against the criteria below. Evidence class for each row: measured (M), reasoned from the protocol
(R), or UNVERIFIED (U).

| Criterion | Remote decoy (today: www.microsoft.com) | Local decoy (node serves its own TLS site) |
|---|---|---|
| New-connection latency | +~60-80 ms (M) | +~10 ms (M) |
| What an active prober sees | the real third-party site's certificate and behaviour; IP and SNI do not match (the IP is a Hetzner address, the site is Microsoft's) — a known REALITY consistency tradeoff (R) | the node's own site; SNI and IP match **if** the name resolves to this IP and the certificate is valid for it (R). A self-made or mismatched certificate is *worse* than today (R) |
| TLS requirements | TLS 1.3 + X25519 on the decoy (checked at install: "answered with TLS 1.3") (M) | the local site must also offer TLS 1.3/X25519 and a plausible h2 stack; a bare `openssl s_server` (used in my experiment) is not plausible traffic (R) |
| Certificate / domain | none needed | needs a domain you control and a valid certificate for the SNI used in every client profile (ACME renewal on the node, port 80/443 interplay with nginx) (R) |
| Client interoperability | any client that sends the decoy SNI works today (M: sing-box, Hiddify core) | the `sni=`/server_name in **every** client profile changes -> every user must re-import; Hiddify caches profiles (R); not tested with native clients (U) |
| Operational complexity | one hostname in `deployment.toml`; the installer probes it | a second TLS service to run, patch and monitor; failure of the local site breaks REALITY handshakes (R) |
| Failure mode | a third party's outage/changes (rate limits, TLS config, geo-blocking of the decoy) breaks handshakes; latency depends on the decoy's PoP (R) | self-contained (R) |
| Service impact of switching | restart + profile re-import for all users (R) | same (R) |
| Reversibility | edit the hostname back, restart, re-import | same, plus removing the site/cert |

**What would justify switching:** the owner wants the lower setup latency *and* is willing to operate a real,
properly certificated site for a domain they control, and accepts the all-user re-import. **What does not:** the
latency number by itself.

**Cheaper levers first (none applied):** prefer Hysteria2 where UDP/443 works (+~2 ms). ~~A closer remote decoy saves only ~8 ms~~ - **superseded 2026-10-21**: measured through REALITY, a different
remote decoy (amazon.com 9 ms, cloudflare.com 9 ms, ikea.com 11 ms vs microsoft.com 73 ms) removes almost all of the added latency without a local decoy; see the CORRECTION in `REALITY_DECOY_LATENCY.md`. A local decoy is therefore not needed for I-60.

**Before any switch**, prove on a *disposable* pair of nodes: (1) real client (Hiddify core at least) connects with
the new SNI; (2) `doctor --protocol` passes; (3) the added latency drops as measured; (4) a native iPhone
imports the changed profile and connects. Record each as evidence; nothing here is verified on a device (U).
