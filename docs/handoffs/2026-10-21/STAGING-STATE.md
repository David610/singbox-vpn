# Staging node state (62.238.46.190), end of session 2026-10-21

- Installed: development build (`update.sh --dev-rebuild`) of source `97ef78f`, tree byte-identical to its `git archive`; vpn-admin/vpn
  `0c64b801…`, vpn-subscription-svc `e19230fa…`. NOT a CI-built release; `install-state.json` still names v1.3.1-rc.2, so doctor WARNs
  "release identity INCONSISTENT" by design and `--repair` is refused (verified, binaries unchanged).
- Code qualification: 107/107 gates at 3570927 (1097 workspace tests). 97ef78f adds only a cfg(test) assertion message and the registry generator.
- I-54: egress table uses positive `meta skuid 995` rules (0 negated), gateway fe80::1 REACHABLE, external IPv6 0% loss.
- Doctor: 0 FAIL; REALITY and Hysteria2 loopback self-tests OK; UDP 68 now INFO (I-72); only WARN = release identity.
- Interrupted-update matrix: 16/16 twice at 268808d; update.sh/install.sh/recovery/harness are byte-identical at HEAD.
- Monitor: timer active; last real run SUSPECT/EGRESS_REFUSED (0/5 Shorts REFUSED, path controls OK). The staging egress is known-bad
  for Shorts, so CRITICAL after the usual escalation is the CORRECT result. A failed `vpn-shorts-health@exit-direct.service` unit is the
  monitor's designed way to surface it (open design question: it also makes `systemctl --failed` non-empty during an external outage).
- Pre-install test gate of `--dev-rebuild` failed once on two different tests across three attempts (I-71, I-75, both FLAKY, both kept as
  evidence); each time the updater installed nothing and left MainPID/NRestarts unchanged.
- Not on the node and not done: CI-built rc.3, fresh-install test, reboot, SSH key-only login, real Shorts, native clients.
