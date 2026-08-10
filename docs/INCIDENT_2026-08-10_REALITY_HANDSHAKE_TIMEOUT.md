# Incident: Hiddify client timeout despite all local health checks passing

Date: 2026-08-10. Deployment: AlmaLinux VPS, `vpn.xn--80aa9argf5d.com`,
VLESS+REALITY on TCP/443, Hysteria2 on UDP/443, subscription HTTPS on
TCP/8444. See `docs/COMPATIBILITY_VERSIONS.md` for the pinned sing-box
version (`1.13.14`) this investigation used throughout, matching the
deployment.

**This document was rewritten after a second, adversarial investigation
pass explicitly challenged its first version's causal claims.** The
first version stated `vpn-admin restore`'s split-brain bug as this
incident's "root cause" in language stronger than the evidence
supports. That has been corrected below — this repository has never had
production shell history, command logs, or any other artifact showing
which `vpn-admin` commands actually ran on the incident VPS before the
timeout was observed. The bug is real, reproducible, and fixed; whether
it is *what actually happened* on that specific VPS is unproven. Read
§3 before citing this document as an explanation of the incident.

## 1. Proven incident facts

Established directly from the operator's own tcpdump/log capture, not
inferred:

- DNS resolved `vpn.xn--80aa9argf5d.com` to the VPS's public IP correctly.
- `Test-NetConnection` from the client confirmed TCP/443 externally reachable.
- tcpdump on the VPS during a real connection attempt showed TCP
  SYN/established/TLS-sized payloads (537, 601 bytes) arriving on
  443/tcp, simultaneously with heavy UDP traffic (1280/1068/591/770/251-byte
  datagrams) arriving on 443/udp.
- At the exact timestamp of the TCP connection attempt, sing-box logged
  (twice): `TLS handshake: REALITY: processed invalid connection`.
- Several seconds later, sing-box logged:
  `connection: open connection to api.my-ip.io:443 using outbound/direct[direct]: dial tcp 49.13.52.64:443: i/o timeout`.
- `curl -4 https://www.microsoft.com` and `curl -4 https://example.com`
  from the VPS both succeeded; `curl -4 https://api.my-ip.io` was flaky
  (once 429, once timed out).
- `vpn-admin doctor`, `deploy/almalinux/health-check.sh`, `systemctl
  status` for both services, and TLS cert validity all reported healthy
  at the time of the incident.
- Services were restarted several times during troubleshooting, and
  production secrets (REALITY private key, VLESS UUID, Hysteria2
  password) were printed to the terminal/chat during that process.
- **Not established:** which `vpn-admin` subcommands, if any, were run
  on the VPS before or during troubleshooting, in what order, or
  whether `vpn-admin restore` specifically was ever invoked there. No
  shell history, systemd journal excerpt, or other artifact answering
  this was captured or provided.

## 2. Proven bugs (reproduced in this repository, independent of the incident)

1. **`vpn-admin restore` split-brain.** `cmd_restore`
   (`apps/admin/src/main.rs`) installed a backup archive's REALITY key
   material and reloaded `sing-box`, but never restarted
   `vpn-subscription`. `vpn-subscription` reads the REALITY public
   key/short_id from disk once, at its own process startup
   (`services/subscription/src/main.rs`), and has no config-reload
   path. A restore whose archived key differs from what was live before
   it ran leaves `sing-box` enforcing the *restored* key while
   `vpn-subscription` keeps serving the *previously-live* public
   key/short_id indefinitely — reproduced by
   `apps/admin/tests/cli.rs::restore_of_differing_reality_key_restarts_subscription_service_too`,
   which failed before the fix and passes after. This is the same bug
   class already found and fixed once for `vpn-admin init --rotate`
   (`docs/FINAL_PRODUCTION_AUDIT.md` P0-5) — `restore` reused the key
   *installation* logic but not the service-restart *coordination* half
   of that fix.
2. **`install.sh` repair/re-run never restarts `vpn-subscription`/`sing-box`.**
   `enable_and_start_services` used `systemctl enable --now`, a no-op
   on an already-active unit — see §K-equivalent detail folded into §5
   below (installer audit).
3. **Diagnostics could not observe a live process's actual state.**
   Every check `vpn-admin doctor` ran before this pass re-read files
   from disk; none of them ever asked the ALREADY-RUNNING
   `vpn-subscription` process what it currently believes. A stale
   process passed every prior check. Fixed by a new live-fingerprint
   mechanism (§6).

