# TELEGRAM_TROUBLESHOOTING.md

Telegram has been reported unreliable under Russian censorship for vpn1
users, while YouTube and Instagram work well through the same VPN. This
document is the client-side troubleshooting procedure for that specific
gap. See `docs/TELEGRAM_RESILIENCE_PLAN.md` for the full investigation,
what was actually changed in this repository, and what is still an open
hypothesis.

**This document does not claim Telegram is "fixed."** It gives a
structured way to narrow down *why* a given connection attempt failed,
so a real report from a real Russian network can say something more
useful than "Telegram doesn't work."

`sudo vpn doctor --telegram` on the server runs the server-side half of
this checklist and prints the same disclaimer found at the bottom of
this document — it proves the server is healthy, never that Telegram
itself works from Russia.

## Before you start: what "Telegram works" actually means

Telegram is not one test. A VPN connection can pass some of these and
fail others — always note *which one* failed, not just "Telegram is
broken":

- App startup / connects at all
- Text messages send and receive
- Image download
- Video/media download
- Media upload
- Channels (especially large/high-traffic ones)
- Notifications and reconnect-from-background
- Voice calls
- Video calls

Record the transport (Reality / Hysteria2 / Auto) and which of the above
failed. `docs/DEVICE_ACCEPTANCE_TESTS.md` has the full matrix and the
exact template to fill in.

## Step 1 — Disable Telegram's own internal proxy

Telegram has its own SOCKS5/MTProto proxy setting, completely
independent of vpn1/Hiddify. If it's enabled (even pointing at a
now-dead proxy), Telegram can fail while every other app works fine
through the VPN — and this looks identical to a VPN problem from the
outside.

```
Telegram -> Settings -> Data and Storage -> Proxy -> disable it
```

Do this **before** any of the steps below, unless you are intentionally
testing Telegram's own proxy as a separate variable. Re-check it after
any Telegram app update — some builds have re-enabled a previously
configured proxy.

## Step 2 — Test each transport separately

The vpn1 subscription now starts every fresh profile on **Reality**
deterministically (not on whichever transport won a fast Google
connectivity test) — see `docs/TELEGRAM_RESILIENCE_PLAN.md` §A. Test
each of the three options in Hiddify's server list on its own, not just
whatever loads first:

1. **Reality** — the conservative default. Select it explicitly.
2. **Hysteria2** — switch to it explicitly, confirm it actually connects
   (not just "selected" — send a message, load a page).
3. **Auto** (`urltest`) — sing-box's own automatic switcher. It only
   proves a fast HTTPS request to a Google endpoint succeeded; it says
   nothing about Telegram specifically, so a working "Auto" is not
   proof Telegram will work on whatever it happened to pick.

Record pass/fail for **each** transport independently, and for each of
the Telegram functions in the list above.

## Step 3 — Functionality checklist

For each transport, actually exercise the list in "what Telegram works
means" above — not just "it connects." A connection that lets you read
old messages but times out sending photos is a different failure than
one that never connects at all, and points to a different cause
(possibly a UDP/QUIC path issue for Hysteria2, possibly a large-packet/
PMTU issue for either transport — see Step 7).

## Step 4 — Android

- **Per-app VPN exclusion / split tunneling**: confirm Telegram is not
  excluded from the VPN app's tunnel. Some Android skins add "app
  bypass" lists separate from the VPN client's own settings.
- **Full-device mode**: confirm Hiddify is set to route the whole
  device, not a partial app list.
- **Always-on VPN** / **Block connections without VPN** (Settings ->
  Network & internet -> VPN -> [gear icon next to Hiddify]): these are
  useful *diagnostic* tools, not something to leave on permanently
  unless you want it. Turning on "Block connections without VPN"
  forces every app (including Telegram) through the tunnel or nothing —
  if Telegram suddenly can't connect at all with this on, but worked
  with it off, that is evidence Telegram was bypassing the VPN before.
  Exact menu labels vary by Android version and vendor (this applies to
  HONOR MagicOS as much as stock Android) — look for wording like
  "Always-on VPN" / "Block connections without VPN" near the VPN app's
  settings, not assumed to be in an identical place on every device.
- **Battery/background restrictions**: aggressive battery optimization
  (common on HONOR/Huawei/Xiaomi) can kill the VPN's background process
  or Telegram's own background service, which looks like "notifications
  don't arrive" or "have to reopen the app for messages to load." Check
  the OS's app battery settings for both Hiddify and Telegram.

## Step 5 — iOS

- Confirm the VPN toggle (Settings -> VPN) actually stays on — some
  iOS configurations silently drop the VPN under certain conditions
  (Low Power Mode, some carrier profiles).
- Check for a **conflicting VPN/proxy profile** — a second VPN
  configuration, a carrier-installed profile, or Wi-Fi-specific proxy
  settings (Settings -> Wi-Fi -> [network] -> Configure Proxy) can
  fight with Hiddify's tunnel.
- Confirm Telegram's own proxy is disabled (Step 1) — this applies on
  iOS exactly as on Android/desktop.
- Compare **Wi-Fi vs cellular** explicitly — note which one fails,
  since they can go through different upstream ISP paths with
  different DPI behavior.

