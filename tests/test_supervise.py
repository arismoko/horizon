import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from tests.git_fixture import git_in, isolated_git_env
from tools import supervise


class TrialMergeTests(unittest.TestCase):
    def test_the_largest_compatible_set_avoids_every_conflicting_pair(self):
        conflicts = {frozenset(("a", "b")), frozenset(("b", "c"))}
        self.assertEqual(supervise.largest_compatible(["a", "b", "c", "d"], conflicts), ["a", "c", "d"])
        self.assertEqual(supervise.largest_compatible(["b", "a"], {frozenset(("a", "b"))}), ["b"])

    def test_plan_keeps_branches_that_merge_cleanly_together(self):
        with (
            tempfile.TemporaryDirectory() as temp,
            mock.patch.object(supervise, "ROOT", Path(temp)),
            mock.patch.dict(os.environ, isolated_git_env(), clear=True),
        ):
            git = git_in(temp)

            def commit(branch, name, text, start="base"):
                git("checkout", "-q", "-B", branch, start)
                Path(temp, name).write_text(text)
                git("add", name)
                git("commit", "-q", "-m", f"{branch} {name}")

            git("init", "-q", "-b", "base")
            # The planner's trial commits need the fixture's identity too.
            git("config", "--local", "user.name", "t")
            git("config", "--local", "user.email", "t@t")
            Path(temp, "shared.txt").write_text("one\n")
            git("add", "shared.txt")
            git("commit", "-q", "-m", "start")
            commit("parity/a", "a.txt", "a\n")
            commit("parity/b", "shared.txt", "b\n")
            commit("parity/c", "shared.txt", "c\n")  # conflicts with parity/b only
            commit("parity/d", "d.txt", "d\n")
            commit("parity/e", "e.txt", "e\n", start="base")
            git("checkout", "-q", "base")
            commit("base", "shared.txt", "base\n")  # parity/b and parity/c now conflict with the base too
            chosen, head = supervise.plan("base", ["parity/a", "parity/b", "parity/c", "parity/d"], verbose=False)
            self.assertEqual(chosen, ["parity/a", "parity/d"])
            files = git("ls-tree", "--name-only", head)
            self.assertEqual(files.split(), ["a.txt", "d.txt", "shared.txt"])
