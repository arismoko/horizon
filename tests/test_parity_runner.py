from pathlib import Path
import argparse
import contextlib
import io
import json
import os
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest import mock

from tools import parity, parity_shard
from tools.ci import parity_shards, parity_verdict


class ParityRunnerTests(unittest.TestCase):
    def test_dep_info_keeps_escaped_spaces(self):
        with tempfile.TemporaryDirectory() as temp:
            binary = Path(temp) / "probe"
            Path(temp, "probe.d").write_text(f"{binary}: /a/x.rs /b/with\\ space.rs\n\n/a/x.rs:\n")
            self.assertEqual(parity.dep_info(binary), ["/a/x.rs", "/b/with space.rs"])

    def test_artifacts_follow_profile_and_stop_at_program_arguments(self):
        self.assertEqual(parity.artifacts_of(["cargo", "build", "--release", "--example", "a", "--bin", "b"]),
                         [["example", "a", "release", [], True], ["bin", "b", "release", [], True]])
        self.assertEqual(parity.artifacts_of(["cargo", "run", "--profile", "fast", "--example", "c", "--", "--example", "x"]),
                         [["example", "c", "fast", [], True]])
        self.assertEqual(parity.artifacts_of(["cargo", "build", "--example", "d"]), [["example", "d", "dev", [], True]])

    def test_the_test_profile_builds_into_devs_folder(self):
        # lab.parity_fixtures() builds sunrise-parity-fixtures with --profile test: its dep-info is under debug/.
        self.assertEqual(parity.artifact_path("bin", "sunrise-parity-fixtures", "test"),
                         parity.TARGET / "debug" / "sunrise-parity-fixtures")
        self.assertEqual(parity.artifact_path("example", "probe", "fast"), parity.TARGET / "fast" / "examples" / "probe")

    def test_artifacts_keep_the_package_and_feature_selection(self):
        self.assertEqual(parity.artifacts_of(["cargo", "build", "--locked", "--profile=fast", "-j", "4", "--package",
                                              "sunrise-client", "--bins", "-F", "x"]),
                         [["bins", "", "fast", ["-p", "sunrise-client", "--features", "x"], False]])
        self.assertEqual(parity.artifacts_of(["cargo", "build", "--release", "-p", "sunrise-world", "--bin=sunrise-world"]),
                         [["bin", "sunrise-world", "release", ["-p", "sunrise-world"], False]])

    def test_prebuild_follows_a_moved_target_without_losing_features(self):
        selection = ["-p", "sunrise-client", "--no-default-features", "--features", "sunrise-client/rng-trace"]
        artifacts = [["example", "client_probe", "fast", selection, True],
                     ["bin", "sunrise-ctl", "fast", selection, False]]
        packages = {("example", "client_probe"): "sunrise-client-lib", ("bin", "sunrise-ctl"): "sunrise-client-lib"}
        commands = parity.prebuild_commands(artifacts, packages)
        self.assertEqual(len(commands), 2)
        for command in commands:
            self.assertEqual(command[:5], ["cargo", "build", "--locked", "--profile", "fast"])
            self.assertEqual(command[5:5 + len(selection)], selection)
            self.assertEqual(command[5 + len(selection):7 + len(selection)], ["-p", "sunrise-client-lib"])
        self.assertEqual(artifacts[0][3], selection)

    def test_old_broad_client_selections_include_the_moved_targets(self):
        packages = {("bin", "sunrise-client"): "sunrise-client", ("bin", "sunrise-ctl"): "sunrise-client-lib",
                    ("example", "client_probe"): "sunrise-client-lib"}
        for flag in ["--bins", "--examples"]:
            command = ["cargo", "build", "-p", "sunrise-client", "--no-default-features", flag]
            artifacts = parity.artifacts_of(command)
            before = [list(artifact[3]) for artifact in artifacts]
            self.assertEqual(parity.prebuild_commands(artifacts, packages),
                             [["cargo", "build", "--locked", "--profile", "dev", "-p", "sunrise-client",
                               "--no-default-features", "-p", "sunrise-client-lib", flag]])
            self.assertEqual([artifact[3] for artifact in artifacts], before)

    def test_prebuild_repeats_each_harness_selection(self):
        artifacts = [
            *parity.artifacts_of(["cargo", "build", "--release", "-p", "sunrise-world", "--example", "a"]),
            *parity.artifacts_of(["cargo", "build", "--release", "-p", "sunrise-world", "--example", "b"]),
            *parity.artifacts_of(["cargo", "build", "--release", "-p", "sunrise-world", "--bin", "sunrise-world"]),
            *parity.artifacts_of(["cargo", "build", "--release", "--bin", "sunrise-world"]),
            *parity.artifacts_of(["cargo", "run", "--profile", "fast", "-p", "sunrise-client", "--example", "c"]),
            *parity.artifacts_of(["cargo", "build", "--release", "-p", "sunrise-world", "--example", "renamed"]),
            ["example", "old", "release"],  # recorded before selections were kept: its own crate's -p
            ["example", "unknown", "release"],  # no longer in the workspace: left to its harness
        ]
        packages = {("example", "a"): "sunrise-world", ("example", "b"): "sunrise-world", ("example", "c"): "sunrise-client",
                    ("bin", "sunrise-world"): "sunrise-world", ("example", "old"): "sunrise-world"}
        self.assertEqual(parity.prebuild_commands(artifacts, packages), [
            ["cargo", "build", "--locked", "--profile", "fast", "-p", "sunrise-client", "--example", "c"],
            ["cargo", "build", "--locked", "--profile", "release", "--bin", "sunrise-world"],
            ["cargo", "build", "--locked", "--profile", "release", "-p", "sunrise-world", "--bin", "sunrise-world"],
            ["cargo", "build", "--locked", "--profile", "release", "-p", "sunrise-world", "--example", "a", "--example", "b",
             "--example", "old"],
        ])

    def test_a_probe_binary_depends_only_on_the_probes_it_ran(self):
        commands = [["/t/release/examples/world_probe", "a_fixture", "/dir"],
                    ["cargo", "run", "-p", "sunrise-client", "--example", "client_probe", "--", "b_fixture"]]
        self.assertEqual(parity.probes_run(commands), {"a_fixture", "b_fixture"})
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)):
            probe = Path(temp, "crates/w/examples/probe")
            (probe / "support").mkdir(parents=True)
            (probe / "a_fixture.rs").write_text('mod integration { include!("d_fixture.rs"); }')
            (probe / "d_fixture.rs").write_text("use crate::support::shared as s;")
            (probe / "b_fixture.rs").write_text("use crate::e_fixture;")
            for name in ("c_fixture.rs", "dispatch.rs", "e_fixture.rs", "main.rs", "support/shared.rs", "support/other.rs"):
                (probe / name).write_text("")
            paths = [f"crates/w/examples/probe/{name}" for name in ("a_fixture.rs", "b_fixture.rs", "c_fixture.rs",
                                                                    "d_fixture.rs", "dispatch.rs", "e_fixture.rs", "main.rs",
                                                                    "support/other.rs", "support/shared.rs")]
            self.assertEqual(parity.only_probes_run(["crates/w/src/lib.rs", *paths], {"a_fixture", "b_fixture"}),
                             ["crates/w/src/lib.rs", *paths[:2], *paths[3:6], paths[8]])
            self.assertEqual(parity.only_probes_run(paths, set()), paths)

    def test_probe_names_are_each_crates_own_target(self):
        self.assertEqual(parity.probes_run([
            ["C:/t/debug/examples/client_probe.exe", "a_fixture"],
            ["cargo", "run", "--release", "--example=world_probe", "--", "b_fixture", "x"],
            ["/t/release/examples/probe", "old_fixture"],  # the shared name before 2026-10-24
            ["cargo", "run", "--example", "probe", "--", "old_fixture"],
            ["/t/release/examples/mod_port_probe", "not_a_fixture"],
            ["cargo", "run", "--example", "mod_port_probe", "--", "not_a_fixture"],
            ["cargo", "build", "--example", "world_probe"],
        ]), {"a_fixture", "b_fixture"})

    def test_records_of_the_shared_probe_name_build_their_crates_probe(self):
        record = {"dependencies": ["crates/sunrise-world/examples/probe/a_fixture.rs", "crates/sunrise-world/src/lib.rs"],
                  "artifacts": [["example", "probe", "release", ["-p", "sunrise-client"], True],
                                ["example", "probe", "release"],  # no selection: the crate of its probe sources
                                ["example", "probe", "release", ["-p", "sunrise-core", "-p", "sunrise-world"], True],
                                ["bin", "sunrise-world", "release", ["-p", "sunrise-world"], False]]}
        artifacts = parity.recorded_artifacts(record)
        self.assertEqual(artifacts, [["example", "client_probe", "release", ["-p", "sunrise-client"], True],
                                     ["example", "world_probe", "release"],
                                     ["example", "world_probe", "release", ["-p", "sunrise-core", "-p", "sunrise-world"], True],
                                     ["bin", "sunrise-world", "release", ["-p", "sunrise-world"], False]])
        packages = {("example", "client_probe"): "sunrise-client", ("example", "world_probe"): "sunrise-world",
                    ("bin", "sunrise-world"): "sunrise-world"}
        self.assertEqual(parity.prebuild_commands(artifacts, packages), [
            ["cargo", "build", "--locked", "--profile", "release", "-p", "sunrise-client", "--example", "client_probe"],
            ["cargo", "build", "--locked", "--profile", "release", "-p", "sunrise-core", "-p", "sunrise-world",
             "--example", "world_probe"],
            ["cargo", "build", "--locked", "--profile", "release", "-p", "sunrise-world", "--bin", "sunrise-world"],
            ["cargo", "build", "--locked", "--profile", "release", "-p", "sunrise-world", "--example", "world_probe"],
        ])
        self.assertEqual(parity.recorded_artifacts({"artifacts": [["example", "probe", "release"]]}),
                         [["example", "probe", "release"]])  # no crate to tell: left to its harness

    def test_java_sources_include_classes_pulled_from_the_sourcepath(self):
        with tempfile.TemporaryDirectory() as temp:
            sources, classes = Path(temp, "src"), Path(temp, "classes")
            (sources / "p").mkdir(parents=True)
            (classes / "p").mkdir(parents=True)
            for name in ("Oracle", "Helper"):
                (sources / "p" / f"{name}.java").write_text("")
            for name in ("Oracle", "Helper$Inner"):
                (classes / "p" / f"{name}.class").write_bytes(b"")
            found = parity.java_sources(["javac", "-sourcepath", str(sources), "-d", str(classes), str(sources / "p/Oracle.java")])
            self.assertEqual(sorted(Path(path).name for path in found), ["Helper.java", "Oracle.java", "Oracle.java"])

    def test_a_pass_goes_stale_when_a_dependency_changes(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)):
            source = Path(temp, "a.rs")
            source.write_text("one")
            state = {"harnesses": {"x": {"status": "pass", "dependencies": ["a.rs"]}}, "hashes": {}}
            hasher = parity.Hasher(state["hashes"])
            state["harnesses"]["x"]["key"] = parity.key(state["harnesses"]["x"], hasher, "env")
            self.assertFalse(parity.stale("x", state, hasher, "env"))
            self.assertTrue(parity.stale("x", state, hasher, "other toolchain"))
            source.write_text("two!")
            self.assertTrue(parity.stale("x", state, hasher, "env"))

    def test_an_unreadable_trace_is_an_error_and_a_missing_one_is_an_unkeyed_pass(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)):
            script = Path(temp, "tools", "test_x_parity.py")
            missing = parity.outcome("x", script, True, 1.0, Path(temp, "none.json"))
            self.assertEqual((missing["status"], missing["dependencies"], missing["artifacts"]), ("pass", [], []))
            cut = Path(temp, "cut.json")
            cut.write_text('{"modules": [', encoding="utf-8")
            with self.assertRaises(ValueError):
                parity.outcome("x", script, True, 1.0, cut)

    def test_a_damaged_reach_cache_is_an_error(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)), \
                mock.patch.object(parity, "SHARED", Path(temp, "shared")):
            Path(temp, "shared").mkdir()
            Path(temp, "shared", "reach-cache.json").write_text("{not json", encoding="utf-8")
            found = {"food": Path(temp, "tools", "test_food_parity.py")}
            with self.assertRaises(ValueError):
                parity.affected_harnesses(found, {"food": {"dependencies": ["crates/a.rs"]}}, {"crates/a.rs"})

    def test_a_change_affects_the_harnesses_that_depend_on_it(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)), \
                mock.patch.object(parity, "SHARED", Path(temp, "shared")):
            found = {name: Path(temp, "tools", f"test_{name}_parity.py") for name in ("food", "light", "new")}
            records = {"food": {"dependencies": ["crates/a.rs", "tools/lab.py"]},
                       "light": {"dependencies": ["crates/lib.rs#keep:light"]}}
            self.assertEqual(parity.affected_harnesses(found, records, {"crates/a.rs"}), {"food", "new"})
            self.assertEqual(parity.affected_harnesses(found, records, {"crates/lib.rs"}), {"light", "new"})
            self.assertEqual(parity.affected_harnesses(found, records, {"tools/test_light_parity.py"}), {"light", "new"})
            self.assertEqual(parity.affected_harnesses(found, records, {"Cargo.lock"}), {"food", "light", "new"})
            self.assertEqual(parity.affected_harnesses(found, records, {"README.md"}), {"new"})

    def test_platform_specific_harnesses_are_skipped_elsewhere(self):
        config = {"linux_only": {"options": "libm"}}
        with mock.patch.object(parity.sys, "platform", "linux"):
            self.assertEqual(parity.platform_skips(config), {})
        with mock.patch.object(parity.sys, "platform", "darwin"):
            self.assertEqual(parity.platform_skips(config), {"options": "libm"})

    def test_termination_signals_are_caught_where_they_exist(self):
        for numbers in ({"SIGTERM": 15, "SIGHUP": 1}, {"SIGTERM": 15}):  # Windows has no SIGHUP
            handled = []
            fake = SimpleNamespace(**numbers, signal=lambda number, _handler: handled.append(number))
            with self.subTest(numbers=numbers), mock.patch.object(parity, "signal", fake):
                parity.exit_on_termination()
                self.assertEqual(handled, list(numbers.values()))

    def test_remote_executor_marks_setup_failure_as_dead(self):
        executor = parity.RemoteExecutor.__new__(parity.RemoteExecutor)
        executor.name = "remote"
        executor.config = {"ssh": "host", "root": "/tmp"}
        executor.mirror = "/tmp/mirror"
        executor.dead = False
        executor._log = mock.Mock()
        with mock.patch.object(parity.subprocess, "run", side_effect=OSError("ssh not found")):
            executor._connect([])
        self.assertTrue(executor.dead)
        executor._log.assert_called_once()

    def test_remote_executor_marks_worker_start_failure_as_dead(self):
        executor = parity.RemoteExecutor.__new__(parity.RemoteExecutor)
        executor.name = "remote"
        executor.config = {"ssh": "host", "root": "/tmp"}
        executor.mirror = "/tmp/mirror"
        executor.dead = False
        executor._log = mock.Mock()
        successful = SimpleNamespace(returncode=0, stderr="")
        with mock.patch.object(parity.subprocess, "run", return_value=successful), \
                mock.patch.object(parity.subprocess, "Popen", side_effect=OSError("ssh not found")):
            executor._connect([])
        self.assertTrue(executor.dead)
        executor._log.assert_called_once()

    def test_a_hosts_excludes_are_skipped_when_it_runs_parity_itself(self):
        config = {"hosts": {"playsunrise": {"exclude": {"remote_fireworks": "starved"}}, "macmini": {"exclude": {}}}}
        with mock.patch.object(parity.sys, "platform", "linux"):
            self.assertEqual(parity.platform_skips(config, "playsunrise"), {"remote_fireworks": "starved"})
            self.assertEqual(parity.platform_skips(config, "macmini"), {})
            self.assertEqual(parity.platform_skips(config, "elsewhere"), {})

    def test_named_only_harnesses_are_listed_from_the_host_config(self):
        self.assertEqual(parity.named_only({"named_only": {"lan_integration": "slow"}}), {"lan_integration": "slow"})
        self.assertEqual(parity.named_only({}), {})
        self.assertIn("lan_integration", parity.named_only(parity.host_config()))

    def test_the_daily_set_is_every_harness_except_named_only_and_their_shards(self):
        with tempfile.TemporaryDirectory() as temp, \
                mock.patch.object(parity, "load_state", return_value={"hashes": {}, "harnesses": {}}), \
                mock.patch.object(parity, "save_state"), mock.patch.object(parity, "exit_on_termination"), \
                mock.patch.object(parity, "environment_key", return_value="test"), \
                mock.patch.object(parity, "platform_skips", return_value={}), mock.patch("builtins.print"):
            manifest = Path(temp, "selection.json")
            found = parity.harnesses()
            names = parity.named_only(parity.host_config())
            self.assertLessEqual({"frame_identity", "command_exec"}, names.keys())
            # Every row holds back a harness an automatic run would select: not a PARITY_MANUAL proof, which no run
            # selects, nor a name that no longer exists.
            self.assertLessEqual(names.keys(), {parity.base_name(name) for name in found})
            expected = {name for name in found if parity.base_name(name) not in names}
            for args, selected in ((["--all", "--skip-named-only"], expected), (["--all"], set(found)),
                                   (["frame_identity"], {"frame_identity"})):
                with self.subTest(args=args), mock.patch.object(
                        sys, "argv", ["parity.py", *args, "--list", "--selection-file", str(manifest)]):
                    self.assertEqual(parity.main(), 0)
                    self.assertEqual(set(json.loads(manifest.read_text())), selected)

    def test_the_planner_splits_sorted_labels_round_robin(self):
        with tempfile.TemporaryDirectory() as temp:
            selection = Path(temp, "input.json")
            selection.write_text(json.dumps(["wolf", "chest#2", "boat", "chest#1", "zombie"]))
            out = Path(temp, "plan")
            self.assertEqual(parity_shards.write_selection(selection, out, count=2), 2)
            self.assertEqual(json.loads((out / "selection.json").read_text()),
                             ["boat", "chest#1", "chest#2", "wolf", "zombie"])
            self.assertEqual(json.loads((out / "bucket-0.json").read_text()), ["boat", "chest#2", "zombie"])
            self.assertEqual(json.loads((out / "bucket-1.json").read_text()), ["chest#1", "wolf"])

    def test_a_planned_share_runs_exactly_the_harnesses_it_names(self):
        planned = ["boat", "chest#2", "wolf"]
        found = dict.fromkeys([*planned, "chest#1", "zombie"])
        with tempfile.TemporaryDirectory() as temp, \
                mock.patch.object(parity, "harnesses", return_value=found), \
                mock.patch.object(parity, "load_state", return_value={"hashes": {}, "harnesses": {}}), \
                mock.patch.object(parity, "save_state"), mock.patch.object(parity, "exit_on_termination"), \
                mock.patch.object(parity, "environment_key", return_value="test"), \
                mock.patch.object(parity, "platform_skips", return_value={"chest": "worker exclusion"}), \
                mock.patch.object(parity, "named_only", return_value={"wolf": "worker named-only"}), \
                mock.patch.object(parity, "changed_harnesses", return_value={"zombie"}), mock.patch("builtins.print"):
            manifest = Path(temp, "selection.json")
            share = Path(temp, "bucket-0.json")
            share.write_text(json.dumps(planned))
            with mock.patch.object(sys, "argv", [
                    "parity.py", "--all", "--skip-named-only", "--exclude", "boat", "--flake-check", "main",
                    "--harness-file", str(share), "--list", "--selection-file", str(manifest)]):
                self.assertEqual(parity.main(), 0)
            self.assertEqual(json.loads(manifest.read_text()), planned)

    def test_a_share_naming_a_harness_this_tree_lacks_is_refused(self):
        with tempfile.TemporaryDirectory() as temp, \
                mock.patch.object(parity, "load_state", return_value={"hashes": {}, "harnesses": {}}), \
                mock.patch.object(parity, "save_state"), mock.patch.object(parity, "exit_on_termination"), \
                mock.patch.object(parity, "environment_key", return_value="test"), \
                mock.patch.object(parity, "platform_skips", return_value={}), mock.patch("builtins.print"):
            share = Path(temp, "bucket-0.json")
            share.write_text(json.dumps(["food", "no_such_harness_in_this_tree"]))
            with mock.patch.object(sys, "argv", ["parity.py", "--all", "--harness-file", str(share), "--list"]), \
                    mock.patch("sys.stderr"):
                with self.assertRaises(SystemExit):
                    parity.main()

    def test_only_a_whole_run_marks_the_run_finished(self):
        with tempfile.TemporaryDirectory() as temp, \
                mock.patch.object(parity, "load_state", return_value={"hashes": {}, "harnesses": {}}), \
                mock.patch.object(parity, "save_state"), mock.patch.object(parity, "exit_on_termination"), \
                mock.patch.object(parity, "environment_key", return_value="test"), \
                mock.patch.object(parity, "stale", return_value=False), \
                mock.patch.object(parity, "platform_skips", return_value={}), mock.patch("builtins.print"):
            marker = Path(temp, "last-run.json")
            empty = Path(temp, "bucket-0.json")
            empty.write_text("[]")
            # Nothing to do: a whole run still records one for tools/gate.py.
            with mock.patch.object(parity, "STATE_DIR", Path(temp)):
                with mock.patch.dict(os.environ, PARITY_RUN_ID="whole"), \
                        mock.patch.object(sys, "argv", ["parity.py"]):
                    self.assertEqual(parity.main(), 0)
                previous = marker.read_bytes()
                self.assertEqual(json.loads(previous), {"run": "whole", "failed": []})
                # A share must not overwrite the marker with its own run ID.
                with mock.patch.dict(os.environ, PARITY_RUN_ID="share"), \
                        mock.patch.object(sys, "argv", ["parity.py", "--harness-file", str(empty)]):
                    self.assertEqual(parity.main(), 0)
                self.assertEqual(marker.read_bytes(), previous)

    def test_a_pass_from_another_worktree_counts_only_for_the_same_content(self):
        with tempfile.TemporaryDirectory() as temp:
            elsewhere, here, shared = Path(temp, "a"), Path(temp, "b"), Path(temp, "shared")
            for root in (elsewhere, here):
                root.mkdir()
                (root / "x.rs").write_text("same")
            with mock.patch.object(parity, "SHARED", shared):
                with mock.patch.object(parity, "ROOT", elsewhere):
                    record = {"status": "pass", "dependencies": ["x.rs"], "artifacts": []}
                    record["key"] = parity.key(record, parity.Hasher({}), "env")
                    parity.publish("x", record)
                with mock.patch.object(parity, "ROOT", here):
                    state = {"harnesses": {}, "hashes": {}}
                    self.assertFalse(parity.stale("x", state, parity.Hasher(state["hashes"]), "env"))
                    self.assertTrue(state["harnesses"]["x"]["shared"])
                    (here / "x.rs").write_text("changed")
                    state = {"harnesses": {}, "hashes": {}}
                    self.assertTrue(parity.stale("x", state, parity.Hasher(state["hashes"]), "env"))


