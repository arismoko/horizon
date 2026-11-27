"""Every test moved to the daily run is run by some leg of the daily matrix (tools/ci/check_ignored_test_rows.py)."""
import importlib.util
import json
import subprocess
import sys
import tempfile
import textwrap
import unittest
import unittest.mock
from pathlib import Path

from tools.ci import ignored_shards, ignored_tests

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("check_ignored_test_rows", ROOT / "tools/ci/check_ignored_test_rows.py")
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

FULL = [*ignored_shards.expand(ignored_shards.SHARDS), *ignored_shards.DAILY_ROWS_LEGS.values()]


def row(package, test, target="lib"):
    return ignored_tests.Row(package, test, target, "daily", f"daily-{package}.txt", "slow")


class DailyRowsCoverage(unittest.TestCase):
    TARGETS = {"sunrise-world": {"lib", "test:integration"}, "sunrise-relay": {"lib", "test:tunnel"},
               "sunrise-client": {"lib", "bin:sunrise-client", "test:lan_bot"}}

    def test_the_committed_rows_all_run_in_the_full_daily_matrix(self):
        rows = [entry for entry in ignored_tests.listed_entries() if entry.mark == "daily"]
        self.assertTrue(rows)
        self.assertEqual(check.daily_problems(rows, ignored_shards.workspace_targets(), FULL), [])

    def test_the_full_matrix_has_the_daily_rows_leg(self):
        printed = subprocess.run([sys.executable, str(ROOT / "tools/ci/ignored_shards.py"), "--matrix", "--all"],
                                 capture_output=True, text=True, check=True).stdout
        legs = json.loads(printed.removeprefix("matrix="))["shard"]
        self.assertIn(ignored_shards.DAILY_ROWS, legs)

    def test_a_row_no_leg_runs_fails(self):
        rows = [row("sunrise-world", "a"), row("sunrise-relay", "b", "test:tunnel")]
        self.assertEqual(check.daily_problems(rows, self.TARGETS, FULL), [])
        # Without the daily-rows leg the relay's integration test would run nowhere; the world's library is its shard's.
        problems = check.daily_problems(rows, self.TARGETS, ignored_shards.expand(ignored_shards.SHARDS))
        self.assertEqual(len(problems), 1)
        self.assertIn("sunrise-relay b: no leg runs it", problems[0])
        problems = check.daily_problems(rows, self.TARGETS, [ignored_shards.DAILY_ROWS])
        self.assertEqual(len(problems), 1)
        self.assertIn("no leg runs sunrise-world's shard", problems[0])

    def test_a_windows_row_is_run_by_the_windows_job_not_a_linux_leg(self):
        rows = [ignored_tests.Row("sunrise-relay", "b", "test:tunnel", "daily", "daily-sunrise-relay.txt", "windows")]
        self.assertEqual(ignored_tests.daily_rows_groups(rows), {})
        # Accepted with only the shards and the daily-rows leg, as a row no Linux leg runs would not be.
        self.assertEqual(check.daily_problems(rows, self.TARGETS, FULL), [])
        self.assertEqual(check.daily_problems(rows, self.TARGETS, []), [])
        with unittest.mock.patch.object(check, "windows_daily_runs", return_value=False):
            self.assertIn("daily.yml must call", check.daily_problems(rows, self.TARGETS, FULL)[0])

    def test_own_sdk_tests_are_run_by_exact_name_only_in_quickstart(self):
        log = row("cargo-sunrise-mod", "run_writes_native_registration_logs_to_latest_log", "test:run")
        log = log._replace(reason="own-sdk")
        self.assertEqual(ignored_tests.daily_rows_groups([log]), {})
        targets = {log.package: {log.reference}}
        self.assertEqual(check.daily_problems([log], targets, []), [])
        missing = log._replace(test="run_writes_missing_log")
        self.assertIn("by exact name", check.daily_problems([missing], targets, FULL)[0])

    def test_an_own_sdk_call_runs_after_resolve_and_sdk_and_is_not_switched_off(self):
        log = row("cargo-sunrise-mod", "run_writes_native_registration_logs_to_latest_log", "test:run")
        with tempfile.TemporaryDirectory() as tmp:
            workflows = Path(tmp) / ".github/workflows"
            workflows.mkdir(parents=True)
            (workflows / "book-quickstart.yml").write_bytes(
                (ROOT / ".github/workflows/book-quickstart.yml").read_bytes())
            cases = [("", True), ("if: false", False), ("if: ${{ false }}", False),
                     ("if: ${{false}}", False), ("if: ${{  false  }}", False)]
            for needs in ("[resolve, sdk]", "[resolve]", "[sdk]"):
                cases_for = cases if needs == "[resolve, sdk]" else [("", False)]
                for condition, runs in cases_for:
                    (workflows / "daily.yml").write_text(textwrap.dedent(f"""\
                        jobs:
                          book-quickstart:
                            needs: {needs}
                            {condition}
                            uses: ./.github/workflows/book-quickstart.yml
                            with:
                              commit: ${{{{ needs.resolve.outputs.commit }}}}
                              own_sdk: true
                        """), encoding="utf-8")
                    self.assertEqual(check.own_sdk_daily_runs(Path(tmp), log), runs, (needs, condition))

    def test_a_row_must_name_a_package_and_one_of_its_targets(self):
        problems = check.daily_problems([row("sunrise-nothing", "a"), row("sunrise-relay", "b", "test:missing")],
                                        self.TARGETS, FULL)
        self.assertIn("sunrise-nothing is not a workspace package", problems[0])
        self.assertIn("sunrise-relay has no target test:missing", problems[1])

    def test_each_row_runs_in_exactly_one_place(self):
        rows = [row("sunrise-world", "a"), row("sunrise-world", "b", "test:integration"),
                row("sunrise-client", "c", "test:lan_bot"), row("sunrise-relay", "d"),
                row("sunrise-gameplay-common", "e"), row("sunrise-gameplay", "f"),
                row("sunrise-client-gameplay", "g")]
        groups = ignored_tests.daily_rows_groups(rows)
        for entry in rows:
            in_shard = ignored_tests.shard_runs(entry.package, entry.reference)
            in_leg = entry.test in groups.get((entry.package, entry.reference), ())
            self.assertNotEqual(in_shard, in_leg, entry)
        for package in ("sunrise-gameplay-common", "sunrise-gameplay", "sunrise-client-gameplay"):
            self.assertTrue(ignored_tests.shard_runs(package, "lib"), package)
            self.assertNotIn((package, "lib"), groups)