## Step 6 — Detecting an IPv6 leak

If Telegram fails specifically on a network where IPv6 is present, an
IPv6 leak (Telegram traffic going out directly over the device's native
IPv6 route instead of through the VPN tunnel) is a plausible cause vpn1
cannot fully rule out from the server side alone — see
`docs/TELEGRAM_RESILIENCE_PLAN.md` §E for the server's own explicit
IPv6 policy and what `vpn doctor` can and cannot verify about it.

To check from a client:

1. With the VPN connected, visit an IP-echo site that reports both
   protocols (e.g. a site that shows "your IPv4" and "your IPv6"
   separately) from the device's browser.
2. If an IPv6 address is shown that is **not** the VPS's own IPv6 (or
   an IPv6 address is shown at all when the VPS has no AAAA record),
   traffic is bypassing the tunnel over IPv6 — a leak.
3. If no IPv6 address is shown (or it errors out), IPv6 is not being
   used for that connection, which is the safe state on a deployment
   with no AAAA record.
4. As a blunt diagnostic, temporarily disabling IPv6 entirely on the
   device (Wi-Fi network settings, or mobile-data APN settings where
   supported) and re-testing Telegram isolates whether IPv6 was part of
   the problem — revert this afterward, don't leave IPv6 off
   permanently without a reason.

## Step 7 — Controlled MTU experiments

MTU/PMTU mismatches are a plausible cause of "connects but large
transfers fail" symptoms (media upload/download, voice/video calls),
especially over Hysteria2 (QUIC/UDP, more PMTU-sensitive than
Reality's plain TCP). vpn1 does **not** apply a global MTU override —
there is no single correct value across every ISP/mobile-network path,
and a wrong global value would silently degrade every user, every
transport, all the time, in exchange for possibly fixing one path for
one user.

If Hiddify/sing-box on your client exposes an MTU override for the
relevant outbound (check current Hiddify version — not guaranteed on
every platform/version), you can test controlled values as a diagnostic
only:

- **1400**: a common safe-ish reduction from the Ethernet default
  (1500) that absorbs typical tunnel-overhead cases.
- **1360**: a common value used by other VPN protocols under known
  additional-overhead conditions (e.g. mobile carrier PPPoE-style
  encapsulation).
- **1280**: the IPv6 minimum MTU — a conservative floor that should
  work on almost any path but caps throughput.

Test one value at a time, re-run the Step 3 functionality checklist,
and **revert to the default when done testing** unless a specific value
is proven to fix a specific symptom on your specific network — this is
a diagnostic tool, not a config nobody remembers rolling back six
months from now.

Why the server doesn't just clamp MSS/MTU for everyone: server-side MSS
clamping only affects TCP (Reality), not Hysteria2/QUIC (a UDP,
non-TCP-header protocol) — so it cannot be "the fix" for both
transports even where it helps one, and a value tuned for one user's
path can hurt a different user's path with different upstream overhead.
Reality (TCP) and Hysteria2 (QUIC) fundamentally differ here: TCP PMTU
discovery/blackhole detection is a well-understood, decades-old problem
space with existing OS-level mitigations (MSS clamping, PMTUD); QUIC
implements its own path-MTU discovery inside the encrypted stream, which
server-side network middleboxes (including vpn1's own sing-box) cannot
inspect or clamp at all — the only real lever is the client-side MTU
setting described above, if the client exposes one.

## Step 8 — Collect evidence before asking for help

When reporting a failure, include:

- Client app + version (e.g. "Hiddify 2.x.x")
- OS + version (e.g. "Android 14, HONOR MagicOS 8")
- Hiddify version and sing-box core version (Hiddify -> Settings ->
  About, or equivalent)
- ISP / mobile carrier
- Wi-Fi or mobile data
- Transport used (Reality / Hysteria2 / Auto)
- Exact Telegram function that failed (from the list in the intro)
- Timestamp (with timezone) of the failed attempt
- Sanitized logs only — see below

**Never include:**

- Your REALITY private key or public key
- Your VLESS UUID
- Your Hysteria2 password or the shared Salamander obfuscation password
- Your subscription URL or token
- Any full sing-box `config.json`/subscription JSON without first
  stripping the above

`sudo vpn doctor --report` on the server produces a sanitized diagnostic
bundle (secrets redacted, see `docs/TELEGRAM_RESILIENCE_PLAN.md` §H) —
prefer sharing that over a raw log paste from the server side.

## Server-side diagnostics: what they do and do not prove

Running `sudo vpn doctor --telegram` on the VPS checks: general outbound
connectivity, DNS resolution, IPv4/IPv6 reachability, the currently
active sing-box configuration, Reality/Hysteria2 listener and
obfuscation state, and current public hostname resolution.

```
Server-side diagnostics passed.
This does NOT verify:
- Russian DPI compatibility
- Hiddify TUN routing
- Telegram app proxy settings
- Russian mobile ISP behavior
Run the client acceptance checklist next.
```

A green `vpn doctor --telegram` on the server, combined with a failing
Telegram on a real Russian device, is not a contradiction — it means the
failure is specific to something between the client and the network,
which is exactly what Steps 1-7 above are for.
