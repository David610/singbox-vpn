# COMPATIBILITY_VERSIONS.md

Pinned upstream versions for the Hiddify/VLESS-REALITY/Hysteria2
compatibility stack. Re-verify against primary sources before bumping any
of these; do not track `latest` in production.

| Component | Version pinned | Source | Checked |
|---|---|---|---|
| sing-box | `1.13.19` (latest stable, non-beta) | https://github.com/SagerNet/sing-box/releases | 2026-08-17 |
| Hysteria2 | bundled inbound inside sing-box (not the standalone `apernet/hysteria` binary) | https://sing-box.sagernet.org/configuration/inbound/hysteria2/ | 2026-08-09 |
| Xray-core | not used this phase (sing-box chosen instead, see ADR below) | https://github.com/XTLS/Xray-core | 2026-08-09 |
| Hiddify (Android) | any current release supporting sing-box-format subscriptions (client-side, not pinned by us) | https://github.com/hiddify/hiddify-app | 2026-08-09 |

`1.13.14 -> 1.13.18` bump (checked 2026-08-12): audited the full commit
range (`v1.13.17` does not exist as a stable release — SagerNet went
`1.13.16 -> 1.13.18` directly). No REALITY-related commits, no
Hysteria2/vless/reality config-schema changes affecting this repo's
JSON generator fields (`private_key`/`short_id`/`handshake.*`,
`users`/`password`/`masquerade`/`obfs`/`up_mbps`/`down_mbps`/
`ignore_client_bandwidth`). Two QUIC-adjacent stability fixes landed
(`quic-go` write leak in 1.13.15; `sing-quic` "UDP sessions not closed
on connection close" in 1.13.16 — net-positive for Hysteria2
reliability), plus an unrelated AnyTLS client-metadata privacy fix in
1.13.16 (AnyTLS is not used by this deployment). No regressions were
found reported against any version in this range. See
`docs/PERFORMANCE_OPTIMIZATION_PLAN.md` for the full write-up.

`1.13.18 -> 1.13.19` patch bump (checked 2026-08-17): full 10-commit
range audited directly against the upstream repository
(`v1.13.18..v1.13.19`). None of the 10 commits touch VLESS, REALITY,
TLS/uTLS, Hysteria2, QUIC, listeners, routing/direct outbound, or the
configuration schema — every commit is either mobile/Apple/Android
build-and-release tooling (4 commits), a DHCP DNS *transport* fix (this
deployment uses no DNS transports at all — DNS resolution for the
REALITY handshake target and for clients is out of scope of this
project's generated config), a Tailscale endpoint fix (Tailscale is not
used), a Clash-API "reset network"/FakeIP-cache fix (Clash API is never
enabled in generated configs — confirmed no `clash_api`/`cachefile`
usage anywhere in this repo), an oomkiller service-stub build fix
(non-Linux build target), or a dependency bump for "default interface
monitor stuck on system boot" (go.mod/go.sum only, no source change).
One genuine security-relevant fix landed — "Fix unbounded allocations
when reading untrusted binary data" — but it is scoped entirely to
parsing untrusted `.srs` rule-set / geosite binary files
(`common/srs/*.go`, `common/geosite/reader.go`,
`experimental/libbox/profile_import.go`); this repository never
generates or loads `route.rule_set` entries (confirmed: no `rule_set`
string anywhere in `crates/` or `apps/`), so this deployment's sing-box
process never parses attacker-controlled binary rule-set data in the
first place — the fix closes a real upstream vulnerability class but
does not change this project's actual attack surface. No compatibility
risk identified; verified with the real pinned 1.13.19 binary's
`sing-box check` against every config shape this project generates (see
the interop test suite) before pinning.

## Why sing-box, not Xray-core, for this phase

An earlier design pass (native adaptive stack, removed from `main` --
see `docs/SUPPORTED_PRODUCT.md`) had already flagged both sing-box and
Xray-core as plausible process adapters. sing-box was chosen as the
initial compatibility data plane because:

- It implements both target protocols (VLESS+REALITY and Hysteria2) in a
  single binary/process, under one config schema, one systemd unit, and
  one `check`/validate command — smaller operational surface than running
  Xray (VLESS/REALITY) and a separate Hysteria2 binary side by side.
- It is the config format Hiddify (Android) natively consumes for
  subscriptions (`sing-box` format), so the subscription renderer can
  target sing-box's own schema directly instead of translating between
  two incompatible upstream schemas.
- Actively maintained, single static Go binary, official release
  artifacts published per architecture. (Its license is GPL-3.0-only —
  see "License" below; this is not a reason it was chosen, just a fact
  to be accurate about.)

No concrete incompatibility with Xray-core was found that would force a
change; if one is discovered later, `CompatibilityBackend` (see
`crates/compat-config`) is the seam for swapping data planes without
touching user management or subscription logic (constraint from spec §53).

**This is a statement about the SERVER data plane only.** This project
makes no claim, verified or otherwise, about which core a third-party
client runs. A short-lived `?format=xray` subscription variant that
suggested otherwise has been removed: it only appended an `(Xray)`
suffix to a share-link label, which is a display string in the client's
own UI and is never sent over the wire. `docs/YOUTUBE_NATIVE_APP_
INVESTIGATION.md` §9.5a records that Hiddify selects its Xray-core
engine only through an explicit `core=xray`/`xvless://` import syntax
that this project's generated links never set, so the label could not
have selected an engine. See `docs/PROVISIONING_CONTRACT.md`.

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

## License

**This repository's own code** is licensed as stated in the workspace
`Cargo.toml`/root license file — it never links, statically or
dynamically, against sing-box.

**sing-box** (`SagerNet/sing-box`, pinned `v1.13.19`) is licensed
**GPL-3.0-only** upstream — verify against the `LICENSE` file at that
exact tag in the SagerNet/sing-box repository before relying on this
statement for anything beyond this project's own documentation; this is
not a legal opinion, only a citation of what the upstream license file
says. A previous version of this document incorrectly stated "MIT" —
corrected here.

**How sing-box is obtained/used**: `deploy/almalinux/install.sh`
downloads the official pre-built release binary for the pinned version
from `https://github.com/SagerNet/sing-box/releases`, verifies it
against upstream-published checksums when available, and installs it as
an unmodified external binary invoked via subprocess (`sing-box run`,
`sing-box check`, `sing-box generate reality-keypair`) — this project
does not vendor, patch, or compile sing-box's source, and no Rust binary
in this repository links against it. `install.sh` also copies sing-box's
own `LICENSE` file alongside the installed binary
(`/usr/local/bin/sing-box.LICENSE`) so its terms travel with the binary
if this system image is itself redistributed.
