# Audit 2026-10-15 - Arcana VPN client import, reliability and release readiness

Written for the maintainers of `singbox-vpn`, `vpn-web` and `tamara-next`.

| File | Contents |
|---|---|
| `01_SYSTEM_INVENTORY.md` | repositories, the two subscription implementations, the disposable VPS, versions |
| `02_BASELINE_AND_REPRODUCTION.md` | what could and could not be reproduced, with steps and measurements |
| `03_ROOT_CAUSE_ANALYSIS.md` | per-issue verdicts (confirmed / not reproduced / hypothesis) |
| `04_CLIENT_COMPATIBILITY_MATRIX.md` | PASS/FAIL/PARTIAL/BLOCKED/NOT_TESTED per client |
| `05_NETWORK_AND_PERFORMANCE.md` | measurements, IPv6, YouTube, recovery |
| `06_SECURITY_PRIVACY.md` | verified properties, defects, what was not tested |
| `07_CHANGELOG_AND_REGRESSIONS.md` | commits, tests, verification |
| `08_RELEASE_ACCEPTANCE.md` | candidate identity, decision, round 2 (rc.2 server hardening) results, iPhone procedure |
| `issues.json` | machine-readable registry |
| `test-results/` | sanitized raw outputs (no tokens, keys, personal IPs or paths) |
| `harness/` | the scripts that produced the measurements (Windows paths, need a local sing-box 1.14.1) |

No physical iPhone was available. Nothing in these files claims an iOS pass.
