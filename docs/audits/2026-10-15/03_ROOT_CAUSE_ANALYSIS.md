# 03 - Root cause analysis

Conclusions are labelled **confirmed** (reproduced or read directly from code),
**not reproduced**, or **hypothesis**. "Why tests missed it" is stated for each fixed defect.

---

## A. Hiddify iOS: `open .../Library/Caches/Working/configs/<uuid>.json: no such file or directory`

**Verdict: client-side. Root mechanism confirmed in Hiddify's source; the trigger on the
reporter's phone is a hypothesis. Nothing server-side can cause or cure it.**

Evidence (hiddify-app `83911ed`, 2026-10-14):

1. `ios/Shared/FilePath.swift`: `workingDirectory = <AppGroup>/Library/Caches/Working`.
2. `lib/core/directories/directories_provider.dart`: on iOS `workingDir` comes from the native
   `get_paths`; `ProfilePathResolver.directory = <workingDir>/configs`, file = `<profile id>.json`.
3. The profile *list* is in the SQLite database under `getLibraryDirectory()` (persistent).
4. `ConnectionRepository.connect()` calls `singbox.start(profilePathResolver.file(id).path, ...)`
   with no existence check and no re-download/regeneration path.
5. A profile is downloaded to `<id>.tmp.json`, validated with `validateConfigByPath(path, tempPath)`
   (the core is handed both paths; I did not verify in the core source that it renames the file),
   and only then inserted into the database.

So a database row can outlive its file whenever the cache directory is purged (iOS may purge
Caches), an app update or restore keeps the database but not the cache, or a write is
interrupted. The error text in the screenshot is exactly the Go `open` failure for that path.
This matches the "dart: Unexpected failure" the user saw.

Not established: which of the triggers happened. Workaround: delete the profile and add the
subscription again. Upstream fix needed: check/recreate the file (or keep it outside Caches).
Why server tests can't catch it: it never reaches the server.

## B. Hiddify iOS: `failed to start core: CoreStatus.stopped(alert: CoreAlert.startService)`

**Verdict: not reproduced; no server cause found. Hypotheses remain.**

What was run (Windows CLI, built from source; **not** the iOS Network Extension):

| Core | parse + CheckConfig on every server representation | start + traffic through the VPS |
|---|---|---|
| current main `dbf3ffb` | all PASS | PASS (plain links, pinned JSON, single REALITY link, bare partial JSON) |
| `v4.0.0` (same number as the screenshot) | all PASS (identical outcome) | not tested: the old CLI has no `run -c`, and `srun` needs a full raw config |

The bare `/sub/<token>` response (partial sing-box JSON with only `outbounds` and `route`)
is, per this repo's own documentation, the format Hiddify's fork "strict-unmarshals",
yet both cores parsed, validated and (current core) ran it. So "invalid subscription" is not
supported by the evidence.

Plausible causes that remain, none proven: (1) Issue A's missing file surfacing as a generic
start failure (both errors came from the same client); (2) iOS Network Extension lifecycle or
memory limits on the pre-release app; (3) a 4.0.0-dev build that differs from the tagged v4.0.0.
What would decide it: the phone's Hiddify log (Settings -> logs) at the moment of failure and
the same profile on app 4.1.1.

## C. Shadowrocket and INCY cannot import

### C1. Which implementation owns the failure

Decided by the URL: a 27-character token on port 8443 on the node host is
`singbox-vpn`'s own service; a 43-character token on the account website is vpn-web's.
Both were examined; the user must tell which one the phone used (not knowable from here).

### C2. singbox-vpn path (Shadowrocket) - confirmed gap, fixed

`format=hiddify`/`uri` return plain share links. The service's own documentation records
that Shadowrocket rejects that body and that `encoding=base64` fixes it, but no command,
installer text or doc ever printed the Base64 URL. Fix: `subscription_url_shadowrocket`
(a54a2e9). Live: the body decodes byte-for-byte to the plain feed. **Device import not tested.**
Why missed: the opt-in server feature had unit tests; nothing tested what the operator is
actually shown.

### C3. vpn-web path (Shadowrocket, INCY, Hiddify) - confirmed defects, fixed

