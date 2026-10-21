# Staging node state (62.238.46.190), end of session 2026-10-21

- Installed (updated later the same day): development build (`update.sh --dev-rebuild`) of source `19e93fd`, tree byte-identical to its `git archive`; vpn-admin/vpn
  `d82300d3…` (includes the I-75 fix), vpn-subscription-svc `e19230fa…`. (Earlier today: 97ef78f, `0c64b801…`.) NOT a CI-built release; `install-state.json` still names v1.3.1-rc.2, so doctor WARNs
  "release identity INCONSISTENT" by design and `--repair` is refused (verified, binaries unchanged).
- Code qualification: 109/109 gates at 19e93fd (1105 workspace tests, 0 failed/ignored), immutable export.
- I-54: egress table uses positive `meta skuid 995` rules (0 negated), gateway fe80::1 REACHABLE, external IPv6 0% loss.
- Doctor: 0 FAIL; REALITY and Hysteria2 loopback self-tests OK; UDP 68 now INFO (I-72); only WARN = release identity.
- Interrupted-update matrix: 16/16 twice at 268808d; update.sh/install.sh/recovery/harness are byte-identical at HEAD.
- Monitor: timer active; last real run SUSPECT/EGRESS_REFUSED (0/5 Shorts REFUSED, path controls OK). The staging egress is known-bad
  for Shorts, so CRITICAL after the usual escalation is the CORRECT result. A failed `vpn-shorts-health@exit-direct.service` unit is the
  monitor's designed way to surface it (open design question: it also makes `systemctl --failed` non-empty during an external outage).
- Pre-install test gate of `--dev-rebuild` failed once on two different tests across three attempts (I-71, I-75, both FLAKY, both kept as
  evidence); each time the updater installed nothing and left MainPID/NRestarts unchanged.
- Not on the node and not done: CI-built rc.3, fresh-install test, reboot, SSH key-only login, real Shorts, native clients.
