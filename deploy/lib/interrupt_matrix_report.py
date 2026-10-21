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
    if invalid:
        verdict, reasons = "INVALID", invalid
    else:
        need = lambda cond, msg: None if cond else reasons.append(msg)
        need(kv.get("creds_after_kill") == "same" and kv.get("creds_after_recover") == "same" and kv.get("creds_final") == "same" and kv.get("creds_idem") == "same", "credentials changed")
        need(kv.get("marker_after_recover") == "no" and kv.get("stale_dirs_after_recover") == "0", "marker or stale directories remain after recovery")
        # Consistency of the release after recovery. A kill BEFORE the commit leaves a marker: recovery must give
        # back exactly the baseline release. A kill AFTER the commit (the update already finished; only the
        # post-commit manifest/agent refresh was running) leaves no marker: the node legitimately runs the NEW
        # release and recovery must be a no-op. Either way the release must be ONE consistent release, never a mix.
        old_ok = kv.get("version_after_recover") == kv.get("baseline_version") and kv.get("bins_after_recover") == kv.get("baseline_bins")
        new_ok = kv.get("version_after_recover") == kv.get("version_final") and kv.get("bins_after_recover") == kv.get("bins_final")
        if kv.get("marker_after_kill") == "yes" and kv.get("recovery_decision") == "rolled_forward":
            # Kill inside the commit window (manifest already written, marker not yet removed): the updater must roll
            # FORWARD to one consistent NEW release and say so. Rolling back here would discard a committed update.
            need(new_ok and kv.get("version_final", "").startswith("1.3.1"), "recovery claimed a roll-forward but the node is not on one consistent new release")
        elif kv.get("marker_after_kill") == "yes":
            need(old_ok, "an open transaction was not rolled back to exactly the baseline release (version/binaries differ)")
        else:
            need(old_ok or new_ok, "no open transaction, yet the node runs a MIXED release (neither the baseline nor the final binaries)")
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
