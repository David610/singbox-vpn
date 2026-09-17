# Native YouTube / Hiddify: server-side UDP/443 rejection experiment

Status: **EXPERIMENTAL / NOT A FIX / NOT DEVICE-VERIFIED**.

This experiment exists because the two strongest current observations are:

1. direct DE with a Hiddify-pinned VLESS+REALITY profile still fails native YouTube; and
2. direct DE with a Hiddify-pinned Hysteria2 profile still fails native YouTube,
   while AmneziaWG on the affected device works.

Those observations make the RU relay, the VLESS detour, and XUDP specifically
insufficient explanations. They leave Hiddify's client-side TUN/runtime path as
the strongest shared suspect, but do not yet identify the exact failing layer.

## Why this experiment exists

The earlier `compat=quic-reject` client experiment could not answer whether
native YouTube recovers when application UDP/443 fails immediately: Hiddify's
builder replaces imported route rules before the runtime sees them. The test was
therefore invalid for Hiddify, not a negative result for the QUIC hypothesis.

A server-side rule is useful because Hiddify cannot delete a route rule running
on the independently controlled DE exit. It gives us a new A/B experiment:

- A: normal DE exit behavior;
- B: same client, credentials, Hiddify runtime, VPS, and transport, but the DE
  exit immediately rejects application UDP/443.

Only that one variable changes.

## Important semantic limit

Do **not** assume that a server-side `reject` becomes an ICMP Port Unreachable on
the phone.

The sing-box rule-action documentation distinguishes TUN pre-match from ordinary
proxy inbounds. For non-TUN connections, `reject` closes the connection. The DE
server receives VLESS and Hysteria2 proxy inbounds, not the phone's TUN inbound.
Whether that remote close is enough for Hiddify/sing-box on the phone to surface
a failure to the YouTube app is exactly what the device test measures.

This also means that a positive result would support "application UDP/443 / QUIC
fallback is involved", but would **not by itself prove PMTUD**. PMTUD requires
packet-size / loss / ICMP evidence of its own.

## Tool

`deploy/lib/youtube-quic-server-experiment.sh`

The tool is deliberately:

- exit-only: it refuses `role = "relay"`;
- opt-in and reversible;
- ephemeral: any normal `vpn render-config`, user mutation, repair, update, or
  reinstall may replace the generated config and thereby remove the experiment;
- validation-gated: the candidate must pass `sing-box check` before replacing
  the live config;
- rollback-safe: a failed service restart restores the exact previous config;
- secret-safe: it prints no users, UUIDs, passwords, subscription tokens, or
  private keys.

The exact inserted rule is:

```json
{
  "network": "udp",
  "port": 443,
  "action": "reject",
  "method": "default",
  "no_drop": true
}
```

It is inserted as the first route rule on the DE exit. No TCP traffic is changed.

## Operator procedure

Run on the **DE exit only**:

```bash
bash /opt/singbox-vpn/deploy/lib/youtube-quic-server-experiment.sh status
bash /opt/singbox-vpn/deploy/lib/youtube-quic-server-experiment.sh enable
```

Confirm:

```bash
jq '.route.rules[0]' /etc/vpn/compat/sing-box/config.json
systemctl is-active sing-box
```

Do not paste the full `config.json` into a ticket or chat; it contains credentials.

Then test the affected physical phone with two already-isolated profiles:

1. DE pinned REALITY;
2. DE pinned Hysteria2.

For each profile:

1. connect Hiddify;
2. force-close YouTube;
3. reopen YouTube;
4. start at least 10 different videos;
5. seek forward/backward repeatedly;
6. change quality where available;
7. test Shorts;
8. keep one video playing for at least 15 minutes;
9. disconnect/reconnect Hiddify and repeat a video start.

Record only PASS/FAIL and timing; do not expose subscription URLs.

After the experiment:

```bash
bash /opt/singbox-vpn/deploy/lib/youtube-quic-server-experiment.sh disable
```

## Interpretation

### Both REALITY and Hysteria2 start working

The result strongly supports application UDP/443 behavior as a necessary part
of the failure and justifies a second experiment that distinguishes:

- QUIC black-hole / error-propagation behavior;
- PMTU/large-datagram behavior;
- generic UDP/443 reachability.

It does **not** yet prove PMTUD. Before promoting this into a product default,
collect synchronized device/DE packet evidence and compare against AmneziaWG.

### Both still fail

A server-side QUIC rejection is insufficient. Do not ship a global UDP/443
block. The next highest-value test is raw upstream sing-box versus Hiddify using
the same DE credentials, followed by Hiddify runtime/TUN logging on the affected
device.

### REALITY and Hysteria2 differ

The common-client hypothesis is incomplete. Trace the transport that differs and
repeat with synchronized packet metadata.

## Why this is not a permanent default

Globally rejecting application UDP/443 disables HTTP/3/QUIC for every client and
can affect unrelated applications. Shipping that behavior without a real-device
positive A/B would trade an unproven diagnosis for a broad compatibility
regression.

If the experiment is positive and repeatable, promote it in a separate change as
an explicit deployment compatibility option with renderer-level unit tests,
real-sing-box validation, documentation, and real-device acceptance. Do not make
it the default solely from this experiment.