class TraceTests(unittest.TestCase):
    def test_commands_are_recorded_as_argument_lists(self):
        """On Windows the audit event carries the command line Popen built; the trace splits it back, so a harness's
        cargo builds reach the probe prebuild and its probes' sources become dependencies there too."""
        arguments = [sys.executable, "-c", "pass", "build", "-p", "a b", 'say "hi"', "back\\slash\\", ""]
        with tempfile.TemporaryDirectory() as tmp:
            script, trace = Path(tmp) / "harness.py", Path(tmp) / "trace.json"
            script.write_text(f"import subprocess\nsubprocess.run({arguments!r})\n", encoding="utf-8")
            subprocess.run([sys.executable, str(Path(parity.__file__)), "--trace", str(trace), str(script)], check=True)
            commands = json.loads(trace.read_text(encoding="utf-8"))["commands"]
        self.assertIn(arguments, commands)

    @unittest.skipUnless(os.name == "nt", "Windows command lines")
    def test_a_windows_command_line_splits_into_its_arguments(self):
        for arguments in (["cargo", "build", "--profile", "fast"],
                          ["C:\\Program Files\\x.exe", "a b", 'q"uote', "tail\\", ""]):
            self.assertEqual(parity.command_line_arguments(subprocess.list2cmdline(arguments)), arguments)

    def test_a_windows_probe_binary_finds_its_dep_info(self):
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "client_probe.d").write_text(f"{tmp}/client_probe: crates/a/src/lib.rs\n", encoding="utf-8")
            self.assertEqual(parity.dep_info(Path(tmp, "client_probe.exe")), ["crates/a/src/lib.rs"])


