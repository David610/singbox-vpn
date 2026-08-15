# DEPLOYMENT.md

## Local development (this session's target)

The native adaptive stack is outside the supported v1.0 product. Its current
loopback-only development entry point builds the required processes, generates
ephemeral development key/certificate material in memory, writes the generated
relay pool, and starts the slice:

```bash
./deploy/local/run-dev-slice.sh
curl --socks5-hostname 127.0.0.1:1080 http://127.0.0.1:8081/
```

The script prints its temporary log directory and process IDs. It is not a
production deployment path and does not create a durable trust root.

## Split ingress/egress

The two-hop topology is exercised by
`tests/tests/e2e.rs`; there are no checked-in manual ingress/egress TOML
examples. The ingress relay forwards the framed stream to the egress relay over
a second `direct-tls` hop rather than dialing the destination itself.

## Hostile network simulation

`tests/hostile_network/` uses Linux network namespaces + `tc netem` +
`iptables` to simulate packet loss, latency, and UDP blocking between the
client namespace and the relay namespace. Requires `CAP_NET_ADMIN` (root or
`sudo -E`) and is skipped automatically (not failed) when that capability
is absent — see the test module's `require_root!()` guard. Run with:

```bash
sudo -E cargo test -p tests --test hostile_network_scenario -- --ignored
```

## Production key management (ADR-0008)

`apps/keytool` (`vpn-keytool`) is the offline signing-ceremony CLI. It
never opens a socket — every subcommand only reads/writes local files —
and is meant to run once per key tier on progressively less-trusted
machines:

```bash
# 1. On the most isolated machine you have (ideally air-gapped), once.
#    root.key never leaves this machine again.
vpn-keytool root-init --out-dir /secure/root

# 2. Also on that machine (release-issue needs the root key). Copy only
#    release.key + release.cert.json off to the release/build host.
vpn-keytool release-issue --root-key /secure/root/root.key \
  --out-dir /secure/release

# 3. On the release/build host — never needs the offline root. Copy only
#    bundle.key + bundle.cert.json + release.cert.json to the always-online
#    rendezvous host.
vpn-keytool bundle-issue --release-key /secure/release/release.key \
  --release-cert /secure/release/release.cert.json \
  --out-dir /secure/bundle

# 4. Point rendezvous at the persisted hierarchy instead of generating an
#    ephemeral one every boot:
cargo run -p rendezvous -- \
  --key-dir /secure/bundle \
  --release-cert-file /secure/release/release.cert.json \
  --pool-file deploy/local/relay-pool.json
```

`root.pub` (from step 1) is the value that gets pinned into client builds
out of band (baked into `client.toml` / the CLI trust-root argument at
build/packaging time) — it is the only thing a client needs to verify the
entire chain, and it is never read by any network-connected process.

Key files (`root.key`, `release.key`, `bundle.key`) are written
hex-encoded with mode 0600 and `KeyPair::load_from_file` refuses to load
one whose permissions have been loosened. Certificate files
(`*.cert.json`) are public data (a signature over a public key) and are
safe to copy anywhere.

**Rotation**: re-run `bundle-issue` against the same `release.key` to mint
a new bundle key (rendezvous's `--key-dir` then points at the new
directory); if the old key must be actively distrusted (not just retired),
issue a revocation list naming its public key:

```bash
vpn-keytool revoke-issue --release-key /secure/release/release.key \
  --release-cert /secure/release/release.cert.json \
  --revoke <old-bundle-public-key-hex> \
  --out /secure/revocation.json
```

Rotating the *release* key follows the same pattern one tier up, using
`root.key`; rotating the root key itself means re-pinning `root.pub` into
every client build (an intentionally expensive, rare operation).

Point rendezvous at a revocation list with `--revocation-list-file`; it
serves the signed bytes verbatim at `GET /v1/revocation-list` (no
release/root key needed on the rendezvous host to serve it — clients
verify the signature themselves via
`config::revocation::SignedRevocationList::verify`).

`services/relay-agent --identity-dir <dir>` persists the relay's TLS/QUIC
identity (`relay.cert.der` + `relay.key.der`, the latter mode 0600) across
restarts so the `cert_sha256_hex` pin already handed out in signed relay
bundles doesn't go stale on every reboot. Without `--identity-dir` a fresh
identity is generated every boot (fine for local dev only).

The full chain — real persisted hierarchy sign, fresh-client verify,
rotate, and confirm the rotated-out key's old signatures are rejected — is
exercised end to end in `apps/keytool/tests/ceremony.rs` against the real
`vpn-keytool` binary and real files on disk (not mocked).

