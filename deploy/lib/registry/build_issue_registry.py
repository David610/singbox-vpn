#!/usr/bin/env python3
"""Build docs/handoffs/2026-10-20/issue-registry.{json,md} from the 2026-10-15
audit (carried issues, history preserved there) plus this session's issues
(NEW below). States: CONFIRMED_OPEN IN_PROGRESS FIXED_IN_BRANCH CI_VERIFIED
SERVER_VERIFIED DEVICE_VERIFIED PRODUCTION_VERIFIED BLOCKED UNVERIFIED FLAKY.
A carried issue keeps its audit status text; it is NOT promoted here."""
import json
import sys

AUDIT = "docs/audits/2026-10-15/issues.json"
OUT = "docs/handoffs/2026-10-20/issue-registry"
CARRY = {"open": "CONFIRMED_OPEN", "blocked": "BLOCKED", "fixed-on-branch": "UNVERIFIED",
         "fixed-on-main": "UNVERIFIED", "informational": "UNVERIFIED", "environmental": "UNVERIFIED"}


def n(id_, sev, comp, state, title, fix="", test="", device="no", risk="", evidence=""):
    return dict(id=id_, severity=sev, component=comp, state=state, title=title, fix=fix,
                regression_test=test, device_required=device, remaining_risk=risk, evidence=evidence)


