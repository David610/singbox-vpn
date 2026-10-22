import importlib.util
import os
import subprocess
import shutil
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("sks", os.path.join(HERE, "..", "scan_known_secrets.py"))
sks = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sks)
SECRET = "Zx9-Qw7Lm2Kp4Rt6Yv8Bn1Cd3Fg5Hj0AaBbCc"


def git(d, *a):
    subprocess.run(["git", "-C", d, "-c", "user.name=t", "-c", "user.email=t@t", *a], check=True, capture_output=True)


class Scan(unittest.TestCase):
    def repo(self, content):
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        git(d, "init", "-q", "-b", "main")
        open(os.path.join(d, "a.txt"), "w").write("base\n")
        git(d, "add", "-A"); git(d, "commit", "-qm", "base")
        open(os.path.join(d, "b.txt"), "w").write(content)
        git(d, "add", "-A"); git(d, "commit", "-qm", "work")
        return d

    def run_scan(self, d, secret):
        sf = os.path.join(d, ".secret"); open(sf, "w").write(secret)   # untracked, like /etc/vpn/...
        cwd = os.getcwd(); os.chdir(d)
        try:
            vals = sks.collect([f"k={sf}"], None, None)
            return vals, sks.scan(vals, "HEAD~1..HEAD", "HEAD")
        finally:
            os.chdir(cwd)

    def test_a_committed_secret_is_found_and_never_echoed(self):
        d = self.repo(f"token = {SECRET}\n")
        vals, res = self.run_scan(d, SECRET)
        self.assertEqual(len(vals), 1)
        self.assertTrue(all(res.values()))

    def test_a_clean_repo_reports_not_found(self):
        d = self.repo("nothing here\n")
        vals, res = self.run_scan(d, SECRET)
        self.assertEqual(len(vals), 1)
        self.assertFalse(any(res.values()))

    def test_secret_in_history_only_is_found_by_the_range(self):
        d = self.repo(f"{SECRET}\n")
        git(d, "rm", "-q", "b.txt"); git(d, "commit", "-qm", "remove")     # removed from the tree, still in history
        cwd = os.getcwd(); os.chdir(d)
        try:
            vals = {"k#1": SECRET}
            res = sks.scan(vals, "HEAD~2..HEAD", "HEAD")
        finally:
            os.chdir(cwd)
        self.assertEqual(res["k#1"], ["commits HEAD~2..HEAD"])

    def test_hex_digests_and_short_values_are_not_treated_as_secrets(self):
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        f = os.path.join(d, "s"); open(f, "w").write("a" * 64)
        self.assertEqual(sks.collect([f"k={f}"], None, None), {})
        open(f, "w").write("short"); self.assertEqual(sks.collect([f"k={f}"], None, None), {})

    def test_nothing_collected_is_exit_2_not_clean(self):
        self.assertEqual(sks.main(["--secret-file", "k=/nonexistent", "--users-json", "/nonexistent", "--url-file", "/nonexistent"]), 2)

    def test_output_contains_labels_and_lengths_only(self):
        import io, contextlib
        d = self.repo(f"{SECRET}\n"); sf = os.path.join(d, ".s"); open(sf, "w").write(SECRET)
        buf = io.StringIO(); cwd = os.getcwd(); os.chdir(d)
        try:
            with contextlib.redirect_stdout(buf):
                rc = sks.main(["--secret-file", f"k={sf}", "--users-json", "/nonexistent", "--url-file", "/nonexistent", "--range", "HEAD~1..HEAD", "--rev", "HEAD"])
        finally:
            os.chdir(cwd)
        self.assertEqual(rc, 1)
        self.assertNotIn(SECRET, buf.getvalue())
        self.assertIn("FOUND in", buf.getvalue())


if __name__ == "__main__":
    unittest.main()