class WindowsDailyWorkflow(unittest.TestCase):
    DAILY = textwrap.dedent("""\
        jobs:
          windows-runtime-leases:
            uses: ./.github/workflows/windows-runtime-leases.yml
            with:
              commit: ${{ needs.resolve.outputs.commit }}
        """)
    COMMAND = 'python tools/ci/ignored_tests.py --windows-rows --run --report "$RUNNER_TEMP/daily-report.json"'
    WINDOWS = textwrap.dedent(f"""\
        on:
          workflow_call:
        jobs:
          runtime-leases:
            if: inputs.commit != '' || github.event_name == 'workflow_dispatch'
            runs-on: windows-latest
            steps:
              - run: {COMMAND}
        """)

    def wired(self, *, daily=DAILY, windows=WINDOWS, unrelated=None):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            workflows = root / ".github/workflows"
            workflows.mkdir(parents=True)
            (workflows / "daily.yml").write_text(daily)
            (workflows / "windows-runtime-leases.yml").write_text(windows)
            if unrelated is not None:
                (workflows / "unrelated.yml").write_text(unrelated)
            return check.windows_daily_runs(root)

    def test_the_daily_calls_the_enabled_windows_runner(self):
        self.assertTrue(self.wired())

    def test_a_shell_comment_is_not_a_runner(self):
        self.assertFalse(self.wired(windows=self.WINDOWS.replace(f"run: {self.COMMAND}",
                                                                f"run: |\n          # {self.COMMAND}")))

    def test_a_runner_in_an_uncalled_workflow_does_not_count(self):
        self.assertFalse(self.wired(windows=self.WINDOWS.replace(self.COMMAND, "echo skipped"), unrelated=self.WINDOWS))

    def test_a_missing_uses_does_not_count(self):
        self.assertFalse(self.wired(daily=self.DAILY.replace("uses: ./.github/workflows/windows-runtime-leases.yml",
                                                           "# uses: ./.github/workflows/windows-runtime-leases.yml")))

    def test_a_disabled_job_or_step_does_not_count(self):
        for mutation in (self.WINDOWS.replace("- run:", "- if: ${{ false }}\n        run:"),
                         self.WINDOWS.replace("inputs.commit != '' || github.event_name == 'workflow_dispatch'",
                                              "false")):
            with self.subTest(workflow=mutation):
                self.assertFalse(self.wired(windows=mutation))

    def test_a_list_only_invocation_does_not_count(self):
        self.assertFalse(self.wired(windows=self.WINDOWS.replace(self.COMMAND,
                                                                "python tools/ci/ignored_tests.py --windows-rows")))


