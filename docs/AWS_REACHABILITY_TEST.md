# Node B (AWS) reachability test — run this BEFORE deploying Node B to any real user

## Why this exists

An AWS VPS for a potential "Node B" already exists. Fresh research
(2026-08) found no reliable current evidence one way or the other about
whether AWS IP ranges are more or less reachable from Russia than
Node A's current provider (Evolushost) — see
`docs/TELEGRAM_RESILIENCE_PLAN.md`'s 2026-08 addenda. Generic "AWS is/
isn't blocked" claims are not a substitute for testing the *specific*
IP this deployment would actually use. **Do not import Node B into
anyone's subscription until this test has actually been run from
inside Russia and the result is recorded here or in an equivalent
note.**

This is intentionally the smallest test that produces a real answer,
runnable by someone who is not a network engineer, on both a computer
and a phone.

## What you need

- The public IPv4 address of Node A (already known:
  `157.173.27.46` / `vpn.xn--80aa9argf5d.com`).
- The public IPv4 address of the candidate AWS Node B (fill in below —
  this document does not assume a specific value since none was
  provided to this repository).
- Someone physically inside Russia, on two different networks:
  1. A residential/Wi-Fi ISP connection.
  2. A mobile/cellular data connection (a different ISP than #1 if at
     all possible — mobile and fixed-line filtering can differ).
- No VPN active during the test — you are testing raw reachability to
  the VPS IPs themselves, not through any tunnel.

## Test matrix (run ALL cells, not just until one works)

For **each** of the two IPs (Node A, Node B) x **each** of the two
networks (Wi-Fi, cellular), run:

### 1. TCP 443 reachability (tests the REALITY port)

macOS/Linux/Android (Termux) or any shell:
```sh
for i in 1 2 3 4 5; do
  curl -v --connect-timeout 5 -o /dev/null -s -w "attempt $i: %{http_code} in %{time_connect}s\n" \
    "https://<IP_OR_HOST>:443/" 2>&1 | grep -E "attempt|SSL|TLS|Connected|refused|timed out"
  sleep 2
done
```
Windows PowerShell:
```powershell
1..5 | ForEach-Object {
  Test-NetConnection -ComputerName <IP_OR_HOST> -Port 443
  Start-Sleep -Seconds 2
}
```
iPhone (no terminal needed): install a simple "port check"/"network
tools" app from the App Store, or ask the person to run the
Windows/Mac/Android command on another device on the SAME network
(same ISP/same cell tower is what matters, not the same physical
device).

**Run this 5 times, not once** — intermittent throttling/probing (per
the net4people/bbs reports in `TELEGRAM_RESILIENCE_PLAN.md`) can look
like a transient success or failure on a single attempt. Record how
many of the 5 succeeded, and how long each took.

### 2. UDP 443 reachability (tests the Hysteria2 port)

**Do not trust a bare `nc -u` "success."** UDP is connectionless —
`nc -u <host> 443` sending a packet with nothing listening on a normal
open port typically reports as "succeeded" from the client's point of
view regardless of whether the packet was silently dropped somewhere
in the network, because there is no handshake to fail and no response
is expected either way. It can only usefully catch one specific
signal: an explicit ICMP "port unreachable" causing an IMMEDIATE
`nc` failure. A hang or a "sent" report proves nothing on its own.

**The real, meaningful test is a live Hysteria2 handshake**, which the
protocol actually acknowledges or rejects: connect a real Hysteria2
client (Hiddify, selecting the Hysteria2 entry for the node under
test) and record connect success/failure and how long it takes, 3
attempts per node per network. This is what an actual user experiences
— use it as the primary UDP signal, not `nc`.

If you want a lightweight secondary signal before importing a full
profile, `nc -u -v -w3 <IP_OR_HOST> 443 </dev/null` can still be run —
but read a **hang with no local error at all** as "inconclusive, not
proof of reachability," and only treat an immediate, explicit
"connection refused"/"port unreachable" as a real (negative) signal.

### 3. Generic HTTPS baseline (control — confirms the network itself is up)

```sh
curl -o /dev/null -s -w "%{http_code}\n" https://www.google.com/generate_204
```
If this fails too, the network itself is down/misconfigured — the
Node A/B results from that network are not meaningful, retest later.

## Recording results

Fill in this table and keep it with the deployment notes (do not
publish the raw AWS IP in a public/shared doc if this repo is public —
substitute a placeholder when copying results elsewhere):

| Network | Node A TCP443 (x/5) | Node A UDP443/Hy2 (x/3) | Node B TCP443 (x/5) | Node B UDP443/Hy2 (x/3) | Control HTTPS OK? |
|---|---|---|---|---|---|
| Russia Wi-Fi (ISP: ______) | | | | | |
| Russia cellular (carrier: ______) | | | | | |

## Interpreting the result

- **Node B clearly succeeds where Node A clearly fails (or vice
  versa)**: the second node is doing its job — different IP/ASN/path
  behaves differently under Russian filtering. Deploy it as a genuine
  fallback option in the merged subscription.
- **Both succeed on both networks**: good news, but it means this
  specific test didn't distinguish resilience value — Node B is still
  reasonable to add as redundancy against a future single-node block,
  just not proven to help against a CURRENT one.
- **Both fail together, or fail inconsistently in the same pattern**:
  the AWS IP is not adding real resilience for this specific
  deployment/network combination. Do not deploy it to users on that
  basis — per `TELEGRAM_RESILIENCE_PLAN.md`, a different, independent,
  non-hyperscaler provider (e.g. a small European VPS, different ASN
  from both Evolushost and AWS) is at least as defensible a next
  candidate, and the merged-subscription architecture does not care
  which provider actually fills the "Node B" slot.
- **Node B fails while Node A passes**: do not deploy Node B; keep
  Node A as the sole node until a working second provider is found.

## What this test does NOT tell you

- It does not test Telegram specifically — see
  `docs/TELEGRAM_TROUBLESHOOTING.md` and the Telegram function matrix
  in the main investigation report for that.
- It does not test iOS/Hiddify client behavior — see
  `docs/clients/HIDDIFY_IOS.md`.
- A single day's result is not permanent — Russian filtering behavior
  changes; re-run this test if Node B (or Node A) starts being
  reported as unreliable by users, rather than assuming today's result
  still holds months later.
