"""The canonical issue registry must survive regeneration and retain recovery findings."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]


class IssueRegistry(unittest.TestCase):
    def test_registry_is_reproducible_and_recovery_findings_are_not_lost(self):
        spec = importlib.util.spec_from_file_location(
            "issue_builder", ROOT / "deploy/lib/registry/build_issue_registry.py")
        builder = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(builder)
        builder.AUDIT = str(ROOT / builder.AUDIT)
        with tempfile.TemporaryDirectory() as tmp:
            builder.OUT = str(Path(tmp) / "registry")
            with contextlib.redirect_stdout(io.StringIO()):
                builder.main()
            generated = json.loads(Path(builder.OUT + ".json").read_text())
        checked = json.loads((ROOT / "docs/handoffs/2026-10-20/issue-registry.json").read_text())
        self.assertEqual(generated, checked)
        ids = [row["id"] for row in generated["new"] + generated["carried"]]
        self.assertEqual(len(ids), len(set(ids)), "issue IDs must be unique")
        for issue in ("I-31", "I-53", "I-54", "I-56", "I-65", "I-68", "I-81", "I-83"):
            self.assertIn(issue, ids)


if __name__ == "__main__":
    unittest.main()
