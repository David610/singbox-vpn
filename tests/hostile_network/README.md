# Hostile network simulation

Uses Linux network namespaces + `tc netem` + `iptables` to simulate a
hostile network between a client namespace and a relay namespace, per
`docs/DEPLOYMENT.md` and spec §30.

`run.sh` sets up:

```
netns "client" <--veth--> netns "relay"
```

then applies, per scenario:

- **packet loss**: `tc qdisc add dev veth-client root netem loss 30%`
- **latency/jitter**: `tc qdisc add dev veth-client root netem delay 300ms 100ms`
- **UDP blocking**: `iptables -A OUTPUT -p udp --dport 9444 -j DROP` inside
  the client namespace, to simulate a censor blocking QUIC while TCP still
  works
- **TCP blocking**: same idea with `-p tcp --dport 9443 -j DROP`, to check
  the noise-quic transport still gets through

and then runs `cargo test -p tests --test e2e` with `RELAY_TLS_ADDR`/
`RELAY_QUIC_ADDR` pointed at the relay namespace's veth address, asserting
the client can still complete a connection through whichever transport
isn't blocked.

## Why this isn't executed as part of the normal test suite

Creating network namespaces and iptables rules requires `CAP_NET_ADMIN`
(root), and this session's sandboxed environment does not grant that —
mutating network namespaces/iptables here risked leaving the *host* the
agent runs on in a broken state with no safe rollback available in this
context. `run.sh` is written and reviewed but marked `--ignored` and gated
on a root check so it fails closed (skips, not silently "passes") when the
capability isn't available, rather than being faked as passing.

Run for real with:

```bash
sudo -E bash tests/hostile_network/run.sh
```
