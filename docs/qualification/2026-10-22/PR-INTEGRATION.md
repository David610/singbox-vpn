# PR integration report

Read-only GitHub refresh on 2026-10-22 found five open singbox-vpn PRs.
All five patches apply to main `6825fcf491fd44e80543edc0b076c5b114cade16`.
This proves textual applicability only. No reviews, inline comments or issue
comments were returned for these five PRs. All are diverged; old green runs
do not qualify current-main integration.

| PR | Head SHA | Behind main | Old CI / Security | Disposition |
|---|---|---:|---|---|
| [162](https://github.com/David610/singbox-vpn/pull/162) | `03ffb69810c7d355f422c014bdb93ef319d91821` | 111 | [FAIL](https://github.com/David610/singbox-vpn/actions/runs/39787701523) / success | MERGE CANDIDATE, historical documentation only, after fresh combined checks |
| [148](https://github.com/David610/singbox-vpn/pull/148) | `cd4cfce3c40ab5150c5a5d3af8f96c775fd873af` | 111 | [success](https://github.com/David610/singbox-vpn/actions/runs/39191700707) / success | MERGE CANDIDATE after fresh combined checks |
| [161](https://github.com/David610/singbox-vpn/pull/161) | `fbcbc75f132a762f4e73bf63c28513b8ff91d607` | 111 | [success](https://github.com/David610/singbox-vpn/actions/runs/39190759809) / success | DEFER from minimum release until dependency qualification is executable |
| [160](https://github.com/David610/singbox-vpn/pull/160) | `ead10c8c77b8760e289bae2010b8722d604fbe93` | 111 | [success](https://github.com/David610/singbox-vpn/actions/runs/38972010722) / success | FIX FIRST, paired commercial-platform integration |
| [121](https://github.com/David610/singbox-vpn/pull/121) | `c403a69aab9cf831ea5229ff110a2fa4446c265d` | 322 | [FAIL](https://github.com/David610/singbox-vpn/actions/runs/36370449838) / success | DEFER; do not merge in this release |

The integration branch reconstructs #162 and #148 on current main, plus
I-84/I-88 qualification fixes. #161/#160 were applied only in separate local
review worktrees. #121 remains unapplied. Main was not changed. None is
unconditionally approved to merge; new CI and Security are mandatory.

## #162: evidence and remaining decoy defect

Changed files: five audit narratives, evidence README, outage JSONL, decoy
trace, and eleven soak evidence files (19 total). Recomputed from raw files:
8,340 + 9,000 new connections, zero reported failures; four long-lived
connections with 20,747 samples and zero reported failures; 90 + 90 positive
bulk results. RSS starts at 58,072 KiB, ends at 64,812, max 66,160; FDs
10 -> 16, max 22. There are 539 sampler rows spanning **5,390 seconds**.
That supports a nominal 90-minute lab run, not independently a >=5,400-second
production soak gate. Bulk byte results lack complete HTTP/status/hash proof.
The files substantiate their recorded counters, not independent authenticity
or performance on this candidate, a public network, or 100 paying users.

Twelve outage rows cover both transports, 5/20/40 seconds, twice each.
First new requests recover within 0.10–0.37 s after the outage. REALITY's
open-connection max is ~37–38 s for a 20 s outage and ~75 s for 40 s;
Hysteria2's open connection closes at 40 s. These are historical lab results.

The failed audit job `119855098429` names exactly RUSTSEC-2026-0382
(rustls-webpki 0.103.15) and RUSTSEC-2026-0381 (webpki-roots 1.0.9), plus
a yanked yoke-derive warning. Current main pins 0.103.16 and 1.0.10 and its
audit passes. This explains that failure, not a fresh audit of the integration.

W19 remains OPEN: the upstream 16,384-byte unparsed-decoy-buffer limit is
not fixed by using a smaller test certificate. The 30/30 guard runs and
negative 1.13.19 results are summarized in a trace file, not full per-run logs.
I-71's later abandoned-handshake flake is a distinct observation. No decoy
change was applied to a host. Main already contains the fixture/diagnostic
changes; #162 adds evidence, not an implementation fix.

## #148: supply chain

Three workflow files change only action pins: upload-artifact v7.0.2 and
CodeQL v4.39.0. GitHub tag lookup confirms upload-artifact commit
`cf430e030ddbb5b0abf93d22962f4752f3646cd9`; CodeQL's annotated v4.39.0 tag
resolves to `6f2ac81f5cd81fe351a197ba0b00d91d03cd2aee`. Full commit pins
remain. Token permissions, checkout behavior, artifact paths, publication
dependencies, checksums, SBOM, signing and attestation steps are unchanged.
Local release archive/SBOM/reproducibility contract tests pass after applying
the patch. Actual attested artifacts still require an authorized release run.

## #161 and #121: dependency scope

#161 changes Cargo.lock only. On a separate current-main worktree it retains
both corrected trust packages. Package count becomes 263 from 265; proptest
1.12 brings additional transitive changes (borsh/syn, rand 0.10 path), beyond
the six advertised updates. Pinned Rust is 1.94.1; compatibility of the whole
new graph, fmt/clippy/tests/audit and real interoperability has not been
rerun here because Cargo/Rust are absent and shell networking is broken.
No currently demonstrated blocker requires these updates. Rebase/reconstruct
against current main; never replace main's lockfile with the old PR's lockfile.

Subsequent isolated qualification: reconstructed #161 on current main at
`0010aadb65815026a9e0cf1d218e53bd3fae88ec` passed the full
[22-job CI run](https://github.com/David610/singbox-vpn/actions/runs/39827160202),
including pinned-toolchain fmt/clippy/build/tests/audit and real interoperability.
This removes the local toolchain execution blocker for that graph. It is still
optional and excluded from the minimum release; its standalone test branch did
not trigger a separate exact-commit Security workflow. Preserve that distinction
before adopting it or updating the original PR.

#121 changes the workspace rand requirement and lockfile without adapting
call sites using the older API. Its old build, clippy, docs and singbox-validate
jobs failed. Main already contains transitive rand 0.10 while application
code uses 0.8; those are separate requirements. No verified advisory requires
this breaking application migration. Defer it, or close and replace it later
with an intentional migration. No closure or Dependabot comment was sent.

## #160 paired with vpn-web #84

Eight agent files add shared pacing, event long polls, conditional v2
authorizations and schedule changes. Current main lacks these optimizations.
The paired [vpn-web #84](https://github.com/David610/vpn-web/pull/84) is open at
`bc278b7178b13fd1d6317cf2ee8c5d84423e99d0` (46 files). Fresh checks include
successful Postgres 16 migration replay/SQL tests and lint/test/build. Its body
still says scratch replay is needed; that is obsolete CI wording. **Deployment
and production migration application remain UNKNOWN.**

Inspection: agent GETs `authorizations?schema=2`, ACKs the exact revision;
ETag is retained only after parse/adoption and resets on process start or
invalid data. A 304 retains desired state and still enters local enforcement.
Server ETags include entitlement, revocation, validity and encrypted-secret
identity; missing batch entitlement RPC falls back to bounded concurrency 5
and retries capability detection after 60 s. Events are hints, with 60 s
default periodic reconciliation (clamped 15–600); absent/unhealthy channel
falls back to normal polling. Legacy unversioned server responses remain.
The missing wake-token RPC does **not** have the entitlement RPC's equivalent
fallback: event establishment fails until migration is installed, so test
the agent's polling fallback rather than assume events are live.

Confirmed FIX FIRST (I-87): `events.rs` applies 0.75–1.25 jitter to
`max(Retry-After, floor)` for HTTP 409; a 5 s floor may become 3.75 s.
Additional unresolved risks needing tests: in-flight event success resets the
shared breaker after a different endpoint's 429; sequential HTTP calls precede
local expiry, so the comment claiming poll-interval expiry bounds is not a
proof; malformed 200 event bodies/default empty fields can cause rapid polls;
node-auth cache and event sessions require a measured revocation bound.
These are not certified exploits or executed integration failures here.

Before adoption run old-agent/new-server and new-agent/old-server matrices,
with/without migration, 304/ETag changes, revoked/expired credentials, DB outage,
lost/superseded events, HTTP-date/delta Retry-After, restart/stale snapshots,
multiple API instances, and measured fleet request budgets. Preserve the
disabled provisioning-agent decision; no agent was activated.

## Dependency graph and recommended order

`main #163 -> (#162 evidence, #148 pins, I-84/I-88 fixes) -> exact combined
CI + Security -> owner merge -> authorized immutable rc.3 -> staging -> native
acceptance -> stable decision`.

#162 and #148 are independent; integrate #162 first, #148 second, then rerun
all combined gates. #161 is independent and deferred. #121 is independent and
excluded. Commercial branch: `scratch SQL replay -> owner-approved additive
migration -> vpn-web #84 deployment/fallback checks -> corrected #160 -> mixed
version/request-budget qualification`. No commercial prerequisite is silently
added to the locally managed standalone release.

Main's active ruleset has PR/no-delete/no-force-push rules but **no required
checks**. Classic branch-protection read is 403. Require the aggregate
`offline-full-verdict` and both CodeQL jobs before merge; tag protections are
active. Settings changes require owner approval.
