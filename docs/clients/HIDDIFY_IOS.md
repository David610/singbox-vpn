# Hiddify on iOS / iPadOS

Not independently verified against a real device in this session — see
`docs/DEVICE_ACCEPTANCE_TESTS.md`. Steps below reflect Hiddify's
publicly documented iOS onboarding flow plus known client-side failure
modes reported against Hiddify's iOS app.

## Four different claims — know which one you're checking

A working VPN connection on iOS actually requires four separate things
to all be true. They are not the same claim, and Hiddify's own UI can
only ever tell you about the first two:

| # | Claim | What proves it | Who/what controls it |
|---|-------|-----------------|------------------------|
| A | Profile imported | Hiddify shows REALITY/Hysteria2 entries after scanning the QR/URL | This server's subscription content |
| B | Transport "connected" in Hiddify | Hiddify's in-app UI shows a green/connected state | Hiddify's internal proxy engine |
| C | iOS VPN tunnel active | Settings → General → VPN & Device Management shows a Hiddify VPN profile; the iOS status bar/Control Center shows the VPN indicator | Hiddify's iOS "Service Mode" setting + the iOS "Allow VPN Configurations" permission |
| D | Traffic actually routed through the tunnel | Your public IP (checked with a browser or `curl`) changes to this server's IP | C, plus no split-tunnel/bypass rules |

**"Connected" in Hiddify (B) does NOT by itself prove C or D.** Hiddify
has at least two internal modes: a **VPN/TUN mode** that installs a real
iOS `NEPacketTunnelProvider` (this is what makes the status-bar VPN icon
appear and actually captures system traffic), and a **"Proxy Only"
mode** that runs a local proxy without ever touching iOS's VPN stack —
by design, that mode never shows the VPN icon and never changes your
public IP, and Hiddify can still show its own transport as "connected"
while running in it. There are also known Hiddify iOS bugs where the
"Add VPN Configurations" permission prompt is dismissed/denied and the
app's UI does not surface that failure clearly, leaving the app-level
state and the OS-level state out of sync. None of this is something a
subscription URL or its content can detect, cause, or fix — it lives
entirely inside the Hiddify app and iOS's own permission system.

## Setup

1. Install **Hiddify** from the App Store.
2. Get your subscription URL or QR code from your administrator — it
   looks like `https://vpn.example.com:8444/sub/<long-random-token>?format=hiddify`.
   Treat it like a password: anyone with it can connect as you until it
   is rotated. **This is not the same string as your User ID** (which
   looks like `user_<uuid>`) — the User ID only names your account for
   admin commands and will always 404 if pasted after `/sub/`.
3. Open Hiddify.
4. Tap **New Profile** (or the **+** button).
5. Choose **Add from Clipboard** if you have the URL copied, or
   **Scan QR code** if your administrator gave you a QR code.
6. Hiddify downloads the subscription. You will see two connection
   options inside it — **REALITY** and **Hysteria2** — nothing to type
   in by hand.
7. Tap **Connect**. iOS will show a system prompt: *"Hiddify" Would
   Like to Add VPN Configurations* — tap **Allow**, then authenticate
   (Face ID/Touch ID/passcode) if prompted. If you don't see this
   prompt at all, or you tapped "Don't Allow" by accident, iOS will
   never install a VPN profile — see troubleshooting below.
8. In Hiddify's own settings, confirm **Service Mode** is set to
   **VPN mode** (sometimes labeled TUN mode), not **Proxy Only**. This
   is the single most common cause of "connected but nothing changes" —
   see the troubleshooting section.
9. You are connected. By default the subscription starts you on
   **REALITY** (deterministic, not a race) — REALITY is the
   conservative recommended transport on restrictive networks. You can
   manually switch to **Hysteria2** or to **auto** (sing-box's own
   `urltest`, which picks whichever transport currently wins a fast
   Google connectivity check — not a Telegram-specific test) from
   Hiddify's proxy-group/server list. See
   `docs/TELEGRAM_TROUBLESHOOTING.md` for testing each transport
   independently.
