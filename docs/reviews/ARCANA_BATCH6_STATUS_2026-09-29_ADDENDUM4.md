# Addendum 4 — two_hop_system s03/s05/s12 root-caused and fixed; 22/22 in real CI (2026-09-29)

Continuation of `ARCANA_BATCH6_STATUS_2026-09-29_ADDENDUM3.md`, at `abf792d`.
That pass got `two_hop_system` to 19/22 in real CI and left 3 failures
(`s03_relay_is_not_an_internet_exit`,
`s05_wrong_exit_credential_fails_after_reaching_the_relay`,
`s12_unpaired_relay_cannot_reach_anything_as_an_exit`) unresolved, all
showing an identical REALITY-layer "connection reset by peer"/EOF on
`first_hop_only_client`'s control check, with a recommendation to add a
temporary, test-scoped sing-box log-level bump to see the real rejection
reason. This pass did exactly that, got a real answer, and it was not a
REALITY/TLS/MTU issue at all.

## Diagnostic step — what the real log showed

Added an env-gated (`ARCANA_DIAGNOSTIC_SINGBOX_LOG_LEVEL`) override in
`two_hop_system.rs`'s `Node::apply`, applied strictly after the production
`render_server_config_for_deployment` call so the production renderer and
its default log level were never touched, plus a temporary non-blocking CI
step running only s03/s05/s12 with the relay at `debug`. Real run:
https://github.com/David610/singbox-vpn/actions/runs/36564686398 (job
`109393631248`).

The relay's debug log showed the VLESS/REALITY handshake succeeding and
the connection being authenticated —
`[alice] inbound connection to 127.0.0.1:<sub_port>` — immediately
followed by `router: match[1] inbound=[...] => reject` and
`router: connection closed: rejected`. sing-box's own fast reject after an
already-accepted stream surfaces to the client as a raw TCP RST on the
underlying connection — exactly the "connection reset by peer" every
prior pass's client-side (production-level) log showed. There was never a
REALITY/TLS/MTU/dummy-interface problem; the addressing fix from the
previous pass was correct and unrelated to this.

The diagnostic log-level override and its CI step were removed once root
cause was found (commit `3e51ac5`); nothing diagnostic remains in the
final state.

## Root cause 1 (test bug) — wrong identity for the loopback control check

`server.rs`'s loopback self-test exception rule (C-16 "Exceptions" /
NEW-01) is deliberately scoped to `auth_user: probe_ids`, i.e. only
principals with `is_reserved_probe: true`. s03/s05/s12 built their
"control: tunnel alive" check using the ordinary `relay_user` identity,
which C-16 correctly `reject`s at that same destination by design — the
control check could never have passed for that user, independent of
addressing, MTU, or interface. s12's own pre-existing comment even said
outright that no `is_reserved_probe` user existed in its scenario, then
the same test immediately built a "control" check that could only pass
with one — a direct, previously-undetected internal contradiction that a
green loopback run had never before let these three tests execute far
enough to hit.

Fix (commit `3e51ac5`): added `probe_user()` (an `is_reserved_probe: true`
`CompatUser`), applied to the relay alongside `relay_user` in
`Lab::start()` and in s12's re-apply, and switched the loopback control
check in all three scenarios to authenticate with the probe identity
instead of the customer identity under test. The customer-identity
`refused()` assertions in all three scenarios were already correct and
untouched.

## Root cause 2 (real production bug) — probe confinement shadowed the loopback exception it was supposed to coexist with

Applying the fix above immediately hit the SAME symptom again, even for
the probe identity. Second diagnostic run
(https://github.com/David610/singbox-vpn/actions/runs/36566695152, job
`109396278155`) showed the identical accept-then-reject pattern for the
probe user too. Reading `apply_probe_user_confinement`
(`crates/compat-config/src/server.rs`) found the real bug: it
unconditionally **prepended** its 3 rules (allow `1.1.1.1:443`, allow
`gstatic`/`icanhazip:443`, reject everything else for that identity)
ahead of every other rule — including the relay's own loopback self-test
exception rule, which is *also* `auth_user`-scoped to the exact same
`is_reserved_probe` principal. Because confinement's blanket reject
matched first for any destination outside its narrow allowlist, the
loopback exception rule was dead code on any relay with a probe user
provisioned — in real production, not just in this test suite. Nothing
had previously combined a probe user with a live connection attempt to
that destination in CI, so this had never been caught.

