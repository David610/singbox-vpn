# Performance lab (disposable machines only)

A reproducible lab for measuring what the VPN data plane costs, using the
**pinned sing-box binary** and server/client documents rendered by the
**production renderer** (`vpn-admin`, `subscription`). It produces two
evidence classes only:

| Class | What it is | What it is not |
|---|---|---|
| **B. same-host protocol overhead** | throughput and CPU cost with no added delay (`netem` off) | real Internet speed |
| **C. controlled network simulation** | throughput/latency under a chosen RTT, loss and a shaped access link | proof of behaviour on a real mobile/ISP path, through a censor, or on an iPhone |

Never present a number from here as a VPS, client-network or native-device
result. Results and their interpretation for the 2026-10-14 audit are in
[`docs/reviews/2026-10-14-vpn-audit/`](../../../docs/reviews/2026-10-14-vpn-audit/).

## Safety

Run it on a throwaway VM or container host, as root. It creates network
namespaces, a TUN device, `/etc/netns/<ns>/resolv.conf` files and background
processes, and it uses fresh lab-only key material. It never contacts a VPN
server and must never be run on one: `lab-net.sh` and `lab.sh` refuse to start
on a host with `/etc/vpn/deployment.toml` or an active `sing-box.service`
(the installer copies this tree to `/opt/singbox-vpn`). `lab-net.sh down`
removes the namespaces.

## Topology

```
cli 11.0.1.2 ─┐                        ┌─ inet 11.0.3.2  iperf3, bulk HTTP, echo,
exit 11.0.2.2 ─┤  rtr (kernel forwarding ├─ relay 11.0.4.2  TLS 1.3 decoy, DNS
               │  or netem-tun on tun0)  └─ pair 11.0.5.2
```

Addresses are outside every C-16 deny range, so the exit keeps its real
production route policy (C-16 resolve + reject rules, probe confinement).

`netem-tun` replaces `tc netem` on kernels without `sch_netem`: policy routing
sends every forwarded packet through a TUN device, the program delays or drops
it by (source, destination) rule and writes it back. Queues are FIFO per rule
(jitter never reorders), loss is Bernoulli. The access link toward the client
is shaped with `htb` (default 100 Mbit/s, 300-packet `pfifo`).

## Run

```sh
# prerequisites: iproute2 iperf3 jq curl openssl go; pinned sing-box at /usr/local/bin/sing-box
sha256sum sing-box-<v>-linux-amd64.tar.gz   # must equal deploy/lib/versions.env
cargo build --locked -p admin -p subscription

L=deploy/lab/perf
$L/lab-net.sh up
$L/lab.sh build
$L/lab.sh render          # exit + paired relay, rendered by vpn-admin
$L/lab.sh standard_up     # services, servers (exit on CPU 1), clients

# B: CPU-bound, no added delay (server pinned to one CPU)
HOST=11.0.3.2 $L/tput.sh no-tunnel 5201 8 1
CLIENT_PID_NAME=cR $L/tput.sh reality 15201 8 1      # REALITY download
CLIENT_PID_NAME=cH $L/tput.sh hy2 15301 8 1          # Hysteria2 download
CLIENT_PID_NAME=cH $L/tput.sh hy2-up 15301 8 1 0     # Hysteria2 upload

# C: controlled network matrix (rtt_ms:loss_pct pairs on the client<->exit leg)
REPS=3 DUR=15 CONDS="20:0 80:0 160:0 80:1 160:1 80:3" $L/matrix.sh > results.jsonl

$L/lab.sh stop
$L/lab-net.sh down
```

Profiling (lab only): `profile.sh reality|hy2` restarts the exit and both
clients with a debug listener on 127.0.0.1:6060 inside the `exit` namespace,
profiles one download and restarts them without it.

## Scripts

| Script | Purpose |
|---|---|
| `lab-net.sh up\|down` | namespaces, veths, router, 16 MiB socket-buffer ceilings |
| `lab.sh build\|render\|standard_up\|stop …` | build tools, render configs with `vpn-admin`, start services/servers/clients |
| `netem.sh on RTT LOSS\|off` | delay/loss per leg + `htb` access link (`RATE`, `QLEN`) |
| `tput.sh`, `matrix.sh`, `summarize.sh` | single runs, the RTT × loss matrix, Markdown summary |
| `classb.sh`, `profile.sh` | CPU-bound throughput and profiles (netem off) |
| `concurrency.sh`, `loaded-latency.sh`, `brutal.sh` | many users, latency under load, fixed-rate clients |
| `restart-recovery.sh` | client recovery after an exit restart (`PRE_IDLE`, `IDLE_GAP`, `SERVER_CFG`) |
| `egress-probe.sh` | C-16 and relay fail-closed probes |
| `soak.sh` | long-running mixed load with RSS/fd/CPU sampling |
| `outage.sh SECONDS` | a full client↔exit outage: does an open connection survive, how soon does a new request work |

Restart the clients whenever the exit restarts: a running Hysteria2 client
otherwise stalls for up to 30 s against a 1.14.1 exit (see the audit), and the
next measurement is that stall, not the thing you meant to measure.

## What each probe measures

| Probe | Meaning |
|---|---|
| `tput.sh` | iperf3 through a client `direct` inbound forwarder; one proxied TCP stream per iperf3 stream; records server/client CPU % |
| `labtool connlat` | new proxied TCP connection: SOCKS CONNECT → first echoed byte (REALITY: new TCP + REALITY handshake; Hysteria2: new stream on a warm QUIC connection) |
| `labtool rtt` | 64-byte ping-pong on one established proxied connection; `rtt_loaded` runs it during a saturating download |
| `labtool udprtt -fresh` | new UDP session through SOCKS UDP ASSOCIATE per sample (REALITY: new VLESS connection per session) |

## Known limits

IPv4 only (the kernel used for the audit had no IPv6). `netem-tun` forwards
in user space and caps near ~150 Mbit/s; keep shaped links at or below
100 Mbit/s. The REALITY decoy is a
local Go TLS 1.3 server 2 ms away; a real decoy can be farther. The lab DNS
answers locally. veth backlogs drop packets at multi-Gbit/s rates, which shows
up as TCP retransmissions in class-B runs. Everything shares one machine, so
absolute numbers depend on its CPU; compare transports within one run.