10. Verify it actually worked: check your public IP (any "what is my
    IP" page, or `curl ifconfig.me` from a terminal app) before and
    after connecting. It must change to this server's public IP. Run
    `vpn-admin doctor --client` on the server for a fill-in-by-hand
    checklist covering exactly this.

### Full tunnel versus Russia-region split routing

singbox-vpn's normal acceptance target is a **full tunnel**. For that test select
**Region = Other** (or otherwise disable Russia/RU bypass rules), remove custom
split-tunnel exclusions, use VPN/TUN rather than Proxy Only, manually select
REALITY, and reconnect after every setting change. Hiddify's global routing
policy is client state; the singbox-vpn subscription cannot override it.

Record the public IP before and after connecting at both a neutral,
non-Russian IP-check endpoint and a `.ru` endpoint. If the neutral endpoint
uses the VPS exit but the `.ru` endpoint retains the Russian ISP address, that
is client-side region bypass/split routing, **not** a singbox-vpn server failure.
Never include the subscription URL, UUID, token, or key in a diagnostic report.

As a client-independent control, run the same raw profile with upstream
sing-box on the same network and transfer a multi-megabyte response through
its local SOCKS listener. If raw sing-box works while Hiddify fails,
investigate Hiddify/TUN/routing; if both fail, investigate the network-to-VPS
REALITY path. Test REALITY and Hysteria2 separately.

## "Connected but IP unchanged" — troubleshoot in this exact order

Work through these in order — each step is something checkable on the
phone itself, cheapest and most likely first. Do not start rotating
server keys or suspecting the VPS until you've exhausted steps 1-6.

1. **Verify Hiddify is in VPN/TUN mode**, not "Proxy Only", in
   Hiddify's own settings.
2. **Verify iOS actually granted the VPN permission** — if you don't
   remember seeing/accepting the "Add VPN Configurations" prompt, that's
   very likely why nothing happened.
3. **Verify a VPN configuration exists in iOS**: Settings → General →
   VPN & Device Management. If nothing is listed there, iOS never
   registered a tunnel regardless of what Hiddify's UI says. Remove any
   stale/duplicate profiles from a previous install and reconnect.
4. **Verify there's no split-tunnel/bypass rule** excluding the traffic
   or app you're testing with, and no stray "Connect on Demand" rule
   routing around what you expect.
5. **Select REALITY explicitly** in Hiddify's server list (don't rely on
   auto-selection for this test).
6. **Check your public IP** again with steps 1-5 satisfied.
7. **Test Hysteria2 separately** — disconnect, select Hysteria2
   explicitly, reconnect, recheck the public IP.
8. **Inspect server logs** only after 1-7: `journalctl -u sing-box -u
   vpn-subscription` on the VPS, and `vpn-admin doctor --protocol
   --require-protocol` to prove a real throwaway sing-box client can
   complete a REALITY handshake against this server FROM THE SERVER'S
   OWN NETWORK LOCATION.
9. **Read what a step-8 PASS and FAIL each actually prove — and what
   neither one proves.**
   - **FAIL**: server-side health is not established. Stop and fix the
     server first (`vpn-admin doctor --protocol --require-protocol`
     output tells you exactly which layer failed) — do not go further
     down this list while the server itself is unproven.
   - **PASS**: only proves this server's own REALITY listener, key
     material, and authentication path work, dialed from the VPS's own
     network. **A PASS here does NOT prove**, and must never be read as
     "confirming," any of the following, all of which remain untested:
     external reachability from the specific network the phone is on
     (especially Russian ISPs — DPI/throttling can sit entirely between
     the phone and this server, invisible to a same-host self-test),
     the iPhone's NetworkExtension/VPN-permission state, Hiddify's own
     TUN/routing behavior, or DNS/IPv6 behavior on the phone. A step-8
     PASS narrows the search — it does not by itself finish it. Keep
     going through steps 10-11 before concluding anything.
10. **Check whether the specific network the phone is on can even
    reach this server at all**, independent of Hiddify: from the same
    Wi-Fi/cellular network, try `curl -v --connect-timeout 5
    https://<this server's host>:443` (expect a TLS handshake to start;
    REALITY will present as a normal HTTPS connection to whatever
    `handshake_server` this deployment uses) or a plain
    `Test-NetConnection <host> -Port 443` on Windows / `nc -vz <host>
    443` elsewhere. If this fails from the phone's network but the
    server's own self-test (step 8) passed, the fault is network-path
    blocking (ISP/DPI/firewall) between that specific network and this
    server — not the server, and not necessarily Hiddify either.