This matters beyond the test: the loopback exception rule's own doc
comment says it exists so "the post-install/post-update protocol
self-test... can prove a real first-hop handshake" — i.e. this bug would
also break that self-test in real production on any relay with a probe
user configured.

Fix (commit `8ad34ac`): `apply_probe_user_confinement` now inserts its 3
rules immediately after any existing rule(s) already exclusively scoped
to the same `auth_user` set (the loopback exception, when present),
instead of always at index 0. Updated `relay_role_policy.rs`'s two tests
that had encoded the old (buggy) ordering as expected behavior
(`probe_rules_precede_relay_policy_which_is_otherwise_unchanged`,
`unpaired_relay_loopback_selftest_is_scoped_to_the_reserved_probe_principal`)
to assert the corrected order, and generalized `assert_probe_rules_lead`
to take a starting offset shared between the exit-role (0) and
relay-role-with-loopback-exception (1) cases. All 62 `relay_role_policy.rs`
cases and the full `compat-config` lib/integration suite pass locally.

Follow-on fixups (commit `89d9b3a`): `s12`'s expected route-rule count
needed updating from a wrong guess of 2 to the correct 5 (loopback
exception + all 3 confinement rules + final reject — confinement's 3
rules are added whenever any `is_reserved_probe` user is present,
regardless of the loopback exception's own presence), and `s14`'s
"idempotent render" check was comparing a `before` snapshot that included
`probe_user` (captured from `Lab::start()`'s own apply) against a re-apply
that only passed `relay_user` — never actually the same input twice, once
`probe_user` became part of the default lab setup.

Verified real CI:
https://github.com/David610/singbox-vpn/actions/runs/36567137752 (job
`109401733938`) — **`two_hop_system`: 22/22, all passing.** Re-run once
more to rule out flakiness
(https://github.com/David610/singbox-vpn/actions/runs/36567137752, job
`109403322874`, via `gh run rerun --failed`) — 22/22 again, stable.

## A third, separate bug found and fixed while chasing full green

With `two_hop_system` passing, CI reached a step it had never reached
before on this branch: "vpn-admin doctor --protocol against a real live
sing-box + subscription process" (an exit-role live-handshake self-test,
unrelated to `two_hop_system`). It failed with
`[FAIL] [L2] exit node renders relay forwarding rules — role/renderer
mismatch` against a vanilla exit node with no probe user and no hairpin —
a state that must be unconditionally clean.

