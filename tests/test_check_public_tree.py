"""The public-tree check scans HEAD's history, not every branch's."""
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
import unittest
from unittest import mock

from tests.git_fixture import decoy_git_env, isolated_git_env

SCRIPT = Path(__file__).resolve().parents[1] / "tools/check_public_tree.py"


def git(repo, *args):
    subprocess.run(["git", "-C", repo, "-c", "user.name=t", "-c", "user.email=t@example.com", *args],
                   env=isolated_git_env(), check=True, capture_output=True)


def check(repo, *args):
    return subprocess.run([sys.executable, str(SCRIPT), *args], cwd=repo, env=isolated_git_env(),
                          capture_output=True, text=True)


class HistoryScopeTests(unittest.TestCase):
    def repo_with_artifact_on_a_branch(self, directory):
        git(directory, "init", "-q", "-b", "main")
        (Path(directory) / "readme.md").write_text("public\n")
        git(directory, "add", "readme.md")
        git(directory, "commit", "-qm", "main")
        git(directory, "checkout", "-qb", "other")
        (Path(directory) / "Leak.java").write_text("class Leak {}\n")
        git(directory, "add", "Leak.java")
        git(directory, "commit", "-qm", "artifact")
        git(directory, "checkout", "-q", "main")

    def test_an_artifact_only_on_another_branch_does_not_fail_head_history(self):
        with TemporaryDirectory() as directory:
            self.repo_with_artifact_on_a_branch(directory)
            result = check(directory, "--history")
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_an_artifact_in_head_history_fails(self):
        with TemporaryDirectory() as directory:
            parent = Path(directory)
            repo = parent / "repo"
            repo.mkdir()
            self.repo_with_artifact_on_a_branch(repo)
            git(repo, "merge", "-q", "--no-edit", "other")
            with mock.patch.dict("os.environ", decoy_git_env(parent / "decoy"), clear=True):
                result = check(repo, "--history")
            self.assertEqual(result.returncode, 1)
            self.assertIn("Leak.java", result.stderr)

    def test_the_jfr_summary_is_allowed_and_another_java_tool_is_not(self):
        with TemporaryDirectory() as directory:
            git(directory, "init", "-q", "-b", "main")
            tools = Path(directory) / "tools"
            tools.mkdir(parents=True)
            (tools / "JfrSummary.java").write_text("class JfrSummary {}\n")
            git(directory, "add", "-A")
            git(directory, "commit", "-qm", "oracle")
            self.assertEqual(check(directory, "--history").returncode, 0)
            (tools / "Other.java").write_text("class Other {}\n")
            git(directory, "add", "-A")
            git(directory, "commit", "-qm", "other")
            self.assertEqual(check(directory, "--history").returncode, 1)




class CommercialDenylistTests(unittest.TestCase):
    def repo(self, directory, files):
        git(directory, "init", "-q", "-b", "main")
        for path, body in files:
            dest = Path(directory) / path
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text(body)
        git(directory, "add", "-A")
        git(directory, "commit", "-qm", "tree")

    def test_a_removed_crate_or_launcher_page_in_the_tree_fails(self):
        cases = [
            "crates/sunrise-cosmetics/Cargo.toml",
            "crates/sunrise-client/src/game/cosmetics.rs",
            "crates/sunrise-client/src/game/interface/cosmetics/emotes.rs",
            "crates/sunrise-gameplay-common/src/game/cosmetics.rs",
            "crates/sunrise-gameplay/src/game/interface/cosmetics/emotes.rs",
            "crates/sunrise-client-gameplay/src/game/cosmetics/wheel.rs",
            "crates/sunrise-official-servers/src/lib.rs",
            "launcher/src/partner.rs",
            "launcher/src/shop/mod.rs",
            "launcher/src/wardrobe.rs",
            "launcher/src/wardrobe_page.rs",
            "launcher/assets/sculk-banner.webp",
            "launcher/src/main.rs",
        ]
        for path in cases:
            with self.subTest(path=path), TemporaryDirectory() as directory:
                self.repo(directory, [(path, "x\n"), ("readme.md", "ok\n")])
                result = check(directory)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn(path, result.stderr)
                self.assertIn("Removed commercial features must not come back", result.stderr)

    def test_kept_paths_and_nearby_names_are_allowed(self):
        allowed = [
            "crates/sunrise-official/Cargo.toml",
            "docs/cosmetics.md",
            "crates/sunrise-client/src/game/interface/names.rs",
        ]
        with TemporaryDirectory() as directory:
            self.repo(directory, [(path, "ok\n") for path in allowed])
            result = check(directory)
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_a_removed_path_only_in_history_does_not_fail(self):
        with TemporaryDirectory() as directory:
            self.repo(directory, [
                ("crates/sunrise-cosmetics/Cargo.toml", "old\n"),
                ("readme.md", "ok\n"),
            ])
            git(directory, "rm", "-q", "crates/sunrise-cosmetics/Cargo.toml")
            git(directory, "commit", "-qm", "removed")
            self.assertEqual(check(directory).returncode, 0)
            history = check(directory, "--history")
            self.assertEqual(history.returncode, 0, history.stderr)
