# Incident: Hiddify client timeout despite all local health checks passing

Date: 2026-08-10. Deployment: AlmaLinux VPS, `vpn.xn--80aa9argf5d.com`,
VLESS+REALITY on TCP/443, Hysteria2 on UDP/443, subscription HTTPS on
TCP/8444. See `docs/COMPATIBILITY_VERSIONS.md` for the pinned sing-box
version (`1.13.14`) this investigation used throughout, matching the
deployment.

## A. Root cause

**Confirmed, reproducible bug:** `vpn-admin restore` (`apps/admin/src/main.rs`,
`cmd_restore`) installs a backup archive's REALITY key material
(`reality/private.key`, `reality/public.key`, `reality/short_id.txt`)
directly onto disk and reloads `sing-box`, but never restarted
`vpn-subscription`. `vpn-subscription` reads the REALITY public
key/short_id from disk **once, at process startup**
(`services/subscription/src/main.rs`) and holds them in memory for its
entire lifetime — it has no config-reload path. If a restore installs
REALITY key material that differs from what was live before the restore
(restoring an older backup, or restoring onto a fresh/replacement host
after key material had already diverged), `sing-box` starts enforcing
the *restored* key while `vpn-subscription` keeps serving the
*previously-live* public key/short_id to every client that fetches a
subscription afterwards — indefinitely, with `restore` reporting success.

A client that imports a subscription in that state receives a REALITY
public key that does not correspond to `sing-box`'s actual private key.
`sing-box`'s REALITY server-side implementation
(`github.com/metacubex/utls`, `reality.go`) cannot validate that
client's auth data, cannot complete the handshake, and (depending on
exactly which check fails first — public key vs. short_id vs. an
unmatched client entirely) either logs `REALITY: processed invalid
connection` and silently proxies the connection to the REALITY decoy
target (`www.microsoft.com:443` in this deployment) as camouflage
traffic, or the client detects the mismatch itself and reports
`reality verification failed`. Both are the observed symptom: a real
TCP connection that completes at the transport level (matching the
packet capture: SYN, established, TLS-sized payloads) but never
authenticates, which a client surfaces as "timeout."

This exact class of bug (`sing-box` and `vpn-subscription` disagreeing
about REALITY key material after a state-mutating `vpn-admin` command)
was already found and fixed once, for `vpn-admin init --rotate`, in
`docs/FINAL_PRODUCTION_AUDIT.md` P0-5. `restore` reused the *key
installation* logic but not the *service-restart coordination* half of
that fix — the same bug class, reached through a second, unaudited
mutation path.

**Fix:** `cmd_restore` now restarts `vpn-subscription` via the same
`CompatibilityServiceManager::reload_and_verify()` pattern
`cmd_reality_rotate` already uses, immediately after
`regenerate_singbox_config`, and fails loudly (not silently) if that
restart doesn't succeed. See `apps/admin/tests/cli.rs::restore_of_differing_reality_key_restarts_subscription_service_too`
for a regression test that fails without the fix and passes with it.

## B. Why existing checks passed anyway

`vpn-admin doctor` and `deploy/almalinux/health-check.sh` checked: is
sing-box active, does its config parse, are the REALITY key files
present and not world-readable, is the port open, is the TLS cert
valid. None of those checks ever compared the REALITY key material
`sing-box` is actually enforcing against what `vpn-subscription` is
actually advertising — they check that *a* config is valid and *a*
process is running, never that the two independently-running processes
agree with each other, and never that a real client can complete a
handshake. This is a coherence property, not a per-process health
property, and nothing checked it. Fixed by the new `doctor` `[L4]`
layer (see D below).

## C. Fixes implemented

- `apps/admin/src/main.rs` — `cmd_restore` now restarts
  `vpn-subscription` after installing restored REALITY key material
  (the root-cause fix, C above).
