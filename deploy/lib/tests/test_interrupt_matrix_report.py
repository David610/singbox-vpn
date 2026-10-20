import json, os, subprocess, sys, tempfile, unittest
TOOL = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "interrupt_matrix_report.py")
BASE = {"harness_sha": "h", "harness_sha_end": "h", "stage": "3", "kill_fired": "true", "kill_scope": "tree", "survivors_after_kill": "0",
        "baseline_version": "1.3.0", "baseline_bins": "OLD,", "marker_after_kill": "yes", "stale_dirs_after_kill": "2",
        "creds_after_kill": "same", "creds_after_recover": "same", "creds_final": "same", "creds_idem": "same",
        "marker_after_recover": "no", "stale_dirs_after_recover": "0", "version_after_recover": "1.3.0", "bins_after_recover": "OLD,",
        "services_after_recover": "all-active", "doctor_fails_after_recover": "0", "protocol_rc_after_recover": "0",
        "final_update_rc": "0", "version_final": "1.3.1", "bins_final": "NEW,", "services_final": "all-active", "doctor_fails_final": "0",
        "protocol_rc_final": "0", "marker_final": "no", "stale_dirs_final": "0", "idempotent_rc": "0", "idempotent_bins_same": "yes",
        "tmp_leaks_delta": "0", "free_root_mb_end": "20000", "free_tmp_mb_end": "1900"}

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
    def test_mixed_release_is_a_failure_even_without_a_marker(self):
        v, _ = verdict(marker_after_kill="no", bins_after_recover="OLD,NEW,", version_after_recover="1.3.1")
        self.assertEqual(v, "FAIL")
    def test_post_commit_kill_leaving_the_new_release_is_a_pass(self):
        self.assertEqual(verdict(marker_after_kill="no", version_after_recover="1.3.1", bins_after_recover="NEW,")[0], "PASS")
    def test_credential_change_fails(self): self.assertEqual(verdict(creds_after_recover="CHANGED")[0], "FAIL")
    def test_tmp_leak_fails(self): self.assertEqual(verdict(tmp_leaks_delta="3")[0], "FAIL")
    def test_non_idempotent_rerun_fails(self): self.assertEqual(verdict(idempotent_bins_same="NO")[0], "FAIL")
    def test_stale_state_after_recovery_fails(self): self.assertEqual(verdict(stale_dirs_after_recover="1")[0], "FAIL")
    def test_unhealthy_after_recovery_fails(self): self.assertEqual(verdict(doctor_fails_after_recover="1")[0], "FAIL")

if __name__ == "__main__": unittest.main()