## Containerization: what's containerized, what isn't, and why

Two deployment paths exist for this repository's services, and they're
meant to run side by side, not as alternatives to each other:

1. **systemd, on the host** — the Hiddify-compatible data plane:
   `sing-box` (`deploy/almalinux/systemd/sing-box.service`) and, if not
   using the Docker path below, `services/subscription`
   (`vpn-subscription.service`). Driven by `deploy/almalinux/install.sh`.
2. **Docker, via `deploy/docker/`** — the four pure-Rust network services
   (`subscription`, `rendezvous`, `relay-agent`, `test-service`). See
   `deploy/docker/Dockerfile` and `deploy/docker/docker-compose.yml`.

### Why sing-box (and WireGuard/Outline, if you're expecting them) stay on systemd

**sing-box is not containerized, deliberately:**

- It binds `443/tcp` **and** `443/udp` simultaneously and needs
  `CAP_NET_BIND_SERVICE` to do it as a non-root user — straightforward in
  either systemd or Docker, not a deciding factor by itself.
- Its systemd unit (`deploy/almalinux/systemd/sing-box.service`) is
  already hardened to a level a fresh container wouldn't get for free
  (`ProtectSystem=strict`, a minimal `CapabilityBoundingSet`, `PrivateTmp`,
  `RestrictNamespaces`, `LockPersonality`) — rebuilding that inside a
  container buys no additional isolation, just a second copy of the same
  hardening to maintain.
- **`RestrictAddressFamilies` must include `AF_NETLINK`, not just
  `AF_INET`/`AF_INET6`/`AF_UNIX`.** sing-box opens a netlink socket for its
  network-interface-change monitor (used for outbound-interface
  auto-detection) even with no TUN device configured. Omit `AF_NETLINK`
  and the kernel rejects that `socket()` call with `EAFNOSUPPORT` and
  sing-box refuses to start — found the hard way against a real AlmaLinux
  install (commit `08a2fa7`). This is exactly the kind of fact a container
  rewrite could silently lose, so it's preserved here *and* stays as the
  inline comment directly above `RestrictAddressFamilies=` in the unit
  file — check both if this ever needs touching again.
- sing-box is a third-party binary (not built from this source), so a
  container buys no build/dependency-caching benefit either — the only
  thing multi-stage Docker builds are good at here doesn't apply.

**WireGuard and Outline/`shadowbox`, mentioned in some deployment
discussions of this project, do not exist anywhere in this repository** —
there is no WireGuard config generation and no Outline/shadowbox
integration in the source tree (verified by full-tree search; see the
Phase 1 audit that preceded this containerization pass). If a real
deployment has either of those running, it was added outside this repo,
and this doc has nothing to say about how to containerize it.

### Why the four Rust services *are* containerized

`subscription`, `rendezvous`, `relay-agent`, `test-service` are ordinary
network services with no special capabilities, no kernel module
dependencies, and a real Rust build-caching win from `cargo-chef`
(dependency compilation reused across all four via one shared `builder`
stage in `deploy/docker/Dockerfile`, keyed off `Cargo.lock`/`Cargo.toml`
rather than source edits). `apps/admin` (`vpn-admin`) is **not**
containerized — it manages host state directly (`systemctl`,
`/etc/vpn/compat` as root) and putting it in a container would only add
indirection, not isolation, since it would still need a host bind mount
and host PID/systemd access to do its job.

Build one image:

```bash
docker build -f deploy/docker/Dockerfile --target subscription -t vpn1/subscription .
```

or bring up a whole profile with Compose (both validated end-to-end in a
sandboxed Docker daemon while writing this: built, run non-root with a
read-only rootfs and all capabilities dropped, and confirmed serving real
traffic — see below for what each profile is for):

```bash
# The four services as a Docker alternative to `cargo run` /
# deploy/local/run-dev-slice.sh — rendezvous + relay-agent + test-service
# talking to each other over an internal bridge network, each port
# published to 127.0.0.1 only (never a public bind):
docker compose -f deploy/docker/docker-compose.yml --profile dev up

# Just the subscription control-plane service, alongside an existing
# deploy/almalinux install (sing-box + vpn-admin keep running natively —
# this only replaces the vpn-subscription.service unit):
SUBSCRIPTION_GID=$(getent group vpn-subscription | cut -d: -f3) \
  docker compose -f deploy/docker/docker-compose.yml --profile production up -d subscription
```

### Operational details worth knowing before touching this

