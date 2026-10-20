# Handoff 2026-10-20 — RECONSTRUCTED (staging VPS 62.238.46.190)

**Provenance.** The previous session's handoff directory was never committed
(not on `origin/main`, no branch, no history). This document is reconstructed
by the current session from version-controlled evidence plus fresh
measurements. Every claim carries an evidence class (`CLAUDE.md`): code
verified, server verified, browser verified, native device verified, user
reported, unverified hypothesis. Nothing here is promoted above its class.

## Scope and authorization (owner, 2026-10-20)
`62.238.46.190` / `fin1.xn----8sba1cvaif4e.com` is an **isolated staging and
acceptance host, not production**. No real user is to be switched to it before
acceptance. Other VPSs, DNS records, live credentials and subscription
infrastructure are out of scope without separate authorization.
Persistent host changes are logged in `docs/handoffs/2026-10-20/HOST_CHANGES.md`.

## Repository state
Base `de8f7d5` (main, 2026-10-16, v1.3.1-rc.2). Work branch
`staging/2026-10-20-remediation`. Stable tag v1.3.0; rc tags up to v1.3.1-rc.2.
Open branches/PRs other than `main`: none visible on the remote.

## Host facts (server verified, 2026-10-20)
Ubuntu 26.04.1, kernel 7.0.0-30, 2 vCPU, 3.7 GB RAM, 34 GB free. Public
IPv4 62.238.46.190, IPv6 2a01:4f9:c014:13bd::1/64. Fresh: no sing-box, nginx,
certificates, or singbox-vpn files before this session. DNS A =
62.238.46.190 on 1.1.1.1/8.8.8.8/9.9.9.9; **no AAAA record**.
Firewall: ufw inactive, iptables ACCEPT-all, nft empty. Provider firewall: unknown.

## Baseline YouTube measurements (browser verified; TCP only; headless Chromium, iPhone 13 emulation)
Probe: `deploy/lib/youtube_evidence_probe.py` (JSON in `evidence/`). PLAY requires
googlevideo media responses AND `<video>.currentTime` advancing.
Source address verified per family (`ifconfig.me` through the forced-family proxy).

| Path | Result |
|---|---|
| Direct IPv4, domain destination | true Shorts 0/3 (recorded 1/4 — see correction), normal videos 0/2 |
| Direct IPv6, domain destination | true Shorts 0/3 (recorded 1/4), normal videos 0/2 |
| Direct IPv4, repeat (2 trials) | true Shorts 0/6 (recorded 2/8), normal videos 0/4 |
| Direct IPv4, literal-IPv4 destination | Shorts 0/4 (stdout of `youtube_shorts_probe.py`, not saved as JSON) |
| Direct IPv6, literal-IPv6 destination | Shorts 0/4 |

Signature: `playabilityStatus: OK` with media requested but the clock stuck at
0, and on a normal video `LOGIN_REQUIRED "Sign in to confirm you're not a bot"`
— the same signature recorded historically for this IP
(`docs/YOUTUBE_FINAL_ROOT_CAUSE.md`). **Correction (same day):** the first probe version counted `dQw4w9WgXcQ` as a
Short. It is not one (YouTube serves it as an ordinary video), so the recorded
1/4, 1/4 and 2/8 overstated playback. The probe now counts an item as a Short
only if the page stays on `/shorts/` and reports other items separately. Recomputed
from the saved JSON, true Shorts played **0/3, 0/3 and 0/6**. That one ordinary
video playing still shows the harness can play media, and that this IP is not
blanket-blocked for media; the refusal is specific to Shorts/the bot wall. Caveats: no known-good control egress is available on this host, so
"refused" is inferred from the historical equivalence and the signature, not from
a same-run control; Short IDs may age out.

**Conclusion (browser verified, not device verified):** both address families
of this host are currently refused for Shorts. This is the Case D/C situation
in the task statement. Per `docs/YOUTUBE_FIX_INVARIANTS.md` a QUIC/MTU/DNS/transport
change cannot rescue it; the approved remedy is `google_egress_hairpin` through a
node whose IP YouTube accepts. **That needs owner authorization of a paired node
(historically Selectel; not contacted, status unknown).** Until then the
standalone staging install cannot be accepted for Shorts.

## Fixed this session (branch, CI-verifiable)
- I-37 (new) `vpn doctor` reported sing-box outbound UDP flow sockets
  (`ss -ulnp`, ephemeral ports) as FAIL "internal process on unexpected public
  port". Now INFO; non-ephemeral UDP and any TCP stay FAIL. Regression tests
  `public_surface_tests::own_process_*` (red before, green after).
  State: FIXED_IN_BRANCH, device requirement none.

## Security findings (server verified)
- sshd: `PermitRootLogin yes`, `PasswordAuthentication yes`.
- `/root/.ssh/authorized_keys` is **empty**: password is the only way in. Password
  hardening is therefore BLOCKED until the owner supplies a public key and a
  second-session key login is verified.
- Brute force in progress: 375 failed SSH password attempts in the journal from
  scattered scanners; only successful logins are root from 78.109.69.17 (assumed
  owner; confirm). No evidence of other successful logins in the journal.

## Open items / decisions required
1. Owner authorizes (or names) the paired Google-egress hairpin node.
2. Owner provides an SSH public key (then: key login test, then disable passwords).
3. Owner decides whether to publish an AAAA record (none today; not changed).
4. Real iPhone acceptance (Hiddify) remains UNVERIFIED.

See `ISSUE_REGISTRY.md` for the consolidated issue states.
