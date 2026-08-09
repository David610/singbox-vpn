# Hiddify on HONOR MagicOS

Not independently verified against a real MagicOS device in this
session — see `docs/DEVICE_ACCEPTANCE_TESTS.md`. MagicOS is a named
target for this project (spec requirement), so this is called out
separately from generic Android rather than assumed to behave
identically — MagicOS's background/battery management has a documented
history of killing always-on VPN apps that stock Android leaves running.

## Installation source

Google Play availability on MagicOS varies by region/device. If Play
Store doesn't have Hiddify:

1. Download the APK from Hiddify's official GitHub releases:
   `https://github.com/hiddify/hiddify-app/releases`.
2. You will need to allow "Install unknown apps" for the browser/file
   manager you use to open the APK (Settings → Apps → \<app\> → Install
   unknown apps → Allow).

## Connecting

1. Open Hiddify → **New Profile** → **Add from Clipboard** (paste your
   subscription URL) or **Scan QR code**.
2. Tap **Connect**. Grant the VPN permission prompt when MagicOS shows
   it.
3. Hiddify auto-selects between REALITY and Hysteria2.

## MagicOS-specific: keeping the connection alive in the background

This is the part that differs from stock Android and is the most common
cause of "it works, then disconnects a few minutes later" reports on
MagicOS-family devices:

1. **Settings → Battery → App launch** (or **App launch management**) →
   find **Hiddify** → switch off **Manage automatically** → manually
   enable **Auto-launch**, **Secondary launch**, and **Run in
   background**.
2. **Settings → Apps → Hiddify → Battery usage** → set to
   **No restrictions** (avoid "Managed"/"Restricted").
3. Long-press Hiddify in the recent-apps switcher and tap the lock icon
   to pin it, so the system's memory-cleaning doesn't kill it first
   under memory pressure.
4. If the connection still drops on screen-off, check **Settings →
   Battery → More battery settings** for a MagicOS-wide "deep sleep"/
   "ultra power saving" toggle and exempt Hiddify from it.

These exact menu paths vary slightly by MagicOS version and device
model — the underlying settings (background launch, battery
optimization exemption, recent-apps pin) are the ones that matter if the
wording doesn't match exactly what you see.

## Privacy

Same as every other client: the subscription URL is a personal
credential — don't share it.