11. **If steps 1-10 are all satisfied and it still doesn't work**:
    record your exact Hiddify app version/build before assuming a
    behavioral bug. As of this writing (2026-08), Hiddify's iOS release
    pipeline has a **currently open, confirmed** problem:
    hiddify/hiddify-app#2317 documents the App Store build reporting
    itself as "4.0.0 dev", CI/signing/configuration inconsistencies in
    the iOS release workflow, and later tagged releases not producing
    normal iOS artifacts. **This does NOT establish that #2317 causes
    the "connected but no VPN tunnel / IP unchanged" symptom** — no
    causal link between the release-pipeline problems and this specific
    VPN/TUN-routing symptom has been confirmed. What it does mean: the
    current Hiddify iOS release pipeline has documented problems, which
    makes the exact installed iOS client version/build an important
    diagnostic variable to record (check Hiddify's own About/version
    screen) when comparing notes or reporting a problem — not proof of
    cause. Check hiddify/hiddify-app#2317 for current status, and
    consider whether a different, more recently verified build is
    available, but treat this as one more diagnostic data point, not a
    root-cause explanation.
    Separately: several OLDER Hiddify iOS reports of "connected in-app
    but no OS tunnel" exist in Hiddify's issue history (e.g.
    hiddify/hiddify-app#1812, #1485, #290) — but as of 2026-08-11 these
    are all **closed** (mostly auto-closed by a stale-bot with no
    linked fix), not currently open or tracked. Treat them as
    historical evidence that this failure class has happened before in
    Hiddify's iOS app, not as proof of a presently open bug. (A fourth
    issue previously cited here, #1478, was miscited — it is a
    **Windows** issue about a "Proxy Only"/"VPN mode" status label, not
    an iOS issue, and has been removed from this list.) If your
    Hiddify app is on a current, non-dev build and none of steps 1-10
    explain the symptom, this is worth reporting fresh to Hiddify's
    issue tracker with your exact app version and iOS version, since no
    currently-open upstream report matches it precisely.

Do not treat "Hiddify's server list shows REALITY/Hysteria2 as
connected but the IP never changes" as proof of any single cause. Do
not treat a passing `vpn-admin doctor --protocol --require-protocol`
as proof the problem is client-side — it only proves the server's own
listener/key/auth path works from the server's own vantage point (see
step 9). This project has not yet reproduced the failure on a real
device end-to-end; treat any specific-cause claim beyond this list as
unverified until it is.

## Other things to check if it doesn't connect at all

- In Hiddify's server list, try manually switching between the
  REALITY and Hysteria2 entries — the two are independent, and one is
  more likely to work if the other is blocked on your network.
- Check that your device's date/time is correct (Settings → General →
  Date & Time → Set Automatically) — REALITY's handshake is sensitive to
  clock skew.
- Ask your administrator to re-check your account is enabled, or to
  rotate your subscription token if it may have been revoked.

## Privacy

The subscription URL is your personal credential. Don't share it —
anyone who fetches it gets your VLESS UUID and Hysteria2 password and
can import their own profile from them.

Every admin action that touches your credentials has a DIFFERENT blast
radius. Do not assume any one of these behaves like another:

| Admin command | Subscription URL | Already-imported REALITY | Already-imported Hysteria2 | Reversible? |
|---|---|---|---|---|
| `user rotate-token` / `user qr` | Old URL 404s immediately; fetch/refresh only | Unaffected — keeps connecting | Unaffected — keeps connecting | New URL replaces old one |
| `user rotate-vless` | Unaffected, still fetches | **Rejected on next handshake** | Unaffected | Re-import the subscription to pick up the new UUID |
| `user rotate-hysteria` | Unaffected, still fetches | Unaffected | **Rejected on next handshake** | Re-import the subscription to pick up the new password |
| `user rotate-credentials` | Unaffected, still fetches | **Rejected on next handshake** | **Rejected on next handshake** | Re-import the subscription (both changed) |
| `hysteria-obfs-rotate` (deployment-wide, all users) | Unaffected | Unaffected | **Rejected for every user** on next handshake | Every user must re-import |
| `init --rotate` (REALITY server key, deployment-wide, all users) | Unaffected | **Rejected for every user** on next handshake | Unaffected | Every user must re-import |
| `user disable` | 404s immediately | **Rejected immediately** | **Rejected immediately** | Yes — `user enable` restores everything |
| `user remove` | 404s immediately | **Rejected immediately** | **Rejected immediately** | **No** — account must be recreated from scratch |

Practical reading of this table:

- Rotating the subscription token alone (leaked-URL scenario, nothing
  else compromised) does **not** cut off a connection someone already
  established from the old URL — it only stops them fetching a fresh
  copy. If you need to actually cut off a specific device, that's
  `user disable`, or `user rotate-credentials` if you want the account
  to keep working for everyone else who re-imports.
- `hysteria-obfs-rotate` and `init --rotate` are deployment-wide: they
  affect every user's Hysteria2 or REALITY profile respectively, not
  just one account. Only run these for a real deployment-wide reason
  (suspected shared-secret/key compromise), not to punish one user.
- `user disable`/`user remove` are the only commands that stop an
  already-imported profile from connecting **immediately**, because
  they drop the user from the server's own authorization list, not
  just from the subscription/token layer.
