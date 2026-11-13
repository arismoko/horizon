import contextlib
import io
import unittest

from hotspots_repo import Repo
from tools.hotspots import open_prs


class OpenPrsTest(unittest.TestCase):
    def test_lists_the_prs_whose_own_diff_touches_a_path(self):
        r = Repo()
        self.addCleanup(r.close)
        r.commit("main", {"a/x.txt": "x\n", "a/z.txt": "z\n", "b/y.txt": "y\n"})
        r.pull(1, r.commit("p1", {"a/x.txt": "x1\n"}, base="main"))
        r.pull(2, r.commit("p2", {"b/y.txt": "y2\n"}, base="main"))
        # main moves on after both branched: its own change to a/z.txt is not theirs.
        r.commit("main", {"a/z.txt": "z main\n"})
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = open_prs.main(["--repo", str(r.path), "--main", "main", "a/", "*/y.txt"],
                               stdin=["#1 one\n", "2\ttwo\n", "3\n"])
        self.assertEqual(out.getvalue(), "1\ta/x.txt\n2\tb/y.txt\n")
        self.assertEqual(rc, 2)
        self.assertIn("#3", err.getvalue())


if __name__ == "__main__":
    unittest.main()