Every Hysteria2 link had (a) the REALITY decoy as `sni` and (b) no obfuscation, so it could
never connect (`02` R1); Hiddify's parser accepts it, so the import "succeeds". The endpoint
`/sub/[token]` had no tests. Fixed in cb3b608. Why missed: renderer tests used golden files
that encoded the wrong link, and no test ever ran a real client against a real node.

PR #83's own contribution (client-specific URL, Base64 for Shadowrocket, IPv6 brackets) was
reviewed line by line and is correct; two UI defects fixed (7416429).

### C4. INCY - not diagnosed

INCY's documentation: Base64 or plain body, JSON = array of full **Xray** configs,
`vless://` and `hysteria2://` supported, `hy2://` alias. A user report against INCY 2.4.5
says a VLESS-REALITY-only subscription creates a provider with zero servers while Hysteria2-only
works, and mixed fails. Both repos declare INCY as VLESS-only, i.e. the reported failing shape.
vpn-web's `xray` format is not a valid Xray config. **Unverified on a device**; test plan in `08`.

---

## D. Server-side defects found while building the baseline

| | Defect | Root cause | Fix | Why tests missed it |
|---|---|---|---|---|
| D1 | tunnel can reach the node's own services (S-1) | installer records no own address when `public_host` is a DNS name; renderer only adds the deny rule for IP literals or when `public_ipv4/6` are set | installer now records the node's own addresses (9ca903e, fix/server-hardening-rc2); verified on a real VPS | unit tests set the fields explicitly; no test ran the installer with a DNS `public_host` and then probed the tunnel |
| D2 | start-limit outage (I-11) | every authorization change is a restart; unit has `StartLimitBurst=8`; `reload-or-restart` is counted | `systemctl reset-failed` before administrative restarts (c1c0a7f) | tests used a fake `systemctl` with no rate limiter |
| D3 | user operations drop all connections (I-10) | sing-box has no in-place reload; SIGHUP also rebuilds the instance (upstream SagerNet/sing-box#3731) | not removable; documented; fingerprint short-circuit already avoids no-op restarts | n/a (architecture) |
| D4 | Hysteria2 caps lockout (I-12) | `ignore_client_bandwidth: true` emitted together with caps (sing-quic: deny BBR clients) | on `main` (9a07503), not in a release | `sing-box check` passes; self-test is REALITY-only |
| D5 | doctor false FAIL (I-14) | expectation rebuilt the C-16 block without the own-address rule | 63ad42c | no test combined `public_ipv4` with the exact-shape check |
| D6 | dev installer reuses old source (I-16) | repair path "reuses the existing persistent source tree" even in dev channel | a4e8f16 + 10ad1b0 (fix/server-hardening-rc2): reuse only on an identical content digest, otherwise a staged, verified two-rename swap with crash recovery | no test of dev-channel-over-existing |
| D7 | the fix for D6 initially refused every real pinned-SHA install | `gzip -dc \| head -c 4096 \| grep` under `pipefail`: head closes the pipe, gzip dies of SIGPIPE, and the `\|\| SOURCE_COMMIT=""` fallback discarded the id grep had found | 10ad1b0 | the fixture archive was a few hundred bytes, so gzip had always finished before head exited; found by the first real-VPS deploy, fixed, and the fixture now pads the archive with 3 MiB |
| D8 | doctor reports `release identity INCONSISTENT` on every healthy source build (I-17) | a source build links `vpn` and `vpn-admin` as separate executables, but one hash was recorded and both were compared to it | f070601: `vpn_sha256` recorded and compared separately, with a fallback for older manifests | unit tests used one hash for both names |

## E. Hysteria2 reconnection delay (7.3)

Mechanism is documented in the repo (a 1.14.1 server sends no QUIC CONNECTION_CLOSE; the
client waits its fixed 30 s idle timeout; 1.14.3 closes connections on shutdown). Not
reproduced: the only available path was loopback, which delivers ICMP unreachable and recovers
in 0.3 s. The REALITY side recovers in 1.8-3.4 s. The controllable part (avoid needless
restarts) is already in place for no-op changes; the rest needs the sing-box pin moved and a
network with working UDP/443 to measure.
