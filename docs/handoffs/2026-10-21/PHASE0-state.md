# Phase 0 current-state report (session 3, 2026-10-21 05:2x UTC, read-only)

## Identity
- Host `test`, Ubuntu 26.04.1, kernel 7.0.0-30; eth0 62.238.46.190/32 + 2a01:4f9:c014:13bd::1/64; v4+v6 default routes present.
- Disk: / 35% used (24G free), /tmp tmpfs 1.9G free (3.6M used); inodes fine. No failed systemd units.

## Git
- `/root/singbox-vpn`, branch `staging/2026-10-20-remediation`, HEAD `019fd20`, clean tree, no stash, single worktree,
  **no upstream configured** (nothing pushed). Backups: `/root/backups/` (bundle + tarballs of 66f14f7, pre-be542e2 etc).
- Tags: latest published `v1.3.1-rc.2`; `v1.3.0` stable. rc.3 notes are drafts only.

## Running
- sing-box (443/tcp+udp), nginx (8443), vpn-subscription-svc (127.0.0.1:9100), sshd (22), ufw active
  (443/tcp, 443/udp, 8443/tcp, OpenSSH LIMIT; v4+v6). No updater/installer/harness/cosign/cargo processes. The only
  `claude` processes are this session's harness.

## Update / recovery state
- No transaction marker, no stale `/opt/.singbox-vpn-*`, no lock holder. State = **clean, healthy** (doctor: 0 FAIL).
- `install-state.json` says `v1.3.1-rc.2`, but the running binaries are a dev-rebuild of `3d372d5`
  (doctor WARN: release identity INCONSISTENT with recorded rc.2). Expected for staging; it must be re-installed from a
  real release candidate before any acceptance claim.
- IPv6 neighbour discovery to fe80::1 OK; egress-isolation table uses positive uid match (I-54 fixed on this host only).

## Interruption matrix: what the evidence actually says
- `matrix-final` (harness 4772d26c): 12/12 PASS (the handoff's headline).
- `matrix-final2` (harness 5bfdec43, later, same updater logic): **stage 10 FAIL**, 11/12.
  Same kill label ("writing install-state manifest") but this time the marker survived the kill; recovery rolled
  forward to a healthy, consistent 1.3.1 (creds same, doctor 0 FAIL, protocol rc 0, idempotent). The report judged it
  "not rolled back to baseline". In `matrix-final` the same stage ended with the marker already gone.
- Conclusion: stage 10 straddles the commit boundary and its outcome is timing dependent. The product behaviour looks
  correct (roll-forward) but the harness/report verdict is not deterministic => per policy this stage is **FLAKY**,
  not PASS. "Matrix complete" must NOT be claimed until the report classifies a post-commit kill correctly and a
  repeat run is stable on one commit.

## Registry notes
- IDs in the brief differ from the registry: the brief's "I-62 hyphen REALITY key" is **I-61** in the registry; I-62 there
  is the hairpin system test. Keep the registry authoritative; cross-reference in the final report.

## Open critical/high
I-54 (fixed on branch, only live-verified), I-56 (stage 10 flaky), I-38/I-20 (egress refused for Shorts, BLOCKED on
authorized good egress), I-39 (SSH passwords on; BLOCKED on owner public key), I-51/I-08/I-09 (Hiddify iOS, UNVERIFIED),
I-13, I-33 (installer double cosign download), I-10, I-19.

## Safe next action
Fix stage-10 classification (judge by commit evidence, not by baseline-only), add a unit test, rerun the full matrix.
