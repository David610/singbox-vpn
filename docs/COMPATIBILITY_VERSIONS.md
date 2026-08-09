# COMPATIBILITY_VERSIONS.md

Pinned upstream versions for the Hiddify/VLESS-REALITY/Hysteria2
compatibility stack. Re-verify against primary sources before bumping any
of these; do not track `latest` in production.

| Component | Version pinned | Source | Checked |
|---|---|---|---|
| sing-box | `1.13.14` (latest stable, non-beta) | https://github.com/SagerNet/sing-box/releases | 2026-08-09 |
| Hysteria2 | bundled inbound inside sing-box (not the standalone `apernet/hysteria` binary) | https://sing-box.sagernet.org/configuration/inbound/hysteria2/ | 2026-08-09 |
| Xray-core | not used this phase (sing-box chosen instead, see ADR below) | https://github.com/XTLS/Xray-core | 2026-08-09 |
| Hiddify (Android) | any current release supporting sing-box-format subscriptions (client-side, not pinned by us) | https://github.com/hiddify/hiddify-app | 2026-08-09 |

## Why sing-box, not Xray-core, for this phase

`docs/ADR/0002-transport-portfolio.md` already flagged both as plausible
process adapters. sing-box was chosen as the initial compatibility data
plane because:

- It implements both target protocols (VLESS+REALITY and Hysteria2) in a
  single binary/process, under one config schema, one systemd unit, and
  one `check`/validate command — smaller operational surface than running
  Xray (VLESS/REALITY) and a separate Hysteria2 binary side by side.
- It is the config format Hiddify (Android) natively consumes for
  subscriptions (`sing-box` format), so the subscription renderer can
  target sing-box's own schema directly instead of translating between
  two incompatible upstream schemas.
- MIT-licensed, actively maintained, single static Go binary, official
  release artifacts published per architecture.

No concrete incompatibility with Xray-core was found that would force a
change; if one is discovered later, `CompatibilityBackend` (see
`crates/compat-config`) is the seam for swapping data planes without
touching user management or subscription logic (constraint from spec §53).

## Configuration syntax checked (sing-box 1.13.x)

- VLESS inbound: `type: "vless"`, `users[]` with `uuid` + `flow`
  (`xtls-rprx-vision`), `tls.reality` sub-object.
  https://sing-box.sagernet.org/configuration/inbound/vless/
- REALITY TLS settings: `tls.reality.enabled`, `tls.reality.handshake`
  (`server`, `server_port` — the disguise/decoy target dialed for the real
  handshake), `tls.reality.private_key`, `tls.reality.short_id[]`.
  https://sing-box.sagernet.org/configuration/shared/tls/
- Hysteria2 inbound: `type: "hysteria2"`, `users[]` with `password`,
  `tls` (required), optional `obfs` (salamander), `masquerade`.
  https://sing-box.sagernet.org/configuration/inbound/hysteria2/
- REALITY keypair generation: `sing-box generate reality-keypair` (emits
  `PrivateKey`/`PublicKey`); `sing-box generate rand --hex <n>` usable for
  short IDs.

## Share-link / subscription formats checked

- `vless://<uuid>@<host>:<port>?encryption=none&security=reality&sni=<sni>&fp=chrome&pbk=<public_key>&sid=<short_id>&type=tcp&flow=xtls-rprx-vision#<label>`
  — the de facto community standard consumed by Hiddify, v2rayN, NekoBox,
  and sing-box's own share-link export.
- `hysteria2://<password>@<host>:<port>?sni=<sni>&insecure=0#<label>`
  (optionally `&obfs=salamander&obfs-password=<obfs_password>`).
- sing-box native JSON subscription (`outbounds[]` array) — the format
  Hiddify prefers when the subscription server declares
  `content-type: application/json` or is requested with
  `?format=singbox`; avoids any share-link ambiguity because it is the
  client's own native config schema.

These are documented, not invented — see `docs/CLIENT_COMPATIBILITY.md`
for which client/format combinations were actually validated versus
assumed compatible by spec conformance.