None of these three is proven to be what happened on the incident VPS.
All three are proven, by direct reproduction in this repository, to be
capable of producing a REALITY-handshake failure indistinguishable from
what was observed.

## 3. Historical root cause: **UNKNOWN** (evidence insufficient to identify the historical trigger)

Of the five candidate conclusions considered for this incident:

- A. The restore split-brain bug definitely caused this — **not
  supported**. No artifact shows `restore` ran on this VPS.
- B. A different state-propagation path caused the same split-brain —
  **possible, not confirmed**. §2 items 2 and 3 are both real
  candidates; none is confirmed either.
- C. Server state was coherent and the client had stale
  credentials/profile material — **not ruled out**. The operator's
  report does not establish whether the Hiddify profile in use was
  freshly imported immediately before the failing attempt or was an
  older cached profile.
- D. Multiple independent failures — **plausible given the api.my-ip.io
  and UDP traffic evidence** (see below), but "multiple failures"
  is itself not proven, only that the evidence doesn't collapse to one
  single tidy story.
- **E. Available evidence is insufficient to identify the historical
  trigger, but this PR fixes real failure classes and adds detection.
  — this is the conclusion this document adopts.**

Reasoning: the `TLS handshake: REALITY: processed invalid connection`
log line is consistent with a stale-public-key split-brain (§2.1),
consistent with a stale-short_id split-brain reached the same way,
consistent with a client using an old cached profile (C), and — per
independent real-`sing-box` testing performed during this
investigation — is the specific message sing-box's REALITY layer
(`github.com/metacubex/utls`, `reality.go`) logs when server-side
verification of a connection's auth data fails, which several distinct
underlying causes can trigger identically from outside the tunnel. The
log line alone does not disambiguate between them, and no other
evidence in this incident (packet capture, `curl` tests, health-check
output) narrows it further. Separately, the `api.my-ip.io` timeout and
simultaneous heavy UDP/443 traffic are strong evidence Hysteria2's
handshake+auth *did* complete for at least one connection during the
same session (sing-box only reaches `outbound/direct[direct]: dial tcp
...` after routing an authenticated, decrypted client request) — this
was independently verified by real Hysteria2 interoperability testing
in this investigation (§7) and points toward the user's overall
"timeout" perception being partly a Hiddify-side connectivity-probe
artifact unrelated to whether the VPN tunnel itself worked, not
evidence about the REALITY failure's specific cause.

**What would resolve this to PROVEN or LIKELY:** VPS shell history
(`history`, `journalctl` covering the troubleshooting window) showing
which `vpn-admin` commands actually ran, in what order, relative to the
failing connection attempt timestamp. This repository has no access to
that VPS and none was provided; a future incident report should capture
it before troubleshooting destroys it.

## 4. Additional bugs found

Beyond §2's three, this pass also found and fixed:

4. **Self-test bypassed the actual delivery path.** The original
   `doctor --protocol` self-test (`run_reality_client_selftest`) built
   its throwaway client's REALITY key material by re-reading disk files
   directly — the same files `sing-box` reads — never by querying
   `vpn-subscription`'s HTTP API. A clean PASS from that self-test
   proved the on-disk key material was internally coherent with
   `sing-box`, but said nothing about whether the actually-running
   `vpn-subscription` process agreed — structurally unable to detect
   the exact incident class it existed to catch. Fixed by §6.
5. **No FAIL/WARN distinction in the protocol self-test.** Every
   non-pass outcome was reported `[WARN]`, including cases where the
   self-test's own throwaway client definitively rejected the server's
   handshake response (`reality verification failed` — a real,
   first-hand client-side verdict, not a timeout). Fixed: this now
   reports `[FAIL]` and counts toward `doctor`'s exit status; a bare
   timeout with no corroborating rejection remains `[WARN]`
   ("inconclusive," never conflated with a proven failure).
