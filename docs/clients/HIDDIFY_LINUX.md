# Hiddify on Linux (and notes for Windows / macOS)

Not independently verified against a real device in this session — see
`docs/DEVICE_ACCEPTANCE_TESTS.md`. Hiddify's desktop app (Linux, Windows,
macOS) shares one codebase, so the flow below is the same across all
three; anything genuinely OS-specific is called out.

## Install

- Download the desktop build for your OS from
  `https://github.com/hiddify/hiddify-app/releases`
  (Linux: AppImage or `.deb`; Windows: `.exe`/`.msix`; macOS: `.dmg`).
- Linux AppImage: `chmod +x Hiddify-*.AppImage && ./Hiddify-*.AppImage`.

## Connect

1. Open Hiddify.
2. **New Profile** → **Add from Clipboard** (paste your subscription
   URL) or **Scan QR code** (uses your webcam, if available) or
   **Add from URL** and paste it there.
3. Click **Connect**.
   - **Linux**: the first connection may prompt for your password via
     `pkexec`/`sudo` — Hiddify needs elevated privileges to set up the
     TUN interface and routing, same as any system-wide VPN client.
   - **Windows**: expect a UAC prompt and, the first time, a Windows
     Defender Firewall network-permission dialog — allow it.
   - **macOS**: expect a system network-extension permission prompt
     (System Settings → General → VPN & Network, or a direct approval
     dialog) — allow it.
4. Hiddify auto-selects between REALITY and Hysteria2.

## If it doesn't connect

- Try switching manually between the REALITY and Hysteria2 entries in
  the server list.
- Check your system clock is correct — REALITY is sensitive to clock
  skew.
- Corporate/managed machines: firewall or endpoint-security software can
  block the TUN interface Hiddify creates — check with your IT policy if
  you're on a managed device.
- Ask your administrator to confirm your account is enabled or to
  rotate your token.

## Privacy

The subscription URL is a personal credential — don't share it, don't
commit it to a repository, don't paste it into a public chat.
