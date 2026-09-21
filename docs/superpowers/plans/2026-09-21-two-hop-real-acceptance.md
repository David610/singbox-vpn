# Two-Hop (Privacy+) Real-Infrastructure Acceptance Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to run this plan step-by-step, with review checkpoints between phases. This is an infrastructure acceptance/verification plan, not a code-feature plan — most steps run real commands against real cloud VPSs rather than writing new source, so the usual TDD failing-test structure doesn't apply. Every step still has a concrete, runnable command and a checkable expected result; no step is a placeholder.

**Goal:** Take the existing, CI-verified two-hop relay implementation (role=relay/exit, Core `detour` routing, S1-S17 loopback test scenarios) from CODE-VERIFIED/CI-VERIFIED to SERVER-VERIFIED and DEVICE-VERIFIED, per this repo's own evidence ledger rules, so "Privacy+" (a real two-hop route on two separate real VPS providers) can be offered to users honestly.

**Architecture:** No new application code is written by this plan. It provisions two real VPSs (different providers, per the existing design rationale — provider/ASN separation is the point of a relay), installs singbox-vpn with `--role relay` on one and `--role exit` on the other, re-runs the existing S1-S17 scenario checklist against that real topology instead of loopback, adds the leak/packet-capture evidence loopback cannot provide, and records every result as a new dated entry in `docs/DEVICE_ACCEPTANCE_TESTS.md` using the vocabulary already defined there (SERVER-VERIFIED / DEVICE-VERIFIED / UNVERIFIED).

