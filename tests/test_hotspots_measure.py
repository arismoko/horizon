import unittest

from hotspots_repo import Repo
from tools.hotspots import measure

LIST = "pub use a::{\n    A,\n    B,\n};\n"


def with_name(name):
    return LIST.replace("    B,\n", f"    B,\n    {name},\n")


class MeasureTest(unittest.TestCase):
    def setUp(self):
        self.repo = Repo()
        self.addCleanup(self.repo.close)
        r = self.repo
        r.commit("main", {"src/list.rs": LIST, "y.txt": "x\n", "other.txt": "o\n"}, "upstream's last commit")
        r.git("update-ref", "refs/remotes/upstream/main", "main")

    def run_measure(self, *extra):
        return measure.measure(measure.parse_args(["--repo", str(self.repo.path), "--main", "main", *extra]))

    def test_a_member_without_a_number_and_main_merged_into_a_pr_branch(self):
        r = self.repo
        pr1 = r.commit("pr1", {"src/list.rs": with_name("B1")}, base="main")
        pr2 = r.commit("carry-2", {"src/list.rs": with_name("B2")}, base="main")
        pr3 = r.commit("pr3", {"other.txt": "o3\n"}, base="main")
        r.commit("pr4", {"src/list.rs": with_name("B4")}, base="main")
        for n, sha in ((1, pr1), (3, pr3)):
            r.pull(n, sha)
        r.git("checkout", "-q", "-b", "fork-review-rollup-1", "main")
        r.merge("fork-review-rollup-1", "pr1", "Merge PR #1 into fork-review-rollup-1")
        r.merge("fork-review-rollup-1", "pr3", "Merge PR #3 into fork-review-rollup-1")
        r.merge("fork-review-rollup-1", "carry-2", "Merge fork-review-carry-2 into fork-review-rollup-1",
                resolve={"src/list.rs": LIST.replace("    B,\n", "    B,\n    B1,\n    B2,\n")})
        r.merge("main", "fork-review-rollup-1", "Merge pull request #10 from o/fork-review-rollup-1")
        r.merge("pr4", "main", "Merge branch 'main' into pr4",
                resolve={"src/list.rs": LIST.replace("    B,\n", "    B,\n    B1,\n    B2,\n    B4,\n")})
        r.pull(4, r.git("rev-parse", "pr4"))
        r.merge("main", "pr4", "Merge pull request #4 from o/pr4")

        git, raw = self.run_measure()
        entry = raw["files"]["src/list.rs"]
        self.assertEqual(entry["prs"], sorted(["#1", "#4", pr2[:12]]))
        c = measure.counts(entry)
        self.assertEqual((c["rollup"], c["branch"], c["sync"]), (1, 1, 0))
        self.assertEqual(raw["prs"], 4)
        body = measure.report(git, raw, min_rollup=1, min_prs=99)
        self.assertIn("### `src/list.rs`", body)
        self.assertIn("- `pub use a::{` (2)", body)
        self.assertNotIn("other.txt", body)

    def rollup_of_two_conflicting_prs(self):
        r = self.repo
        heads = {n: r.commit(f"p{n}", {"y.txt": f"{n}\n"}, base="main") for n in (21, 22, 24)}
        for n, sha in heads.items():
            r.pull(n, sha)
        r.git("checkout", "-q", "-b", "fork-review-rollup-1", "main")
        r.merge("fork-review-rollup-1", "p21", "Merge PR #21 into fork-review-rollup-1")
        r.merge("fork-review-rollup-1", "p22", "Merge PR #22 into fork-review-rollup-1", resolve={"y.txt": "21 22\n"})
        r.git("checkout", "-q", "-b", "fork-review-rollup-2", "main")
        r.merge("fork-review-rollup-2", "p24", "Merge PR #24 into fork-review-rollup-2")
        r.merge("fork-review-rollup-2", "fork-review-rollup-1", "Merge rollup 1 into fork-review-rollup-2",
                resolve={"y.txt": "21 22 24\n"})

    def test_a_rollup_merged_into_the_next_is_replayed_once_and_is_no_pr(self):
        self.rollup_of_two_conflicting_prs()
        self.repo.merge("main", "fork-review-rollup-2", "Merge pull request #30 from o/fork-review-rollup-2")

        _, raw = self.run_measure()
        entry = raw["files"]["y.txt"]
        self.assertEqual(entry["prs"], ["#21", "#22", "#24"])
        # #22 against #21 in rollup 1, and rollup 1 against #24 in rollup 2.
        self.assertEqual(measure.counts(entry)["rollup"], 2)
        self.assertEqual(raw["nested_rollups"], 1)
        # main's merge, rollup 1's into rollup 2, #24's, #21's and #22's
        self.assertEqual(raw["replayed"], 5)

    def test_a_rollup_merged_into_main_and_into_the_next_counts_once(self):
        self.rollup_of_two_conflicting_prs()
        r = self.repo
        r.merge("main", "fork-review-rollup-1", "Merge pull request #30 from o/fork-review-rollup-1")
        r.merge("fork-review-rollup-2", "main", "Merge main into fork-review-rollup-2")
        r.merge("main", "fork-review-rollup-2", "Merge pull request #31 from o/fork-review-rollup-2")

        _, raw = self.run_measure()
        entry = raw["files"]["y.txt"]
        self.assertEqual(entry["prs"], ["#21", "#22", "#24"])
        self.assertEqual(measure.counts(entry)["rollup"], 2)
        self.assertEqual(raw["units"], 3)

    def test_a_sync_riding_a_rollup_is_a_sync_not_a_pr(self):
        r = self.repo
        upstream = r.commit("up", {"y.txt": "upstream\n"}, base="main")
        r.git("update-ref", "refs/remotes/upstream/main", upstream)
        r.pull(7, r.commit("p7", {"y.txt": "7\n"}, base="main"))
        r.merge("main", "p7", "Merge pull request #7 from o/p7")
        r.git("checkout", "-q", "-b", "sync-1", "main")
        r.merge("sync-1", "up", "Merge upstream", resolve={"y.txt": "7 upstream\n"})
        r.pull(40, r.git("rev-parse", "sync-1"))
        r.pull(8, r.commit("p8", {"other.txt": "8\n"}, base="main"))
        r.git("checkout", "-q", "-b", "fork-review-rollup-3", "main")
        r.merge("fork-review-rollup-3", "p8", "Merge PR #8 into fork-review-rollup-3")
        r.merge("fork-review-rollup-3", "sync-1", "Merge PR #40 into fork-review-rollup-3")
        r.merge("main", "fork-review-rollup-3", "Merge pull request #41 from o/fork-review-rollup-3")

        _, raw = self.run_measure()
        entry = raw["files"]["y.txt"]
        self.assertEqual(entry["prs"], ["#7"])
        c = measure.counts(entry)
        self.assertEqual((c["sync"], c["rollup"], c["branch"]), (1, 0, 0))
        self.assertEqual(raw["prs"], 2)

    def test_a_rollup_carrying_a_sync_merged_into_the_next_keeps_its_prs(self):
        r = self.repo
        upstream = r.commit("up", {"y.txt": "upstream\n"}, base="main")
        r.git("update-ref", "refs/remotes/upstream/main", upstream)
        r.git("checkout", "-q", "-b", "sync-1", "main")
        r.merge("sync-1", "up", "Merge upstream")
        r.pull(40, r.git("rev-parse", "sync-1"))
        r.pull(8, r.commit("p8", {"other.txt": "8\n"}, base="main"))
        r.pull(9, r.commit("p9", {"z.txt": "9\n"}, base="main"))
        r.git("checkout", "-q", "-b", "fork-review-rollup-5", "main")
        r.merge("fork-review-rollup-5", "p8", "Merge PR #8 into fork-review-rollup-5")
        r.merge("fork-review-rollup-5", "sync-1", "Merge PR #40 into fork-review-rollup-5")
        r.git("checkout", "-q", "-b", "fork-review-rollup-6", "main")
        r.merge("fork-review-rollup-6", "p9", "Merge PR #9 into fork-review-rollup-6")
        r.merge("fork-review-rollup-6", "fork-review-rollup-5", "Merge rollup 5 into fork-review-rollup-6")
        r.merge("main", "fork-review-rollup-6", "Merge pull request #43 from o/fork-review-rollup-6")

        _, raw = self.run_measure()
        self.assertEqual(raw["files"]["other.txt"]["prs"], ["#8"])
        self.assertEqual(raw["files"]["z.txt"]["prs"], ["#9"])
        self.assertNotIn("y.txt", raw["files"])
        self.assertEqual(raw["prs"], 2)

    def test_a_carry_branch_and_an_earlier_head_map_to_their_prs(self):
        r = self.repo
        r.pull(5, r.commit("p5", {"a.txt": "5\n"}, base="main"))
        r.commit("carry-5", {"a.txt": "5 carried\n"}, base="p5")
        merged = r.commit("p6", {"b.txt": "6\n"}, base="main")
        r.pull(6, r.commit("p6", {"b.txt": "6 later\n"}))
        r.git("checkout", "-q", "-b", "fork-review-rollup-4", "main")
        r.merge("fork-review-rollup-4", "carry-5", "Merge carry into fork-review-rollup-4")
        r.merge("fork-review-rollup-4", merged, "Merge an earlier head into fork-review-rollup-4")
        r.merge("main", "fork-review-rollup-4", "Merge pull request #42 from o/fork-review-rollup-4")

        _, raw = self.run_measure()
        self.assertEqual(raw["files"]["a.txt"]["prs"], ["#5"])
        self.assertEqual(raw["files"]["b.txt"]["prs"], ["#6"])

    def test_the_window_leaves_out_older_merges(self):
        r = self.repo
        r.pull(5, r.commit("p5", {"other.txt": "5\n"}, base="main"))
        r.merge("main", "p5", "Merge pull request #5 from o/p5")
        r.tick(10 * 86400)
        r.pull(6, r.commit("p6", {"y.txt": "6\n"}, base="main"))
        r.merge("main", "p6", "Merge pull request #6 from o/p6")

        _, raw = self.run_measure("--days", "3")
        self.assertEqual(raw["prs"], 1)
        self.assertNotIn("other.txt", raw["files"])


if __name__ == "__main__":
    unittest.main()
