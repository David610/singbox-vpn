#!/usr/bin/env python3
"""Build docs/handoffs/2026-10-20/issue-registry.{json,md} from the 2026-10-15
audit (carried issues, history preserved there) plus this session's issues
(NEW below). States: CONFIRMED_OPEN IN_PROGRESS FIXED_IN_BRANCH CI_VERIFIED
SERVER_VERIFIED DEVICE_VERIFIED PRODUCTION_VERIFIED BLOCKED UNVERIFIED.
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
    n("I-53", "low", "tests", "UNVERIFIED",
      "intermittent single failures in the hermetic shell suite (test-install-manifest-idempotency once, test-uninstall-hardening once) not reproduced in 3 full runs and 24 isolated runs",
      "runner now prints failing test output to catch the next occurrence", "", "no", "cause unknown; possibly concurrent host load"),
    n("I-54", "critical", "nftables egress isolation (vpn-egress-isolation.service)", "SERVER_VERIFIED",
      "the host-layer table broke IPv6 neighbour discovery: 'meta skuid != UID accept' does not match socketless kernel packets, so ND to fe80::/10 and ff00::/8 was rejected; the whole host lost IPv6 after install/update (gateway FAILED, no IPv6 egress for anyone)",
      "positive uid-scoped rejects with counters in deploy/almalinux/nftables-egress-isolation.sh",
      "deploy/lib/tests/test-egress-isolation-ipv6-nd.sh (two-netns behavioural; red before, also proves uid scoping)", "no",
      "published v1.3.1-rc.2 and every earlier release carry the defect; existing nodes keep the old table until the script is re-run",
      "staging: IPv6 dead with the table, restored at once without it; after the fix a flushed-cache ND succeeds, v6 egress and YouTube/v6 HTTP 200"),
    n("I-55", "high", "admin/doctor", "FIXED_IN_BRANCH",
      "doctor had no IPv6 health check: it passed (rc 0) while the host's IPv6 was completely broken by I-54",
      "WARN when a global IPv6 address exists and the default gateway's neighbour entry is FAILED (classify_host_ipv6)",
      "apps/admin public_surface_tests::host_ipv6_* (unit); live check reports OK on staging", "no",
      "WARN not FAIL (IPv6 is only mandatory when an AAAA record exists); the broken state itself was exercised by hand, not by an automated end-to-end test"),
    n("I-56", "medium", "update.sh interrupted-transaction recovery", "SERVER_VERIFIED",
      "after update.sh is SIGKILLed mid-switch the host stays healthy and byte-identical, but the next run refuses (correct) and recovery was hand-editing: remove the marker AND the stale /opt/.singbox-vpn-prev-*/update-staging.* dirs",
      "deploy/lib/recover-interrupted-update.sh (health-gated, pattern-checked, dry-run default, archives the marker); update.sh error text points to it",
      "deploy/lib/tests/test-recover-interrupted-update.sh (refusal paths mutation-checked)", "no",
      "the helper never restores binaries/config: an unhealthy host still needs the backup_dir restore",
      "staging: kill -9 after SWITCHING -> fingerprint identical, doctor clean; manual procedure then repair succeeded, credentials preserved"),
    n("I-57", "low", "preflight_is_public_ip", "FIXED_IN_BRANCH",
      "192.88.99.0/24 (deprecated 6to4 anycast; also the product's own arcana-2hop0 addresses) counted as public host addresses, producing a spurious 'more public IPv4 addresses than deployment.toml can record' warning on every install/update",
      "exclude 192.88.99.0/24 in preflight_is_public_ip", "deploy/lib/tests/test-own-address-detection.sh (red before)", "no"),
    n("I-58", "medium", "configuration contract / drift", "FIXED_IN_BRANCH",
      "no version-controlled production contract or drift check; routing/ssh/host/port drift was only found by hand",
      "deploy/contracts/fin1-staging.contract.json + deploy/lib/check_deployment_contract.py (read-only; PASS/FAIL/UNDECIDED/UNOBSERVABLE; routing fingerprint)",
      "deploy/lib/tests/test_deployment_contract.py (12 tests)", "no",
      "contract still has UNDECIDED fields awaiting owner approval; not yet wired as a deployment gate", "live run on staging: 11 PASS, 2 FAIL (SSH), 7 UNDECIDED"),
    n("I-51", "high", "Hiddify iOS (client)", "UNVERIFIED",
      "profile JSON missing from Library/Caches/Working/configs (carried as I-08); upstream hiddify-app#2372 open, no fix",
      "upstream proposal in docs/clients/HIDDIFY_IOS.md; server cannot fix", "", "yes",
      "trigger on the owner's iPhone unconfirmed"),
]


def main():
    audit = json.load(open(AUDIT))
    items = audit if isinstance(audit, list) else list(audit.values()) if "id" not in audit else [audit]
    if isinstance(audit, dict) and "issues" in audit:
        items = audit["issues"]
    carried = [dict(id=i["id"], severity=i["severity"], component=i.get("repo", ""), state=CARRY.get(i["status"], "UNVERIFIED"),
                    title=i["title"], audit_status=i["status"], fix=i.get("fix", ""), note=i.get("note", ""),
                    source="docs/audits/2026-10-15/issues.json (not re-verified this session)") for i in items]
    newids = {x["id"] for x in NEW}
    for x in NEW:
        x["supersedes_audit_entry"] = x["id"] in {c["id"] for c in carried}
    carried = [c for c in carried if c["id"] not in newids]
    reg = {"schema_version": 1, "generated_for": "2026-10-20 staging remediation", "new": NEW, "carried": carried}
    json.dump(reg, open(OUT + ".json", "w"), indent=1)
    md = ["# Issue registry (generated from issue-registry.json — do not edit by hand)\n",
          "States: CONFIRMED_OPEN, IN_PROGRESS, FIXED_IN_BRANCH, CI_VERIFIED, SERVER_VERIFIED, DEVICE_VERIFIED, PRODUCTION_VERIFIED, BLOCKED, UNVERIFIED.\n",
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
