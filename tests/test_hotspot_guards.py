"""tools/hotspots/check_guards.py: each guard fails on growth and lets what the base already has stand."""
import io
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

with patch.object(sys, "path", [*sys.path, str(Path(__file__).resolve().parents[1] / "tools/hotspots")]):
    import check_guards as guards


def summary(*lines):
    return "\n".join(["# Summary", "", "# Concepts", "", *lines[:2], "", "# How-to", "", *lines[2:], ""])


class SummaryGuard(unittest.TestCase):
    def test_a_new_line_for_a_discovered_page_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp)
            for page in ("concepts/listed.md", "concepts/new.md", "concepts/listed/sub.md", "how-to/kept.md",
                         "how-to/noted.md", "other/page.md"):
                (src / page).parent.mkdir(parents=True, exist_ok=True)
                (src / page).write_text(f"# {Path(page).stem}\n")
            before = summary("- [Listed](concepts/listed.md)")
            now = summary("- [Listed](concepts/listed.md)", "  - [Sub](concepts/listed/sub.md)",
                          "- [New](concepts/new.md)", "- [Noted](how-to/noted.md) <!-- a comment -->",
                          "- [Other](other/page.md)")
            found = guards.summary_problems(guards.redundant_summary_lines(now, src),
                                            guards.redundant_summary_lines(before, src))
        self.assertEqual([p.split(" lists ")[0] for p in found],
                         [f"{guards.SUMMARY}: the new line for concepts/listed/sub.md",
                          f"{guards.SUMMARY}: the new line for concepts/new.md",
                          f"{guards.SUMMARY}: the new line for how-to/noted.md"])


class Repository(unittest.TestCase):
    """A temporary checkout with the guarded paths, its `main` as the base, and `main()` run against it."""

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        (self.root / guards.BOOK_SRC / "concepts").mkdir(parents=True)
        (self.root / guards.BOOK_SRC / "how-to").mkdir()
        (self.root / guards.SUMMARY).write_text(summary())
        (self.root / guards.NOTES).parent.mkdir(parents=True)
        (self.root / guards.NOTES).write_text("pub trait Notes {}\n")
        (self.root / guards.LANG).mkdir(parents=True)
        for code in ("de", "fr", "es", "it", "ja", "pl", "ru", "uk"):
            (self.root / guards.LANG / f"{code}.json").write_text("{}\n")
        self.git("init", "-q", "-b", "main")
        self.commit("base")

    def git(self, *args):
        subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *args], cwd=self.root, check=True,
                       capture_output=True)

    def commit(self, message):
        self.git("add", ".")
        self.git("commit", "-qm", message)

    def run_guards(self, *args):
        with redirect_stdout(io.StringIO()) as out:
            code = guards.main(["--base", "main", *args], root=self.root)
        return code, out.getvalue()


class LangGuard(Repository):
    def translate(self, branch, codes):
        self.git("switch", "-qc", branch, "main")
        for code in codes:
            (self.root / guards.LANG / f"{code}.json").write_text('{"Play": "x"}\n')
        self.commit(branch)

    def test_a_rollup_of_two_small_translations_passes_and_a_large_one_fails(self):
        self.translate("first", ["de", "fr", "es", "it"])
        self.assertEqual(self.run_guards()[0], 0)
        self.translate("second", ["ja", "pl", "ru", "uk"])
        self.git("switch", "-qc", "fork-review-rollup-9", "main")
        self.git("merge", "-q", "--no-ff", "first", "second")
        code, out = self.run_guards()
        self.assertEqual(code, 1)
        self.assertIn(f"{guards.LANG}: this change touches 8 language files", out)
        self.assertEqual(self.run_guards("--branch", "fork-review-rollup-9")[0], 0)
        self.assertEqual(self.run_guards("--branch", "upstream-sync-12")[0], 0)
        self.assertEqual(self.run_guards("--branch", "translations", "--labels", "skip-release sync")[0], 0)
        self.assertEqual(self.run_guards("--branch", "translations", "--labels", "skip-release")[0], 1)


class RenamedSummaryLine(Repository):
    def test_the_message_says_a_rename_made_the_line_new(self):
        old = self.root / guards.BOOK_SRC / "concepts/old.md"
        old.write_text("# Old\n")
        (self.root / guards.SUMMARY).write_text(summary("- [Old](concepts/old.md)"))
        self.commit("list a page")
        self.git("switch", "-qc", "rename")
        old.rename(old.with_name("new.md"))
        (self.root / guards.SUMMARY).write_text(summary("- [Old](concepts/new.md)"))
        code, out = self.run_guards()
        self.assertEqual(code, 1)
        self.assertIn("the new line for concepts/new.md", out)
        self.assertIn("It was the line for concepts/old.md, renamed", out)


class NotesGuard(Repository):
    def test_notes_may_not_grow_past_the_merge_base(self):
        self.git("switch", "-qc", "branch")
        self.assertEqual(self.run_guards()[0], 0)
        (self.root / guards.NOTES).write_text("pub trait Notes {}\n// More prose.\n")
        code, out = self.run_guards()
        self.assertEqual(code, 1)
        self.assertIn(f"{guards.NOTES}: 2 lines, 1 on the base", out)


if __name__ == "__main__":
    unittest.main()
