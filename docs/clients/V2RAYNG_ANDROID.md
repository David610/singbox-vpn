# v2rayNG on Android (VLESS+REALITY fallback client)

Not independently verified against a real device in this session — see
`docs/DEVICE_ACCEPTANCE_TESTS.md`. **Hiddify is the recommended client**
(`docs/clients/README.md`); v2rayNG is documented here only as a fallback
for VLESS+REALITY when Hiddify isn't usable on a given device. Recent
v2rayNG builds have gained some Hysteria2 support, but this is less
consistently available across versions than Hiddify's — treat Hysteria2
via v2rayNG as unsupported/not guaranteed unless you've confirmed your
installed version handles it.

## Install

Download from v2rayNG's official GitHub releases:
`https://github.com/2dust/v2rayNG/releases`. Google Play availability
varies by region.

## Import the VLESS+REALITY link

Your administrator can give you the raw VLESS URI instead of the full
Hiddify subscription — ask for it, or extract it yourself from
`https://sub.example.com:8443/sub/<token>?format=uri` (the subscription
service's plain-URI format, see `docs/CLIENT_COMPATIBILITY.md`). It
looks like:

```
vless://<uuid>@vpn.example.com:443?security=reality&pbk=<public-key>&sid=<short-id>&flow=xtls-rprx-vision&sni=<handshake-server>&fp=<fingerprint>#<label>
```

1. Open v2rayNG.
2. Tap the **+** button → **Import config from Clipboard** (after
   copying the `vless://...` link above).
3. Tap the imported server entry to select it, then tap the bottom-right
   connect button (play icon).
4. Grant the Android VPN permission prompt when it appears.

## If it doesn't connect

- Double-check the link was copied in full — VLESS+REALITY links are
  long and truncation breaks the `pbk`/`sid` parameters silently.
- Check your device's date/time — REALITY is sensitive to clock skew.
- Ask your administrator to confirm your account is enabled or rotate
  your credentials (`vpn-admin user rotate-vless <name>`).

## Privacy

Both the subscription URL and the raw VLESS link are personal
credentials — don't share them.