NEW = [
    n("I-37", "medium", "admin/doctor", "FIXED_IN_BRANCH",
      "doctor reported sing-box outbound UDP flow sockets as FAIL; first heuristic could hide a mistaken inbound on an ephemeral port",
      "classifier uses the on-disk config's declared inbounds: declared->FAIL, undeclared ephemeral->INFO, unreadable config->WARN",
      "apps/admin public_surface_tests::* (red before/green after)", "no",
      "relies on the on-disk config matching the running process (doctor already checks drift)", "unit tests; full-suite run"),
    n("I-38", "high", "staging host egress", "BLOCKED",
      "62.238.46.190: both address families refused for Shorts (0/3,0/6,0/3 true Shorts; literal v4/v6 0/4)",
      "needs an owner-authorized good hairpin target (docs/operations/YOUTUBE_EGRESS_ARCHITECTURE.md)",
      "youtube_evidence_probe.py (live acceptance, not CI)", "yes", "no same-run known-good control on this host",
      "docs/handoffs/2026-10-20/evidence/direct-*.json"),
    n("I-39", "high", "staging host ssh", "IN_PROGRESS",
      "PermitRootLogin yes + PasswordAuthentication yes, empty authorized_keys, 447+ failed attempts",
      "deploy/lib/ssh-harden.sh (refuses to disable passwords without logged key-login proof); needs owner public key",
      "deploy/lib/tests/test-ssh-harden.sh (mutation-checked)", "no", "passwords still enabled until a key is supplied and proven"),
    n("I-40", "medium", "staging host firewall", "SERVER_VERIFIED",
      "no host firewall before install; provider firewall not visible",
      "installer enabled ufw (22,443/tcp,443/udp,8443/tcp); SSH now rate-limited (LIMIT)",
      "acceptance-test.sh ufw checks", "no", "provider/cloud firewall UNVERIFIED; IPv6 inbound untested"),
    n("I-41", "high", "installer", "SERVER_VERIFIED",
      "install aborts (useradd exit 9) when sing-box/vpn-subscription primary group already exists (leaked by root-run tests or interrupted install)",
      "create_service_users adopts the existing group with -g",
      "deploy/lib/tests/test-service-users-preexisting-group.sh (red before)", "no",
      "uninstall removes an adopted orphan group (intended)", "fresh install succeeded on Ubuntu 26.04 after fix"),
    n("I-42", "low", "tests", "FIXED_IN_BRANCH",
      "SSH-port tests depended on the host's real sshd/ss", "stub sshd/ss in test-installer-hardening.sh",
      "same test", "no"),
    n("I-43", "medium", "deploy/almalinux/acceptance-test.sh", "SERVER_VERIFIED",
      "printed 'PASSED (all required checks passed)' under FAIL lines; no ufw verification",
      "verdict counts non-required FAILs ('PASSED WITH N NON-REQUIRED FAILURE(S)'), ACCEPTANCE_STRICT=1, ufw rule checks",
      "deploy/lib/tests/test-acceptance-summary-honesty.sh (red against original)", "no",
      "Alma-only SELinux/firewalld checks still non-applicable on Ubuntu (now stated)"),
    n("I-44", "medium", "nginx subscription vhost", "SERVER_VERIFIED",
      "HSTS silently dropped from /sub/ and /v1/provision/ (nginx add_header inheritance)",
      "repeat HSTS (+Referrer-Policy) in each location", "deploy/lib/tests/test-nginx-location-headers.sh (red before)", "no",
      "reaches existing nodes only via installer re-run (I-45)", "live header observed on staging"),
    n("I-45", "medium", "update.sh nginx vhost", "SERVER_VERIFIED",
      "update.sh never re-rendered the nginx vhost, so template-level fixes (HSTS) did not reach deployed nodes",
      "deploy/lib/nginx-vhost.sh: shared renderer (install.sh uses it too) + transactional refresh (no-op if identical, nginx -t + reload, exact restore on failure, never creates a vhost); update_refresh_nginx_vhost on both update paths",
      "deploy/lib/tests/test-nginx-vhost-refresh.sh", "no",
      "published releases still need this updater; a failed refresh is a warning and keeps the old vhost; SELinux/port label changes are not handled",
      "staging: published rc.2 node, branch update --repair: HSTS 1->3, nginx -t ok, subscription acceptance 42/42"),
    n("I-46", "high", "release gate", "FIXED_IN_BRANCH",
      "stable release gate accepted a text 'Device acceptance: PASS' although the device matrix was waived",
      "deploy/lib/release_evidence.py levels CODE/SERVER/DEVICE/PRODUCTION; wired into release.yml for new stable tags",
      "deploy/lib/tests/test_release_evidence.py (13 tests)", "yes", "gate untested in a real release run; v1.0.0-v1.3.0 exempt by name"),
    n("I-47", "medium", "youtube probe", "FIXED_IN_BRANCH",
      "probe counted non-Shorts pages as Shorts (baseline overstated 1/4,1/4,2/8; true 0/3,0/3,0/6)",
      "count only pages staying on /shorts/; report others separately", "deploy/lib/tests/test_shorts_health.py", "no"),
    n("I-48", "low", "tests", "FIXED_IN_BRANCH",
      "five shell tests read the live host (/etc/vpn, /opt, /var/lib, /usr/local/bin) and fail on any installed host",
      "deploy/lib/run-shell-tests-clean-host.sh (mount-namespace overlay whiteouts); CI runners unaffected", "76/76 pass hermetically on staging", "no"),
    n("I-49", "low", "installer release identity", "CONFIRMED_OPEN",
      "local-tree installs are recorded as channel 'main' with a source-tree digest and no git commit",
      "record the git commit when available", "", "no", "weak provenance for staging/dev installs"),
    n("I-50", "low", "tests", "CONFIRMED_OPEN",
      "root-run Rust tests create system groups vpn-subscription/sing-box and never remove them",
      "I-41 removes the harm; a cleanup would still be cleaner", "", "no"),
    n("I-52", "high", "update.sh --dev-rebuild / test isolation", "FIXED_IN_BRANCH",
      "the updater's test-phase guard recorded the doctor's read-only `ufw status` as a refused host-control attempt, so --dev-rebuild could never pass its test phase on a ufw host",
      "allow ufw:status|version in deploy/lib/test-isolation.sh", "deploy/lib/tests/test-isolation-ufw-readonly.sh (red before)", "no",
      "prebuilt-release update path was not exercised", "found while running the update lifecycle on staging"),
    n("I-31", "high", "update.sh own-address backfill", "SERVER_VERIFIED",
      "update.sh did not record public_ipv4/public_ipv6: after v1.3.0 -> v1.3.1-rc.2 tunnel users could reach the node's own services (doctor: 'C-16: no own-public-address deny rule')",
      "deploy/lib/own-address.sh shared by install.sh/update.sh; update_backfill_own_address before every render",
      "deploy/lib/tests/test-update-backfills-own-address.sh (red before)", "no",
      "published artifacts still contain the defect until a corrected release is cut",
      "reproduced on published artifacts; fixed updater (--repair) on staging: keys written, C-16 OK, own-address reject rule rendered, doctor --protocol rc 0"),
    n("I-53", "high", "shell scripts and tests under set -o pipefail", "FIXED_IN_BRANCH",
      "intermittent failures of the shell suite. ROOT CAUSE: SIGPIPE under pipefail. `producer | grep -q` (or `| awk ... exit`, `| head`) makes the consumer exit at its first match; a producer that writes more than one chunk is killed with SIGPIPE and, with pipefail (+set -e), the pipeline reports 141 although the pattern WAS found. Reproduced on a static 10.7 KB file: 14/6000 wrong answers (here-string form 0/6000; 200-byte control 0/6000). It also hit PRODUCTION code: preflight.sh `sshd -T | awk ... exit` (SSH-port detection that gates firewall activation), install.sh's REALITY decoy TLS probe, my own ssh-harden.sh effective() (aborted AFTER writing the drop-in).",
      "306 echo/printf/cat|grep sites rewritten to here-strings (quote-aware, validated by bash -n, shellcheck and the suite); early-exit awk/head/sed/read after real commands get `|| true`; deploy/lib/lint_pipefail_pipes.py (rules R1-R3, reviewed allowlist) + CI-run guard test",
      "deploy/lib/tests/test-no-pipefail-sigpipe-race.sh (lint must be clean AND must detect 5 bad forms); test-ssh-harden.sh BIGOUT cases (red on the old code); controlled experiment: original tree under identical CPU load failed test-ssh-harden in 3/3 iterations, fixed tree 0 failures (see evidence/i53)", "no",
      "the lint cannot see every producer (a quoted `\"$x/cmd\" | grep -q` can still race); a retry-pass is now reported as FLAKY and exits 3 so it never counts as clean",
      "evidence/i53: idle original tree failed ssh-harden in 2 of 6 iterations; loaded original 3/3; loaded fixed see summary"),
    n("I-54", "critical", "nftables egress isolation (vpn-egress-isolation.service)", "SERVER_VERIFIED",
      "the host-layer table broke IPv6 neighbour discovery: 'meta skuid != UID accept' does not match socketless kernel packets, so ND to fe80::/10 and ff00::/8 was rejected; the whole host lost IPv6 after install/update (gateway FAILED, no IPv6 egress for anyone)",
      "positive uid-scoped rejects with counters in deploy/almalinux/nftables-egress-isolation.sh",
      "deploy/lib/tests/test-egress-isolation-ipv6-nd.sh (two-netns behavioural; red before, also proves uid scoping)", "no",
      "published v1.3.1-rc.2 and every earlier release carry the defect, and their update.sh --repair/--version REINSTALLS the broken script (observed on staging: the live fix was reverted by a later repair and IPv6 died again until re-applied) so only a corrected release fixes nodes durably",
      "staging: IPv6 dead with the table, restored at once without it; after the fix a flushed-cache ND succeeds, v6 egress and YouTube/v6 HTTP 200. Detection: doctor now FAILS on the negated-uid table form itself (classify_egress_isolation_table) and reboot_check.py asserts uid scoping"),
    n("I-55", "high", "admin/doctor", "FIXED_IN_BRANCH",
      "doctor had no IPv6 health check: it passed (rc 0) while the host's IPv6 was completely broken by I-54",
      "WARN when a global IPv6 address exists and the default gateway's neighbour entry is FAILED (classify_host_ipv6)",
      "apps/admin public_surface_tests::host_ipv6_* (unit); live check reports OK on staging", "no",
      "WARN not FAIL (IPv6 is only mandatory when an AAAA record exists); the broken state itself was exercised by hand, not by an automated end-to-end test"),
    n("I-56", "high", "update.sh interrupted-transaction recovery", "SERVER_VERIFIED",
      "SIGKILL of a real v1.3.0 -> v1.3.1-rc.2 update left the node with rc.2 binaries and v1.3.0-shaped state (doctor FAIL), every tool refusing (updater: marker; recovery helper: unhealthy host), recovery = hand-restore from backup_dir; a kill during the download left a staging directory with NO marker that blocked every later update",
      "update.sh --recover: strict marker validation, no-snapshot => clear only, healthy+untouched => clear only, else restore from the transaction backup via the shared perform_rollback_from_backup, clear stale staging (never -prev- dirs); recover-interrupted-update.sh kept as a conservative helper",
      "deploy/lib/tests/test-update-recover-mode.sh, test-recover-interrupted-update.sh (adversarial: symlinks, .., newline names, wrong owner, malformed markers, all-or-nothing); dynamic: evidence/interrupt/ (SIGKILL at each of 10 stages)", "no",
      "matrix proved on staging against the published rc.2 payload, not against a CI-built corrected release; host reboot/power loss mid-update and disk-full during update are UNVERIFIED; published releases do not contain --recover (a corrected release is required)", "docs/handoffs/2026-10-21/INTERRUPTION-MATRIX.md; evidence/matrix-A and matrix-B: 16 deterministic SIGKILL stages, 16/16 PASS twice on 3c971cc, identical results"),
    n("I-33", "low", "installer / updater scratch files", "FIXED_IN_BRANCH",
      "the installer downloaded its pinned cosign (~135 MB) twice into bare mktemp dirs that survived failures and SIGKILL (exhausted /tmp)",
      "bootstrap and installer each use one cleaned workspace (EXIT trap + owner-pid sweep of dead runs); the bootstrap's cosign is reused after its digest is re-checked against the installer's own pin; source extracted into its own dir; updater scratch lives under the cleaned staging root",
      "deploy/lib/tests/test-installer-scratch-cleanup.sh (7 checks red before)", "no",
      "real fresh install with the changed installer not yet re-run on the staging host", "hermetic test + clean-host shell suite 87/87"),
    n("I-63", "high", "update.sh recovery/rollback", "SERVER_VERIFIED",
      "a verified rollback or a recovery that finds the switch never started left the transaction backup in /etc/vpn/backups (one per interrupted update, unbounded)",
      "the backup is removed once the restore is verified (kept if it failed)", "deploy/lib/tests/test-update-recover-dynamic.sh (red before); matrix stages 3-10", "no",
      "none known", "matrix run 1 failed 3-10 on this; runs A and B pass"),
    n("I-64", "medium", "update.sh commit boundary", "SERVER_VERIFIED",
      "a kill just before the manifest rename left install-state.json.tmp after recovery",
      "recovery and in-process rollback remove it", "test-update-recover-dynamic.sh (red before); matrix stage 13", "no", "none known", "matrix run 1 stage 13"),
    n("I-65", "high", "update.sh --repair", "SERVER_VERIFIED",
      "--repair re-installs the RECORDED release; after a corrected or dev build it silently replaced the fix with the old material (the way I-54 came back)",
      "--repair refuses when vpn-admin/vpn/subscription hashes differ from the manifest; explicit --force-repair-recorded-release overrides", "deploy/lib/tests/test-update-repair-drift-guard.sh (red before: it started downloading rc.2); live refusal verified on the staging node", "no",
      "published rc.2 updaters do not contain the guard", "live: refused, node unchanged"),
    n("I-66", "medium", "update.sh --dev-rebuild", "SERVER_VERIFIED",
      "--dev-rebuild from a tree other than /opt/singbox-vpn paired new binaries with the installed tree's stale scripts; the corrected doctor then (rightly) failed the update on the old nftables table",
      "dev-rebuild refuses to run outside the installed tree", "test-update-repair-drift-guard.sh case 4", "no", "none known", "staging attempt 2 of the candidate install"),
    n("I-67", "low", "tests", "FIXED_IN_BRANCH",
      "test-installer-signal-rollback.sh hung the whole suite forever when started as an async job (SIGINT inherited as ignored cannot be trapped)",
      "fails fast with a clear message; waits are bounded", "the test itself", "no", "-", "observed during the clean-host run"),
    n("I-68", "high", "youtube_evidence_probe.py / Shorts monitor", "SERVER_VERIFIED",
      "the Shorts evidence probe crashed with KeyError('path') on every run (one() read a page-state key the page script never returned), so the shipped opt-in Shorts monitor could not have detected anything; the monitor also could not tell a dead tunnel from a refused egress, had no alert path, no failure classes and a static 3-id list",
      "STATE_JS returns the path; monitor rewritten: PATH vs MEDIA health, per-Short classes and causes, discovery from the live feed, alert transitions with 2-OK recovery, rate limit + lock, PROXY_<label> wired",
      "deploy/lib/tests/test_shorts_health.py (26 tests, 6/6 mutations caught, contract test red before the fix); live: SUSPECT -> DEGRADED -> CRITICAL with delivered alerts on the refused staging egress", "no",
      "recovery path proven hermetically only (no good egress exists to recover to); not installed by the installer; no automatic re-routing by design (owner decision)", "docs/handoffs/2026-10-21/evidence/monitor-live-*.jsonl"),
    n("I-57", "low", "preflight_is_public_ip", "FIXED_IN_BRANCH",
      "192.88.99.0/24 (deprecated 6to4 anycast; also the addresses of the test suite's leaked arcana-2hop0 dummy interface, see I-59) counted as public host addresses, producing a spurious 'more public IPv4 addresses than deployment.toml can record' warning on every install/update",
      "exclude 192.88.99.0/24 in preflight_is_public_ip", "deploy/lib/tests/test-own-address-detection.sh (red before)", "no"),
    n("I-58", "medium", "configuration contract / drift", "FIXED_IN_BRANCH",
      "no version-controlled production contract or drift check; routing/ssh/host/port drift was only found by hand",
      "deploy/contracts/fin1-staging.contract.json + deploy/lib/check_deployment_contract.py (read-only; PASS/FAIL/UNDECIDED/UNOBSERVABLE; routing fingerprint)",
      "deploy/lib/tests/test_deployment_contract.py (12 tests)", "no",
      "contract still has UNDECIDED fields awaiting owner approval; not yet wired as a deployment gate", "live run on staging: 11 PASS, 2 FAIL (SSH), 7 UNDECIDED"),
    n("I-59", "high", "workspace tests (two_hop_system) / update --dev-rebuild", "SERVER_VERIFIED",
      "the two-hop system tests create a dummy interface arcana-2hop0 with 192.88.99.2-6 as root and NEVER delete it (a process-wide OnceLock is not dropped at exit); update.sh --dev-rebuild runs them on the live node, leaving the interface and host routes permanently (found on the staging node; it also polluted own-address detection)",
      "reference-counted NetLease: the last test to finish deletes the interface; CI step fails if it survives a test run",
      "CI step 'the test run leaves no dummy interface behind'; local: interface absent after the 22-test run (was present after every run before)", "no",
      "SIGKILL of the test process still cannot clean up (documented limit); nodes already polluted must delete the interface by hand (ip link delete arcana-2hop0)"),
    n("I-60", "medium", "REALITY connection setup (performance)", "CONFIRMED_OPEN",
      "every new VLESS+REALITY connection adds ~60-80 ms (Hysteria2 ~2.5 ms): the server completes a TLS handshake with the decoy per connection. Isolated: remote decoy +80.9 ms vs local decoy +9.7 ms in otherwise identical servers",
      "owner decision: serve the decoy from the node (re-import of all profiles + restart) or accept; docs/operations/REALITY_DECOY_LATENCY.md",
      "deploy/lib/tunnel_perf.py (added-latency gate <= 40 ms is expected to FAIL for REALITY with a remote decoy)", "no",
      "provisional 40 ms threshold fails for REALITY by design of the remote decoy; changing the decoy is service-affecting", "experiment recorded in the doc; run evidence/tunnel-perf-*.json"),
    n("I-61", "medium", "vpn-admin google-egress-hairpin set", "FIXED_IN_BRANCH",
      "--relay-reality-public-key rejects any key starting with '-' (clap read it as a flag): ~1 in 64 valid REALITY keys; found when the hairpin simulation failed to set up",
      "allow_hyphen_values on that argument; docs use the --opt=value form (works on every version)",
      "apps/admin public_surface_tests::hairpin_set_accepts_a_reality_public_key_that_starts_with_a_hyphen (red before)", "no",
      "published releases still carry the bug; use --relay-reality-public-key=<key>"),
    n("I-62", "medium", "Google hairpin (system test)", "FIXED_IN_BRANCH",
      "no end-to-end test of the hairpin existed (only render-level tests)",
      "deploy/lib/hairpin_simulation.py: real vpn-admin renderer + real sing-box in a throwaway netns; 22 checks (whole domain set via the target, lookalikes stay direct, target down fails closed with no direct fallback, recovery, wrong credential) and two mutations that must fail; wired into CI",
      "evidence/hairpin/clean-22-of-22.json; mutation-*-must-fail.json", "yes",
      "SIMULATED: proves routing and fail-closed only, nothing about any real address's YouTube acceptance (C1-C8 in HAIRPIN_TARGET_QUALIFICATION.md). Verified by local runs (22/22; both mutations fail); the CI job was added but has NOT executed yet (nothing is pushed)"),
    n("I-69", "high", "Shorts monitor state machine (shorts_health.py)", "FIXED_IN_BRANCH",
      "one shared streak counter was advanced by runs that produced NO media evidence (probe could not start, too few Shorts, path down) as well as by real refusals: a broken probe followed by the first real refusal read as streak 2 and went straight to DEGRADED (policy: consecutive MEDIA failures), and a HARNESS alert at a level suppressed the egress alert at the same level",
      "separate media and no-evidence streaks (a run without evidence leaves the media streak untouched; a media failure ends the other streak; OK clears both), per-class alert dedup, media streak expires after 3 h",
      "deploy/lib/tests/test_shorts_health.py: 8 new tests red before / green after; 3 mutations (class collapse, shared dedup, no expiry) each fail them; the old test that asserted the mixed count was wrong and was replaced", "no",
      "found live on staging (probe had no playwright: UNVERIFIED then a real refusal -> DEGRADED at the first refusal); the fix is committed but the installed staging tree still runs the previous monitor until the next verified rebuild; recovery to HEALTHY with a real good egress remains unverified",
      "docs/handoffs/2026-10-21/evidence/monitor-live-*.jsonl; shorts-health.jsonl on staging"),
    n("I-70", "medium", "Shorts monitor lifecycle", "CONFIRMED_OPEN",
      "the monitor is opt-in and nothing in install/update/repair deploys, refreshes or checks it: on staging its unit had been hand-edited to run from a scratch checkout (/root/monitor-src/<old commit>) and the probe's playwright/browser prerequisite was undocumented, so it ran as UNVERIFIED/HARNESS (exit 3, visibly, never HEALTHY)",
      "docs: prerequisite env file and the rule that the unit must point at the installed tree; staging unit reinstalled from /opt/singbox-vpn with /etc/vpn/shorts-health.env",
      "", "no",
      "no automated check that the monitor timer is active, that its last run is recent, or that its unit matches the shipped one; an update can leave an old monitor in place. Needs an owner decision on whether the installer should deploy it",
      "this session's staging restore; deploy/lib/monitoring/README.md"),
    n("I-71", "medium", "crates/compat-config tests/reality_decoy_budget.rs", "FLAKY",
      "historically_oversized_decoy_certificate_remains_usable failed once in the pre-install test phase of a staging --dev-rebuild (2026-10-21 10:40:53Z): the server logged `processed invalid connection` for one connection",
      "no product change: the saved trace shows connection 49610 AUTHENTICATED, the server sent the whole certificate flight, then `readClientFinished() err: EOF` - the client closed before Finished (a probe/urltest connection abandoned 5 ms in) while the test's own relay connection succeeded. The test's assertion treats any such line as a decoy-budget regression, which is a likely false assumption, but the reason the client abandoned it is NOT established. Added diagnosis to the failure message (assertions unchanged) so a recurrence classifies itself",
      "reality_decoy_budget.rs::diagnosis_recognises_a_client_abandoned_authenticated_handshake (diagnostic only)", "no",
      "NOT reproduced in ~150 runs under five conditions (25 isolated, 30 under 6 CPU burners, 6 full compat-config suites, 72 as 6 concurrent processes, 15 cold-cache) plus 3 clean qualifications. Why the client closed an authenticated handshake is unexplained and a client-side rejection of the 12 KB flight is NOT excluded, so the assertion is deliberately not loosened. The failure message now carries the client trace log and host load. Release policy: docs/handoffs/2026-10-21/FLAKY-POLICY.md",
      "docs/handoffs/2026-10-21/evidence/staging-268808d/candidate-268808d-devrebuild.log (first failure, kept)"),
    n("I-72", "low", "admin/doctor", "FIXED_IN_BRANCH",
      "doctor WARNed about systemd-networkd's DHCPv4 client socket (UDP 68 bound to the interface address) as an unexpected public listener",
      "narrow INFO case: UDP, port 68, owner systemd-network, interface-bound address (addr%ifname); every other unexpected listener still WARNs",
      "apps/admin public_surface_tests::dhcpv4_client_socket_* and udp_68_is_only_benign_*: 2 mutations (exemption removed, owner check dropped) each fail them", "no",
      "the installed staging binary predates this change, so the live doctor still shows the WARN until the next rebuild", "ss output on staging; netplan DHCP=ipv4"),
    n("I-73", "medium", "process discovery/termination in scripts", "FIXED_IN_BRANCH",
      "command-line-substring process matching (pgrep -f) can return the calling shell, a watcher or an unrelated process: the benchmark re-found the client it had itself started, uninstall decided CRITICAL residue from an unanchored match, and in this work a watcher loop reported a finished run as running",
      "deploy/lib/proc-safe.sh (anchored full-argv match, never the caller/ancestors/PID 1, identity-verified kill); benchmark uses the PID it started; uninstall uses the helper; the existing no-broad-kill guard now also rejects unanchored pgrep -f",
      "deploy/lib/tests/test-proc-safe.sh (protected-PID checks run in a disposable tree) and test-no-broad-process-kills.sh; mutations: substring match, removed self-guard, reintroduced pgrep -f", "no",
      "the interruption-matrix harness already tracked exact PIDs and was not changed; operator-side ad-hoc commands are outside the repo", ""),
    n("I-74", "low", "shell fixture tests (temp hygiene)", "CONFIRMED_OPEN",
      "a clean-host shell qualification run left 92 mktemp directories (91 empty) in /tmp; the test that creates them is not identified",
      "", "", "no", "tmpfs only (7 MB), cleared at reboot; harmless but unbounded growth across runs would not be", "ls /tmp after the 268808d qualification"),
    n("I-75", "medium", "apps/admin service.rs reload_and_verify", "FLAKY",
      "a_burst_of_administrative_reloads_is_never_refused_by_the_start_limit failed once in the pre-install test phase of a staging --dev-rebuild (reload 15 of 20). Root cause (now REPRODUCED DETERMINISTICALLY): reload_and_verify bounded its verification by a wall-clock deadline (settle*10) that the is-active subprocess latency itself consumed, so a HEALTHY unit was declared 'did not remain active' when polls were slow (and with slow polls it gave up after 1-2 observations). In production (settle 300 ms) the same defect needs polls > ~700 ms, but its effect is to roll back a legitimate configuration change",
      "the bound is now a COUNT: three consecutive healthy observations within at most 10 polls, independent of subprocess latency; the failure message carries the underlying error",
      "apps/admin service::tests: slow_is_active_spawns_do_not_make_a_healthy_service_fail_verification (150 ms is-active; red before), a_unit_that_never_comes_up_is_abandoned_after_a_fixed_number_of_observations, a_flapping_unit_never_verifies, a_slow_starter_is_accepted_within_the_bound; mutation 'restore the deadline' fails 4. The original burst test is unchanged", "no",
      "state stays FLAKY by policy until three further full qualification runs on distinct revisions are clean (FLAKY-POLICY.md exit rule 3); the exact original failure was never captured with its error text",
      "docs/handoffs/2026-10-21/evidence/staging-3570927/devrebuild-attempt1-FAILED.log"),
    n("I-51", "high", "Hiddify iOS (client)", "UNVERIFIED",
      "profile JSON missing from Library/Caches/Working/configs (carried as I-08); upstream hiddify-app#2372 open, no fix",
      "upstream proposal in docs/clients/HIDDIFY_IOS.md; server cannot fix. Control experiment added (iPhone procedure step 7b) to locate the failure boundary between an app-wide fault and a profile-specific one", "", "yes",
      "trigger on the owner's iPhone unconfirmed"),
]