class DependencyPathTests(unittest.TestCase):
    def test_traced_dependencies_use_repository_posix_paths(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp).resolve()):
            source = Path(temp, "crates/w/src/lib.rs")
            source.parent.mkdir(parents=True)
            source.write_text("")
            trace = {"modules": [str(source)], "read": [], "commands": []}
            self.assertEqual(parity.dependencies(trace), (["crates/w/src/lib.rs"], []))

    def test_legacy_local_and_shared_paths_select_without_rewriting_records(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)), \
                mock.patch.object(parity, "SHARED", Path(temp, "shared")):
            root = Path(temp)
            found = {name: root / "tools" / f"test_{name}_parity.py" for name in ("local", "shared", "empty")}
            record = {"dependencies": [r"crates\w\src\lib.rs#keep:used"], "key": "recorded-content-key"}
            local = root / "state.json"
            local.write_text(json.dumps({"local": record, "empty": {"dependencies": []}}), encoding="utf-8")
            shared = parity.SHARED / "dependencies/shared.json"
            shared.parent.mkdir(parents=True)
            shared.write_text(json.dumps(record), encoding="utf-8")
            records = json.loads(local.read_text(encoding="utf-8"))
            before = local.read_bytes(), shared.read_bytes()
            self.assertEqual(parity.affected_harnesses(found, records, {"crates/w/src/lib.rs"}, reach=False),
                             {"local", "shared"})
            self.assertEqual(parity.affected_harnesses(found, records, {"crates/w/src/other.rs"}, reach=False), set())
            self.assertEqual(records, {"local": record, "empty": {"dependencies": []}})
            self.assertEqual((local.read_bytes(), shared.read_bytes()), before)

    def test_fixture_fallback_selects_and_runs_past_old_and_incomplete_passes(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)), \
                mock.patch.object(parity, "SHARED", Path(temp, "shared")), \
                mock.patch.object(parity, "STATE_DIR", Path(temp, "state")):
            root = Path(temp)
            marker = root / "crates/sunrise-mod-api-gen/source-layout"
            marker.parent.mkdir(parents=True)
            marker.write_text("split")
            owners = ["fixture-common", "fixture-server", "fixture-client"]
            paths = {owner: f"crates/{owner}/src/lib.rs" for owner in owners}
            dispatcher = "crates/sunrise-parity-fixtures/src/main.rs"
            for path in (*paths.values(), dispatcher):
                (root / path).parent.mkdir(parents=True)
                (root / path).write_text("fn run() {}")
            mapping = root / parity.FIXTURE_ROOTS
            mapping.parent.mkdir(parents=True)
            roles = {
                "lean": {"packages": owners[:2], "dispatcher": dispatcher, "inputs": [],
                         "fixtures": {"legacy::serve": paths["fixture-server"]}},
                "client-defaults": {"packages": owners, "dispatcher": dispatcher, "inputs": [],
                                    "fixtures": {"legacy::render": paths["fixture-client"]}},
            }
            mapping.write_text(json.dumps({"version": parity.FIXTURE_RECORD_VERSION, "variants": roles}))
            found = {name: root / f"tools/test_{name}.py" for name in roles}
            hasher = lambda path: path.read_text() if path.is_file() else "missing"
            records = {}
            for variant, suffix, selector in (("lean", "no-default-features", "legacy::serve"),
                                              ("client-defaults", "client-defaults", "legacy::render")):
                record = {"status": "pass", "fixture_record_version": parity.FIXTURE_RECORD_VERSION,
                          "fixture_ownership": True, "fixture_runs": parity.fixture_runs([
                              [f"sunrise-parity-fixtures-{suffix}", selector, "--exact"],
                          ]), "dependencies": [], "artifacts": []}
                coverage = parity.fixture_coverage(record, roles)
                self.assertIn(roles[variant]["fixtures"][selector], coverage[1])
                record["dependencies"] = sorted(parity.fixture_inputs(coverage))
                record["key"] = parity.key(record, hasher, "fixture")
                records[variant] = record
                self.assertFalse(parity.stale(variant, {"harnesses": records}, hasher, "fixture"))

            added = "crates/fixture-common/src/new_mount.rs"
            (root / added).write_text("pub fn reached_indirectly() {}")
            self.assertEqual(parity.affected_harnesses(found, records, {added}, reach=False), set(found))
            for variant in found:
                self.assertTrue(parity.stale(variant, {"harnesses": records}, hasher, "fixture"))

            old = {"status": "pass", "dependencies": [paths["fixture-common"]], "artifacts": []}
            old["key"] = parity.key(old, hasher, "fixture")
            parity.publish("lean", old)
            self.assertTrue(parity.stale("lean", {"harnesses": {}}, hasher, "fixture"))
            records["lean"] = old
            self.assertEqual(parity.affected_harnesses(found, records, {added}, reach=False), set(found))
            client_change = {paths["fixture-client"]}
            self.assertEqual(parity.affected_harnesses(found, records, client_change, reach=False),
                             {"client-defaults", "lean"})

    def test_changed_probes_and_oracles_accept_legacy_separators(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)):
            found = {"probe": Path(temp, "tools/test_probe_parity.py")}
            for path in ("crates/w/examples/probe.rs", "crates/w/src/bin/probe.rs",
                         "mods/sunrise/src/test/java/Oracle.java"):
                with self.subTest(path=path), mock.patch.object(parity, "changed_files", return_value={path}):
                    records = {"probe": {"dependencies": [path.replace("/", "\\")]}}
                    self.assertEqual(parity.changed_harnesses(found, records, "main"), {"probe"})
            with mock.patch.object(parity, "changed_files", return_value={"crates/w/examples/probe.rs"}):
                records = {"probe": {"dependencies": [r"crates\w\examples\probe.rs#keep:used"]}}
                self.assertEqual(parity.changed_harnesses(found, records, "main"), {"probe"})

    def test_legacy_probe_records_still_narrow_to_reachable_sources(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)), \
                mock.patch.object(parity, "SHARED", Path(temp, "shared")), \
                mock.patch.object(parity, "STATE_DIR", Path(temp, "state")):
            root = Path(temp)
            files = {
                "crates/sunrise-world/Cargo.toml": '[package]\nname = "sunrise-world"\n'
                                                   '\n[[example]]\nname = "world_probe"\npath = "examples/probe.rs"\n',
                "crates/sunrise-world/src/lib.rs": "pub mod used;\npub mod unused;\n",
                "crates/sunrise-world/src/used.rs": "pub fn f() {}\n",
                "crates/sunrise-world/src/unused.rs": "pub fn f() {}\n",
                "crates/sunrise-world/examples/probe.rs": "fn main() { sunrise_world::used::f(); }\n",
            }
            for path, text in files.items():
                target = root / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(text, encoding="utf-8")
            dependencies = [path.replace("/", "\\") for path in files]
            dependencies[1] += "#keep:used"
            record = {"dependencies": dependencies, "artifacts": [["example", "probe", "release"]]}
            found = {"probe": root / "tools/test_probe_parity.py"}
            for path, expected in (("crates/sunrise-world/src/used.rs", {"probe"}),
                                   ("crates/sunrise-world/src/unused.rs", set())):
                with self.subTest(path=path):
                    self.assertEqual(parity.affected_harnesses(found, {"probe": record}, {path}), expected)
            self.assertEqual(record["dependencies"], dependencies)


