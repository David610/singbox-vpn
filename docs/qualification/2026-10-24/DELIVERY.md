# Private-use release qualification, 2026-10-24

Decision: **NO-GO for stable release**. No VPS is accessible in this session
(owner confirmed). The owner will run physical-iPhone acceptance after the
repository work. No live configuration, credential, DNS or service was changed.

## Source and implementation

Baseline main: `c89b7289920635f7db2fa1915f841687857a0cf1`.
Its full CI and Security passed; those results do not qualify later changes.
PR #201 tightens release evidence, not the VPN transport: ten distinct Shorts,
at least 30 seconds of actual playback for each counted Short, repeated
switching, integer zero failures, and exactly one aggregate record per matrix
cell. Failed attempts cannot be hidden by a later duplicate PASS. Existing
900-second continuity, recovery and exact-runtime identity gates remain.
I-105 tracks this in the canonical generated registry. No playback fix is claimed.

Local baseline: 18 release-evidence tests passed. The new measurement regression
failed in 18 subcases before the validator fix; duplicate-cell regression failed
in four subcases before its fix. The corrected focused suite has 20 tests.
Full Python discovery on the complete checkout passed 234 tests before the
duplicate-cell addition; final checks and immutable CI results belong to the
PR and release workflow, not to historical qualification records.

## Findings and boundaries

- Historical accepted-IP hairpin browser success and the owner's older native
  report do not qualify today's signed-in or signed-out behavior.
- The [October 23 capture report](../2026-10-23/SHORTS-FREEZE-EGRESS.md)
  records 472/484 unanswered YouTube-named TLS handshakes from the relay versus
  working controls and 481/484 successes from the entry. It strongly supports
  upstream-path interference. It does not prove Selectel itself filters traffic,
  nor fully establish the native freeze's cause without an alternative-path test.
- Google/datacenter media restrictions are a separate historical observation;
  no current IP has been qualified here. Session/CDN/family selection remains
  unverified for the native signed-in/Incognito differential.
- Previous fixes regressed through hand-edited configs being reconciled away,
  credential/re-init transitions and repair reinstalling older release scripts.
  Source regression guards now cover these transitions. None proves deployed
  configuration or restart/reboot recovery on an inaccessible node.
- Keep sniff ordering, the complete Google domain set, literal-IPv6 `no_drop`,
  dedicated credentials and fail-closed routing. No direct fallback, WARP,
  unapproved residential egress or purchased VPS is introduced.
- I-86 login attribution is resolved: owner recognized the five historical
  sessions, including reconfirmation today. Current host health/trust and
  staging lifecycle are still unverified; attribution is not a host audit.

## Exact acceptance matrix for the owner

Use one immutable, verified corrected RC for every server/device check. Record
tag, source SHA, archive digest, running executable digest, routing fingerprint,
iPhone/iOS/Hiddify/core/YouTube versions and UTC windows privately.

| Transport | Network | Signed in | Completely signed out / Incognito |
|---|---|---|---|
| REALITY | Wi-Fi | NOT RUN | NOT RUN |
| REALITY | Cellular | NOT RUN | NOT RUN |
| Hysteria2 | Wi-Fi | NOT RUN | NOT RUN |
| Hysteria2 | Cellular | NOT RUN | NOT RUN |

For **each of the eight cells**:

1. Establish VPN-off controls with the same available Short and normal video.
2. Play at least ten distinct genuine Shorts, including the known failing
   control. Choose Shorts longer than 30 seconds and observe at least 30 seconds
   of advancing playback per Short, then switch repeatedly. Count all attempts,
   failures and actual distinct successes. A loop of one Short is not ten Shorts.
3. Observe at least 900 seconds continuous playback; test a normal video and
   seeking. Record startup failures and stalls rather than retrying them away.
4. Disconnect/reconnect, restart the client and YouTube, and confirm playback
   again. Repeat after reconciliation and server restart/reboot. Test Wi-Fi to
   cellular handover; verify intended two-hop/Google egress and no DNS/traffic leak.
5. Record `shorts_consecutive_played`, `shorts_distinct_played`,
   `shorts_min_playback_seconds`, `repeated_switching`, `observed_failures` and
   all pre-existing identity/recovery fields. Only zero failures qualifies.

Aggregate records must contain no cookies, tokens, signed media URLs, account
identifiers or browsing history. Evidence references require human review;
synthetic fixture data is never device evidence. Follow the existing
[runbook](../2026-10-22/RUNBOOK.md) and [release policy](../../RELEASE.md).

## Remaining execution order

1. Finish exact-commit CI/Security and merge only passing changes; produce a
   clearly labeled immutable RC through the normal release workflow if possible.
   A prerelease is for qualification, not an assertion of private-use readiness.
2. Once an authorized trusted VPS is accessible, preserve snapshot and verified
   rollback, then run the exact-artifact clean install/upgrade/repair, rollback,
   interruption and reboot harness. Verify both protocols, active config, DNS,
   IPv4/IPv6, isolation, credential revocation, renewal, backup/restore and alerts.
3. Qualify an owner-authorized alternative egress only if the existing path
   still fails. Use `deploy/lib/diag/candidate_qualify.sh` with pinned SSH host
   identity and a sustained control. Its browser/TCP PASS is only a screening
   result; repeat through the real two-hop VPN and the native matrix above.
4. Bind actual records to that artifact and validate with
   `deploy/lib/release_evidence.py --require PRODUCTION_READY --tag TAG --commit SHA`.
   Publish stable only after every mandatory gate passes; otherwise retain NO-GO.

No new server or egress purchase is authorized. The strictly necessary external
inputs are accessible authorized hosts, verified SSH identity/access and the
owner's physical-device observations. Infrastructure and native results cannot
be obtained by adding more mock tests.