# Carried audit issues re-assessed this session WITH evidence. Nothing is promoted
# without a pointer to the evidence that supports the new state.
UPGRADES = {
    "I-12": ("SERVER_VERIFIED", "Hysteria2 inbound renders ignore_client_bandwidth=true with no declared caps (the shipped default) and standard clients are NOT locked out: pinned sing-box 1.14.1 clients and the real Hiddify core v5.0.3 (hiddify-sing-box 1.13.0-rc.2) both connect over Hysteria2 with Salamander (evidence/staging-subscription-acceptance*.json, evidence/hiddify-core-v5.0.3-acceptance.json). Not tested: a deployment with declared bandwidth caps."),
    "I-14": ("SERVER_VERIFIED", "doctor reports FAILS=0 and C-16 OK on nodes with public_ipv4/public_ipv6 recorded (staging, dev install and rc.2 + backfill)."),
    "I-15": ("SERVER_VERIFIED", "fresh install with a DNS public_host records the node's own addresses; own-address isolation proven through real REALITY and Hysteria2 tunnels (evidence/c16-isolation-through-tunnel.json)."),
    "I-16": ("SERVER_VERIFIED", "re-running install.sh on a dev install used the new source tree (the vhost template fix, absent from the old tree, was applied; evidence/lifecycle/)."),
    "I-17": ("SERVER_VERIFIED", "doctor on a dev-channel install reports release identity OK with all three executables matching the recorded hashes."),
    "I-18": ("SERVER_VERIFIED", "doctor --protocol --require-protocol now runs both a REALITY and a Hysteria2 self-test (both lines present and passing on staging)."),
}


