import json, os, subprocess, sys, tempfile, unittest
TOOL = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "interrupt_matrix_report.py")
BASE = {"harness_sha": "h", "harness_sha_end": "h", "stage": "3", "kill_fired": "true", "kill_scope": "tree", "survivors_after_kill": "0",
        "baseline_version": "1.3.0", "baseline_bins": "OLD,", "marker_after_kill": "yes", "stale_dirs_after_kill": "2",
        "creds_after_kill": "same", "creds_after_recover": "same", "creds_final": "same", "creds_idem": "same",
        "marker_after_recover": "no", "stale_dirs_after_recover": "0", "version_after_recover": "1.3.0", "bins_after_recover": "OLD,",
        "services_after_recover": "all-active", "doctor_fails_after_recover": "0", "protocol_rc_after_recover": "0",
        "final_update_rc": "0", "version_final": "1.3.1", "bins_final": "NEW,", "services_final": "all-active", "doctor_fails_final": "0",
        "protocol_rc_final": "0", "marker_final": "no", "stale_dirs_final": "0", "idempotent_rc": "0", "idempotent_bins_same": "yes",
        "expected_committed": "no", "observed_committed": "no", "manifest_version_baseline": "v1.3.0", "manifest_version_after_kill": "v1.3.0",
        "manifest_version_after_recover": "v1.3.0", "manifest_matches_bins_after_recover": "yes", "recovery_decision": "rolled_back_or_none",
        "orphans_after_recover": "0", "orphans_end": "0", "lock_after_recover": "free", "lock_end": "free", "leftover_tmp_after_recover": "0",
        "backups_after_recover": "0", "repair_rc": "0", "repair_bins_same": "yes", "creds_repair": "same", "services_repair": "all-active", "doctor_fails_repair": "0",
        "tmp_leaks_delta": "0", "free_root_mb_end": "20000", "free_tmp_mb_end": "1900"}

FWD = dict(expected_committed="yes", observed_committed="yes", manifest_version_after_kill="v1.3.1", manifest_version_after_recover="v1.3.1",
           recovery_decision="rolled_forward", version_after_recover="1.3.1", bins_after_recover="NEW,")

def verdict(**over):
    kv = dict(BASE, **over)
    with tempfile.NamedTemporaryFile("w", delete=False) as f:
        f.write("\n".join(f"{k}={v}" for k, v in kv.items()) + "\n")
    try:
        out = subprocess.run([sys.executable, "-I", TOOL, f.name, "2048", "1024"], capture_output=True, text=True).stdout
    finally:
        os.unlink(f.name)
    d = json.loads(out)
    return d["verdict"], d["reasons"]

class Report(unittest.TestCase):
    def test_clean_pass(self): self.assertEqual(verdict()[0], "PASS")
    def test_kill_never_reached_is_invalid_never_pass(self): self.assertEqual(verdict(kill_fired="false")[0], "INVALID")
    def test_edited_harness_is_invalid(self): self.assertEqual(verdict(harness_sha_end="x")[0], "INVALID")
    def test_capacity_collapse_is_invalid(self):
        self.assertEqual(verdict(free_tmp_mb_end="10")[0], "INVALID")
        self.assertEqual(verdict(free_root_mb_end="10")[0], "INVALID")
    def test_surviving_processes_after_a_tree_kill_are_invalid(self): self.assertEqual(verdict(survivors_after_kill="2")[0], "INVALID")
    def test_open_transaction_must_return_exactly_the_baseline(self):
        v, r = verdict(version_after_recover="1.3.1", bins_after_recover="NEW,")
        self.assertEqual(v, "FAIL"); self.assertTrue(any("baseline" in x for x in r))
    def test_commit_window_kill_rolled_forward_is_a_pass(self):
        self.assertEqual(verdict(**FWD)[0], "PASS")
    def test_roll_forward_claim_over_a_mixed_release_fails(self):
        v, _ = verdict(**dict(FWD, bins_after_recover="OLD,NEW,"))
        self.assertEqual(v, "FAIL")
    def test_roll_forward_does_not_excuse_a_baseline_mismatch_without_the_claim(self):
        self.assertEqual(verdict(**dict(FWD, recovery_decision="rolled_back_or_none"))[0], "FAIL")
    def test_mixed_release_is_a_failure_even_without_a_marker(self):
        v, _ = verdict(**dict(FWD, marker_after_kill="no", bins_after_recover="OLD,NEW,"))
        self.assertEqual(v, "FAIL")
    def test_post_commit_kill_leaving_the_new_release_is_a_pass(self):
        self.assertEqual(verdict(**dict(FWD, marker_after_kill="no", recovery_decision="rolled_back_or_none"))[0], "PASS")
    def test_an_updater_that_survived_the_kill_is_invalid(self): self.assertEqual(verdict(updater_alive_after_kill="yes")[0], "INVALID")
    def test_credential_change_fails(self): self.assertEqual(verdict(creds_after_recover="CHANGED")[0], "FAIL")
    def test_tmp_leak_fails_for_the_branch_updater(self): self.assertEqual(verdict(tmp_leaks_delta="3", updater="branch")[0], "FAIL")
    def test_tmp_leak_is_informational_for_the_old_installed_updater(self): self.assertEqual(verdict(tmp_leaks_delta="2", updater="installed")[0], "PASS")
    def test_non_idempotent_rerun_fails(self): self.assertEqual(verdict(idempotent_bins_same="NO")[0], "FAIL")
    def test_stale_state_after_recovery_fails(self): self.assertEqual(verdict(stale_dirs_after_recover="1")[0], "FAIL")
    def test_unhealthy_after_recovery_fails(self): self.assertEqual(verdict(doctor_fails_after_recover="1")[0], "FAIL")

    def test_kill_on_the_wrong_side_of_the_commit_boundary_is_invalid(self):
        self.assertEqual(verdict(expected_committed="no", observed_committed="yes")[0], "INVALID")
        self.assertEqual(verdict(**dict(FWD, expected_committed="no"))[0], "INVALID")
    def test_wrong_checkpoint_is_invalid(self):
        self.assertEqual(verdict(marker="@manifest-renamed", checkpoint_reached="@other")[0], "INVALID")
    def test_rolling_forward_an_uncommitted_update_fails(self):
        self.assertEqual(verdict(recovery_decision="rolled_forward")[0], "FAIL")
    def test_roll_forward_needs_manifest_hashes_to_match_the_installed_binaries(self):
        v, r = verdict(**dict(FWD, manifest_matches_bins_after_recover="no")); self.assertEqual(v, "FAIL"); self.assertTrue(any("manifest" in x for x in r))
    def test_rollback_must_restore_the_baseline_manifest(self):
        self.assertEqual(verdict(manifest_version_after_recover="v1.3.1")[0], "FAIL")
    def test_orphans_locks_leftovers_and_backups_fail(self):
        for k, bad in (("orphans_after_recover", "1"), ("orphans_end", "2"), ("lock_end", "HELD"), ("leftover_tmp_after_recover", "1"), ("backups_after_recover", "1")):
            self.assertEqual(verdict(**{k: bad})[0], "FAIL", k)
    def test_repair_after_recovery_must_work_and_keep_identity(self):
        for k, bad in (("repair_rc", "1"), ("repair_bins_same", "NO"), ("creds_repair", "CHANGED"), ("doctor_fails_repair", "1")):
            self.assertEqual(verdict(**{k: bad})[0], "FAIL", k)

if __name__ == "__main__": unittest.main()
