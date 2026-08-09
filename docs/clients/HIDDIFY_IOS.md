# Hiddify on iOS / iPadOS

Not independently verified against a real device in this session — see
`docs/DEVICE_ACCEPTANCE_TESTS.md`. Steps below reflect Hiddify's
publicly documented iOS onboarding flow.

## Setup

1. Install **Hiddify** from the App Store.
2. Get your subscription URL or QR code from your administrator — it
   looks like `https://sub.example.com:8443/sub/<long-random-token>`.
   Treat it like a password: anyone with it can connect as you until it
   is rotated.
3. Open Hiddify.
4. Tap **New Profile** (or the **+** button).
5. Choose **Add from Clipboard** if you have the URL copied, or
   **Scan QR code** if your administrator gave you a QR code.
6. Hiddify downloads the subscription. You will see two connection
   options inside it — **REALITY** and **Hysteria2** — nothing to type
   in by hand.
7. Tap **Connect**. iOS will show a system prompt: *"Hiddify" Would
   Like to Add VPN Configurations* — tap **Allow**, then authenticate
   (Face ID/Touch ID/passcode) if prompted.
8. You are connected. Hiddify automatically tests and selects between
   REALITY and Hysteria2 (sing-box's own `urltest` selector) — this is
   not something you need to choose manually.

## If it doesn't connect

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
your account.