class RustScopeTests(unittest.TestCase):
    """PARITY_RUST_SCOPE drops Rust sources only when nothing in the scope can reach them."""

    def crate(self, temp, files):
        root = Path(temp)
        (root / "crates/w/src").mkdir(parents=True)
        (root / "crates/w/examples").mkdir(parents=True)
        (root / "crates/w/Cargo.toml").write_text("")
        for name, text in files.items():
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            (root / name).write_text(text)
        return root

    FILES = {
        "crates/w/src/lib.rs": "pub mod a;\npub mod b;\n/// The other one.\npub mod c;\npub const V: i32 = 1;\n",
        "crates/w/src/a.rs": 'pub struct Tag;\npub fn f() -> i32 { let s = "crate::c::x"; crate::V }\n'
                             "mod tests { fn g() { super::super::b::h(); } }\n",
        "crates/w/src/b.rs": "pub fn h() {}\n",
        "crates/w/src/c.rs": "pub struct Other;\nimpl std::fmt::Display for Other { }\nimpl<R: Copy> Foo for R {}\n",
        "crates/w/examples/probe.rs": "use w::a::f;\nfn main() { f(); }\n",
    }

    def narrow(self, files, scope):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", self.crate(temp, files)):
            return parity.narrow_rust_sources(sorted(files), scope)

    def test_a_closed_scope_drops_the_rest(self):
        kept, note = self.narrow(self.FILES, ["crates/w/src/a.rs", "crates/w/src/b.rs"])
        self.assertIsNone(note)
        self.assertEqual(kept, ["crates/w/examples/probe.rs", "crates/w/src/a.rs", "crates/w/src/b.rs",
                                "crates/w/src/lib.rs#keep:a,b"])

    def test_references_out_of_the_scope_keep_everything(self):
        files = dict(self.FILES, **{"crates/w/src/b.rs": "pub fn h() { crate::c::Other; }\n"})
        kept, note = self.narrow(files, ["crates/w/src/a.rs", "crates/w/src/b.rs"])
        self.assertIn("crate::c::Other", note)
        self.assertEqual(kept, sorted(files))

    def test_impls_elsewhere_for_scope_types_keep_everything(self):
        files = dict(self.FILES, **{"crates/w/src/c.rs": "impl From<i32> for crate::a::Tag { }\n"})
        self.assertIn("Tag", self.narrow(files, ["crates/w/src/a.rs", "crates/w/src/b.rs"])[1])

    def test_new_modules_in_the_crate_root_do_not_count(self):
        keep = {"a", "b"}
        root = self.FILES["crates/w/src/lib.rs"]
        self.assertEqual(parity.crate_root_without_modules(root, keep),
                         parity.crate_root_without_modules(root + "#[cfg(test)]\nmod d;\n", keep))
        self.assertNotEqual(parity.crate_root_without_modules(root, keep),
                            parity.crate_root_without_modules(root + "pub const W: i32 = 2;\n", keep))

    def test_rust_code_blanks_literals_and_comments(self):
        code = parity.rust_code('let a = "{ crate::x }"; // crate::y\nlet c = \'{\'; fn f<\'a>() { r#"}"# }')
        self.assertNotIn("crate", code)
        self.assertEqual(code.count("{"), code.count("}"))
        self.assertIn("<'a>", code)


class ShardTests(unittest.TestCase):
    def test_invalid_shards_are_rejected(self):
        for spec in ("0/8", "9/8", "1/0", "1/-8"):
            with self.subTest(spec=spec), mock.patch.dict(os.environ, {"PARITY_SHARD": spec}):
                with self.assertRaisesRegex(ValueError, "PARITY_SHARD"):
                    parity_shard.shard(list(range(8)))
                with self.assertRaisesRegex(ValueError, "PARITY_SHARD"):
                    parity_shard.label()

    def test_valid_shard_selects_every_nth_case(self):
        with mock.patch.dict(os.environ, {"PARITY_SHARD": "2/3"}):
            self.assertEqual(parity_shard.shard(list(range(7))), [1, 4])
            self.assertEqual(parity_shard.label(), "2-3")

    def test_a_sharded_harness_runs_as_numbered_entries(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)):
            (Path(temp) / "tools").mkdir()
            (Path(temp) / "tools/test_x_parity.py").write_text("PARITY_SHARDS = 3\n")
            (Path(temp) / "tools/test_y_parity.py").write_text("")
            self.assertEqual(sorted(parity.harnesses()), ["x#1", "x#2", "x#3", "y"])
            self.assertEqual(parity.SHARDS["x#2"], "2/3")
            self.assertEqual(parity.base_name("x#2"), "x")

    def test_a_manual_proof_script_is_left_out(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)), \
                mock.patch.dict(parity.MANUAL, clear=True):
            (Path(temp) / "tools").mkdir()
            (Path(temp) / "tools/test_x_parity.py").write_text('import sys\nPARITY_MANUAL = "--side java, then --side native"\n')
            (Path(temp) / "tools/test_y_parity.py").write_text("")
            self.assertEqual(sorted(parity.harnesses()), ["y"])
            self.assertEqual(parity.MANUAL, {"x": "--side java, then --side native"})


class OracleArgumentTests(unittest.TestCase):
    def test_an_agent_jar_is_an_input(self):
        from tools import parity_shim
        jar, oracle = Path("/w/seed-agent.jar"), Path("/w/out.jsonl")
        self.assertEqual(parity_shim.named_paths([f"-javaagent:{jar}=a=b", "-Xmx3G", str(oracle)]),
                         [os.path.abspath(jar), os.path.abspath(oracle)])