class DailyRowsMatchTheSource(unittest.TestCase):
    """The rows and the `#[ignore = "daily"]` attributes agree both ways, by package and function name."""
    CLIENT = {"sunrise-client", "sunrise-client-lib"}

    def test_an_attribute_without_a_row_fails_with_what_to_do(self):
        problems = check.source_problems([], [("crates/sunrise-relay/tests/tunnel.rs", 9, "slow", {"sunrise-relay"})])
        self.assertEqual(len(problems), 1)
        self.assertTrue(problems[0].startswith("crates/sunrise-relay/tests/tunnel.rs:9: `slow` is #[ignore"))
        self.assertIn("tools/ci/move_to_daily.py apply", problems[0])

    def test_a_row_without_an_attribute_fails_with_what_to_do(self):
        problems = check.source_problems([row("sunrise-relay", "gone", "test:tunnel")], [])
        self.assertEqual(len(problems), 1)
        self.assertIn("sunrise-relay gone: no function `gone`", problems[0])
        self.assertIn("restore the attribute", problems[0])

    def test_a_function_built_into_two_targets_has_one_attribute_and_a_row_per_target(self):
        functions = [("crates/sunrise-client-render/src/gfx/sunrise_fan.rs", 186, "angular", {"sunrise-client-render"})]
        rows = [row("sunrise-client-render", "gfx::sunrise_fan::tests::angular"),
                row("sunrise-client-render", "sunrise_fan::tests::angular", "test:sunrise_shader")]
        self.assertEqual(check.source_problems(rows, functions), [])

    def test_a_shared_source_folder_belongs_to_both_packages(self):
        owners = check.source_packages()
        self.assertEqual(owners["crates/sunrise-client"], self.CLIENT)
        functions = [("crates/sunrise-sim/src/save.rs", 3, "slow", self.CLIENT)]
        self.assertEqual(check.source_problems([row("sunrise-client-lib", "save::tests::slow")], functions), [])
        self.assertEqual(len(check.source_problems([row("sunrise-world", "save::tests::slow")], functions)), 2)



class RowlessIgnores(unittest.TestCase):
    """An #[ignore]d test no daily shard runs needs a row of some mark; in a shard's own targets it runs anyway."""

    def ignored(self, path, packages, kind, name="slow"):
        return (path, 7, name, packages, kind, False)

    def test_only_tests_no_shard_runs_need_a_row(self):
        functions = [self.ignored("crates/sunrise-world/src/a.rs", {"sunrise-world"}, "lib"),
                     self.ignored("crates/sunrise-client/src/game/b.rs", {"sunrise-client", "sunrise-client-lib"}, "bin"),
                     self.ignored("crates/sunrise-world/tests/c.rs", {"sunrise-world"}, "test"),
                     self.ignored("crates/sunrise-core/src/d.rs", {"sunrise-core"}, "lib")]
        problems = check.rowless_problems([], functions)
        self.assertEqual([problem.split(":", 1)[0] for problem in problems],
                         ["crates/sunrise-world/tests/c.rs", "crates/sunrise-core/src/d.rs"])
        self.assertIn("runs nowhere", problems[0])
        self.assertIn("manual-PACKAGE.txt", problems[0])
        rows = [ignored_tests.Row("sunrise-world", "it::slow", "by-hand", "manual", "manual-sunrise-world.txt"),
                ignored_tests.Row("sunrise-core", "d::slow", "#12", None, "issue-12.txt")]
        self.assertEqual(check.rowless_problems(rows, functions), [])

    def test_where_a_source_file_builds(self):
        self.assertEqual([check.target_kind("crates/x", path) for path in (
            "crates/x/src/lib.rs", "crates/x/src/main.rs", "crates/x/src/bin/tool.rs", "crates/x/tests/it.rs",
            "crates/x/examples/probe.rs", "crates/x/src/a/tests.rs")], ["lib", "bin", "bin", "test", "example", "lib"])



class WindowsDailyTests(unittest.TestCase):
    def test_the_windows_daily_job_runs_exactly_the_packages_daily_rows(self):
        rows = [row("sunrise-mod-package", "b::slow"), row("sunrise-mod-package", "a::slow"),
                row("sunrise-mod-package", "it::slow", "test:it"), row("sunrise-world", "c::slow")]
        with unittest.mock.patch.object(ignored_tests, "daily_rows", return_value=rows):
            self.assertEqual(ignored_tests.daily_tests("sunrise-mod-package", "lib"), ["a::slow", "b::slow"])
        printed = subprocess.run([sys.executable, str(ROOT / "tools/ci/ignored_tests.py"), "--daily-tests",
                                  "sunrise-mod-package", "lib"], capture_output=True, text=True, check=True).stdout
        self.assertIn("holder::tests::private_version_is_refused_before_normal_startup", printed.split())
        self.assertNotIn("holder::game_entry::tests::write_missing_registration_actor", printed.split())
        workflow = (ROOT / ".github/workflows/windows-runtime-leases.yml").read_text()
        self.assertIn("ignored_tests.py --daily-tests sunrise-mod-package lib", workflow)
        self.assertNotIn("< <(", workflow)  # a process substitution would hide the reader's failure
        # No rows: no output at all, not one empty name.
        empty = subprocess.run([sys.executable, str(ROOT / "tools/ci/ignored_tests.py"), "--daily-tests",
                                "no-such-package", "lib"], capture_output=True, text=True, check=True).stdout
        self.assertEqual(empty, "")
        self.assertIn('-- --ignored --exact "${tests[@]}"', workflow)


if __name__ == "__main__":
    unittest.main()
