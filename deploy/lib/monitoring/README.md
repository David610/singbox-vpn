# Shorts media-health monitor (opt-in)

Detects what no HTTPS / `playabilityStatus` / tunnel check can: YouTube withholding Shorts **media** from an egress IP.
Tunnel health and media health are reported separately, every run:

* **PATH** - a Google control and a neutral control are fetched through the path under test (direct, or the SOCKS of a
  throwaway tunnel client = what a user's traffic really takes). If a control fails the run is `PATH_DOWN` (DNS/CONNECT/TLS/
  TIMEOUT classified) and the browser is not started: a dead tunnel is never reported as "YouTube refused us".
* **MEDIA** - genuine Shorts (2 from a corpus + 3 discovered from the live feed, so no static id decides forever) are played in a
  mobile browser; a Short counts only if the page stayed on `/shorts/`, media was delivered and the clock advanced.
  Each failing Short gets one class: `BOT_CHECK`, `REFUSED`, `MEDIA_WITHHELD`, `STALLED`, `PROBE_TIMEOUT`, `PROBE_ERROR`;
  items that are not Shorts are `NOT_SHORT` and excluded. The run's `cause` is `EGRESS_REFUSED`, `PLAYBACK_STALLED`,
  `PATH_DOWN`, `HARNESS` (probe broken / too few genuine Shorts: not evidence against the egress) or `MIXED`.

Two streaks, never mixed: a **media** streak (real playback evidence against the egress: `EGRESS_REFUSED`, `PLAYBACK_STALLED`,
`MIXED`) and a **no-evidence** streak (`HARNESS`, `PATH_DOWN`: the probe could not run, too few genuine Shorts, the path was down).
A media failure advances the media streak and ends the other; a run without evidence advances only its own streak and leaves the
media streak as it was; OK clears both. A media streak not refreshed for 3 h is not "consecutive" with a new failure. Alerts are
deduplicated per class, so "the monitor is blind" can never mask "the egress is refused" (found live: a broken probe followed by the
first real refusal used to read as streak 2 and go straight to DEGRADED).

Escalation (`docs/operations/ACCEPTANCE_THRESHOLDS.md`): 1st miss SUSPECT (no alert), 2nd DEGRADED, 4th CRITICAL;
**RECOVERED** only after 2 consecutive OK runs. Alerts fire on transitions (6 h reminder), go to `SHORTS_ALERT_CMD` as JSON and are
always appended to `alerts.jsonl`; an undeliverable alert fails the unit (exit 4). Runs are rate-limited (>=5 min apart), serialized
by a lock, and nice'd; the default schedule is every 30 min.

There is deliberately **no automatic re-routing**. Moving Google traffic to another egress is an owner-approved action
(`docs/operations/YOUTUBE_EGRESS_ARCHITECTURE.md`, INV-6/7/8); on CRITICAL with cause `EGRESS_REFUSED` follow
`docs/runbooks` / the architecture record: qualify another target, never fall back to `direct`.

Deploy and verify (source-controlled; the installer/updater do NOT run this, the monitor is opt-in):

    sudo bash /opt/singbox-vpn/deploy/lib/monitoring/install-shorts-monitor.sh --label exit-direct          # install/refresh + enable
    sudo bash /opt/singbox-vpn/deploy/lib/monitoring/install-shorts-monitor.sh --check --label exit-direct  # read-only verification

`--check` fails (exit 1) if the installed units differ from the shipped ones (e.g. an older release's copy, or a hand edit pointing at a
scratch checkout), the probe prerequisites do not hold, the timer is not enabled/active, the last run is older than 2 h, or the monitor
is blind (status UNVERIFIED/PATH_DOWN: nothing is being verified). A CRITICAL/DEGRADED media verdict is printed as INFO: it is the
monitor doing its job about the egress, not a deployment fault. Run `--check` after every update (an update that refreshes the tree does
not refresh `/etc/systemd/system` units) and re-run the install step if it reports drift. Install refuses, installing nothing, when the
prerequisites do not hold.

Output in `/var/lib/singbox-vpn/shorts-health/`: `shorts-health.jsonl` (one JSON line per run: status, cause, path classes,
per-class counts, sampled public ids, seconds), `alerts.jsonl`, `<label>.state.json`. No cookies, tokens or user traffic are stored.
Evidence class: browser verified, TCP only (the probe disables QUIC); it never replaces the native-device acceptance.