@unittest.skipIf(os.name == "nt", "oracle PATH shimming uses Unix executable symlink semantics")
class OracleReuseTests(unittest.TestCase):
    """tools/parity_shim.py replays a run only when its inputs and outputs are as recorded."""

    def setUp(self):
        import os, subprocess, sys
        self.temp = tempfile.TemporaryDirectory()
        root = Path(self.temp.name)
        self.bin, self.shim = root / "bin", root / "shim"
        self.bin.mkdir(); self.shim.mkdir()
        # A fake oracle: copies its input (or the JDK release marker) to its output and counts its runs.
        fake = self.bin / "java"
        fake.write_text(f"#!{sys.executable}\nimport sys, pathlib\n"
                        "runs = pathlib.Path(sys.argv[1]).parent / 'runs'\n"
                        "runs.write_text(str(int(runs.read_text() or 0) + 1) if runs.exists() else '1')\n"
                        "source = pathlib.Path(sys.argv[1])\n"
                        "if '--emit-release' in sys.argv[3:]:\n"
                        "    source = pathlib.Path(__file__).resolve().parent.parent / 'release'\n"
                        "pathlib.Path(sys.argv[2]).write_bytes(source.read_bytes())\nprint('done')\n")
        fake.chmod(0o755)
        # bin's parent is the fake launcher's JDK home (JAVA_HOME below); an update in place changes its release file.
        (root / "release").write_text('JAVA_VERSION="25.0.1"\n')
        (self.shim / "java").symlink_to(Path(parity.__file__).with_name("parity_shim.py"))
        self.input, self.output = root / "work" / "in.txt", root / "work" / "out.txt"
        self.input.parent.mkdir()
        self.input.write_text("a")
        # The shim starts with `#!/usr/bin/env python3`, and a Python's own folder may name it only
        # python3.13 (Homebrew's): a python3 of its own keeps the PATH below enough.
        python = root / "python"
        python.mkdir()
        (python / "python3").symlink_to(sys.executable)
        self.python = sys.executable
        self.env = dict(os.environ, PATH=f"{self.shim}:{self.bin}:{python}", PARITY_SHIM_REAL_PATH=str(self.bin),
                        PARITY_SHIM_CACHE=str(root / "cache"), JAVA_HOME=str(root))
        self.subprocess = subprocess

    def tearDown(self):
        self.temp.cleanup()

    def oracle(self, *options):
        result = self.subprocess.run(["java", str(self.input), str(self.output), *options], env=self.env,
                                     capture_output=True, text=True, stdin=self.subprocess.DEVNULL, check=True)
        self.assertEqual(result.stdout, "done\n")
        return int((self.input.parent / "runs").read_text())

    def test_cold_cache_setup_holds_the_setup_lock(self):
        import hashlib
        import sqlite3
        from tools import file_lock, parity_shim

        cache = Path(self.env["PARITY_SHIM_CACHE"])

        def held():
            # A second open file description conflicts with the shim's flock even in this process.
            with (cache / "hashes.lock").open("a+b") as other:
                return not file_lock.lock(other, wait=False)

        during = []

        class Probed:
            def __init__(self, db):
                self.db = db

            def execute(self, *args):
                during.append(held())
                return self.db.execute(*args)

            def __getattr__(self, name):
                return getattr(self.db, name)

        connect = sqlite3.connect

        def probed_connect(*args, **kwargs):
            during.append(held())
            return Probed(connect(*args, **kwargs))

        with mock.patch.object(parity_shim, "CACHE", cache), mock.patch.object(sqlite3, "connect", probed_connect):
            hashes = parity_shim.Hashes()
        # The connect, the switch to WAL and the create table all run under the lock.
        self.assertEqual(during, [True, True, True])
        self.assertFalse(held())
        with contextlib.closing(hashes.db.db):
            self.assertEqual(hashes.file(str(self.input)), hashlib.sha256(b"a").hexdigest())

    def test_reuses_until_an_input_or_output_changes(self):
        self.assertEqual(self.oracle(), 1)
        self.assertEqual(self.oracle(), 1)  # replayed, stdout included
        self.input.write_text("a")  # same content, new mtime: still reusable
        self.assertEqual(self.oracle(), 1)
        self.input.write_text("b")
        self.assertEqual(self.oracle(), 2)
        self.assertEqual(self.output.read_text(), "b")
        self.output.write_text("tampered")
        self.assertEqual(self.oracle(), 3)
        self.assertEqual(self.output.read_text(), "b")

    def test_a_new_checkout_replays_a_run_from_the_keep(self):
        import shutil
        keep = self.bin.parent / "keep"
        self.env["PARITY_ORACLE_KEEP"] = str(keep)
        self.assertEqual(self.oracle(), 1)
        # A new checkout: no record of its own, no outputs.
        shutil.rmtree(self.env["PARITY_SHIM_CACHE"])
        self.output.unlink()
        self.input.write_text("a")
        self.assertEqual(self.oracle(), 1)  # replayed from the keep, stdout included
        self.assertEqual(self.output.read_text(), "a")
        self.assertEqual(self.oracle(), 1)  # and then from its own record
        shutil.rmtree(self.env["PARITY_SHIM_CACHE"])
        self.output.unlink()
        self.input.write_text("b")
        self.assertEqual(self.oracle(), 2)
        self.assertEqual(self.output.read_text(), "b")
        self.assertEqual(len([entry for entry in keep.glob("??/*") if not entry.name.startswith(".")]), 2)

    def new_checkout(self):
        import shutil
        shutil.rmtree(self.env["PARITY_SHIM_CACHE"], ignore_errors=True)
        self.output.unlink()

    def test_a_kept_run_is_not_replayed_for_another_jdk_or_jvm_options(self):
        keep = self.bin.parent / "keep"
        self.env["PARITY_ORACLE_KEEP"] = str(keep)
        # The fake launcher's JDK home is bin's parent; an update in place changes its release file.
        release = self.bin.parent / "release"
        release.write_text('JAVA_VERSION="25.0.1"\n')
        self.assertEqual(self.oracle(), 1)
        self.new_checkout()
        release.write_text('JAVA_VERSION="25.0.2"\n')
        self.assertEqual(self.oracle(), 2)
        self.new_checkout()
        self.env["JAVA_TOOL_OPTIONS"] = "-Dsunrise.seed=2"
        self.assertEqual(self.oracle(), 3)
        self.new_checkout()
        self.assertEqual(self.oracle(), 3)  # the same JDK and options as the last run: replayed

    def test_with_a_keep_an_update_of_the_jdk_in_place_replays_nothing_from_the_checkout_either(self):
        self.env["PARITY_ORACLE_KEEP"] = str(self.bin.parent / "keep")
        self.assertEqual(self.oracle("--emit-release"), 1)
        self.assertEqual(self.oracle("--emit-release"), 1)
        self.assertEqual(self.output.read_bytes(), b'JAVA_VERSION="25.0.1"\n')
        updated = b'JAVA_VERSION="25.0.10"\n'
        (self.bin.parent / "release").write_bytes(updated)
        self.assertEqual(self.oracle("--emit-release"), 2)  # warm: the checkout's own record was of the old JDK
        self.assertEqual(self.output.read_bytes(), updated)

    def test_with_a_keep_a_launcher_linked_out_of_the_jdk_is_never_reused(self):
        import os
        from tools import parity_shim
        home = self.bin.parent / "jdk"
        (home / "bin").mkdir(parents=True)
        (home / "bin" / "java").symlink_to(self.bin / "java")  # resolves outside $JAVA_HOME/bin
        with mock.patch.dict(os.environ, JAVA_HOME=str(home)):
            self.assertFalse(parity_shim.direct_jdk(str(home / "bin" / "java"), "java"))
        with mock.patch.dict(os.environ, JAVA_HOME=str(self.bin.parent)):
            self.assertTrue(parity_shim.direct_jdk(str(self.bin / "java"), "java"))

    def test_a_file_named_through_an_oracle_variable_is_an_input(self):
        keep = self.bin.parent / "keep"
        self.env["PARITY_ORACLE_KEEP"] = str(keep)
        order = self.bin.parent / "order.txt"
        order.write_text("first")
        self.env["SUNRISE_ORDER"] = str(order)
        self.assertEqual(self.oracle(), 1)
        self.assertEqual(self.oracle(), 1)
        # The same argv, other bytes behind the variable: neither tier replays.
        order.write_text("second")
        self.assertEqual(self.oracle(), 2)
        self.new_checkout()
        order.write_text("first")
        self.assertEqual(self.oracle(), 2)  # the first run's bytes again: replayed from the keep
        self.new_checkout()
        order.write_text("third")
        self.assertEqual(self.oracle(), 3)

    def test_a_damaged_kept_run_runs_the_oracle_and_writes_nothing_of_it(self):
        keep = self.bin.parent / "keep"
        self.env["PARITY_ORACLE_KEEP"] = str(keep)
        self.assertEqual(self.oracle(), 1)
        entry = next(entry for entry in keep.glob("??/*") if not entry.name.startswith("."))
        (entry / "stdout").unlink()
        (entry / "files" / "0").write_text("kept output")
        self.new_checkout()
        self.assertEqual(self.oracle(), 2)
        self.assertEqual(self.output.read_text(), "a")
        self.assertEqual([path.name for path in self.output.parent.iterdir() if "restore" in path.name], [])

    def wrapper(self, folder, put_the_shim_back=False):
        """A `java` that runs the first other `java` on PATH, as JDK selectors and build wrappers do. It stops
        itself after a few rounds, so a shim that loops fails this test instead of filling the machine."""
        folder.mkdir(parents=True)
        path = f"os.environ['PATH'] = {str(self.shim)!r} + os.pathsep + os.environ['PATH']\n" if put_the_shim_back else ""
        (folder / "java").write_text(
            f"#!{self.python}\nimport os, sys, shutil\nfrom pathlib import Path\n"
            "rounds = int(os.environ.get('WRAPPER_ROUNDS') or 0)\n"
            "if rounds > 6: sys.exit(97)\n"
            "os.environ['WRAPPER_ROUNDS'] = str(rounds + 1)\n" + path +
            "me = Path(__file__).resolve()\n"
            "other = next(c for d in os.environ['PATH'].split(os.pathsep) if (c := shutil.which('java', path=d))"
            " and Path(c).resolve() != me)\n"
            "os.execv(other, [other, *sys.argv[1:]])\n")
        (folder / "java").chmod(0o755)

    def test_a_wrapper_that_looks_java_up_on_path_reaches_the_real_one(self):
        wrapper = self.bin.parent / "wrapper"
        self.wrapper(wrapper)
        self.env["PATH"] = f"{self.shim}:{wrapper}:{self.bin}:{self.bin.parent / 'python'}"
        self.env["PARITY_SHIM_REAL_PATH"] = f"{wrapper}:{self.bin}"
        self.assertEqual(self.oracle(), 1)
        self.assertEqual(self.oracle(), 1)

    def test_with_a_keep_a_run_through_a_wrapper_is_never_reused(self):
        # A JDK selector in front of $JAVA_HOME's JDK: the JDK behind it changes while the wrapper's file stays the same.
        keep = self.bin.parent / "keep"
        wrapper = self.bin.parent / "selector" / "bin"
        self.wrapper(wrapper)
        (wrapper.parent / "release").write_text('JAVA_VERSION="selector"\n')
        self.env["PATH"] = f"{self.shim}:{wrapper}:{self.bin}:{self.bin.parent / 'python'}"
        self.env["PARITY_SHIM_REAL_PATH"] = f"{wrapper}:{self.bin}"
        self.env.pop("PARITY_ORACLE_KEEP", None)
        self.assertEqual(self.oracle(), 1)
        self.assertEqual(self.oracle(), 1)  # the selector populated the checkout's cache without the keep
        self.env["PARITY_ORACLE_KEEP"] = str(keep)
        (self.bin.parent / "release").write_text('JAVA_VERSION="25.0.2"\n')
        self.assertEqual(self.oracle(), 2)  # warm: the checkout's own record replays nothing either
        self.new_checkout()
        (self.bin.parent / "release").write_text('JAVA_VERSION="25.0.3"\n')
        self.assertEqual(self.oracle(), 3)  # cold: nothing of the earlier JDKs replayed from the keep
        self.assertEqual(self.output.read_text(), "a")
        self.assertEqual(list(keep.glob("??/*")), [])

    def test_a_wrapper_that_puts_the_shim_back_stops_with_a_reason(self):
        wrapper = self.bin.parent / "wrapper"
        self.wrapper(wrapper, put_the_shim_back=True)
        # The shim's `env python3` must find a python3 on the real PATH.
        self.env["PARITY_SHIM_REAL_PATH"] = f"{wrapper}:{self.bin}:{self.bin.parent / 'python'}"
        result = self.subprocess.run(["java", str(self.input), str(self.output)], env=self.env, capture_output=True,
                                     text=True, stdin=self.subprocess.DEVNULL, timeout=60)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("started inside itself", result.stderr)

    def test_a_real_path_that_holds_the_shim_skips_it(self):
        from tools import parity_shim
        with mock.patch.dict(os.environ, PARITY_SHIM_REAL_PATH=f"{self.shim}:{self.bin}"):
            self.assertEqual(Path(parity_shim.real("java")), self.bin / "java")