**Tech Stack:** singbox-vpn installer/CLI (`vpn-admin`), sing-box 1.13.19 (pinned, `deploy/lib/versions.env:25`), Tamara client (or a bare sing-box client for scenarios that don't need the app), Wireshark/tcpdump for packet captures, a DNS-leak-test tool run from the client device.

**Spec:** Implements item 9 of the "worth implementing" list agreed 2026-09-21. Directly reuses `docs/TWO_HOP_SYSTEM_TESTS.md` (loopback test design and the "Reusing the scenarios for real acceptance" section, lines 113-129) and `docs/DEVICE_ACCEPTANCE_TESTS.md` (evidence vocabulary, lines 9-26; existing "2026-09-13 — Privacy+ two-hop development gate" ledger entry, lines 527-537, which this plan's output supersedes for the two UNVERIFIED rows at lines 536-537).

## Global Constraints

- **This plan requires spending money on two real VPSs and requires the user's own cloud-provider credentials/payment.** Do not provision anything without explicit user confirmation of provider, region, and instance size at Task 1 — this is exactly the kind of hard-to-reverse, cost-incurring action the user must approve, not something to execute autonomously.
- Every result gets recorded in `docs/DEVICE_ACCEPTANCE_TESTS.md` using the file's own evidence vocabulary (`SERVER-VERIFIED`, `DEVICE-VERIFIED`, `UNVERIFIED`, `CODE-VERIFIED`/`CI-VERIFIED` as appropriate) — never a softer or inflated label, consistent with the [[evidence-labels-no-inflation]] rule already established for this project.
- The two VPS providers must actually be different companies (e.g. Hetzner + a non-Hetzner provider), not two instances from the same provider — provider/ASN separation is the entire point of the relay design (`docs/TWO_HOP_SYSTEM_TESTS.md:118`).
- Do not mark any row PASS or VERIFIED without the dated evidence the ledger format requires (date, commit/branch, what was actually observed) — partial runs get recorded as partial, per the existing entries' own style (e.g. `docs/DEVICE_ACCEPTANCE_TESTS.md:75-93`, the `compat=quic-reject` FAIL entry).

---

## File Structure

- `docs/DEVICE_ACCEPTANCE_TESTS.md` — append one new dated section (`### 2026-09-21 — two-hop real-infrastructure acceptance`, or the actual run date) with a results table, following the exact style of the existing `### 2026-09-13 — Privacy+ two-hop development gate` section at lines 527-537.
- No other files are modified. This plan produces evidence, not code changes.

---

### Task 1: Provision two real VPSs (user-approved)

**Files:** None (infrastructure only).

- [x] **Step 1: Confirm provider choice and cost with the user**

Before provisioning anything, present the user with two candidate providers (e.g. Hetzner for one, DigitalOcean or Vultr for the other — pick two that are NOT the same company), the region for each, and the monthly cost, and get explicit approval. Do not provision until approved.

- [x] **Step 2: Provision the relay VPS**

Create a VPS on provider A, AlmaLinux 9 x86-64 (the repo's supported OS per `docs/SUPPORTED_PRODUCT.md`), record its public IP.

- [x] **Step 3: Provision the exit VPS**

Create a VPS on provider B (different company than provider A), AlmaLinux 9 x86-64, record its public IP.

- [x] **Step 4: Verify SSH access to both**

Run: `ssh root@<relay-ip> 'cat /etc/os-release'` and `ssh root@<exit-ip> 'cat /etc/os-release'`
Expected: both print AlmaLinux 9.x.

---

### Task 2: Install singbox-vpn on both roles

**Files:** None (this reuses `install.sh` at the repo root, unmodified).

- [x] **Step 1: Install the exit node first**

Run on the exit VPS: `curl -fsSL <release-url>/install.sh | bash -s -- --role exit` (use the pinned current release per `deploy/lib/versions.env`, not an arbitrary branch).
Expected: install completes, `vpn-admin doctor` reports healthy.

- [x] **Step 2: Install the relay node**

Run on the relay VPS: `curl -fsSL <release-url>/install.sh | bash -s -- --role relay`
Expected: install completes; per `docs/TWO_HOP_SYSTEM_TESTS.md:22-23`, the relay is fail-closed (rejects everything) until paired with the exit.

- [x] **Step 3: Pair relay and exit**

Follow the pairing procedure documented in `docs/REACHABLE_FIRST_HOP_ARCHITECTURE.md` (declare the exit in the relay's `deployment.toml`, exchange credential B per `docs/TWO_HOP_SYSTEM_TESTS.md:42-47`).

- [x] **Step 4: Run the built-in protocol self-test on the relay**

Run: `vpn-admin doctor --protocol` on the relay VPS.
Expected: PASS — proves a real first-hop handshake, per `docs/TWO_HOP_SYSTEM_TESTS.md:24-26`.

- [x] **Step 5: Record install evidence**

Note the exact commit/release tag installed on each VPS, the date, and both doctor outputs — needed verbatim for the Task 5 ledger entry.

---

### Task 3: Re-run the S1-S17 scenario checklist against real infrastructure

**Files:** None — reuses the scenario table in `docs/TWO_HOP_SYSTEM_TESTS.md:84-107` as the checklist; execution is manual/scripted against real hosts instead of `cargo test --test two_hop_system`.

- [x] **Step 1: Provision a client**

Use a real device (phone or laptop) running the Tamara client, or a bare sing-box client if Tamara isn't ready, pointed at the relay's provisioning document.

- [x] **Step 2: Execute each scenario from the table, one at a time**

For each row S1 through S17 in `docs/TWO_HOP_SYSTEM_TESTS.md:88-104`, reproduce the same action against the real relay/exit/client instead of loopback, and record PASS/FAIL exactly as the "Expected" column states. Priority order (do these first, they're the ones that actually matter for a real "Privacy+" claim):
  - S2 (via route works end-to-end, fails when relay stops) — the core functional claim.
  - S6/S7 (exit unavailable / relay unavailable behave independently, no silent fallback to direct) — the leak-prevention claim.
  - S16/S17 (Privacy+ selection never lets the client connect to the exit directly, no Direct downgrade) — the single most safety-critical scenario, since a silent downgrade would defeat the entire point of choosing Privacy+.
  - S12/S13 (unpaired/undeclared targets rejected) — proves the relay doesn't become an open proxy.
  Remaining scenarios (S1, S3-S5, S8-S11, S14-S15, plus the three log-privacy rows) can follow once the priority set passes.

**Actual result:** only the priority set (S2, S6, S7, S12/S13, S16/S17) was
executed against real infrastructure — see
`docs/DEVICE_ACCEPTANCE_TESTS.md`'s 2026-09-22 entry for full evidence.
S1, S3-S5, S8-S11, S14-S15 and the three log-privacy rows were NOT re-run
against real infrastructure this pass and remain at their existing
loopback-only (CI-VERIFIED) status — checked off here because the step's
own text treats them as a follow-on, not a blocking requirement, not
because they were executed.

- [x] **Step 3: Capture packets at each hop**

Run `tcpdump` on both the relay and exit VPS during an S2 run. Confirm what `docs/TWO_HOP_SYSTEM_TESTS.md:122-123` calls out as not provable on loopback: what the relay observes (should see client IP + first-hop credential, never the exit's traffic content) and what the exit observes (should see only the relay's IP as source, never the real client IP). This is the actual proof that provider/ASN separation does what it claims.

- [x] **Step 4: Run DNS/IPv4/IPv6 leak tests from the client**

With Privacy+ selected, run a standard DNS-leak-test (e.g. dnsleaktest.com or equivalent CLI tool) and check both IPv4 and IPv6 egress from the client device. Record whether DNS/IPv6 leaks outside the tunnel — this is the same client-owned-behavior gap flagged as UNVERIFIED at `docs/DEVICE_ACCEPTANCE_TESTS.md:44`, so a leak found here is expected to also motivate the separate Tamara DNS/kill-switch/IPv6 plan (items 5-7), not a surprise specific to relay.

**Actual result:** only a partial, OS-level check was possible — no
admin-elevated packet-capture tool (`pktmon`/Wireshark) was available on
the test client. The Windows DNS client cache showed no entry for a
hostname resolved only through the tunnel, which is real but partial
evidence (see `docs/DEVICE_ACCEPTANCE_TESTS.md`). Full NIC-level DNS
capture and system-wide IPv4/IPv6 leak behavior remain UNVERIFIED and
require either admin-elevated capture tooling or a real TUN-mode client
(Tamara).

---

### Task 4: Latency/throughput sanity check

**Files:** None.

- [x] **Step 1: Measure baseline direct-route throughput and latency**

From the client, run a speed test and a ping/traceroute through the direct (non-relay) route.

- [x] **Step 2: Measure via-relay throughput and latency**

Same test through the Privacy+ (relay) route.
Expected: some latency/throughput cost is normal (extra hop) — record the actual numbers, don't editorialize about whether it's "acceptable," that's a product decision for the user once real numbers exist.

---

### Task 5: Record results in the evidence ledger

**Files:**
- Modify: `docs/DEVICE_ACCEPTANCE_TESTS.md` (append only — do not edit or delete the existing 2026-09-13 entry at lines 527-537; add a new dated section below it)

- [x] **Step 1: Write the new ledger section**

Append to `docs/DEVICE_ACCEPTANCE_TESTS.md`, following the exact table format of the existing "2026-09-13 — Privacy+ two-hop development gate" section (lines 527-537): one row per claim actually tested in Tasks 2-4, each with Claim / Layer / Status / Evidence-or-gap / Date / Commit-or-branch / Scope columns, using real dates, the real VPS provider names (or "provider A"/"provider B" if the user prefers not to name them in the doc), and the real commit/tag installed. Every row must be one of:
  - `SERVER-VERIFIED` — only for rows with dated evidence from the real VPS pair.
  - `DEVICE-VERIFIED` — only for rows with dated evidence from the real client device/network.
  - `UNVERIFIED` — for anything from the S1-S17 list not actually run in Task 3, or any leak/capture check not actually performed.
  Do not mark a row VERIFIED because the loopback test already covers it — the whole point of this ledger addition is that loopback evidence does NOT upgrade these rows (explicitly stated at `docs/TWO_HOP_SYSTEM_TESTS.md:129`).

- [x] **Step 2: Cross-check against `SUPPORTED_PRODUCT.md`**

If the results are good enough to change the "Relay (two-hop) routes are implemented but not yet a supported production path" line at `docs/SUPPORTED_PRODUCT.md:163-170` (i.e. S2, S6, S7, S16, S17 all pass with real packet-capture and leak evidence), propose that specific wording change to the user as a follow-up — do not silently upgrade the product-support claim in the same commit as raw acceptance data, keep evidence and product-claim changes as separate reviewable commits.

- [x] **Step 3: Commit**

```bash
git add docs/DEVICE_ACCEPTANCE_TESTS.md
git commit -m "docs(evidence): record two-hop relay real-infrastructure acceptance results"
```

---

## Self-Review Notes

- **Spec coverage:** covers item 9 in full — provisioning, pairing, the priority-ordered S1-S17 re-run, the leak/capture evidence loopback can't provide, and ledger recording per the project's own evidence rules.
- **Placeholder scan:** `<release-url>` in Task 2 is a real variable the executor fills from the actual current release, not a stand-in for missing design — the release process itself is already defined elsewhere in this repo (`deploy/lib/versions.env`, release workflow) and out of scope to re-derive here.
- **Deviation from standard task template noted deliberately:** this plan has no "write failing test, watch it fail, implement, watch it pass" cycle because there is no new source code — flagged explicitly in the header rather than forcing artificial test-writing steps onto infrastructure acceptance work.
