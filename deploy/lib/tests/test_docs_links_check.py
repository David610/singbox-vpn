"""Tests for deploy/lib/docs-links-check.py (deterministic Markdown link check)."""
import importlib.util
import os
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SPEC = importlib.util.spec_from_file_location("docs_links_check", os.path.join(HERE, "..", "docs-links-check.py"))
mod = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(mod)


def repo(files):
    d = tempfile.mkdtemp()
    subprocess.run(["git", "init", "-q", d], check=True)
    for rel, text in files.items():
        p = os.path.join(d, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        open(p, "w", encoding="utf-8").write(text)
    subprocess.run(["git", "-C", d, "add", "-A"], check=True)
    return d


def kinds(d, code_paths=False):
    return sorted((f[0], f[2], f[4]) for f in mod.check(d, code_paths))


class DocsLinks(unittest.TestCase):
    def test_clean_repo_has_no_findings(self):
        d = repo({"README.md": "# T\n[a](docs/A.md) [s](#t)\n", "docs/A.md": "# A\n## Sub part\n[b](A.md#sub-part)\n"})
        self.assertEqual(kinds(d), [])

    def test_missing_file_and_anchor_are_reported(self):
        d = repo({"README.md": "# T\n[x](nope.md)\n[y](docs/A.md#ghost)\n[z](#absent)\n", "docs/A.md": "# A\n"})
        self.assertEqual([k[1] for k in kinds(d)], ["missing-anchor", "missing-anchor", "missing-target"])

    def test_external_and_mailto_links_are_not_fetched_or_flagged(self):
        d = repo({"README.md": "[e](https://example.invalid/x) [m](mailto:a@b.c)\n"})
        self.assertEqual(kinds(d), [])

    def test_links_inside_fenced_blocks_are_ignored(self):
        d = repo({"README.md": "```\n[x](missing.md)\n```\n"})
        self.assertEqual(kinds(d), [])

    def test_historical_records_are_reported_but_marked(self):
        d = repo({"docs/archive/old.md": "[x](gone.md)\n", "NOTE.md": "> HISTORICAL DOCUMENT\n[x](gone.md)\n"})
        self.assertEqual(kinds(d), [("NOTE.md", "missing-target", True), ("docs/archive/old.md", "missing-target", True)])

    def test_duplicate_headings_get_numbered_anchors(self):
        d = repo({"README.md": "# A\n# A\n[x](#a-1)\n"})
        self.assertEqual(kinds(d), [])

    def test_code_span_and_fenced_paths_only_in_code_paths_mode(self):
        d = repo({"README.md": "see `deploy/lib/gone.sh`\n```\nbash deploy/lib/gone2.sh.\n```\n", "deploy/lib/real.sh": "x\n"})
        self.assertEqual(kinds(d), [])
        self.assertEqual([k[1] for k in kinds(d, True)], ["missing-code-path", "missing-fenced-path"])

    def test_this_repository_has_no_broken_links_in_current_docs(self):
        root = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
        bad = [f for f in mod.check(root, False) if not f[4]]
        self.assertEqual(bad, [], "broken links in current docs: %r" % bad[:5])


if __name__ == "__main__":
    unittest.main()
