import shutil
import subprocess
import unittest
from unittest import mock

from tools.ci.test_scope import NOT_READS, READERS, SCANNED, Workspace, cargo_metadata, flags, reads, under


def package(name, folder, *dependencies, source=None):
    return {
        "name": name,
        "manifest_path": f"/repo/{folder}/Cargo.toml",
        "targets": [{"src_path": f"/repo/{source or folder + '/src'}/lib.rs"}],
        "dependencies": [{"name": dependency} for dependency in dependencies],
    }


def workspace(*packages):
    return Workspace({"workspace_root": "/repo", "packages": list(packages)})


def metadata_package(name, folder, dependencies=(), targets=None):
    return {
        "name": name,
        "manifest_path": f"/repo/{folder}/Cargo.toml",
        "targets": targets or [{"kind": ["lib"], "src_path": f"/repo/{folder}/src/lib.rs"}],
        "dependencies": list(dependencies),
    }


def metadata_dependency(name, kind=None, target=None):
    dependency = {"name": name, "kind": kind}
    if target is not None:
        dependency["target"] = target
    return dependency


class SelectionTests(unittest.TestCase):
    def setUp(self):
        self.graph = workspace(
            package("base", "crates/base"),
            package("mid", "crates/mid", "base"),
            package("top", "crates/top", "mid"),
            package("other", "crates/other"),
            package("twin", "crates/twin", source="crates/base/src"),
        )

    def test_a_change_selects_its_package_and_whatever_depends_on_it(self):
        self.assertEqual(self.graph.scope(["crates/mid/src/a.rs"]), {"mid", "top"})
        self.assertEqual(self.graph.scope(["crates/other/tests/t.rs"]), {"other"})
        self.assertEqual(self.graph.scope(["crates/mid/Cargo.toml", "crates/other/build.rs"]), {"mid", "top", "other"})

    def test_a_package_built_from_another_ones_folder_follows_it(self):
        self.assertEqual(self.graph.scope(["crates/base/assets/a.json"]), {"base", "twin", "mid", "top"})

    def test_everything_selected_is_the_whole_workspace(self):
        self.assertIsNone(self.graph.scope(["crates/base/src/a.rs", "crates/other/src/a.rs"]))

    def test_a_change_outside_every_package_runs_everything(self):
        for path in ["Cargo.lock", "Cargo.toml", ".cargo/config.toml", "clippy.toml", ".github/workflows/ci.yml",
                     ".github/PULL_REQUEST_TEMPLATE.md", "tools/ci/test_scope.py", "tools/pr_ready.py",
                     "native-hooks.json", "third_party/naga/src/lib.rs", "crates/not-a-member/src/lib.rs"]:
            with self.subTest(path=path):
                self.assertIsNone(self.graph.scope(["crates/other/src/a.rs", path]))

    def test_documentation_selects_nothing_unless_a_test_reads_it(self):
        self.assertEqual(self.graph.scope(["README.md", "docs/setup.md", "docs/media/banner.webp"]), set())
        with mock.patch.dict(READERS, {"other": ["docs/api/reference.md"]}):
            self.assertEqual(self.graph.scope(["docs/api/reference.md", "docs/setup.md"]), {"other"})
        # A markdown file in a package's folder may be included by it.
        self.assertEqual(self.graph.scope(["crates/other/README.md"]), {"other"})

    def test_a_reader_is_selected_with_its_dependents(self):
        with mock.patch.dict(READERS, {"mid": ["crates/other/assets"]}):
            self.assertEqual(self.graph.scope(["crates/other/assets/a.json"]), {"other", "mid", "top"})
            self.assertEqual(self.graph.scope(["crates/other/src/a.rs"]), {"other"})

    def test_a_package_that_only_reads_at_run_time_is_selected_alone(self):
        with mock.patch.dict(SCANNED, {"top": ["crates/other"]}):
            self.assertEqual(self.graph.scope(["crates/other/src/a.rs"]), {"other", "top"})
            self.assertEqual(self.graph.scope(["crates/base/src/a.rs"]), {"base", "twin", "mid", "top"})
        with mock.patch.dict(SCANNED, {"mid": ["crates/other"]}):
            self.assertEqual(self.graph.scope(["crates/other/src/a.rs"]), {"other", "mid"})

    def test_the_output_is_the_flags_cargo_takes_or_empty_for_the_workspace(self):
        self.assertEqual(flags({"b", "a"}), "packages=-p a -p b")
        self.assertEqual(flags(None), "packages=")
        self.assertEqual(flags(set()), "packages=")


