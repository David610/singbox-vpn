# IPv6-only destinations addressed by hostname (European staging, 2026-10-26, rc.7)

Question raised by the staging run: IPv6 literals exit through the exit, but IPv6-only hostnames "fail". Verdict:
**not a tunnel, DNS or policy defect; the failing probe was the test method.** No source change.

## Evidence (client in WSL, pinned profile `fin1 · via de1`, exit fin1 with a public IPv6 address)

| Probe (through the tunnel) | Result |
|---|---|
| `curl --proxy socks5h://127.0.0.1:PORT https://ipv6.icanhazip.com` (v6-only name) | prints the exit's IPv6 address, `2a01:4f9:c014:13bd::1` |
| same with `https://ipv6.google.com` (hairpinned to the relay) | HTTP 200 |
| IPv6 literal `https://[2606:4700:4700::1111]/cdn-cgi/trace` | `ip=` is the exit's IPv6 address |
| `curl -6 ... https://ipv6.icanhazip.com` | `curl: (7) Couldn't connect to server` |
| **control** `curl -6 ... https://ipv4.icanhazip.com` (an IPv4-only site) | the same `curl: (7)` |
| `https://v6.ipv6-test.com/api/myip.php` | empty, but also empty when run directly ON the exit host (`curl -6`), i.e. that site, not the tunnel |

Why `-6` fails: it forces IPv6 for curl's own connection, which here is to the IPv4 loopback proxy
(`127.0.0.1`). It fails for an IPv4-only site too (the control). The earlier "IPv6 by hostname fails" came from
using `-6` through a SOCKS proxy.

## Mechanism checked
- The exit renders `{"action":"resolve"}` for its tunnel inbounds before the private-range rejects, so a domain
  destination is resolved by the exit's resolver and the AAAA result is dialled; rule `ipv6_literal=reject_google`
  applies to literal Google addresses only.
- DNS for the test name was seen on the exit (12 packets) and not on the relay.
- No Google-bound packet left the exit while the hairpin was active; the hairpin path is unchanged by this.
- IPv6 policy was NOT changed.

## How to test IPv6 through a proxy correctly
Do not pass `-6` to a client that talks to a local IPv4 proxy. Request the v6-only hostname normally and compare the
printed address with the exit's IPv6 address.
