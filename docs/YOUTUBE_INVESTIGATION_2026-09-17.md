# YouTube playback investigation, 17–19 September 2026

## Status

**UPDATE (19 September 2026, later session):** the root cause is now
established and the fixing profile is shipped. The discriminator is the
**egress IP class — as a risk elevation, not an absolute block**: every
working path leaves from a non-hosting (broadband) address, and broadband
never failed across the whole record. Fresh browser sessions tunnelled
through the hosting exits (Evolus DE / Selectel RU) are *intermittent*: the
mobile Shorts path returned YouTube's "Video unavailable" through the exit
and played through the relay on the same day, so Shorts from hosting IPs is
risk-scored rather than deterministically denied — but the affected phone hit
the rejected branch every time. The fix is `compat=youtube-direct` (route the
Google/YouTube domain set to the client's `direct` outbound), which removes
the whole risk class by using the broadband path that has never failed. It is
**inert in Hiddify** (imported route rules are discarded) and only works on
config-as-is clients. See `docs/YOUTUBE_FINAL_ROOT_CAUSE.md` §15 and §15.1b.

The relay pairing defect was repaired and ordinary YouTube videos now play on
the affected iPhone through the direct and relayed profiles. YouTube Shorts
still fail with the German message `Video nicht verfügbar / Dieser Inhalt ist
nicht verfügbar / Video überspringen` whenever a VPN profile is active. The
same Shorts play with the VPN disconnected.

This session did **not** establish a complete Shorts root cause or a verified
VPN-side repair. It did narrow the failure substantially and falsified the
previous QUIC, Hiddify-only, IP-family, and sing-box-version hypotheses.

The two server addresses, SSH credentials, subscription tokens, user UUIDs,
peer credentials, signed media URLs, and client public addresses are omitted
from this record. Servers are identified only by role:

- **exit**: German exit node, native systemd sing-box;
- **relay**: Russian fail-closed first hop, forwarding its declared path to the
  exit node.

## Production repair retained

The relay initially had no peer endpoints or peer credential, so its fail-closed
policy could not provide Internet access. After backups were created, the exit
was declared as the relay's permitted peer and the existing production relay
account was paired with the existing exit account. Rendering, installed-binary
validation, REALITY handshake self-test, service health, and SSH continuity all
passed.

The relay remains fail-closed. Its policy permits VLESS TCP only to the declared
exit endpoint on port 443 and rejects other Internet destinations. Existing
users, server keys, SSH configuration, and unrelated services were preserved.

Backups created before the retained production repair:

```text
/root/youtube-investigation-20260918T054312Z/before.tar.gz
```

Both servers run sing-box 1.13.19 with native systemd services and the generated
configuration at `/etc/vpn/compat/sing-box/config.json`. The exit has one direct
outbound and no DNS or route block. The relay has the declared-exit allow rule
followed by its final reject rule.

## Real-device results

The affected device was an iPhone. The owner performed each playback test and
reported the result; the investigator did not control the device.

| Client/path | Ordinary video | Shorts | Interpretation |
| --- | --- | --- | --- |
| VPN disconnected | Pass | Pass | Device, account, and content control |
| Hiddify, pinned relay profile | Pass | Fail | Relay path carries ordinary playback |
| Hiddify, direct exit profile | Pass | Fail | Two-hop relay is not required for failure |
| Hiddify, direct TCP-only profile | Pass | Fail | Silent removal of application UDP/QUIC does not repair Shorts |
| Hiddify, Hysteria2 | Pass | Fail | VLESS, REALITY, and Vision are not required for failure |
| sing-box MT | Not accepted as a working replacement | Fail/current profile unusable | No verified repair |
| Shadowrocket, direct VLESS/REALITY | Pass | Fail | Failure is not Hiddify-specific |
| Shadowrocket through temporary sing-box 1.13.18 sidecar | Pass | Fail | 1.13.19 patch upgrade is not the regression |
| Safari with VPN active | Normal video paths work | Fail | Failure is not limited to the native YouTube app |
| Logged-in and private/incognito sessions | — | Fail | Simple Google login state is not sufficient to explain failure |

The ordinary control video was YouTube ID `aqz-KE-bpKQ`. The supplied reproducible
Short was `sAcElROnYIE`. Both its `/shorts/` URL and standard `/watch?v=` URL
failed on the phone with a VPN active and played with the VPN disconnected.

YouTube's own content-restriction diagnostic reported, while connected:

```text
DNS restrictions: disabled
HTTP header restrictions: disabled
```

That rules out YouTube Restricted Mode imposed through DNS or an HTTP header.

## Captures and traffic evidence

Bounded captures were taken on the exit and relay during owner-timed playback.
No capture reported kernel drops. The phone's carrier used multiple public
addresses during the windows, so a first single-address filter was not a clean
per-device view. Generic exit captures remained useful.

During a failing Shorts window:

- the relay-to-exit outer TCP tunnel transferred roughly 10 MiB toward the exit
  and roughly 2 MiB back during the focused window;
- the exit carried substantial bidirectional Google traffic over TCP and UDP,
  including IPv4 and IPv6 Google destinations;
- one Google-facing UDP return flow delivered roughly 9 MiB over about 100
  seconds;
- the tunnel did not die when the Shorts error appeared.

During the TCP-only test, the generic exit capture contained Google TCP traffic
and no UDP traffic. Roughly 22 MiB moved in each direction in the window. The
ordinary video worked and Shorts still failed. This makes “QUIC is required for
the failure” unsupported; it also distinguishes this real network observation
from merely inspecting a generated profile.

An earlier direct-versus-relay capture was not a valid direct comparison because
the apparent direct connection lasted only about 11 seconds while the relay
connection remained active for about 152 seconds. Later client tests, including
Shadowrocket, supplied the independent direct-path evidence instead.

Packet encryption prevents an outer tunnel capture from proving which inner
request produced the UI error. Signed playback URL queries, cookies, credentials,
and response bodies were not retained.

## Controlled experiments

| Hypothesis | Change | Actual result | Verdict |
| --- | --- | --- | --- |
| Hiddify balances across routes | Used `compat=hiddify-pinned`, with exactly one visible route | Ordinary video passed; Shorts failed | Pinned routing remains correct, but it is not the complete Shorts fix |
| Relay drops application UDP because the final VLESS outbound is TCP-only | Used the merged XUDP-preserving pinned relay profile | Ordinary video passed; Shorts failed | XUDP repair remains valid for the relay, but does not repair Shorts |
| Application QUIC black-hole prevents fallback | Used direct TCP-only profile; capture confirmed no UDP | Shorts failed | Falsified as a repair |
| Client-side reject rule fixes fallback | Existing repository analysis shows Hiddify discards imported route rules | No reliable device test can be inferred from the JSON alone | Invalid path for Hiddify |
| Server-side immediate UDP/443 rejection forces TCP fallback | Applied the exact reviewed script from `experiment/server-side-quic-reject`; validated, restarted, tested, then disabled | Shorts failed | Falsified on the affected device; branch must not be merged as a fix |
| Two-hop relay causes the failure | Routed the temporary relay user directly from the relay host | Shorts failed | Falsified |
| German exit alone causes the failure | Tested a direct temporary exit from the relay host | Shorts failed | Not specific to one exit address |
| Mixed IPv4/IPv6 identity causes the failure | Rejected IPv6 only for the temporary exit user | Shorts failed | Falsified |
| Hiddify iOS is the cause | Repeated with Shadowrocket and sing-box MT | Shorts still failed | Falsified as a Hiddify-only bug |
| VLESS/REALITY or Vision causes the failure | Repeated with Hysteria2 | Shorts failed | Falsified |
| sing-box 1.13.19 regressed | Ran checksum-verified 1.13.18 on an isolated temporary listener | Shorts failed | Falsified |
| DNS/HTTP Restricted Mode causes the failure | Used YouTube's restriction page | Both restriction mechanisms disabled | Falsified |

Every unsuccessful experiment was reverted. Both active production configs pass
`sing-box check`; `sing-box` and `vpn-subscription` are active on both nodes.

## Exact YouTube player and media probes

The supplied Short was queried directly from each server. Only non-sensitive
player status and format counts were retained.

Using a bare YouTube player request:

| Player identity | Exit result | Relay-host direct result |
| --- | --- | --- |
| WEB | `UNPLAYABLE`, zero formats | `UNPLAYABLE`, zero formats |
| MWEB | `UNPLAYABLE`, zero formats | `UNPLAYABLE`, zero formats |
| WEB embedded | Error 152, zero formats | Error 152, zero formats |
| iOS | `OK`, 128 adaptive formats | `OK`, 128 adaptive formats |
| Android | `OK`, more than 100 formats | `OK`, more than 100 formats |

The WEB/MWEB request is supporting evidence, not by itself a faithful replay of
the phone: a bare request lacks all state held by a real YouTube session. The
iOS result is decisive for the server data plane because it produced real signed
media URLs for the exact Short.

The highest-bitrate H.264/MP4 candidate selected from the iOS response was then
requested from the same source address. A 4 MiB byte range returned HTTP 206:

- exit IPv4: about 4.6 MB/s after one CDN redirect;
- exit IPv6: about 2.1 MB/s without a redirect;
- relay-host direct IPv4: about 4.3 MB/s after one CDN redirect;
- relay-host direct IPv6: unavailable because that host has no IPv6 route.

This proves that both server networks can obtain an iOS player manifest and
deliver signed H.264 media for the exact failing Short. It rules out a general
CDN block, insufficient throughput, broken signed URL delivery, and a server-wide
Googlevideo routing failure.

## Narrowest demonstrated boundary

The failure is between the phone's real YouTube/Safari session and YouTube's
playability decision when a VPN is active. It is not reproduced by a clean iOS
player request made from either server, and it is not explained by server media
delivery. TLS prevents sing-box from reading or rewriting the YouTube player
request, account cookies, visitor data, attestation tokens, or response.

A retained YouTube/Google visitor session becoming inconsistent with a changed
source address is a plausible inference, not a proven root cause. The evidence
supports testing a genuinely new YouTube session created while already connected
to the VPN: delete rather than offload the YouTube app, clear YouTube/Google
Safari website data, reboot, connect the VPN first, reinstall, and test before
sign-in. No owner result for that final session-reset test was recorded before
this report was committed.

The user's historical report that Shorts previously worked through this service
is accepted. No historical server snapshot with a materially different active
data-plane configuration was found. The preserved pre-investigation server
configuration has the same relevant exit shape as the live configuration. The
1.13.18 sidecar test also rules out the only identified binary-version change.

## Repository conclusions

The merged `compat=hiddify-pinned` work remains correct: it prevents Hiddify from
building a per-connection round-robin group and keeps relay infrastructure hidden
from selectable routes. The merged relay XUDP change also remains correct: it
removes an accidental application-level TCP restriction from the pinned relayed
VLESS outbound. Both changes fixed real configuration defects and ordinary
playback, but neither may be described as a verified Shorts repair.

The `experiment/server-side-quic-reject` branch is intentionally not merged.
Its script was safely implemented and its test coverage is useful, but the real
device result falsified the experiment as a repair for this incident.

No speculative DNS, MTU, sysctl, IPv6, firewall, hard-coded CDN address, or
binary change is merged from this investigation.

## Final cleanup

- Temporary diagnostic users were removed from both nodes.
- Temporary peer mapping for the diagnostic relay user was removed with that
  user; the permanent production relay pairing remains.
- The server-side QUIC rule was disabled and confirmed absent.
- The per-user IPv4-only and direct-relay experiments were restored from their
  backups.
- The sing-box 1.13.18 sidecar, temporary config, listener, executable, archive,
  and runtime firewall opening were removed.
- Bounded captures expired or were stopped.
- Both active configs pass validation and both production services are active.
- Local credential-bearing test state was deleted and is not committed.

Root-only packet captures and the original redacted investigation backups remain
on the servers as evidence. They may contain network addresses and therefore are
not copied into the repository.

## Rollback of the retained relay repair

The retained relay pairing is the only production change from this investigation.
Rollback must remove the paired production peer credential, restore the relay's
pre-repair deployment TOML from the dated backup, render with
`--require-applied`, validate with the installed sing-box, and confirm
`sing-box`, `vpn-subscription`, and SSH remain active. The administrative user ID
and credential are intentionally omitted from this repository report; obtain them
from the root-owned live user store when performing an authorized rollback.

Rolling back returns the relay to its original fail-closed, unpaired state and
therefore removes Internet service through that endpoint. The exit needs no
production rollback.