class ModApiGenDiscoveryTests(unittest.TestCase):
    def setUp(self):
        self.graph = workspace(
            metadata_package("sunrise-mod-api", "crates/sunrise-mod-api",
                             [metadata_dependency("api-leaf")]),
            metadata_package("sunrise-mod-vocab", "crates/sunrise-mod-vocab",
                             [metadata_dependency("target-leaf", "normal", "cfg(unix)")]),
            metadata_package("sunrise-core", "crates/sunrise-core", [
                metadata_dependency("build-helper", "build"),
                metadata_dependency("dev-helper", "dev"),
            ], targets=[
                {"kind": ["lib"], "src_path": "/repo/crates/sunrise-core/src/lib.rs"},
                {"kind": ["bin"], "src_path": "/repo/crates/sunrise-core/tools/cli.rs"},
            ]),
            metadata_package("sunrise-mod-runtime", "crates/sunrise-mod-runtime",
                             [metadata_dependency("runtime-leaf", "normal")]),
            metadata_package("api-leaf", "crates/api-leaf",
                             [metadata_dependency("api-transitive", "normal")]),
            metadata_package("api-transitive", "crates/api-transitive"),
            metadata_package("target-leaf", "crates/target-leaf"),
            metadata_package("runtime-leaf", "crates/runtime-leaf"),
            metadata_package("build-helper", "crates/build-helper",
                             [metadata_dependency("build-transitive", "normal")]),
            metadata_package("build-transitive", "crates/build-transitive"),
            metadata_package("dev-helper", "crates/dev-helper"),
            metadata_package("unrelated", "crates/unrelated"),
            metadata_package("sunrise-mod-api-gen", "crates/sunrise-mod-api-gen"),
        )

    def test_a_changed_workspace_package_manifest_selects_the_generator(self):
        chosen = self.graph.scope(["crates/unrelated/Cargo.toml"])
        self.assertIn("sunrise-mod-api-gen", chosen)

    def test_reachable_library_roots_follow_normal_and_target_specific_dependencies(self):
        for path in [
            "crates/sunrise-mod-api/src/lib.rs",
            "crates/api-leaf/src/lib.rs",
            "crates/api-transitive/src/lib.rs",
            "crates/sunrise-mod-vocab/src/lib.rs",
            "crates/target-leaf/src/lib.rs",
            "crates/sunrise-core/src/lib.rs",
            "crates/sunrise-mod-runtime/src/lib.rs",
            "crates/runtime-leaf/src/lib.rs",
        ]:
            with self.subTest(path=path):
                self.assertIn("sunrise-mod-api-gen", self.graph.scope([path]))

    def test_unrelated_and_non_library_or_build_dev_inputs_do_not_select_the_generator(self):
        for path in [
            "crates/unrelated/src/lib.rs",
            "crates/sunrise-core/build.rs",
            "crates/sunrise-core/tests/dev_only.rs",
            "crates/sunrise-core/tools/cli.rs",
            "crates/build-helper/src/lib.rs",
            "crates/build-transitive/src/lib.rs",
            "crates/dev-helper/src/lib.rs",
        ]:
            with self.subTest(path=path):
                chosen = self.graph.scope([path])
                self.assertIsNotNone(chosen)
                self.assertNotIn("sunrise-mod-api-gen", chosen)


@unittest.skipUnless(shutil.which("cargo"), "needs cargo metadata")
class RealWorkspaceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workspace = Workspace(cargo_metadata())

    def test_sunrise_client_lib_is_built_from_sunrise_clients_files(self):
        chosen = self.workspace.scope(["crates/sunrise-client/src/cut_layout/game/x.rs"])
        self.assertLessEqual({"sunrise-client", "sunrise-client-lib", "sunrise-game-data"}, chosen)
        self.assertNotIn("sunrise-release-sign", chosen)
        self.assertIn("sunrise-client-lib", self.workspace.scope(["crates/sunrise-client/tests/lan_bot.rs"]))
        gameplay = self.workspace.scope(["crates/sunrise-gameplay-common/src/game.rs"])
        self.assertLessEqual(
            {"sunrise-gameplay-common", "sunrise-gameplay", "sunrise-client-gameplay", "sunrise-client"},
            gameplay,
        )

    def test_every_reader_names_a_package_and_a_path_in_the_tree(self):
        tracked = subprocess.check_output(["git", "ls-files"]).decode().split("\n")
        for name, paths in [*READERS.items(), *SCANNED.items()]:
            self.assertIn(name, self.workspace.dirs)
            for path in paths:
                with self.subTest(reader=name, path=path):
                    self.assertTrue(any(under(file, path) for file in tracked), f"{path} is not in the tree")

    def test_every_cross_crate_read_is_in_the_readers_table(self):
        missing = []
        for name, found in reads(self.workspace).items():
            for path, where in sorted(found.items()):
                if path in NOT_READS.get(name, ()):
                    continue
                if name in self.workspace.readers(path) or name in self.workspace.readers(path, SCANNED):
                    continue
                if path == "." and (READERS.get(name) or SCANNED.get(name)):
                    continue
                missing.append(f"{name} reads {path} ({where})")
        self.assertEqual(missing, [], "add each to READERS in tools/ci/test_scope.py, or to NOT_READS if it is no read")

    def test_the_scan_finds_the_reads_the_table_was_built_from(self):
        found = reads(self.workspace)
        self.assertIn("crates/sunrise-client/assets/sunrise/lang", found["sunrise-game-data"])
        self.assertIn("docs/api/server-reference.md", found["sunrise-server"])
        self.assertIn("crates", found["sunrise-game-data"])


if __name__ == "__main__":
    unittest.main()
