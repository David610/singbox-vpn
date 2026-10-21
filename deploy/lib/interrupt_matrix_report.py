#!/usr/bin/env python3
"""Turn one stage's key=value file into a JSON line with a verdict (PASS | FAIL | INVALID).
INVALID wins: a run that did not test what it claims is never a PASS or a FAIL of the product."""
import json
import sys

kv = {}
for line in open(sys.argv[1]):
    k, _, v = line.rstrip("\n").partition("=")
    kv[k] = v
min_root = int(sys.argv[2]) if len(sys.argv) > 2 else 0
min_tmp = int(sys.argv[3]) if len(sys.argv) > 3 else 0
reasons = []
verdict = kv.get("verdict")
if verdict != "INVALID":
    invalid = []
    if kv.get("kill_fired") != "true":
        invalid.append("kill point not reached")
    if kv.get("harness_sha") != kv.get("harness_sha_end"):
        invalid.append("harness file changed during the run")
    for k, lim in (("free_root_mb_end", min_root // 2), ("free_tmp_mb_end", min_tmp // 2)):
        if k in kv and int(kv[k] or 0) < lim:
            invalid.append(f"capacity collapsed during the run ({k}={kv[k]} MB)")
    if kv.get("kill_scope") == "tree" and kv.get("survivors_after_kill") not in ("0", None):
        invalid.append("tree kill left surviving processes")
    if kv.get("updater_alive_after_kill") == "yes":
        invalid.append("the updater process itself was still alive after the kill (the experiment did not kill what it claims)")
    if kv.get("expected_committed") in ("yes", "no") and kv.get("expected_committed") != kv.get("observed_committed"):
        invalid.append(f"the kill did not land on the claimed side of the commit boundary (expected committed={kv.get('expected_committed')}, observed {kv.get('observed_committed')})")
    if kv.get("checkpoint_reached") is not None and kv.get("checkpoint_reached", "").find(kv.get("marker", "\0")) != 0:
        invalid.append("the updater reported a different checkpoint than the one requested")
    if invalid:
        verdict, reasons = "INVALID", invalid
    else:
        need = lambda cond, msg: None if cond else reasons.append(msg)
        need(kv.get("creds_after_kill") == "same" and kv.get("creds_after_recover") == "same" and kv.get("creds_final") == "same" and kv.get("creds_idem") == "same", "credentials changed")
        need(kv.get("marker_after_recover") == "no" and kv.get("stale_dirs_after_recover") == "0", "marker or stale directories remain after recovery")
        # The commit state is OBSERVED independently of the updater (the install-state manifest names the target at
        # the moment of the kill) and must match what the kill point claims to be. Then:
        #   not committed -> recovery must give back EXACTLY the baseline release (binaries, version, manifest);
        #   committed     -> recovery must end on ONE consistent new release (manifest hashes == installed binaries) and,
        #                    if the marker survived, the updater must have said it rolled FORWARD.
        # A healthy node is not enough: the direction must follow the commit evidence.
        old_ok = kv.get("version_after_recover") == kv.get("baseline_version") and kv.get("bins_after_recover") == kv.get("baseline_bins")
        new_ok = kv.get("version_after_recover") == kv.get("version_final") and kv.get("bins_after_recover") == kv.get("bins_final")
        if kv.get("observed_committed") == "yes":
            need(new_ok, "committed update did not end on one consistent new release")
            need(kv.get("manifest_version_after_recover") == kv.get("manifest_version_after_kill") and kv.get("manifest_matches_bins_after_recover") == "yes", "roll-forward: manifest identity/hashes do not match the installed executables")
            if kv.get("marker_after_kill") == "yes":
                need(kv.get("recovery_decision") == "rolled_forward", "a committed update with a surviving marker was not rolled forward")
        else:
            need(old_ok, "an uncommitted update was not rolled back to exactly the baseline release (version/binaries differ)")
            need(kv.get("recovery_decision") != "rolled_forward", "an uncommitted update was rolled forward")
            need(kv.get("manifest_version_after_recover") == kv.get("manifest_version_baseline"), "rollback: install-state manifest is not the baseline's")
        need(kv.get("orphans_after_recover", "0") == "0" and kv.get("orphans_end", "0") == "0", "orphan updater/cosign processes remain")
        need(kv.get("lock_after_recover", "free") == "free" and kv.get("lock_end", "free") == "free", "update lock still held")
        need(kv.get("leftover_tmp_after_recover", "0") == "0", "temporary manifest or staging leftovers after recovery")
        need(kv.get("backups_after_recover", "0") == "0", "transaction backups left in /etc/vpn/backups after recovery")
        need(kv.get("repair_rc", "0") == "0" and kv.get("repair_bins_same", "yes") == "yes" and kv.get("creds_repair", "same") == "same", "repair after recovery failed or changed the release/credentials")
        need(kv.get("services_repair", "all-active") == "all-active" and kv.get("doctor_fails_repair", "0") == "0", "unhealthy after repair")
        need(kv.get("services_after_recover") == "all-active" and kv.get("doctor_fails_after_recover") == "0", "services/doctor unhealthy after recovery")
        need(kv.get("protocol_rc_after_recover") == "0", "protocol self-test failed after recovery")
        need(kv.get("final_update_rc") == "0" and kv.get("version_final", "").startswith("1.3.1"), "normal update after recovery failed")
        need(kv.get("services_final") == "all-active" and kv.get("doctor_fails_final") == "0" and kv.get("protocol_rc_final") == "0", "unhealthy after the final update")
        need(kv.get("marker_final") == "no" and kv.get("stale_dirs_final") == "0", "marker/stale dirs after the final update")
        need(kv.get("idempotent_rc") == "0" and kv.get("idempotent_bins_same") == "yes", "re-running the update was not idempotent")
        # The cosign /tmp leak is a defect of OLDER updaters (fixed in the branch updater). It is informational when
        # the old installed updater ran (the number is still recorded); it fails the stage for the branch updater.
        if kv.get("updater") == "branch":
            need(kv.get("tmp_leaks_delta") in ("0", "-1", "-2"), f"/tmp leaked {kv.get('tmp_leaks_delta')} directories")
        verdict = "PASS" if not reasons else "FAIL"
else:
    reasons = [kv.get("reason", "")]
out = dict(kv, verdict=verdict, reasons=reasons)
out["stage"] = int(kv.get("stage", 0))
print(json.dumps(out))