class OracleKeepTests(unittest.TestCase):
    def test_pruning_drops_old_runs_then_the_least_recently_used(self):
        import os, time
        with tempfile.TemporaryDirectory() as tmp:
            keep, now = Path(tmp), time.time()
            def entry(name, age, size):
                (keep / "ab" / name).mkdir(parents=True)
                (keep / "ab" / name / "stdout").write_bytes(b"x" * size)
                os.utime(keep / "ab" / name, (now - age, now - age))
            entry("old", 15 * 86400, 1)
            entry(".killed-writer.tmp", 2 * 86400, 1)
            entry(".being-written.tmp", 60, 1)
            entry("used-long-ago", 3600, 8)
            entry("used-lately", 60, 8)
            # Within the budget, only age decides.
            with mock.patch.object(parity, "ORACLE_KEEP_BUDGET", 100):
                parity.prune_oracle_keep(keep)
            self.assertEqual(sorted(path.name for path in (keep / "ab").iterdir()),
                             [".being-written.tmp", "used-lately", "used-long-ago"])
            # Over it, the least recently used go first.
            with mock.patch.object(parity, "ORACLE_KEEP_BUDGET", 10):
                parity.prune_oracle_keep(keep)
            self.assertEqual(sorted(path.name for path in (keep / "ab").iterdir()), [".being-written.tmp", "used-lately"])

    def test_only_a_fresh_pass_after_a_reuse_failure_is_remembered_and_only_for_a_while(self):
        import json, time
        with tempfile.TemporaryDirectory() as tmp:
            keep = Path(tmp)
            long_ago = time.strftime("%Y-%m-%d", time.gmtime(time.time() - 40 * 86400))
            lately = time.strftime("%Y-%m-%d", time.gmtime(time.time() - 5 * 86400))
            (keep / "fresh-only.json").write_text(json.dumps({"expired": long_ago, "inherited": lately}))
            parity.remember_fresh_only(keep, {
                "loot": {"status": "fail", "retried": "reused oracle results", "fresh_rerun": "pass",
                         "oracle_reuse": False},
                "inherited": {"status": "pass", "oracle_reuse": False},
                "failed": {"status": "fail", "retried": "reused oracle results", "fresh_rerun": "fail",
                           "oracle_reuse": True},
                "light": {"status": "pass", "oracle_reuse": True}})
            self.assertEqual(parity.fresh_only(keep), {"loot": time.strftime("%Y-%m-%d", time.gmtime()),
                                                       "inherited": lately})
            # Two shares of one daily on one host, each adding its own: the second keeps the first's.
            parity.remember_fresh_only(keep, {"sculk": {"status": "fail", "retried": "reused oracle results",
                                                        "fresh_rerun": "pass"}})
            self.assertEqual(set(parity.fresh_only(keep)), {"loot", "inherited", "sculk"})
            # A share with nothing to add doesn't write: a peer's list written meanwhile stays as it is.
            (keep / "fresh-only.json").write_text(json.dumps({"peer": lately}))
            parity.remember_fresh_only(keep, {"light": {"status": "pass", "oracle_reuse": True}})
            self.assertEqual(parity.fresh_only(keep), {"peer": lately})

    def test_a_fresh_only_list_of_the_wrong_shape_is_an_error(self):
        import json
        with tempfile.TemporaryDirectory() as tmp:
            keep = Path(tmp)
            for text in ("[1, 2]", json.dumps({"loot": 3, "light": "2026-11-20"})):
                (keep / "fresh-only.json").write_text(text)
                with self.assertRaises(ValueError):
                    parity.fresh_only(keep)

    def kept_run(self, root, outputs):
        import json
        entry = root / "keep" / "ab" / "abcd"
        (entry / "files").mkdir(parents=True)
        for index in range(len(outputs)):
            (entry / "files" / str(index)).write_text(f"kept {index}")
        (entry / "stdout").write_bytes(b"done")
        (entry / "stderr").write_bytes(b"")
        (entry / "run.json").write_text(json.dumps({"outputs": [str(path) for path in outputs]}))
        return entry

    def test_a_replay_that_cannot_publish_every_output_publishes_none(self):
        from tools import parity_shim
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            first, blocked = root / "out" / "a.txt", root / "out" / "b"
            entry = self.kept_run(root, [first, blocked])
            (blocked / "in-the-way").mkdir(parents=True)  # a folder where a file must go
            self.assertIsNone(parity_shim.restore(entry))
            self.assertEqual(sorted(path.name for path in (root / "out").iterdir()), ["b"])
            # A file already in place is never replaced, even when a later output is the one in the way.
            first.write_text("the checkout's own")
            self.assertIsNone(parity_shim.restore(entry))
            self.assertEqual(first.read_text(), "the checkout's own")
            self.assertEqual(sorted(path.name for path in (root / "out").iterdir()), ["a.txt", "b"])

    def test_a_kept_run_naming_an_unusable_path_is_a_miss(self):
        from tools import parity_shim
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            entry = self.kept_run(root, [root / "out" / "a.txt"])
            for name in ("b\\u0000c", "b\\ud800c"):  # an embedded NUL; a lone surrogate no file system can name
                with self.subTest(name=name):
                    (entry / "run.json").write_text('{"outputs": ["' + str(root / "out") + "/" + name + '"]}')
                    self.assertIsNone(parity_shim.restore(entry))
                    self.assertFalse((root / "out").exists())

    def test_a_destination_that_appears_while_publishing_keeps_its_bytes_and_the_replay_takes_back_its_own(self):
        import os
        from tools import parity_shim
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            first, second = root / "out" / "a.txt", root / "out" / "b.txt"
            entry = self.kept_run(root, [first, second])
            link = os.link
            def racing_link(source, destination):
                if destination == str(second):
                    second.write_text("written meanwhile")
                link(source, destination)
            with mock.patch.object(parity_shim.os, "link", racing_link):
                self.assertIsNone(parity_shim.restore(entry))
            self.assertEqual(second.read_text(), "written meanwhile")
            self.assertEqual(sorted(path.name for path in first.parent.iterdir()), ["b.txt"])

    def test_a_replay_never_writes_over_a_file_at_its_staging_name(self):
        import os
        from tools import parity_shim
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            first, second = root / "out" / "a.txt", root / "out" / "b.txt"
            entry = self.kept_run(root, [first, second])
            first.parent.mkdir()
            staging = Path(f"{second}.{os.getpid()}.restore")
            staging.write_text("someone else's")
            self.assertIsNone(parity_shim.restore(entry))
            self.assertEqual(staging.read_text(), "someone else's")
            self.assertEqual(sorted(path.name for path in first.parent.iterdir()), [staging.name])

    def test_a_run_whose_streams_are_too_large_or_whose_keep_is_unwritable_is_not_kept(self):
        from tools import parity_shim
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            output = root / "out.txt"
            output.write_text("small")
            entry = root / "keep" / "ab" / "abcd"
            entry.parent.mkdir(parents=True)
            with mock.patch.object(parity_shim, "KEPT_FILE", 16):
                parity_shim.keep(entry, {str(output): None}, {"stdout": b"x" * 17, "stderr": b""})
            self.assertFalse(entry.exists())
            # A keep that can't be written to: nothing raises, nothing is kept.
            blocked = root / "file" / "ab" / "abcd"
            (root / "file").write_text("not a folder")
            parity_shim.keep(blocked, {str(output): None}, {"stdout": b"", "stderr": b""})
            parity_shim.keep(entry, {str(output): None}, {"stdout": b"done", "stderr": b""})
            self.assertEqual((entry / "files" / "0").read_text(), "small")