def main():
    audit = json.load(open(AUDIT))
    items = audit if isinstance(audit, list) else list(audit.values()) if "id" not in audit else [audit]
    if isinstance(audit, dict) and "issues" in audit:
        items = audit["issues"]
    carried = [dict(id=i["id"], severity=i["severity"], component=i.get("repo", ""), state=UPGRADES.get(i["id"], (CARRY.get(i["status"], "UNVERIFIED"),))[0], evidence_2026_10_20=UPGRADES.get(i["id"], (None, ""))[1],
                    title=i["title"], audit_status=i["status"], fix=i.get("fix", ""), note=i.get("note", ""),
                    source="docs/audits/2026-10-15/issues.json (not re-verified this session)") for i in items]
    newids = {x["id"] for x in NEW}
    for x in NEW:
        x["supersedes_audit_entry"] = x["id"] in {c["id"] for c in carried}
    carried = [c for c in carried if c["id"] not in newids]
    reg = {"schema_version": 1, "generated_for": "2026-10-20 staging remediation", "new": NEW, "carried": carried}
    json.dump(reg, open(OUT + ".json", "w"), indent=1)
    md = ["# Issue registry (generated from issue-registry.json — do not edit by hand)\n",
          "States: CONFIRMED_OPEN, IN_PROGRESS, FIXED_IN_BRANCH, CI_VERIFIED, SERVER_VERIFIED, DEVICE_VERIFIED, PRODUCTION_VERIFIED, BLOCKED, UNVERIFIED, FLAKY (a failure that was observed and not reproduced stays FLAKY; a clean retry never erases it, see docs/handoffs/2026-10-21/FLAKY-POLICY.md).\n",
          "## New / re-assessed this session\n", "| ID | Sev | Component | State | Title | Regression test | Device | Remaining risk |", "|---|---|---|---|---|---|---|---|"]
    for r in NEW:
        md.append(f"| {r['id']} | {r['severity']} | {r['component']} | {r['state']} | {r['title']} | {r['regression_test'] or '-'} | {r['device_required']} | {r['remaining_risk'] or '-'} |")
    md += ["\n## Carried from the 2026-10-15 audit (not re-verified unless listed above)\n", "| ID | Sev | Audit status | State | Title |", "|---|---|---|---|---|"]
    for r in carried:
        md.append(f"| {r['id']} | {r['severity']} | {r['audit_status']} | {r['state']} | {r['title'][:120].replace('|','/')} |")
    open(OUT + ".md", "w").write("\n".join(md) + "\n")
    print(len(NEW), "new,", len(carried), "carried")


if __name__ == "__main__":
    sys.exit(main())
