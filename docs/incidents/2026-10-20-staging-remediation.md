# 2026-10-20 — staging remediation findings (62.238.46.190)

Summary of what failed and why, in order found; details and evidence in
`docs/handoffs/2026-10-20/`.

1. **Shorts refused on both families** of the reused `fi1-exit` address (0 of 3,
   0 of 6, 0 of 3 true Shorts; literal v4/v6 0/4 each). Cause: source-IP refusal
   by YouTube, as in `docs/YOUTUBE_FINAL_ROOT_CAUSE.md`. Not fixable by transport,
   DNS, MTU or IPv6 changes (INV-2). Blocked on a hairpin target.
2. **Fresh install aborted on Ubuntu 26.04** (`useradd` exit 9): same-named
   groups leaked by root-run tests. Fixed (I-41); installer adopts the group.
3. **Acceptance script printed PASSED under FAIL lines** (I-43). Fixed.
4. **HSTS missing on subscription responses** despite the template setting it
   (nginx add_header inheritance) (I-44). Fixed; reaches old nodes only by
   installer re-run (I-45 open).
5. **Update test phase impossible on ufw hosts** (I-52). Fixed.
6. **A "verified" rollback that never ran**: the first lifecycle run reported the
   state "identical after rollback" although both updates stopped in the test
   phase before the switch. Caught by checking that the injection fired; the
   result was retracted and the harness now marks such runs INVALID.
7. **Baseline overcounted Shorts** by treating a non-Short video as a Short; fixed
   and recomputed (I-47).
8. **SSH**: password root login with no authorized keys and continuous
   brute-force. Tooling and rate limit added; hardening awaits the owner's key (I-39).
