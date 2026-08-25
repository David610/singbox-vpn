# Incident: real Hiddify clients cannot connect; `doctor --protocol` reports INCONCLUSIVE, not FAIL

Date: 2026-08-11. Follow-on to
`docs/INCIDENT_2026-08-10_REALITY_HANDSHAKE_TIMEOUT.md` — read that document
first; this one does not repeat its background on REALITY's decoy-proxy
mechanism or the `processed invalid connection` ambiguity, both of which are
directly relevant here.

Labels follow this repository's standing convention
(`docs/FINAL_PRODUCTION_AUDIT.md` §"Verification levels"):
`STATIC` (source-read only), `LOCAL-EXEC` (actually run in this sandbox,
against a real pinned `sing-box` binary where stated), `VPS-verified` (run
against the operator's actual production VPS). **Nothing in this document is
VPS-verified.** This sandbox has no network path to the operator's VPS —
only the operator can run the commands in §7 there and confirm the result.

## 1. What was reported

- `vpn doctor` (L1-L4): all OK — binary present, config valid, REALITY
  private/public key cryptographically correspond, file ownership/mode
  correct, subscription service state matches on-disk config, config not
  stale.
- `vpn doctor --protocol` (L5-6): **INCONCLUSIVE** — "the client did not
  return an HTTP success response through the live VLESS+REALITY listener."
- `journalctl -u sing-box`: both real client attempts (from the operator's
  own network) and the self-test's own attempt (source `127.0.0.1`) log
  `TLS handshake: REALITY: processed invalid connection`.
- Two clusters of 2-3 `sing-box.service` restarts within ~1 second each,
  consistent with the reload/rollback transaction in `cmd_reality_rotate` /
  `regenerate_singbox_config` firing.

## 2. Root cause found in this pass (LOCAL-EXEC, reproduced against the
   pinned real `sing-box 1.13.14` binary)

**The `doctor --protocol` self-test's PASS/FAIL/INCONCLUSIVE classification
in `run_reality_client_selftest` (`apps/admin/src/main.rs`) could not
reliably detect a genuine REALITY handshake rejection, and this — not
necessarily the underlying REALITY key state itself — is what produced the
INCONCLUSIVE verdict described in §1.**

### The mechanism

Before this pass, the self-test decided `HandshakeRejected` vs
`Inconclusive` by grepping the THROWAWAY CLIENT's own stderr for two static
strings: `"reality verification failed"` and `"processed invalid
connection"`. Both strings are real (confirmed present in the pinned
`sing-box` binary via `strings`), but neither is reliably what the CLIENT
itself logs when the SERVER rejects a REALITY handshake:

- `"processed invalid connection"` is logged by the SERVER's REALITY layer
  (`github.com/metacubex/utls`, `reality.go`) when it cannot complete the
  hijack. It is not something the client process emits about itself.
- When the server rejects a connection's REALITY authentication, it does not
  drop the connection — it transparently proxies the raw bytes through to
  the real `handshake_server` decoy, exactly as REALITY is designed to do
  (the entire point is that a rejected connection is indistinguishable from
  a normal TLS session with the decoy, which is what makes REALITY resist
  active probing). So the CLIENT typically completes what looks like an
  entirely ordinary TLS handshake — with the decoy, not the intended server
  — and only then hangs trying to speak VLESS over it. No REALITY-specific
  string is ever logged, at any client log level, because from the client's
  point of view nothing REALITY-specific happened.

### Reproduction

Built a real REALITY server config (`vless-reality-in`) and a real REALITY
client config using the pinned `sing-box 1.13.14` binary directly (not a
mock), a genuinely mismatched — but well-formed — REALITY keypair (matching
private key on the server, a different real public key on the client), and
a local self-signed TLS 1.3 decoy (matching the determinism rationale
already established by `crates/compat-config/tests/reality_decoy_budget.rs`
for not dialing a third-party CDN in a test). Ran the client with this
self-test's exact production settings (`"log": {"level": "error"}`).

Server log (trace level, for diagnosis only — production runs at `warn`):

```
TRACE ... REALITY remoteAddr: 127.0.0.1:44200 hs.c.conn == conn: false
ERROR ... process connection from 127.0.0.1:44200: TLS handshake: REALITY: processed invalid connection
```

Client log, at the self-test's actual production log level:

```
ERROR[0002] connection: open connection to 127.0.0.1:19999 using outbound/vless[reality-selftest]:
x509: certificate signed by unknown authority (possibly because of "x509: invalid signature: parent
certificate cannot sign this kind of certificate" while trying to verify candidate authority
certificate "localhost")
```

