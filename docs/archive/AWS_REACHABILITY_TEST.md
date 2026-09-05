> **HISTORICAL DOCUMENT — NOT CURRENT PRODUCT DOCUMENTATION.**
> This is a point-in-time audit/report snapshot, preserved for engineering
> history. It may describe code, findings, or product state that has since
> changed or been superseded. For the current product boundary and status,
> see `docs/SUPPORTED_PRODUCT.md`, `docs/IMPLEMENTATION_STATUS.md`, and
> `docs/DEVICE_ACCEPTANCE_TESTS.md`.

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

### 3. REALITY: real protocol handshake + real data transfer (the test that actually matters)

**Why steps 1 and 2 are not enough on their own**: a TCP three-way
handshake and even a full TLS handshake completing does not prove the
tunnel is usable. Some DPI/filtering behavior only engages after a
connection is already open — permitting the handshake and then
throttling, resetting, or silently dropping packets once real data
starts flowing (this matches the pattern reported in
`net4people/bbs#490`/`#546`, cited in `TELEGRAM_RESILIENCE_PLAN.md`:
connections that establish fine and then get frozen/reset partway
through a transfer). A connect-only test cannot see that. This step
can.

**Prefer a desktop sing-box client for this specific test, not
Hiddify.** The point of this test is to isolate "does this NODE work
through this NETWORK," not "does Hiddify's iOS app work." If you run
this test only through Hiddify on iPhone and it fails, you cannot tell
whether the node/network failed or whether it's the same iOS TUN/
NetworkExtension uncertainty `docs/clients/HIDDIFY_IOS.md` already
documents. A desktop sing-box (or NekoBox/v2rayN-class client) on the
SAME Wi-Fi/cellular network (tethered from the test phone if a laptop
has no SIM) removes that confound. Only fall back to Hiddify iOS if no
desktop device is available on that network — and if you do, note that
explicitly in the results, since a failure there is ambiguous between
"node/network" and "Hiddify/iOS."

Procedure, per node (A and B) per network (Wi-Fi, cellular):

1. Get real REALITY connection material for the node under test —
   either its subscription URL (`vpn-admin user subscription <id>` on
   that node, or the existing subscription URL if already imported) or
   ask the operator for a throwaway test user's `vless://...` URI.
2. Import it into a desktop sing-box (`sing-box run -c config.json`
   with the rendered client config, or any GUI front-end that wraps
   sing-box — NekoBox/v2rayN/sing-box's own `SFA`/`SFM` on the platform
   available). Select the REALITY entry explicitly (not `auto`).
3. Connect, then through the tunnel: download a real file of at least
   1–5 MB — e.g. `curl -x socks5h://127.0.0.1:<local-proxy-port> -o
   /dev/null -w "%{http_code} %{size_download} bytes in %{time_total}s
   (%{speed_download} B/s)\n" https://speed.cloudflare.com/__down?bytes=5000000`
   (or any large, non-blocked HTTPS file/CDN endpoint reachable from
   Russia — pick one known to work independent of this test, e.g. a
   generic large object from a CDN already confirmed reachable).
4. Record: connect success (Y/N), download completed fully (Y/N), time
   taken, average throughput, and whether it stalled/reset partway
   through (this is the specific failure mode step 1's TCP-only test
   cannot see).
5. **Repeat 3 times**, not once, with a short pause between attempts —
   intermittent throttling can look like a one-off success or failure.

### 4. Hysteria2 (with Salamander): real protocol handshake + real data transfer

Same procedure and same rationale as step 3, but for the Hysteria2
entry:

1. Import the same subscription/URI, select the Hysteria2 entry
   explicitly in the desktop sing-box client (again, prefer desktop
   over Hiddify iOS for the same isolation reason as step 3).
2. Confirm the connection actually completes a Hysteria2+Salamander
   handshake (sing-box's own logs will show this; a client-side proxy
   test alone can silently fall back to `direct` and give a false
   positive — verify traffic is actually tunneled, e.g. by checking
   your public IP through the proxy matches the node's IP).
3. Download the same 1–5 MB test file through it, recording the same
   metrics as step 3.
4. **Repeat 3 times.**

### 5. Generic HTTPS baseline (control — confirms the network itself is up)

```sh
curl -o /dev/null -s -w "%{http_code}\n" https://www.google.com/generate_204
```
If this fails too, the network itself is down/misconfigured — the
Node A/B results from that network are not meaningful, retest later.

## Recording results

Fill in this table and keep it with the deployment notes (do not
publish the raw AWS IP in a public/shared doc if this repo is public —
substitute a placeholder when copying results elsewhere):

| Network | Node | TCP443 connect (x/5) | REALITY handshake+5MB download (x/3, avg throughput, any stalls?) | Hysteria2+Salamander handshake+5MB download (x/3, avg throughput, any stalls?) | Control HTTPS OK? |
|---|---|---|---|---|---|
| Russia Wi-Fi (ISP: ______) | A | | | | |
| Russia Wi-Fi (ISP: ______) | B | | | | |
| Russia cellular (carrier: ______) | A | | | | |
| Russia cellular (carrier: ______) | B | | | | |

## Interpreting the result

- **Connects (steps 1-2) but the download stalls/resets partway
  through (steps 3-4)**: this is a DIFFERENT, more important failure
  mode than a connect failure — it matches the pattern of DPI
  permitting the handshake and interfering only once real data flows
  (see `net4people/bbs#490`/`#546` in `TELEGRAM_RESILIENCE_PLAN.md`). A
  node/transport that only passes steps 1-2 is NOT reachable in any
  practically useful sense — treat it as failed, not passed.
- **Node B clearly succeeds (full handshake + full download, steps
  3-4) where Node A clearly fails (or vice versa)**: the second node is
  doing its job — different IP/ASN/path behaves differently under
  Russian filtering. This is real evidence to justify building the
  actual multi-node wiring (a dedicated follow-up — see
  `TELEGRAM_RESILIENCE_PLAN.md` §K; that wiring does not exist yet in
  this codebase).
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
  candidate.
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
