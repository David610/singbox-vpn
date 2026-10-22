# FLAKY results: definition and release-blocking policy (2026-10-21)

**Definition.** A test that failed at least once on a revision where the same code later passed. The first failure is evidence and is
kept (log, trace, environment). A clean retry does not erase it, does not make the gate PASS, and is never described as "fixed".

**Where it shows.** The registry state `FLAKY` (I-71 today); the release-gate input status `FLAKY` for the gate it affects.
`release_gate_report.py` treats a mandatory FLAKY gate as not passed: the verdict cannot reach READY while one exists.

**What blocks what.**
| Release | A mandatory FLAKY gate |
|---|---|
| any release candidate that will be installed on a production node, and every stable tag | **blocks**. Cleared only by the exit rule below |
| a prerelease tag used only to obtain the CI-built, attested artifact for staging (the rc.3 sequence R5/R6) | may proceed only with an explicit owner waiver recorded in the release notes, naming the test, the observed failure and the unresolved risk. A waiver is a limited-risk decision; it never raises a readiness level |

**Exit rule (all three).**
1. A mechanism is established from evidence (trace, instrumentation or a reproduction), not guessed.
2. Either the product is fixed with a test that fails on the old behaviour, or the test is shown to assert a false assumption and the
   assertion is corrected in a reviewed commit that says why. Weakening an assertion only to get green is not an exit.
3. The affected test then passes in the next three full qualification runs on three distinct revisions with no failure.

**Handling rules.** No endless re-runs hoping to reproduce: each extra run needs a stated purpose (instrumentation, a new hypothesis,
a changed load pattern). Record counts honestly (I-71: 1 failure; 25/25 isolated, 30/30 under CPU load, 6/6 full-suite runs, 1 passing
268808d qualification). Keep working on unrelated gates while it is open.

**I-71 today.** Mechanism partly established: the failing connection authenticated, received the whole certificate flight, then the
client closed before Finished (`readClientFinished() err: EOF`), and sing-box reports any incomplete handshake as
`processed invalid connection`. Why the client abandoned a probe connection is not established, and the same trace shows the test's
own relay succeeding, so this does not look like a REALITY/decoy interoperability defect - but that is a reading of one trace.
Next evidence step: the failure message now classifies each such line, so the next occurrence says whether it was a client-abandoned
authenticated handshake or something else. Status stays FLAKY until the exit rule is met.

**I-75 update (2026-10-21, later).** Exit rule 1 met: the mechanism is reproduced deterministically (a healthy unit whose `is-active` takes 150 ms
was declared failed because the verify budget was a wall-clock deadline the subprocess latency consumed). Rule 2 met: the budget is now a count
of observations, with a test that fails on the old logic and 3 neighbours pinning the bound. Rule 3 NOT yet met (three clean qualification
runs on distinct revisions), so the state stays FLAKY and the gate stays FLAKY.

**I-71 update.** Five reproduction conditions tried, none produced the failure; the client log and host load are now in the failure message.
The decoy test's assertion is intentionally unchanged: ignoring "authenticated, then the client closed" could hide a client that rejects the
12 KB certificate flight, which is exactly the historical defect class.