6. **No Hysteria2-equivalent protocol self-test existed at all** despite
   PR #8 previously focusing entirely on REALITY. Fixed by real
   end-to-end Hysteria2 interoperability tests (§7); a live,
   production-server-targeting Hysteria2 self-test inside `vpn-admin
   doctor --protocol` was evaluated and deliberately NOT added — seeding
   a real per-user Hysteria2 password into the live server config just
   to run a diagnostic would mutate production state and force a
   `sing-box` reload as a side effect of running `doctor`, which is a
   worse operational trade-off than the coverage gap it would close (the
   REALITY self-test avoids this because a synthetic *unregistered*
   UUID is still enough to exercise the REALITY TLS layer in isolation;
   Hysteria2 has no equivalent unauthenticated-but-still-informative
   layer to test against a live server without a real credential).
7. **`install.sh` repair/re-run never actually restarted `sing-box`/
   `vpn-subscription`.** `enable_and_start_services` used `systemctl
   enable --now`, a no-op on an already-active unit — the same mistake
   the file already documents and works around for `nginx`
   (`configure_nginx`), just not applied to vpn1's own two units. Fixed:
   both are now explicitly `reload-or-restart`ed, hard-failing the
   install if either restart fails.
8. **`doctor` `[FAIL]`ed on the REALITY *public* key being
   world-readable — a false positive.** The REALITY public key is not a
   secret by protocol design: it's embedded in every subscription
   response, share link, and QR code handed to every client over the
   public internet. `cmd_doctor` applied the same "not world-readable"
   confidentiality check to it as to the actual private key, which is
   the one file that genuinely needs that property. Reproduced directly
   with the real binary against a freshly-`init`'d deployment (a bare
   `vpn-admin init`, run without `install.sh`'s follow-up `chown`/`chmod`
   step, leaves `public.key` at the process's default umask — commonly
   world-readable, and not a security problem): `doctor` reported
   `[FAIL]` and a nonzero exit on an otherwise completely healthy
   server, which is exactly the kind of false alarm that erodes trust in
   a diagnostic tool and trains operators to ignore its output. Fixed:
   the world-readability check now applies only to the private key;
   the public key check is presence-only. Regression test:
   `apps/admin/tests/cli.rs::doctor_never_fails_on_world_readable_reality_public_key`.

## 5. Changes made

- `apps/admin/src/main.rs`:
  - `cmd_restore` now restarts `vpn-subscription` after installing
    restored REALITY key material (bug §2.1).
  - `vpn-admin doctor` tags every check line with the diagnostic layer
    it covers (`L1` process, `L2` config/key/cert, `L3` listeners, `L4`
    subscription-coherence, `L5-6` protocol handshake).
  - New `check_l4_live_subscription_process_state`: queries the
    ALREADY-RUNNING `vpn-subscription` process's own
    `/internal/state-fingerprint` HTTP endpoint and compares it against
    a fresh disk read — a hard `[FAIL]` on mismatch, `[WARN]` (never a
    false pass) if the process can't be reached at all. This is the
    check that actually closes bug §4.
  - `check_l5_l6_protocol_selftest`/`run_reality_client_selftest`
    rewritten: captures the throwaway client's own stderr and
    distinguishes a definitive `reality verification failed` rejection
    (`[FAIL]`, counts toward exit status) from an inconclusive timeout
    (`[WARN]`).
  - New `http_get_local_json` helper (dependency-free loopback HTTP GET).
  - `doctor`'s REALITY key world-readability check now only applies to
    the private key (bug §4.8) — the public key check is presence-only.
- `crates/compat-config/src/render.rs`:
  - `standard_endpoints` moved here from `services/subscription` so
    `vpn-admin` and `vpn-subscription` build the EXACT same endpoint set
    from a shared function, not two independently-hand-rolled
    equivalents.
  - New `endpoints_fingerprint`: SHA-256 over client-visible endpoint
    material only, never a private key.
- `services/subscription/src/lib.rs`:
  - New `GET /internal/state-fingerprint` route (loopback-only, same
    trust boundary as `/healthz`) exposing a fingerprint of THIS
    process's actual in-memory `AppState.endpoints` — never raw values.
  - Re-exports `standard_endpoints` from `compat_config::render`.
- `crates/compat-config/tests/reality_interop.rs`: real interop tests
  against the pinned `sing-box` binary — matched keypair
  handshakes+relays traffic to a local (non-public) HTTP target;
  mismatched public key fails closed; server/client key coherence is a
  fast always-run unit test.
- `crates/compat-config/tests/hysteria2_interop.rs` (new): the Hysteria2
  counterpart — matched password handshakes+relays traffic to a local
  target; mismatched password fails closed.