The server unambiguously rejected the handshake. The client logged neither
of the two strings the old classifier checked for — it logged an ordinary
x509 chain-validation error from falling through to the (self-signed, in
this repro) decoy. On the real production deployment, whose decoy
(`www.google.com`) presents a publicly-trusted certificate, the client would
not even get an x509 error — it would complete a fully valid TLS session
with the decoy and then simply hang waiting for a VLESS response that never
comes, producing **no client-side error output whatsoever**. Both outcomes
were previously classified `Inconclusive`, indistinguishable from an
environmental timeout with no bearing on REALITY at all — which is exactly
the verdict reported in §1.

This is `apps/admin/src/main.rs::reality_selftest_classification_tests::a_real_key_mismatch_does_not_reliably_produce_either_client_side_string`,
committed as a regression test with the exact captured stderr string from
this reproduction.

### What this does and does not prove about the operator's VPS

This proves the self-test's OLD classifier was structurally unable to turn
a real rejection into `[FAIL]` in the general case, which is sufficient by
itself to explain an `INCONCLUSIVE` verdict on a VPS where the underlying
REALITY handshake genuinely is failing (consistent with real client
attempts also failing, per §1). It does **not** by itself identify *why*
the server rejects the handshake on the operator's VPS — REALITY logs the
identical `processed invalid connection` for a key/short_id mismatch *and*
for the operator's configured `handshake_server` returning a TLS record over
the 8192-byte budget (the exact ambiguity `check_l5_l6_protocol_selftest`'s
own FAIL message already documents, and the one
`reality_decoy_budget.rs` pins with a regression test). Distinguishing those
two remaining candidates requires evidence this sandbox cannot gather (see
§6) — most directly, re-running `doctor --protocol` after this fix and
reading whichever verdict it now reaches.

## 3. Fix (this pass)

`apps/admin/src/main.rs`:

- `run_reality_client_selftest` now ALSO cross-checks the live SERVER's own
  journal (`journalctl -u sing-box --since -20s`) for a `processed invalid
  connection` entry logged during the self-test's own connection attempt,
  not just the client's stderr. Confirmed (LOCAL-EXEC, via `strings` on the
  pinned binary and the reproduction above) that this message is logged at
  `ERROR` severity, so it survives the production default `"log": {"level":
  "warn"}` and is exactly what `journalctl -u sing-box` shows an operator
  directly — the same evidence the operator's own report in §1 already used
  by hand. This promotes an otherwise-silent rejection out of `Inconclusive`
  into `HandshakeRejected` (a hard `[FAIL]`, counted toward `doctor`'s exit
  status) whenever the server's own log corroborates it. This is
  best-effort, not proof of cause: it does not distinguish a key mismatch
  from an oversized-decoy rejection (see §2), and unrelated scanner traffic
  hitting the same port during the self-test's brief window could in
  principle produce a false-positive correlation — it is never used to
  fabricate a PASS, only to stop actively hiding a real failure.
  Classification logic extracted into a pure, unit-tested function
  (`reality_selftest_stderr_or_journal_indicates_rejection`).

- **The apply/rotate transaction gap the operator's own hypothesis named**
  (`render_and_apply_singbox_config` / `cmd_reality_rotate`) previously
  committed a config change as "reloaded and verified active" based solely
  on `CompatibilityServiceManager::reload_and_verify`, which is
  `systemctl is-active` three times in a row (`apps/admin/src/
  service.rs`) — read directly and confirmed by its own unit tests
  (`reload_and_verify_succeeds_when_service_comes_up_healthy` uses a fake
  `systemctl` script that only ever checks exit codes, never a protocol).
  `sing-box` stays "active" for a syntactically valid REALITY config
  regardless of whether the key material in it can authenticate anything —
  it does not itself validate that. Both `render_and_apply_singbox_config`
  (used by every `user create`/`disable`/`rotate-*`/`render-config` call)
  and `cmd_reality_rotate` now additionally run the real handshake self-test
  above (`verify_reality_handshake_or_warn`) against the just-reloaded live
  config before reporting success; a definitive `HandshakeRejected` verdict
  now triggers the SAME rollback path an outright reload failure already
  used, instead of being silently accepted. `Inconclusive`/no-test-user
  results do not block the transaction — this self-test cannot always reach
  a verdict (§2), and turning that into a false failure would be worse than
  the gap it closes.