Root cause: `report_relay_policy` (`apps/admin/src/main.rs`) skips leading
`probe_confinement_rule_count` rules before judging an exit's
`route.rules` shape, but never skipped the C-16 egress-isolation rules
that `apply_c16_egress_policy` unconditionally prepends to every exit's
`route.rules` (resolve + 2x `ip_cidr` reject + the SMTP TCP/25 reject).
`server.rs` even has a `c16_egress_policy_rule_count` helper whose own doc
comment says it exists "so doctor can validate the rest of an exit's
document exactly as before" — nothing in `apps/admin` ever called it.
This check appears to have never run to a real pass on this branch since
C-16 landed: `main` (which predates this branch's C-16 work) never
exercises this exact code path, and on this branch `two_hop_system`'s
earlier failures kept CI from ever reaching this step until the fixes
above landed.

Fix (commit `ea0d104`): skip past the leading probe-confinement rules,
then the C-16 rules immediately after them (inline, not by calling
`c16_egress_policy_rule_count` directly, since that helper assumes C-16
rules start at index 0, which no longer holds now that probe-confinement
rules may be inserted ahead of them — see the root-cause-2 fix above),
before judging whether anything unexpected remains. Verified: the
"role/renderer mismatch" FAIL is gone in the next real CI run
(https://github.com/David610/singbox-vpn/actions/runs/36568597120, job
`109406615308`) — `Error: 8 check(s) failed`, matching `main`'s own
baseline count exactly (both showing only the expected CI-environment
`required group "..." does not exist` warnings, not this bug).

## Remaining, NOT resolved: `[L5-6]` protocol self-test reports INCONCLUSIVE

After the fix above, that same CI run still fails overall: the doctor's
`[L5-6]` line reports `protocol self-test INCONCLUSIVE: the client did
not return an HTTP success response through the live VLESS+REALITY
listener`, and both `singbox.log`/`subscription.log` are empty at
production log level (the actual rejection reason is not visible without
another diagnostic pass, same pattern as root cause 1/2 above).

This is a **separate, pre-existing issue outside this task's assigned
scope** (`two_hop_system` s03/s05/s12), on a CI step that had never
previously been reached on this branch for the same reason
(`two_hop_system` blocking the job earlier). It is not caused by either
fix in this addendum: root cause 2's fix only changes rule *order* for
relays with a loopback exception present, and this exit-role step never
provisions a probe user, so `apply_probe_user_confinement` is a no-op
here (`ids.is_empty()` returns immediately, confirmed by reading the
code); root cause 3's fix only changed which of the exit's *always-on*
C-16 rules `report_relay_policy` tolerates when reporting, a diagnostics
function that reads a document it does not itself construct — it cannot
affect the real client's actual connection outcome.

Best remaining diagnosis for a future pass: add the same kind of
test-scoped debug-log-level instrumentation used in root causes 1/2 (this
time to the `vpn-admin doctor --protocol` CI step's own `sing-box run`
invocation, e.g. temporarily rendering with `"log": {"level": "debug"}`
before the real, unmodified `render-config` output is used, or adding a
`--log-level` override flag scoped to this one CI step only) to see the
real rejection/acceptance reason for the doctor's own throwaway REALITY
client, the same way this addendum's root causes 1/2 were found. Given
this is a distinct problem from the assigned task and CI budget was
already spent getting `two_hop_system` itself to a stable 22/22 plus two
incidental production-bug fixes, this addendum stops here rather than
open-ending further into a second full diagnostic cycle, per this task's
own instruction to stop and document honestly rather than guess further.

## Current state

- `two_hop_system`: **22/22 passing**, verified in two separate real CI
  runs (job `109401733938` and, on rerun, job `109403322874`).
- `gh pr checks 123` (as of commit `ea0d104`): only `singbox-validate`
  fails, and specifically only on the `[L5-6]` doctor line described
  above — `codeql-rust` was still `pending` at capture time (unrelated,
  timing). Every other check (fmt, clippy, docs, `test`, all 7
  `os-matrix` targets, audit, license, secret-logging, shell,
  workspace-version-consistency, no-legacy-identity, codeql-actions,
  release-container-smoke) passes.
- PR #123 is **not yet fully green** — blocked on the newly-reached,
  pre-existing `[L5-6]` doctor self-test issue described above, not on
  anything this task was assigned to fix. Not merged, per instructions.

## Commits this pass (newest last)

1. `c86a3c3`, `01bb88a`, `2760e9b` — diagnostic-only REALITY log-level
   bump + CI step (removed once root-caused).
2. `3e51ac5` — root cause 1 fix: reserved-probe identity for the loopback
   control check in s03/s05/s12; diagnostic removal.
3. `8ad34ac` — root cause 2 fix: `apply_probe_user_confinement` ordering,
   real production bug.
4. `89d9b3a` — s12/s14 rule-count and idempotence fixups following (2).
5. `ea0d104` — root cause 3 fix: `report_relay_policy`'s exit-role check
   never accounted for C-16's always-on rules.