- `apps/admin/src/main.rs` — `vpn-admin doctor` now tags every check
  line with the diagnostic layer it actually covers (`L1` process,
  `L2` config/key/cert, `L3` listeners, `L4` subscription-render
  coherence, `L5-6` real protocol handshake), and adds two new always-on
  `L4` checks: (1) the client subscription rendered from current state
  agrees with the current server config on REALITY public key/short_id,
  and (2) the sing-box `config.json` actually on disk matches what the
  current key files/`users.json` would render right now (catches drift
  from a skipped `render-config`, a hand-edited key file, or exactly
  this incident's restore scenario, without any network access).
- `apps/admin/src/main.rs` — new `vpn-admin doctor --protocol` flag:
  best-effort `L5-6` self-test that spins up a real, throwaway
  `sing-box` client against the server's own REALITY listener on
  loopback and reports whether a real handshake + relay actually
  completes. Never prints secrets; uses a synthetic UUID, never a real
  user's.
- `crates/compat-config/tests/reality_interop.rs` (new) — real
  interoperability tests against the pinned `sing-box` binary: a
  matched keypair handshakes and relays traffic end-to-end; a
  mismatched public key is proven to fail closed; server/client
  key-material agreement is asserted as a fast, always-run unit test.
- `deploy/almalinux/health-check.sh`, `docs/ALMALINUX_DEPLOYMENT.md`,
  `docs/DEVICE_ACCEPTANCE_TESTS.md` — documented the new `doctor`
  layers and pointed operators at `doctor --protocol`; documented the
  `sudo secure_path` `/usr/local/bin` papercut (docs-only fix, not an
  `install.sh` change — editing `sudo` policy from an installer was
  judged too invasive).

## D. Tests added

See `crates/compat-config/tests/reality_interop.rs` (real `sing-box`
handshakes, gated on `SING_BOX_BIN`/`PATH`, skip — not fail — when
unavailable) and the two `doctor_l4_*` tests plus
`restore_of_differing_reality_key_restarts_subscription_service_too` in
`apps/admin/tests/cli.rs`.

## E. Test results

```
cargo +stable fmt --check                                   # clean
cargo +stable clippy --workspace --all-targets -- -D warnings  # clean
cargo +stable test --workspace                               # all pass
cargo +stable test -p compat-config --test reality_interop   # 3 passed (real sing-box 1.13.14)
cargo +stable test -p admin                                  # 30 passed
```

## F. Security

No real production secret (the exposed REALITY private key, VLESS UUID,
or Hysteria2 password) was copied into source, tests, docs, commits, or
logs anywhere in this change. All test fixtures use fake/generated
values. Rotation of the exposed credentials is a separate, operator-run
step (see the operator procedure below) — this change does not perform
it, since that requires access to the live VPS.

## G. Deployment (update the existing VPS)

```bash
# on the VPS, as the deploy user:
cd /opt/vpn1
git fetch origin
git checkout claude/vpn-connectivity-failure-qqwzkm   # or main once merged
sudo ./deploy/almalinux/install.sh                    # idempotent re-run: rebuilds/reinstalls binaries, re-validates config
sudo vpn-admin doctor                                 # expect no [FAIL] lines, note the [L4] lines specifically
sudo vpn-admin doctor --protocol                      # best-effort real handshake self-test
```

## H. Credential rotation (secrets exposed during debugging)

Run in this order — each step restarts the services that need to pick
up the new material, per the fixes above:

```bash
sudo vpn-admin init --rotate            # REALITY keypair + short_id; restarts sing-box AND vpn-subscription
sudo vpn-admin user rotate-vless <user> # new VLESS UUID for this user
sudo vpn-admin user rotate-hysteria <user>  # new Hysteria2 password
sudo vpn-admin user rotate-token <user> # new subscription token
sudo vpn-admin doctor --protocol        # confirm coherence + a real handshake before telling users to reconnect
```

## I. Client reimport (Hiddify)

1. `sudo vpn-admin user subscription <user>` on the VPS to get the
   fresh subscription URL (new token from the rotation above).
2. In Hiddify: remove the old profile, add the new subscription URL,
   let it fetch and import.
3. Connect via VLESS+REALITY first, confirm real traffic (e.g. load a
   page); then explicitly select the Hysteria2 profile and confirm the
   same.

## J. Acceptance criteria

- [x] `vpn-admin doctor` passes (no `[FAIL]`), including the new `[L4]` lines.
- [x] `vpn-admin doctor --protocol` completes and reports `[L5-6]` OK
      against a real throwaway handshake (verified in this sandbox with
      the pinned `sing-box` binary; must be re-confirmed on the actual
      VPS after deployment, since this sandbox is not that host).
- [x] VLESS+REALITY real handshake succeeds — proven via
      `crates/compat-config/tests/reality_interop.rs`.
- [x] Hysteria2 real handshake succeeds — proven manually with the
      pinned `sing-box` binary against the exact renderer output (see
      the Hysteria2 subagent findings in this investigation); traffic
      relayed end-to-end.
- [x] Fresh subscription matches current server state — proven by the
      new `L4` coherence checks and `reality_interop.rs`'s coherence test.
- [x] A concrete split-brain (old key fails, new key/fresh subscription
      succeeds) is proven both for `restore` (new regression test) and
      demonstrated protocol-level (`reality_handshake_fails_with_mismatched_public_key`).
- [x] No secrets leak into logs, tests, or this document.
- [x] CI-equivalent checks (`fmt`, `clippy -D warnings`, `test --workspace`) pass.
- [ ] Real-device (physical Hiddify client) verification — still
      requires a human running through `docs/DEVICE_ACCEPTANCE_TESTS.md`
      against the actual VPS after deploying this change and rotating
      credentials; not something this sandbox can perform. Do not mark
      any `docs/DEVICE_ACCEPTANCE_TESTS.md` matrix cell PASS without
      that real run.

## K. Addendum: `install.sh` repair/re-run audit

A second, related gap was found auditing `deploy/almalinux/install.sh`'s
repair/re-run path (same "does a mutation actually take effect on
already-running services" question as A above, applied to the
installer rather than `vpn-admin`): `enable_and_start_services`
(stage 15) used `systemctl enable --now sing-box.service` /
`vpn-subscription.service`. `enable --now` only *starts* a unit if it
is not already running — on a repair/upgrade re-run of `install.sh`
against an existing deployment, both units are normally already
active, so this was a silent no-op that never picked up a rebuilt
`vpn-subscription-svc` binary from `binaries_stage` (stage 4). This is
the identical class of mistake `configure_nginx` in the same file
already documents and works around for nginx (explicit
`systemctl reload-or-restart nginx`, not `enable --now`) — it just
hadn't been applied to `sing-box.service`/`vpn-subscription.service`.
Fixed: `enable_and_start_services` now explicitly
`systemctl reload-or-restart`s both units (hard-failing install if
either restart fails), so a repair re-run always actually deploys
whatever binaries/config it just staged, instead of leaving the
previously-running processes untouched. `sing-box` was already
explicitly reloaded earlier by `server_config_stage` (stage 11, via
`vpn-admin render-config`) — the restart here is a cheap, idempotent
no-op for it, not new risk. Verified with `bash -n` and
`shellcheck -S warning` (both clean, matching `.github/workflows/ci.yml`'s
own invocation).

This does not change the confirmed root cause in A — no evidence this
specific gap caused the incident (nothing in the reported timeline
involved a repair/upgrade re-run) — but it is the same "mutation didn't
propagate to a running service" bug class, found by extending the audit
to the one mutation path (`install.sh` repair/re-run) not covered by
the parallel subagent investigation, and is fixed for the same reason
the `restore` fix in A is: leaving it would let the next repair/upgrade
silently reintroduce a stale-service incident of this same shape.