- `.github/workflows/ci.yml` (`singbox-validate` job): new step runs the
  real `vpn-admin doctor --protocol` command — not just the crate-level
  config renderer, which `reality_interop.rs` already covered — against a
  real, live `sing-box` server process AND a real, live `subscription`
  process, using a local TLS 1.3 decoy for determinism (same rationale as
  `reality_decoy_budget.rs`). This is the "Level 5" gap
  `docs/PRODUCTION_HARDENING_PLAN.md` and `docs/FINAL_PRODUCTION_AUDIT.md`
  both named for the admin CLI itself, as distinct from the renderer-only
  interop suite. **LOCAL-EXEC verified**: ran this exact step's script in
  this sandbox; `doctor --protocol` reported
  `[OK]   [L5-6] protocol self-test: ... completed a full handshake ... and
  returned application bytes end-to-end`. Not yet observed running on
  GitHub's own runners as of this writing (requires this branch's push to
  land and the workflow to fire).

## 4. What was verified in this pass, and how

| Claim | Level | Evidence |
|---|---|---|
| `crates/compat-config`'s own production renderers produce a config that really authenticates via the real pinned `sing-box` binary | LOCAL-EXEC | `cargo test -p compat-config --test reality_interop` (3/3 pass), incl. `reality_handshake_succeeds_with_matched_keypair` and `reality_handshake_fails_with_mismatched_public_key` (fails closed, with a proven server-side auth-failure signal, not a tautological "no traffic") |
| The decoy record-size budget failure mode (§2 of the prior incident doc) is still real and still fails closed with auth SUCCEEDING | LOCAL-EXEC | `cargo test -p compat-config --test reality_decoy_budget` (1/1 pass) |
| A genuine REALITY key mismatch produces no client-observable string under the self-test's OLD classifier | LOCAL-EXEC | manual reproduction, §2 above, real pinned binary |
| The journal cross-check fix classifies that same reproduction correctly | STATIC + unit test | `reality_selftest_classification_tests::a_real_key_mismatch_does_not_reliably_produce_either_client_side_string`; the live `journalctl -u sing-box` cross-check itself is not exercised by this unit test (it is a pure function taking `journal_hit: bool` as an argument) or by CI (GitHub-hosted runners do not run `sing-box` under systemd, so `journalctl -u sing-box` finds nothing there either — same limitation as this sandbox) |
| `doctor --protocol`'s happy path (`[OK] [L5-6]`) works end-to-end through the real, compiled `vpn-admin` binary against a real, live `sing-box` + `subscription` process pair | LOCAL-EXEC | ran the new CI step's exact script in this sandbox; see §3 |
| The apply/rotate live-health gate change does not regress existing behavior | LOCAL-EXEC | `cargo test --workspace` (with `SING_BOX_BIN` set to the real pinned binary): `apps/admin`'s `cli` integration suite has 3-5 pre-existing flaky failures, reproduced identically (varying by run, same failing test names) on this branch's parent commit before any change in this pass — real `getgrnam(3)` calls in `cmd_restore`'s ownership step race across this sandbox's own concurrently-run test processes, which actually create/observe real OS groups since these tests run as root here. Every failure in every run, before and after this change, is one of exactly 5 named tests (`backup_then_restore_round_trips_users`, `restore_never_widens_permissions_on_restored_secrets`, `restore_of_differing_reality_key_restarts_subscription_service_too`, `doctor_l4_live_check_fails_when_running_subscription_process_is_stale`, `doctor_l4_live_check_passes_when_subscription_process_is_freshly_started`) — no new failing test name ever appeared with this change applied. |
| `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` | LOCAL-EXEC | both clean |
| Whether the operator's live VPS now reports `[OK]` at L5-6, and if not, whether it's cause (a) or (b) from §2 | **NOT VERIFIED — VPS-verified required, see §7** | this sandbox has no network path to the VPS |

## 5. What was NOT found to be the cause

- **REALITY key/short_id generation or rendering
  (`crates/compat-config/src/server.rs`).** Ruled out by the real interop
  suite (§4): the crate's own production renderer, used unmodified, both
  authenticates correctly with a matched keypair and fails closed with a
  mismatched one, against the real pinned binary. Nothing about the
  encoding/field shape is broken.
