# IPv6 and DNS — separate behaviours, separate acceptance

Keep the A record. Do **not** add an AAAA record just because the host has a
public IPv6 address.

| Behaviour | What it is | Evidence so far (staging) |
|---|---|---|
| Client -> VPN inbound over IPv6 | A client connects to the node's IPv6 address (needs an AAAA record or an IPv6 literal) | sing-box listens on `::` for REALITY and Hysteria2; real clients connected locally over the IPv6 literal (PASS, local only). External reachability, provider firewall: UNVERIFIED |
| VPN -> Internet outbound IPv6 | The node egresses a user's flow over IPv6 | Was **broken by the egress-isolation table** (I-54) and is fixed; v6 egress and YouTube/v6 HTTP 200 verified afterward. YouTube Shorts over this IPv6 address: refused (I-38) |
| Client ISP IPv6 leak | A client sends IPv6 outside the tunnel | **Not a server property.** UNVERIFIED — needs real-device observation; never claim a DNS/IPv6 leak test passed without it |

## Preconditions before publishing an AAAA
1. Inbound IPv6 works from a network outside the node's provider (both protocols).
2. The provider's and the host's firewalls allow 443/tcp, 443/udp and 8443/tcp over IPv6 (ufw already does for v6; provider firewall unknown).
3. `vpn-admin doctor` shows no IPv6 warning (including the gateway-neighbour check, I-55) and `check_deployment_contract.py` is updated to expect `dns.aaaa_published: true`.
4. Certificates still validate for the host over IPv6; renewal (HTTP-01) works over both families.
5. A device test on IPv6-only and dual-stack networks shows no traffic or DNS outside the tunnel.
Roll back by deleting the AAAA record; the A record keeps working.

## Egress policy facts
`reject_google` (default on fresh installs) sniffs literal-IPv6 TLS/QUIC and rejects the
Google domain set so a native client can retry over IPv4; rejection is a proxy close, not
proof of native retry (INV-5, INV-13). A source IP refused over both families is not rescued by any IPv6 policy.