class LargeOutputTests(unittest.TestCase):
    def test_a_pass_clears_only_its_own_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            local, state = Path(tmp) / ".local", Path(tmp) / "state"
            own, shared = local / "decorated-region-parity", local / "chunk-generation-parity"
            for directory in (own, shared):
                directory.mkdir(parents=True)
                (directory / "dump.txt").write_bytes(b"x" * 16)
            trace = {"commands": [["java", "-d", str(shared / "classes")], ["probe", str(own / "dump.txt")]]}
            with mock.patch.multiple(parity, LOCAL=local, STATE_DIR=state, LARGE_OUTPUT=8):
                parity.outputs_listing("decorated_region", fresh=True)
                parity.clear_large_outputs("decorated_region", trace)
            self.assertFalse((own / "dump.txt").exists())
            # chunk_generation may still be writing there.
            self.assertTrue((shared / "dump.txt").exists())

    def test_rust_named_directories_and_big_oracle_outputs_go_too(self):
        with tempfile.TemporaryDirectory() as tmp:
            local, state = Path(tmp) / ".local", Path(tmp) / "state"
            own = local / "rust-state-provider-parity"
            own.mkdir(parents=True)
            for name, size in (("rust.txt", 16), ("server.txt", 16), ("client.txt", 64), ("report.json", 4)):
                (own / name).write_bytes(b"x" * size)
            trace = {"commands": [["java", "Oracle", str(own)], ["probe", "fixture", str(own)]]}
            with mock.patch.multiple(parity, LOCAL=local, STATE_DIR=state, LARGE_OUTPUT=8, KEPT_ORACLE_OUTPUT=32):
                listing = parity.outputs_listing("state_provider#2", fresh=True)
                listing.write_text(f"{own / 'server.txt'}\n{own / 'client.txt'}\n")
                parity.clear_large_outputs("state_provider#2", trace)
            self.assertEqual(sorted(path.name for path in own.iterdir()), ["report.json", "server.txt"])


class FakeExecutor:
    """Runs nothing: each start of a harness takes the next status from its script, a list."""
    name, dead, max_peak = "fake", False, None

    def __init__(self):
        self.started, self.finished = [], []

    def free(self):
        return 1

    def accepts(self, name):
        return True

    def fits(self, expected, only_here=True):
        return True

    def start(self, label, script, reuse, expected, shard=None):
        self.started.append((label, reuse))
        self.finished.append((label, {"status": script.pop(0), "seconds": 1.0, "dependencies": []}))
        return True

    def poll(self):
        finished, self.finished = self.finished, []
        return finished


class RerunTests(unittest.TestCase):
    """A failure with reused oracle results runs once more from scratch, and never becomes a pass."""

    def run_harnesses(self, scripts, rounds=None):
        executor, output = FakeExecutor(), io.StringIO()
        state = {"harnesses": {name: {"status": "pass", "oracle_reuse": True} for name in scripts}}
        with tempfile.TemporaryDirectory() as tmp, mock.patch.multiple(parity, STATE_DIR=Path(tmp), SHARED=Path(tmp)), \
                mock.patch.object(parity, "save_state"), mock.patch.object(parity, "time") as clock, \
                contextlib.redirect_stdout(output):
            clock.monotonic.return_value = 0.0
            results, _ = parity.run(scripts, state, None, "env", [executor], True, rounds)
        return results, executor.started, output.getvalue()

    def test_a_fresh_pass_after_a_reuse_failure_still_fails_and_stops_reuse(self):
        results, started, _ = self.run_harnesses({"food": ["fail", "pass"]})
        self.assertEqual(started, [("food", True), ("food", False)])
        record = results["food"]
        self.assertEqual((record["status"], record["retried"], record["fresh_rerun"]),
                         ("fail", "reused oracle results", "pass"))
        self.assertFalse(record["oracle_reuse"])

    def test_the_daily_verdict_reads_the_first_failure_not_the_fresh_pass(self):
        results, _, output = self.run_harnesses({"food": ["fail", "pass"], "loot": ["pass"]})
        failed = sorted(name for name, record in results.items() if record["status"] != "pass")
        # The footer as main() prints it, after the runner's own lines.
        log = output + f"\n{len(results) - len(failed)} passed, {len(failed)} failed in 0.0 min.\n"
        report, _ = parity_verdict.verdict(log, {"food", "loot"}, "1")
        self.assertEqual(report, {"passed": ["loot"], "failed": ["food"], "complete": True})

    def test_only_once(self):
        results, started, _ = self.run_harnesses({"food": ["fail", "fail"]})
        self.assertEqual((results["food"]["status"], results["food"]["fresh_rerun"], len(started)), ("fail", "fail", 2))

    def test_repeated_runs_get_no_rerun(self):
        results, started, _ = self.run_harnesses({"food": ["pass", "fail"]}, rounds={"food": 2})
        self.assertEqual((results["food"]["status"], len(started)), ("fail", 2))
        self.assertNotIn("fresh_rerun", results["food"])

    def test_a_failed_first_round_keeps_its_fresh_rerun_apart_from_the_later_rounds(self):
        # Round 1 fails with reuse, its fresh rerun passes, and round 2 fails.
        results, started, _ = self.run_harnesses({"food": ["fail", "pass", "fail"]}, rounds={"food": 2})
        record = results["food"]
        self.assertEqual(started, [("food", True), ("food", False), ("food~2", True)])
        self.assertEqual((record["status"], record["rounds"], record["fresh_rerun"]), ("fail", 2, "pass"))
        self.assertNotIn("flaky", record)  # both scheduled rounds failed
        with tempfile.TemporaryDirectory() as tmp:
            parity.remember_fresh_only(Path(tmp), results)
            self.assertEqual(set(parity.fresh_only(Path(tmp))), {"food"})

    def test_a_fresh_pass_is_remembered_as_fresh_only_though_it_failed(self):
        results, _, _ = self.run_harnesses({"food": ["fail", "pass"], "loot": ["fail", "fail"]})
        with tempfile.TemporaryDirectory() as tmp:
            parity.remember_fresh_only(Path(tmp), results)
            self.assertEqual(set(parity.fresh_only(Path(tmp))), {"food"})

    def test_reruns_reach_the_job_summary(self):
        with tempfile.TemporaryDirectory() as tmp:
            summary = Path(tmp) / "summary.md"
            with mock.patch.dict(os.environ, {"GITHUB_STEP_SUMMARY": str(summary)}), \
                    mock.patch("builtins.print"):
                parity.report_retries({"food": {"status": "fail", "retried": "reused oracle results",
                                                "fresh_rerun": "pass"},
                                       "loot": {"status": "pass"}})
            text = summary.read_text()
        self.assertIn("food", text)
        self.assertNotIn("loot", text)


class SoloTests(unittest.TestCase):
    """Harnesses driving game windows run alone on a small machine (playsunrise's 4-CPU runner)."""

    def executor(self, running):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(parity, "STATE_DIR", Path(tmp)):
            local = parity.LocalExecutor(4, 1 << 40, 60)
        local.solo = {"remote_fireworks"}
        local.running = {label: {} for label in running}
        return local

    def test_a_solo_harness_waits_for_an_empty_machine_and_then_holds_it(self):
        busy = self.executor(["food"])
        self.assertFalse(busy.accepts("remote_fireworks"))
        self.assertTrue(busy.accepts("light"))
        self.assertEqual(busy.free(), 3)
        self.assertTrue(self.executor([]).accepts("remote_fireworks"))
        alone = self.executor(["remote_fireworks~2"])
        self.assertEqual(alone.free(), 0)

    def test_only_small_machines_and_listed_harnesses(self):
        config = {"solo": {"remote_fireworks": "two game windows"}}
        with mock.patch.dict(os.environ, {"PARITY_SOLO": "1"}):
            self.assertEqual(parity.solo_harnesses(config), {"remote_fireworks"})
        with mock.patch.dict(os.environ, {"PARITY_SOLO": "0"}):
            self.assertEqual(parity.solo_harnesses(config), set())
        with mock.patch.dict(os.environ, {"PARITY_SOLO": ""}), mock.patch.object(parity, "SOLO_CPUS", 10 ** 6):
            self.assertEqual(parity.solo_harnesses(config), {"remote_fireworks"})
        solo = parity.host_config().get("solo", {})
        self.assertTrue(solo and set(solo) <= {parity.base_name(name) for name in parity.harnesses()})