- **Version skew between the two installed `sing-box` binaries
  (`/usr/bin` 1.13.18 vs `/usr/local/bin` 1.13.14).** Not reproduced or
  further investigated in this pass — `vpn-admin` consistently targets
  `singbox_binary` from `deployment.toml`, which the operator's own `vpn
  doctor` output already confirmed resolves to `/usr/local/bin/sing-box`
  (the pinned 1.13.14). Nothing in this pass found a code path where the
  wrong binary could be invoked instead. Still worth the operator manually
  confirming `/usr/local/bin/sing-box version` matches what `sing-box.
  service`'s `ExecStart` actually invokes (`cat
  /etc/systemd/system/sing-box.service` or `systemctl cat sing-box`) — not
  done here because this sandbox has no access to that VPS.

## 6. What this sandbox could not do

- Reach the operator's actual VPS at all (no network path).
- Reach any real third-party site (`www.google.com` or otherwise) as a live
  REALITY decoy to test cause (b) directly — this sandbox's outbound HTTPS
  is intercepted by a corporate MITM proxy that re-terminates TLS, so a raw
  REALITY handshake against a real external decoy cannot be tested here (a
  REALITY decoy dial is a bare TCP+TLS 1.3 connection, not proxyable through
  an HTTP(S) forward proxy). This is exactly why §2's reproduction and the
  new CI step both use a local decoy instead — consistent with, not a
  deviation from, this repository's own established rationale for avoiding
  third-party decoys in tests (`reality_decoy_budget.rs`).
- Exercise the new journal cross-check against a REAL systemd-managed
  `sing-box.service` (this sandbox's sing-box processes were spawned
  directly, not via systemd, so `journalctl -u sing-box` finds nothing for
  them — same limitation GitHub-hosted CI runners have).

## 7. Exact VPS operator commands

Run on the VPS, as root, after this branch merges (or directly against this
branch to test first):

```bash
# 1. Update onto this fix.
cd /opt/singbox-vpn
sudo vpn-admin backup --output /root/singbox-vpn-backup-pre-fix-$(date +%s).tar
git fetch origin
git checkout <this-incident's-fix-branch>   # see git log for the exact pre-rename branch name, or main once merged
sudo ./deploy/almalinux/install.sh

# 2. Re-run the self-test with the fix in place. This alone may now
#    resolve INCONCLUSIVE to a definitive [OK] or [FAIL] — read the
#    verdict before doing anything else.
sudo vpn-admin doctor --protocol
```

**If step 2 now reports `[FAIL]` at L5-6:** the message names both
remaining candidates from §2. Narrow them:

```bash
# Cause (a) — key material — should already be ruled out by L2's
# "public.key cryptographically corresponds to private.key" check, but
# confirm the RUNNING process has the SAME key the self-test just used:
sudo systemctl show -p ActiveEnterTimestamp sing-box   # when did it last actually restart?
sudo cat /etc/vpn/compat/reality/public.key
curl -fsS "https://<subscription_host>:<port>/sub/<any-valid-token>" | jq '.outbounds[] | select(.type=="vless") | .tls.reality.public_key'
#    must match — if it does NOT, this is a live split-brain (the exact
#    class docs/INCIDENT_2026-08-10_REALITY_HANDSHAKE_TIMEOUT.md fixed
#    for `restore`; if it recurs here, that is a NEW instance worth its
#    own report, not an assumption).

# Cause (b) — decoy record size. Try a different, well-known-small-cert
# handshake_server and re-render:
sudo vim /etc/vpn/deployment.toml     # [reality] handshake_server = "..."
sudo vpn-admin render-config
sudo vpn-admin doctor --protocol      # re-check
```

**If step 2 now reports `[OK]`:** the self-test itself was the reason real
clients couldn't be diagnosed, not necessarily the reason they couldn't
connect. Confirm against a real device:

```bash
sudo vpn-admin user subscription <user>   # fresh subscription URL
# In Hiddify: remove the old profile entirely, add the new subscription
# URL, connect via REALITY explicitly, load a real page through it.
```

Record the device-acceptance result in `docs/DEVICE_ACCEPTANCE_TESTS.md` —
per that document's own rule, a cell only becomes PASS from a dated,
filled-in real-device entry, never from this document alone.

## 8. Merge recommendation

**READY TO MERGE** as a diagnostic and safety-gate fix. It closes a real,
reproduced defect (the self-test could not reliably report a real
handshake failure as a failure) and a real, reproduced gap (the
apply/rotate transaction could commit a protocol-broken REALITY state as
"verified active"). It does **not** by itself prove what was wrong on the
operator's specific VPS — that requires §7, which only the operator can
run.

Outstanding, not blocking merge:

- §7 has not been run against the real VPS.
- The new CI step (§3) has not yet been observed running on GitHub's own
  runners — confirm on the next push.
