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
   looks like `https://vpn.example.com:8444/sub/<long-random-token>`.
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
   vpn-subscription` on the VPS, and `vpn-admin doctor --protocol` to
   prove a real client can complete a REALITY handshake against this
   server.
9. **Only then** investigate server/network-side causes (firewall,
   DPI/blocking on your specific network, DNS). `vpn-admin doctor`
   already proves the server's listeners, keys, and subscription
   coherence are healthy — a real device failure past step 8 usually
   means something about the specific network path, not this
   deployment's configuration.

If Hiddify's server list shows REALITY/Hysteria2 as connected but your
IP never changes, the fault has consistently turned out to be step 1 or
2 above (Proxy Only mode, or a missed/denied VPN permission prompt) —
those are Hiddify/iOS-side settings this server's subscription cannot
see or control.

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
anyone who has it can connect as you until an administrator rotates
(`vpn-admin user rotate-token`) or disables (`vpn-admin user disable`)
your account. Rotating the Hysteria2 Salamander obfuscation password
(`vpn-admin hysteria-obfs-rotate`) is a separate, deployment-wide
action — it does not rotate your personal subscription token, but it
does mean your already-imported Hysteria2 profile needs to be
re-imported (the obfuscation password changed) even though your
subscription URL and REALITY connection are unaffected.
