# 04 - Client compatibility matrix

Statuses: `PASS`, `FAIL`, `PARTIAL`, `BLOCKED`, `NOT_TESTED` only. A cell is `PASS` only if I
observed it myself on that client; desktop/CLI results are never promoted to a phone. Server =
the candidate deployment at 91.244.71.165 unless stated. "Evidence" points into `test-results/`.

## Tested clients (Windows 11, this PC)

| Client / version | Protocol | Body / format | Import | Core start | Connect | DNS | IPv4 | IPv6 | Streaming (YouTube) | Reconnect | Evidence |
|---|---|---|---|---|---|---|---|---|---|---|---|
| sing-box 1.14.1 | VLESS+REALITY | native `?format=singbox` outbounds | PASS (`check`) | PASS | PASS | PASS | PASS | PASS (exit policy dependent) | FAIL (0/6, exit IP refused) | PASS (1.8-3.4 s after restart) | `protocol-native-v1.3.0.json`, `protocol-after-reboot.json`, `youtube-matrix-*.json` |
| sing-box 1.14.1 | Hysteria2 | native | PASS (`check`) | PASS | BLOCKED (UDP/443 filtered from this PC) | BLOCKED | BLOCKED | BLOCKED | BLOCKED | BLOCKED | `udp-path-probe.txt`, `tcpdump-udp443-hy2-attempt.txt` |
| sing-box 1.14.1 *on the VPS loopback* | Hysteria2 | native | PASS | PASS | PASS (204) | PASS | PASS | NOT_TESTED | NOT_TESTED | PARTIAL (0.3 s on loopback, not representative) | `hy2-caps-v1.3.0.txt` |
| Hiddify core `dbf3ffb` (CLI) | REALITY (+Hysteria2 node present) | plain share links | PASS | PASS | PASS | PASS | PASS | NOT_TESTED | NOT_TESTED | NOT_TESTED | `hiddify-core-parse-matrix.json`, `hiddify-core-run-matrix.json` |
| Hiddify core `dbf3ffb` (CLI) | REALITY | Base64 body | PASS (parse) | NOT_TESTED | NOT_TESTED | - | - | - | - | - | same |
| Hiddify core `dbf3ffb` (CLI) | REALITY | bare `/sub` partial JSON | PASS | PASS | PASS (one TLS error on one of three requests) | PASS | PASS | NOT_TESTED | NOT_TESTED | NOT_TESTED | same |
| Hiddify core `dbf3ffb` (CLI) | REALITY | pinned JSON, single link | PASS | PASS | PASS | PASS | PASS | NOT_TESTED | NOT_TESTED | NOT_TESTED | same |
| Hiddify core `v4.0.0` (CLI) | both | all server bodies | PASS (parse + CheckConfig) | NOT_TESTED | NOT_TESTED | - | - | - | - | - | `hiddify-core-v4.0.0-parse-matrix.json` |
| Chrome 'iPhone 13' emulation | via REALITY | - | - | - | PASS | PASS | PASS | NOT_TESTED | FAIL (0/6; direct control 6/6) | - | `youtube-matrix-*.json` (browser evidence, not native) |

## Real target clients (not available to me)

| Client | Platform | Protocol / format | Import | Core start | Connect | DNS | IPv4 | IPv6 | Streaming | Reconnect | Note |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Hiddify 4.0.0 dev | iOS | any | NOT_TESTED | **FAIL (user reported)** | NOT_TESTED | NOT_TESTED | NOT_TESTED | NOT_TESTED | NOT_TESTED | NOT_TESTED | Issues A and B; see `03` |
| Hiddify 4.1.1 (latest stable) | iOS | any | NOT_TESTED | NOT_TESTED | NOT_TESTED | NOT_TESTED | NOT_TESTED | NOT_TESTED | NOT_TESTED | NOT_TESTED | |
| Shadowrocket | iOS | Base64 `?format=uri&encoding=base64` | NOT_TESTED | NOT_TESTED | NOT_TESTED | NOT_TESTED | NOT_TESTED | NOT_TESTED | NOT_TESTED | NOT_TESTED | plain body documented as rejected |
| Shadowrocket | iOS | plain `?format=hiddify` | **FAIL (documented by the repo as confirmed)** | - | - | - | - | - | - | - | not re-observed by me |
| INCY 2.4.5 | iOS | VLESS-REALITY-only (vpn-web `incy`) | NOT_TESTED; third-party report says empty provider | - | - | - | - | - | - | - | unverified |
| INCY | iOS | Hysteria2 | NOT_TESTED | - | - | - | - | - | - | - | report says it imports |
| Hiddify | Windows (app) | any | NOT_TESTED | NOT_TESTED | NOT_TESTED | NOT_TESTED | NOT_TESTED | NOT_TESTED | NOT_TESTED | NOT_TESTED | only the CLI core was run |
| Hiddify | Android | any | NOT_TESTED | | | | | | | | |
| v2rayNG | Android | any | NOT_TESTED | | | | | | | | |
| Tamara Next | iOS/Android/Windows | provisioning contract | NOT_TESTED | | | | | | | | not qualified; PR #47 open |

## Format-by-client expectations (what each is *documented* to accept)

| Client | Accepts | Source |
|---|---|---|
| Hiddify | share-link list (plain or Base64), sing-box JSON | parsed by its own core here |
| Shadowrocket | Base64 share-link list; plain rejected | this repo's service docs ("confirmed") |
| INCY | Base64 or plain lists; JSON = array of **Xray** configs; `vless://`, `hysteria2://`, `hy2://`; `ssr/tuic/hysteria` skipped | docs.incy.cc/en/subscription-format |
| generic sing-box | `?format=singbox` | sing-box 1.14.1 `check` + run |

## What would turn `NOT_TESTED` into evidence

See the short iPhone procedure in `08_RELEASE_ACCEPTANCE.md`.