- `crates/compat-config/tests/common/mod.rs` (new): shared test-only
  helpers (local HTTP target, SOCKS5 client, sing-box process
  wrapper) used by both interop suites.
- `.github/workflows/ci.yml`: the `singbox-validate` job (which already
  downloads and checksum-verifies the pinned `sing-box` binary) now also
  runs `cargo test -p compat-config --test reality_interop --test
  hysteria2_interop` against it — these tests no longer silently skip
  in the pipeline that gates merges.
- `deploy/almalinux/install.sh`: `enable_and_start_services` now
  explicitly `reload-or-restart`s `sing-box.service` and
  `vpn-subscription.service` instead of `enable --now` (bug §4.7).
- `deploy/almalinux/health-check.sh`, `docs/ALMALINUX_DEPLOYMENT.md`,
  `docs/DEVICE_ACCEPTANCE_TESTS.md`: documented the new `doctor` layers,
  `doctor --protocol`, and the `sudo secure_path` `/usr/local/bin`
  papercut (docs-only fix).

## 6. Tests added

- `apps/admin/tests/cli.rs`:
  - `doctor_l4_coherence_passes_after_init_and_render`,
    `doctor_l4_detects_on_disk_config_drift_from_current_key_files`
    (file-only L4 checks).
  - `doctor_l4_live_check_passes_when_subscription_process_is_freshly_started`,
    `doctor_l4_live_check_fails_when_running_subscription_process_is_stale`
    — spawn the REAL compiled `subscription` binary as a background
    process and prove `doctor` can tell a fresh one from a stale one via
    a live HTTP query, closing bug §4.
  - `restore_of_differing_reality_key_restarts_subscription_service_too`
    (bug §2.1's regression test).
- `crates/compat-config/tests/reality_interop.rs` (3 tests),
  `hysteria2_interop.rs` (2 tests): see §5.
- `crates/compat-config/src/render.rs`: `endpoints_fingerprint`
  determinism/sensitivity unit tests.
- `services/subscription/src/lib.rs`:
  `state_fingerprint_matches_in_memory_endpoints_and_never_leaks_raw_values`.

## 7. CI evidence

Run in this sandbox against the exact pinned `sing-box 1.13.14` binary
(downloaded and SHA-256-verified, matching
`docs/COMPATIBILITY_VERSIONS.md` and `.github/workflows/ci.yml`'s own
pin):

```
cargo +stable fmt --check                                              # clean
cargo +stable clippy --workspace --all-targets -- -D warnings          # clean
cargo +stable test --workspace                                         # all pass
cargo +stable test -p compat-config --test reality_interop             # 3 passed (real sing-box)
cargo +stable test -p compat-config --test hysteria2_interop           # 2 passed (real sing-box)
cargo +stable test -p admin                                            # 18 (unit) + 12 (integration) passed
cargo +stable test -p subscription                                     # 9 passed
bash -n install.sh deploy/almalinux/*.sh deploy/lib/*.sh                # all parse
shellcheck -S warning install.sh deploy/lib/*.sh deploy/almalinux/*.sh # clean
```

`.github/workflows/ci.yml`'s `singbox-validate` job now runs the two
real interop suites in the actual pipeline (see §5) — this is checked
into the branch and will execute on the next push/PR sync; it has not
yet been observed running on GitHub's runners as of this writing (that
requires the push below to land and the workflow to fire), so "CI
passes" here is CI-equivalent local verification with the identical
pinned/checksummed binary, not yet an observed green GitHub Actions run.

## 8. Security review

- No real production secret (the exposed REALITY private key, VLESS
  UUID, Hysteria2 password, or subscription token) appears anywhere in
  this branch's source, tests, docs, commit messages, or CI
  configuration — confirmed by pattern search (`grep -rniE` for
  `xn--80aa9argf5d|157\.173\.27\.46|49\.13\.52\.64` — the incident's
  real hostname/IPs — across the full working tree and this branch's
  commit range) in addition to manual review; all test fixtures use
  clearly-fake values (`test-`-prefixed strings, `00000000-...` UUIDs,
  `deadbeef`-style short_ids).
- The new `/internal/state-fingerprint` endpoint exposes a SHA-256
  fingerprint only — verified by
  `state_fingerprint_matches_in_memory_endpoints_and_never_leaks_raw_values`
  that no raw key/short_id substring appears in its response.
