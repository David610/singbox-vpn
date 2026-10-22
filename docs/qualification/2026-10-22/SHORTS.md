# Shorts engineering report

**Root cause of the native signed-in/Incognito differential: UNKNOWN.**
No current native playback test or host capture was possible in this session.
No routing/transport/egress fix is claimed or deployed.

The last preserved native result predates fin1's rebuild: VPN-off Incognito
worked; REALITY signed-in worked; REALITY Incognito and Hysteria2 Incognito
failed. Device/client/app versions, exact executable identity and synchronized
flow evidence are incomplete. It is USER-REPORTED historical evidence, not
acceptance of current main. No required native cell is PASS for this candidate.

The strongest discriminator in that report is session mode across two
transports. Test session-dependent media/CDN decisions and session-dependent
destination/family selection first. Transport-only encapsulation defects are
less supported because both transports failed anonymously and REALITY worked
signed-in. That does not eliminate a common DNS/routing/client bug.

| Hypothesis | Evidence and decisive next comparison |
|---|---|
| Session/CDN/service restrictions, account-specific availability | Plausible from signed-in/anonymous difference. Same available Short, alternating sessions and VPN-off controls; do not collect cookies or tokens |
| Family selection or failed IPv4 fallback | Historical per-family browser differences exist, but no native failing-flow identification. Synchronized family aggregates and endpoint/DNS diagnostics; rejecting IPv6 is not proof of retry |
| Missing domain, literal-IP handling, sniff/SNI/ECH | Source has the shared domain set, sniff and literal-family guards. Exercise actual native destinations and both protocols; ECH/absent SNI remains a matching limitation |
| QUIC versus HTTP/2, UDP, transport behavior | Browser probes disable QUIC, so their results cannot settle native QUIC. Use bounded protocol diagnostics and a separately approved controlled change only after baseline capture |
| DNS/cache/client TUN inconsistencies | Compare app restart/reconnect and same network, record client DNS/family behavior; imported Hiddify route rules are not assumed effective |
| Direct versus approved hairpin | Older accepted-egress browser runs do not qualify a current target. Require explicit authorization, healthy existing pair and C1–C8 target qualification before any change |
| Anti-automation/IP-dependent response | Historical synthetic browser failures support an egress-dependent observation; 155-byte UMP alone does not establish an IP ban or explain native mode dependence |

Implementation reviewed: `crates/compat-config/src/{model,server,render}.rs`,
policy invariants/tests, `youtube_evidence_probe.py`, `youtube_shorts_probe.py`,
`monitoring/shorts_health.py`, monitor installer and `diagnostics/af_capture.py`.
No Google domain set, fail-closed route, sniff ordering, credential or renderer
behavior was changed. No third-party egress was used or contacted.

Evidence defects reproduced and corrected:

1. I-84: a synthetic complete release fixture with Incognito removed still
   became PRODUCTION_READY. Added regression tests fail on the old validator;
   fixed validator requires eight transport/network/mode cells, 10 consecutive
   Shorts each, known failing Short, 900 s continuity, long-form/seek/reconnect/
   app-restart/post-reconcile/egress proof, YouTube version and matching runtime
   binary digest. Truthy strings and invalid counts do not count. The fixture
   is synthetic test data, never native evidence.
2. I-88: a legacy aggregate containing 30 s of traffic could produce
   VALID_OVERLAP for a 600 s requested window. New boundary tests fail on old
   code; fixed analysis returns NO_OVERLAP_INCONCLUSIVE when coverage is missing.
   The one-bin coincidence control now uses a window covered by its fixture.
   Aggregate correlation still cannot identify an app, media or a specific user.
3. I-89: the monitor installer's historical classifier test depended on a Git
   object absent in shallow CI and silently substituted an empty file. A
   checked-in inert copy of the actual old classifier now reproduces its
   incorrect EGRESS_REFUSED result before installer refusal is tested.

Reproduction logs: `/workspace/audit-evidence/release-evidence-before.log`
(22 failing assertions/subtests) and `af-capture-before.log` (three longer
window subtests fail). Fixed focused suites: 18 and 16 tests respectively.
CI's existing shell/Python discovery executes these tests; mandatory full
workflow qualification remains separate.

The [runbook](RUNBOOK.md) gives the exact device and synchronized-window
procedure. Acceptance remains BLOCKED until the exact authorized RC is
installed and both transports pass signed-in and Incognito natively.
