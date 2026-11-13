"""Every test moved to the daily run is run by some leg of the daily matrix (tools/ci/check_ignored_test_rows.py)."""
import importlib.util
import json
import subprocess
import sys
import unittest
from pathlib import Path

from tools.ci import ignored_shards, ignored_tests

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("check_ignored_test_rows", ROOT / "tools/ci/check_ignored_test_rows.py")
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

FULL = [*ignored_shards.expand(ignored_shards.SHARDS), ignored_shards.DAILY_ROWS]


def row(package, test, target="lib"):
    return ignored_tests.Row(package, test, target, "daily", f"daily-{package}.txt", "slow")


class DailyRowsCoverage(unittest.TestCase):
    TARGETS = {"horizon-world": {"lib", "test:integration"}, "horizon-relay": {"lib", "test:tunnel"},
               "horizon-client": {"lib", "bin:horizon-client", "test:lan_bot"}}

    def test_the_committed_rows_all_run_in_the_full_daily_matrix(self):
        rows = ignored_tests.daily_rows()
        self.assertTrue(rows)
        self.assertEqual(check.daily_problems(rows, ignored_shards.workspace_targets(), FULL), [])

    def test_the_full_matrix_has_the_daily_rows_leg(self):
        printed = subprocess.run([sys.executable, str(ROOT / "tools/ci/ignored_shards.py"), "--matrix", "--all"],
                                 capture_output=True, text=True, check=True).stdout
        self.assertIn(ignored_shards.DAILY_ROWS, json.loads(printed.removeprefix("matrix="))["shard"])

    def test_a_row_no_leg_runs_fails(self):
        rows = [row("horizon-world", "a"), row("horizon-relay", "b", "test:tunnel")]
        self.assertEqual(check.daily_problems(rows, self.TARGETS, FULL), [])
        # Without the daily-rows leg the relay's integration test would run nowhere; the world's library is its shard's.
        problems = check.daily_problems(rows, self.TARGETS, ignored_shards.expand(ignored_shards.SHARDS))
        self.assertEqual(len(problems), 1)
        self.assertIn("horizon-relay b: no leg runs it", problems[0])
        problems = check.daily_problems(rows, self.TARGETS, [ignored_shards.DAILY_ROWS])
        self.assertEqual(len(problems), 1)
        self.assertIn("no leg runs horizon-world's shard", problems[0])

    def test_a_row_must_name_a_package_and_one_of_its_targets(self):
        problems = check.daily_problems([row("horizon-nothing", "a"), row("horizon-relay", "b", "test:missing")],
                                        self.TARGETS, FULL)
        self.assertIn("horizon-nothing is not a workspace package", problems[0])
        self.assertIn("horizon-relay has no target test:missing", problems[1])

    def test_each_row_runs_in_exactly_one_place(self):
        rows = [row("horizon-world", "a"), row("horizon-world", "b", "test:integration"),
                row("horizon-client", "c", "test:lan_bot"), row("horizon-relay", "d")]
        groups = ignored_tests.daily_rows_groups(rows)
        for entry in rows:
            in_shard = ignored_tests.shard_runs(entry.package, entry.reference)
            in_leg = entry.test in groups.get((entry.package, entry.reference), ())
            self.assertNotEqual(in_shard, in_leg, entry)

    def test_the_command_passes_on_the_committed_rows(self):
        result = subprocess.run([sys.executable, str(ROOT / "tools/ci/check_ignored_test_rows.py")],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("0 that no daily leg runs", result.stdout)


if __name__ == "__main__":
    unittest.main()