class ClashTests(unittest.TestCase):
    """On Windows, harnesses that build one program with different selections don't run at once."""
    PACKAGES = {("bin", "sunrise-client"): "sunrise-client", ("bin", "sunrise-ctl"): "sunrise-client-lib",
                ("example", "client_probe"): "sunrise-client-lib"}
    SERVER = ["bin", "sunrise-client", "fast", ["-p", "sunrise-client"], False]
    BOTH = ["bins", "", "fast", ["-p", "sunrise-client", "-p", "sunrise-client-lib"], False]
    PROBE = ["example", "client_probe", "dev", ["-p", "sunrise-client-lib"], True]

    def test_a_run_naming_every_bin_builds_those_of_its_packages(self):
        self.assertEqual(parity.executables([self.BOTH], self.PACKAGES), {
            ("bin", "sunrise-client", "fast"): (("-p", "sunrise-client", "-p", "sunrise-client-lib"), False),
            ("bin", "sunrise-ctl", "fast"): (("-p", "sunrise-client", "-p", "sunrise-client-lib"), False)})

    def test_another_selection_of_a_running_harness_program_waits(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(parity, "STATE_DIR", Path(tmp)):
            local = parity.LocalExecutor(4, 1 << 40, 60)
        local.builds = {name: parity.executables(built, self.PACKAGES) for name, built in {
            "server": [self.SERVER], "same": [self.SERVER, self.PROBE], "both": [self.BOTH], "probe": [self.PROBE],
            "python_only": []}.items()}
        local.running = {"server": {}}
        self.assertFalse(local.accepts("both"))  # it would relink sunrise-client.exe while "server" runs it
        self.assertTrue(local.accepts("same"))  # Cargo finds it fresh
        self.assertTrue(local.accepts("probe"))
        self.assertTrue(local.accepts("python_only"))
        self.assertTrue(local.accepts("never_ran"))
        local.running = {"both~2": {}}
        self.assertFalse(local.accepts("server"))
        local.running = {}
        self.assertTrue(local.accepts("both"))


if __name__ == "__main__":
    unittest.main()


class WorkspaceManifestTests(unittest.TestCase):
    """A crate split's Cargo.toml and Cargo.lock changes don't select every harness; a dependency change still does."""
    CLIENT = '[package]\nname = "sunrise-client"\n\n[dependencies]\nserde = "1"\nwgpu = { version = "27", features = ["x"] }\n'
    LOCK = ('version = 4\n\n[[package]]\nname = "sunrise-client"\nversion = "0.1.0"\ndependencies = [\n "serde",\n]\n\n'
            '[[package]]\nname = "serde"\nversion = "1.0.1"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n')

    def verdict(self, base, head):
        """base, head: {path: text or None}; the manifests workspace_only_manifests leaves out."""
        with tempfile.TemporaryDirectory() as temp, \
                mock.patch.object(parity, "ROOT", Path(temp)), \
                mock.patch.object(parity, "git_show", lambda _base, path: base.get(path)):
            for path, text in head.items():
                if text is not None:
                    Path(temp, path).parent.mkdir(parents=True, exist_ok=True)
                    Path(temp, path).write_text(text)
            return parity.workspace_only_manifests(set(head) | set(base), "base")

    def test_a_crate_split_is_left_out(self):
        command = '[package]\nname = "sunrise-command"\n\n[dependencies]\nserde = "1"\nsunrise-core = { path = "../sunrise-core" }\n'
        client = self.CLIENT + 'sunrise-command = { path = "../sunrise-command" }\n'
        lock = self.LOCK.replace('[[package]]\nname = "serde"', '[[package]]\nname = "sunrise-command"\nversion = "0.1.0"\n\n'
                                 '[[package]]\nname = "serde"')
        base = {"Cargo.toml": '[workspace]\nmembers = ["crates/sunrise-client"]\nresolver = "2"\n',
                "crates/sunrise-client/Cargo.toml": self.CLIENT, "Cargo.lock": self.LOCK}
        head = {"Cargo.toml": '[workspace]\nmembers = ["crates/sunrise-client", "crates/sunrise-command"]\nresolver = "2"\n',
                "crates/sunrise-client/Cargo.toml": client, "crates/sunrise-command/Cargo.toml": command, "Cargo.lock": lock}
        self.assertEqual(self.verdict(base, head), set(head))

    def test_a_dependency_change_still_counts(self):
        base = {"crates/sunrise-client/Cargo.toml": self.CLIENT, "Cargo.lock": self.LOCK}
        bumped = {"crates/sunrise-client/Cargo.toml": self.CLIENT, "Cargo.lock": self.LOCK.replace("1.0.1", "1.0.2")}
        self.assertEqual(self.verdict(base, bumped), set())
        feature = {"crates/sunrise-client/Cargo.toml": self.CLIENT.replace('["x"]', '["x", "y"]')}
        self.assertEqual(self.verdict({"crates/sunrise-client/Cargo.toml": self.CLIENT}, feature), set())
        resolver = {"Cargo.toml": '[workspace]\nmembers = ["a"]\nresolver = "3"\n'}
        self.assertEqual(self.verdict({"Cargo.toml": '[workspace]\nmembers = ["a"]\nresolver = "2"\n'}, resolver), set())

    def test_a_feature_of_a_workspace_crate_counts_through_its_sources(self):
        items = '[package]\nname = "sunrise-items"\n\n[features]\ntest-support = []\n'
        client = self.CLIENT + '\n[dev-dependencies]\nsunrise-items = { path = "../sunrise-items", features = ["test-support"] }\n'
        head = {"crates/sunrise-items/Cargo.toml": items, "crates/sunrise-client/Cargo.toml": client,
                "crates/sunrise-items/src/lib.rs": '#[cfg(feature = "test-support")]\npub fn for_tests() {}\n'}
        base = {"crates/sunrise-client/Cargo.toml": self.CLIENT}
        # The new crate's files are all changed, the code the feature gates with them.
        self.assertEqual(self.verdict(base, head), {"crates/sunrise-items/Cargo.toml", "crates/sunrise-client/Cargo.toml"})
        # Turning on a feature of a crate whose gated code didn't change selects as before.
        world = '[package]\nname = "sunrise-world"\n\n[features]\nextra = ["more"]\nmore = []\n'
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)):
            Path(temp, "crates/sunrise-world/src").mkdir(parents=True)
            Path(temp, "crates/sunrise-world/Cargo.toml").write_text(world)
            Path(temp, "crates/sunrise-world/src/lib.rs").write_text('#[cfg(feature = "more")]\nfn x() {}\n')
            self.assertFalse(parity.feature_sources_changed("sunrise-world", "extra", {"crates/sunrise-client/Cargo.toml"}))
            self.assertTrue(parity.feature_sources_changed("sunrise-world", "extra", {"crates/sunrise-world/src/lib.rs"}))
            self.assertFalse(parity.feature_sources_changed("sunrise-nothing", "extra", set()))
        # A feature that reaches an external package counts.
        optional = {"crates/sunrise-client/Cargo.toml": self.CLIENT + '\n[features]\nfast = ["wgpu/x"]\n'}
        self.assertEqual(self.verdict({"crates/sunrise-client/Cargo.toml": self.CLIENT}, optional), set())

    def test_external_feature_sources_are_not_discarded(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)):
            root = Path(temp)
            (root / "crates/core").mkdir(parents=True)
            (root / "shared").mkdir()
            (root / "crates/core/Cargo.toml").write_text(
                '[package]\nname = "core"\n[lib]\npath = "../../shared/lib.rs"\n'
                '[features]\nextra = []\n')
            (root / "shared/lib.rs").write_text('#[cfg(feature = "extra")]\nfn extra() {}\n')
            self.assertFalse(parity.feature_sources_changed("core", "extra", {"crates/core/Cargo.toml"}))

    def test_without_a_base_everything_counts(self):
        self.assertEqual(parity.workspace_only_manifests({"Cargo.lock"}, None), set())


# A harness that leaves a child behind: the child locks a file, says so, and sleeps; the harness then exits or hangs.
# The lock frees once the child is dead, on every platform (flock, or msvcrt's byte lock).
LEAVING_HARNESS = """
import subprocess, sys, time
from pathlib import Path
held = Path(sys.argv[0]).with_name("held")
child = subprocess.Popen([sys.executable, "-c", '''
import sys, time
sys.path.insert(0, sys.argv[1])
from tools import file_lock
handle = open(sys.argv[2], "w")
file_lock.lock(handle)
open(sys.argv[2] + ".ready", "w").close()
time.sleep(120)
''', str(Path({root!r})), str(held)], stdout=subprocess.DEVNULL)
while not Path(str(held) + ".ready").exists():
    time.sleep(0.05)
if {hang!r}:
    time.sleep(120)
"""


def child_released(held, seconds=30):
    """Whether the left child's lock on held frees within seconds (it is dead)."""
    from tools import file_lock
    deadline = time.monotonic() + seconds
    with open(held, "a") as handle:
        while time.monotonic() < deadline:
            if file_lock.lock(handle, wait=False):
                return True
            time.sleep(0.1)
    return False


class LeftProcessTests(unittest.TestCase):
    """What a harness starts ends with it, whether it exits, crashes or times out (tools/process_tree.py)."""

    def run_harness(self, hang, timeout):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            script = tmp / "test_leaving_parity.py"
            script.write_text(LEAVING_HARNESS.format(root=str(parity.ROOT), hang=hang), encoding="utf-8")
            with mock.patch.object(parity, "STATE_DIR", tmp / "state"), mock.patch.object(parity, "SHARED", tmp / "shared"):
                local = parity.LocalExecutor(1, 1 << 40, timeout)
                self.assertTrue(local.start("leaving", script, False, 0))
                finished, deadline = [], time.monotonic() + 120
                while not finished and time.monotonic() < deadline:
                    finished = local.poll()
                    time.sleep(0.1)
                local.stop()
            self.assertEqual([name for name, *_ in finished], ["leaving"])
            self.assertTrue((tmp / "held.ready").exists())
            self.assertTrue(child_released(tmp / "held"), "the harness's child outlived it")
            return finished[0]

    @unittest.skipUnless(os.name == "nt" or hasattr(os, "waitid"), "macOS before Python 3.13 has no os.waitid")
    def test_a_finished_harness_takes_what_it_left_running(self):
        _, _, passed, *_ = self.run_harness(hang=False, timeout=120)
        self.assertTrue(passed)

    def test_a_timed_out_harness_takes_what_it_started(self):
        _, _, passed, *_ = self.run_harness(hang=True, timeout=5)
        self.assertFalse(passed)