- **`subscription` uses `network_mode: host`, not a port mapping.** Its
  own code binds `127.0.0.1` unconditionally
  (`services/subscription/src/main.rs`) — by design, it must never be
  reachable except via the host's own loopback (spec §8/§27); nginx
  (running natively on the host) reverse-proxies to it at
  `proxy_pass http://127.0.0.1:9100` today
  (`deploy/almalinux/templates/nginx-vpn-subscription.conf.template`).
  Docker's bridge networking cannot publish a container's own `127.0.0.1`
  to the host, so the only way to keep "loopback-only, never public" true
  *and* keep nginx's config unchanged is host networking. A bridge-network
  alternative would require changing `subscription`'s bind address to
  `0.0.0.0` and relying on `-p 127.0.0.1:9100:9100` instead — deliberately
  not done, since it would weaken a documented security invariant enforced
  in the service's own source rather than only at the network layer.
- **SELinux labels: `:z`, never `:Z`, on every bind mount** (target host is
  AlmaLinux 8, SELinux enforcing). `subscription`'s container is not the
  only thing touching `/etc/vpn/compat` — `sing-box` and `vpn-admin` keep
  running natively on the same host and need continued access to the same
  directory tree. `:Z` relabels a mount exclusively for one container and
  would lock those host processes out; `:z` (shared) keeps it accessible
  to both.
- **`subscription`'s container `user:` must resolve to the host's
  `vpn-subscription` group's actual GID**, not a fixed number — bind
  mounts keep the host's on-disk numeric UID/GID as-is, so a container
  username has no bearing on whether the group-readable (`0640`)
  `users.json`/reality files stay readable. `docker-compose.yml` requires
  `SUBSCRIPTION_GID` to be set explicitly (from `getent group
  vpn-subscription`) rather than guessing a number, so a mismatch fails
  loudly instead of silently 500ing on every request.
- **`rendezvous`'s pool file is mounted under `/tmp` (tmpfs), not its own
  read-only directory.** With no `--key-dir` given (the `dev` profile's
  default), rendezvous falls back to an ephemeral signing key and writes a
  `<pool-file>.root_pub` sidecar next to it on every boot; that write
  fails against a read-only bind-mounted directory. Mounting the pool file
  itself read-only inside the (writable) `/tmp` tmpfs satisfies both.
- **`relay-agent` has no HTTP surface to health-check** (it's a raw
  TLS/QUIC forwarder). Its `HEALTHCHECK` does a bare TCP connect against
  `$RELAY_TLS_PORT` instead of an HTTP probe — set that env var to match
  whatever port `--bind-tls` uses, or the check silently no-ops.
- All four images are non-root (`vpnsvc`, uid/gid `10001`), run with
  `read_only: true` + a `/tmp` tmpfs, `cap_drop: ["ALL"]`, and
  `no-new-privileges`. None of the four bind a port below 1024, so none
  need `CAP_NET_BIND_SERVICE` (unlike sing-box's `443`).
- Base images are pinned by digest (`rust:1.94.1-slim-bookworm` matching
  `rust-toolchain.toml`'s `channel = "1.94.1"`, `debian:12-slim` for
  runtime — same Debian release as the builder, so glibc versions match),
  not floating tags. Re-pin deliberately when bumping the Rust toolchain
  or picking up base-image security fixes — `docker pull <image>:<tag>`
  then `docker inspect --format='{{index .RepoDigests 0}}' <image>:<tag>`
  gives the new digest to paste in.
- `.dockerignore` (repo root) excludes `target/`, key material
  (`*.key`, `*.pem`, `reality/private.key`, `reality/public.key`,
  Hysteria2 TLS files), and `/etc/vpn/**` from the build context — nothing
  under active secret paths is ever sent to the Docker daemon.

### Explicitly out of scope

Public production deployment beyond what `deploy/almalinux/install.sh`
and `deploy/docker/` already cover (real cloud relay fleet, a
monitoring/alerting stack, Kubernetes/orchestrator-managed rollout) is not
built here — no purchase of services, no push to public infrastructure,
per the operating instructions for this session.

## Linux kill switch / TUN (Phase 9 — deferred)

Not implemented this session: the `KillSwitchBackend` trait
(`crates/policy::killswitch`) and a mock backend are implemented and unit
tested (policy: "no secure route => block", tested against the mock), but
the real Linux backend (nftables rules + a TUN device via `tun`/`nix`)
needs `CAP_NET_ADMIN` and mutates host routing tables — building and
testing it live in this sandboxed session risked leaving the host network
namespace in a broken state with no safe rollback path available here. A
concrete implementation plan (nftables ruleset, TUN setup order, rollback
procedure) is written in the trait's module doc comment for the next
session to implement against.
