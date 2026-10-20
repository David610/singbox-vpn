# Shorts health monitor (opt-in)

Detects the failure that no HTTPS/`playabilityStatus` check can: YouTube
withholding Shorts media from the exit's egress IP. It runs the real browser
probe against the path under test every 30 minutes with 3 genuine Shorts,
keeps aggregate counts only, and escalates on consecutive failures
(see `docs/operations/ACCEPTANCE_THRESHOLDS.md`): 1st miss SUSPECT, 2nd DEGRADED,
4th CRITICAL. A run that could not verify anything is UNVERIFIED and never OK.
Items that are not genuine Shorts are excluded from the count.

Requirements (not installed by the installer): `python3`, Playwright, Chromium.
Not enabled by default; nothing here starts a service on its own.

    systemctl enable --now vpn-shorts-health@exit-ipv4.timer   # after copying the units

Output: `/var/lib/singbox-vpn/shorts-health/shorts-health.jsonl` (one JSON line
per run) and the unit's exit status. Wire alerting to a failed unit or to
`status` in {DEGRADED, CRITICAL}. Recovery for a CRITICAL path is an owner
decision (hairpin target / new egress), never an automatic switch to `direct`
(INV-6). Evidence class: browser verified, TCP only; it does not replace the
native-device acceptance.