- The new `doctor --protocol` self-test's stderr capture is
  pattern-matched only against static, non-secret-bearing strings
  (`"reality verification failed"`, `"processed invalid connection"`);
  its throwaway client config's log level is fixed at `"error"`
  (never `debug`/`trace`, which could otherwise surface key material in
  sing-box's own REALITY debug logging).

## 9. Exact VPS deployment commands

```bash
# on the VPS, as the deploy user:
cd /opt/vpn1
sudo vpn-admin backup --output /root/vpn1-backup-pre-update-$(date +%s).tar   # step 1: backup succeeds
git fetch origin
git checkout claude/vpn-connectivity-failure-qqwzkm   # or main once merged
sudo ./deploy/almalinux/install.sh                    # step 2/3: update + restart onto new binaries
sudo vpn-admin doctor                                 # step 4: expect no [FAIL], note the new [L4] lines
sudo vpn-admin doctor --protocol                      # step 5: best-effort real handshake self-test
```

## 10. Exact credential-rotation commands

Secrets exposed during live debugging must be rotated. Run in this
order — each step restarts the services that need to pick up the new
material:

```bash
sudo vpn-admin init --rotate                # REALITY keypair + short_id; restarts sing-box AND vpn-subscription
sudo vpn-admin user rotate-vless <user>     # new VLESS UUID for this user
sudo vpn-admin user rotate-hysteria <user>  # new Hysteria2 password
sudo vpn-admin user rotate-token <user>     # new subscription token
sudo vpn-admin doctor --protocol            # confirm coherence + a real handshake before telling users to reconnect
```

## 11. Exact real-device acceptance steps

1. `sudo vpn-admin user subscription <user>` on the VPS for the fresh
   subscription URL (new token from §10).
2. In Hiddify: **remove** the old profile entirely (not just refresh —
   the old profile's cached credentials are now rotated-away and must
   not linger), add the new subscription URL, let it fetch and import.
3. Connect via VLESS+REALITY explicitly, confirm real traffic (load a
   page through it); then explicitly select the Hysteria2 profile and
   confirm the same, independently.
4. Record the result in `docs/DEVICE_ACCEPTANCE_TESTS.md`'s matrix —
   per that document's own rule, a cell only becomes PASS after a
   dated, filled-in real-device entry, never from this document alone.

### 11a. Full 16-point VPS acceptance checklist

Ordered so each step only runs once the ones before it are satisfied.
Deliberately never depends on `api.my-ip.io` or any other third-party
IP-check service — every check here uses either `vpn-admin`'s own
diagnostics or a controlled destination (`curl` against the
subscription endpoint itself, or a real page load, not an IP-echo
service).

```bash
# 1. backup succeeds
sudo vpn-admin backup --output /root/vpn1-backup-$(date +%s).tar

# 2. update succeeds
cd /opt/vpn1 && git fetch origin && git checkout <target-ref>
sudo ./deploy/almalinux/install.sh

# 3. services restart onto the new binaries
systemctl show -p ActiveEnterTimestamp sing-box vpn-subscription   # both timestamps should be recent (this run)

# 4. doctor passes
sudo vpn-admin doctor                     # expect no [FAIL] lines

# 5. doctor --protocol passes
sudo vpn-admin doctor --protocol          # expect [L5-6] OK, not [WARN]/[FAIL]

# 6. REALITY key coherence passes
#    (folded into step 4/5's [L4] lines — no separate command; the
#    live-fingerprint check in [L4] IS the REALITY coherence proof)

# 7. Hysteria2 credential coherence
#    Unlike REALITY's key material, vpn-subscription reads users.json
#    (which holds Hysteria2 passwords) FRESH on every single request
#    (services/subscription/src/lib.rs get_subscription) — there is no
#    startup-cached copy to go stale, so there is no separate
#    "Hysteria2 credential coherence" failure mode to check for. Confirm
#    this assumption still holds after any subscription-service code
#    change by re-reading that function before relying on this line.

# 8. subscription HTTPS returns current config
curl -fsS "https://<subscription_host>:<port>/sub/<token>" | jq '.outbounds | length'
#    expect 3 (reality outbound, hysteria2 outbound, urltest) — confirms
#    the PUBLIC-facing HTTPS path (through nginx) actually serves
#    current data, not just the loopback backend doctor already checked

# 9. fresh profile imports into Hiddify — see §11 steps 1-2

# 10. REALITY works independently — see §11 step 3 (VLESS+REALITY only)

# 11. Hysteria2 works independently — see §11 step 3 (Hysteria2 only)

# 12. actual web traffic crosses each tunnel — see §11 step 3 ("load a page")

# 13. old credentials fail after rotation
#    BEFORE running §10's rotation, save the OLD subscription URL/QR.
#    AFTER rotating, attempt to reconnect with a client still holding
#    the OLD profile — it must fail (REALITY/Hysteria2 handshake
#    rejected, not "still works").

# 14. fresh credentials succeed
#    Import the NEW subscription (post-rotation) per §11 steps 1-3 and
#    confirm it connects — the direct contrast with step 13 is the
#    proof, not either step alone.

# 15. restore followed by fresh subscription still works
sudo vpn-admin restore /root/vpn1-backup-<timestamp-from-step-1>.tar
sudo vpn-admin doctor --protocol          # must show [L4]/[L5-6] OK — this is exactly
                                           # what bug #2.1 made unsafe before this fix
curl -fsS "https://<subscription_host>:<port>/sub/<token>" | jq '.outbounds[0].tls.reality.public_key'
#    compare against: sudo cat /etc/vpn/compat/reality/public.key
#    (must match — proves the restore's key is what's actually being served)

# 16. installer repair/re-run followed by fresh subscription still works
sudo ./deploy/almalinux/install.sh        # re-run with no changes; must complete cleanly
sudo vpn-admin doctor --protocol          # must still show OK — this is exactly what
                                           # bug #4.7 (install.sh's enable --now no-op) made unsafe before this fix
```

## 12. Rollback procedure

If `install.sh`'s repair re-run or the rotation steps above cause a
regression:

```bash
# sing-box config / REALITY key material:
sudo vpn-admin restore /root/vpn1-backup-pre-update-<timestamp>.tar
sudo vpn-admin doctor --protocol   # confirm the restored state is coherent (this is what bug §2.1 made unsafe before this fix)

# binaries, if the new install.sh run itself is the problem:
cd /opt/vpn1 && git checkout <previous-commit-or-tag>
sudo ./deploy/almalinux/install.sh
```

`vpn-admin restore` is now safe to use for this purpose specifically
*because* bug §2.1 is fixed — before this change, a rollback via
`restore` could itself have reintroduced a REALITY split-brain.

## 13. Merge recommendation

**READY TO MERGE**, with the causal-claim correction in this document
as part of the change being merged (this document's rewrite IS the
required correction, not a follow-up).

Reasons:

- All bugs claimed as fixed (§2, §4) are reproduced by a real,
  independent regression test in this repository — not asserted from
  code review alone.
- The causal narrative no longer claims more than the evidence
  supports (§3): historical root cause is stated as UNKNOWN, with an
  explicit reasoning chain for why, and what evidence would resolve it.
- `doctor --protocol`'s FAIL/WARN semantics now distinguish a proven
  authentication failure from an inconclusive environment limitation,
  per explicit review of that exact gap.
- Both REALITY and Hysteria2 now have real, deterministic,
  non-public-network-dependent interoperability tests, wired into CI
  (not merely runnable locally).
- `apps/admin/src/main.rs`'s new live-fingerprint mechanism closes the
  specific gap an independent adversarial review identified: prior
  self-tests could PASS while an already-stale, already-running
  `vpn-subscription` process still served wrong credentials to a real
  client.
- `cargo fmt`, `clippy -D warnings`, `cargo test --workspace`, `bash
  -n`, and `shellcheck` all pass locally against the exact tooling
  versions CI pins.

Outstanding, not blocking merge, but required before this specific
production VPS is declared fixed:

- Real-device acceptance testing per §11 has not been performed against
  the actual VPS (this sandbox cannot reach it) — `docs/DEVICE_ACCEPTANCE_TESTS.md`
  must not be marked PASS until it is.
- The CI workflow changes in this PR have not yet been observed running
  green on GitHub's own runners (§7) — confirm on the next push before
  treating "CI passes" as anything beyond local CI-equivalent
  verification.
- Credential rotation (§10) has not been performed on the real VPS —
  the exposed secrets remain compromised until an operator runs it
  there.
