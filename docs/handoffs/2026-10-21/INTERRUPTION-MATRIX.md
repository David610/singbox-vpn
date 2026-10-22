# Interruption matrix — update.sh SIGKILL at 16 deterministic kill points (I-56)

Environment: STAGING VPS only (62.238.46.190). Target `v1.3.1-rc.2` published artifacts; updater under test = the committed
revision below, run from an immutable `git archive` export (read-only), harness hash recorded. Baseline = tar snapshot of a verified
v1.3.0 node (sha256 in `evidence/matrix-*/baseline.json`), restored before every stage.

**Revision under test: `3c971cc` (both runs).** Run A and run B: **16/16 PASS each; 0 differences between the runs** in
verdict, commit state, recovery direction, resulting release, orphans, locks, leftovers, backups, repair result.

How a stage proves itself (all recorded in `matrix.jsonl`): the updater pauses itself at the named checkpoint (test-only hook, inert
unless a root-owned 0700 directory is named) and is then SIGKILLed with its whole process tree (0 survivors);
the install-state manifest is read at the instant of the kill and must match the stage's claimed side of the commit
boundary (else INVALID); rollback must restore EXACTLY the baseline (version, binary hashes, manifest); a roll-forward must end on
ONE new release whose manifest hashes equal the installed vpn-admin/vpn/subscription binaries and the updater must say it rolled
forward; then: no marker, staging, manifest .tmp, extra backups, orphan processes, held lock or /tmp leak; credentials identical;
doctor 0 FAIL; real REALITY+Hysteria2 protocol self-test; a normal update, an idempotent re-update and a `--repair` all succeed.

| # | Kill point | Committed? | Recovery | Result (A / B) |
|---|---|---|---|---|
| 1 | downloading | no | nothing to undo | PASS / PASS |
| 2 | STAGE complete | no | nothing to undo | PASS / PASS |
| 3 | SWITCHING | no | switch not started, backup dropped | PASS / PASS |
| 4–9 | unit reload, mode, render, restart, health check, protocol handshake | no | rolled back to baseline | PASS / PASS |
| 10 | writing manifest (log) | no | rolled back | PASS / PASS |
| 13 | manifest .tmp written, not renamed | no | rolled back, .tmp removed | PASS / PASS |
| 14 | manifest renamed, marker present | yes | rolled FORWARD | PASS / PASS |
| 11 | agent binary refreshed | yes | rolled FORWARD | PASS / PASS |
| 15 | before backup removal | yes | rolled FORWARD | PASS / PASS |
| 16 | backup removed, marker present | yes | rolled FORWARD | PASS / PASS |
| 12 | UPDATE COMPLETE | yes | nothing to do | PASS / PASS |

## Defects this work found and fixed (each has a regression test that fails on the old code)
1. Harness/report: stage 10 outcome depended on timing; a marker-present kill after commit was judged against the wrong expectation.
   Fixed by deterministic checkpoints and by judging direction from the observed manifest.
2. **Product:** a verified rollback / "switch not started" / "PREPARE unfinished" recovery left the transaction backup in
   `/etc/vpn/backups` (one per interrupted update, unbounded growth). Now removed once the restore is verified (kept if it failed).
   (Failed run 1: stages 3–10 on this.)
3. **Product:** a kill just before the manifest rename left `install-state.json.tmp`. Recovery/rollback now remove it. (Failed run 1 stage 13.)
4. Recovery's commit evidence compared only vpn-admin's hash; it now checks vpn, vpn-admin and vpn-subscription-svc.

## Superseded / failed runs (kept in /root/evidence, not counted)
- `SUPERSEDED-...backup-count-bug`: harness compared against 0 backups; the baseline snapshot holds 3 older ones.
- `SUPERSEDED-...tmp-leak-counter-polluted`: counter included unrelated tmp.* dirs created by the operator's concurrent tests.
- `SUPERSEDED-...orphan-cosign-from-aborted-run`: an orphan cosign from an aborted run held the update lock; the harness correctly marked the stages INVALID, not PASS.
- `FAILED-matrix-run1-a532e54`: 7/16 PASS, the 9 failures were defects 2 and 3 above.

Not covered here: host reboot / power loss mid-update (UNVERIFIED, needs an owner-scheduled reboot), disk-full during update
(separate test not run), and the matrix targets the published rc.2 payload, not the corrected release candidate (that is tested next).
