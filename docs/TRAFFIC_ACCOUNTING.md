# Traffic accounting

## What is measured

Per-**node** cumulative bytes and open-connection count, read from sing-box's
Clash API and reported to the vpn-web Worker every poll interval (15s by
default) by the provisioning agent.

**Per-user traffic is not measured.** That is a constraint imposed by the
sing-box binary we run, not an omission. The rest of this document is the
evidence, because the obvious fix looks available and is not.

## Why not per-user

The natural mechanism is sing-box's V2Ray-compatible stats service, which
tracks `user>>>NAME>>>traffic>>>uplink` counters per named inbound user. It is
configured under `experimental.v2ray_api.stats`.

It is **not compiled into official sing-box builds.**

Checked against the pinned version (1.13.19, see `COMPATIBILITY_VERSIONS.md`):

```
$ sing-box check -c with-v2ray-api.json
FATAL create v2ray-server: v2ray api is not included in this build,
      rebuild with -tags with_v2ray_api
```

And upstream's own manifest of what ships in a release confirms it —
`release/DEFAULT_BUILD_TAGS_OTHERS` at v1.13.19:

```
with_gvisor,with_quic,with_dhcp,with_wireguard,with_utls,with_acme,
with_clash_api,with_tailscale,with_ccm,with_ocm,badlinkname,
tfogo_checklinkname0
```

`with_clash_api` is present. `with_v2ray_api` is not.

Note that sing-box's *documentation* table marks `with_v2ray_api` as enabled
by default. That table is wrong, or describes something other than the
release binaries. Trust the build-tag file and the binary.

### The Clash API cannot substitute

`with_clash_api` is available, and `GET /connections` returns per-connection
`upload`/`download` plus instance-wide `uploadTotal`/`downloadTotal`. Two
properties make it unusable for per-user accounting:

1. **No user attribution.** Connection metadata is:

   ```
   destinationIP, destinationPort, dnsMode, host, network,
   processPath, sourceIP, sourcePort, type
   ```

   There is no user field — verified by running a VLESS inbound with a named
   user (`"name": "user-alice"`) and pushing real traffic through it. `type`
   carries the inbound tag (`vless/vless-in`), which is shared by every user
   on that inbound.

2. **Closed connections disappear.** Once a connection ends it leaves the
   list. Its bytes survive only in the instance-wide totals, so a poller
   summing per-connection figures under-counts by everything that opened and
   closed between two polls.

The instance-wide totals do survive connection close, which is exactly why
node-level accounting works and per-user does not.

## What it would take

Running a sing-box built from source with `-tags with_v2ray_api`, and
teaching the agent to speak the V2Ray `StatsService` gRPC (the stats service
has no HTTP transport). That is a real commitment:

- a custom binary to build, sign and distribute
- no more dropping in an official release to pick up a security fix
- gRPC (`tonic` + `prost` + a build script) added to an agent that is
  currently plain `reqwest`

That trade is a product decision, not an implementation detail, which is why
this document exists instead of a half-built per-user path.

## How the node-level path works

The agent reads `GET /connections` and reports **cumulative** counters —
never deltas. A dropped or failed report is therefore harmless: the next one
carries the same running total, so a gap costs resolution but never bytes.
The agent does not retry a failed traffic report for the same reason.

The Worker differences each sample against the previous one for that node
inside `record_node_traffic`, which also folds the result into a daily
rollup. It takes a row lock on the node first, so two overlapping reports
cannot both claim the same bytes.

A sing-box restart resets its counters to zero. The Worker detects that as a
reported total *lower* than the previous one and treats the new total as the
delta, flagging the sample with `counter_reset`. Without that, a restart
would produce a negative delta and corrupt the rollup.

## Configuration

Traffic reporting is opt-in. With no `clash_api_url` in the agent config, the
agent logs once at startup and reports nothing — an agent deployed before
sing-box has the Clash API configured behaves exactly as it did before this
feature existed.

sing-box side:

```json
"experimental": {
  "clash_api": {
    "external_controller": "127.0.0.1:9090",
    "secret": "<a long random string>"
  }
}
```

Bind it to loopback. The Clash API exposes runtime state and can modify
routing; it must never be reachable off-host.

Agent side (`/etc/vpn/provisioning-agent.toml`):

```toml
clash_api_url = "http://127.0.0.1:9090"
clash_api_secret = "<the same string>"
```

The secret is redacted from the agent's `Debug` output so it cannot reach the
journal through a config-error log.
