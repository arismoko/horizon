import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from tests.git_fixture import isolated_git_env
from tools import check_layers, merge_base

CONFIG = {
    "may_depend": {"engine": ["engine"], "platform": ["engine", "platform"], "sunrise": ["platform", "sunrise"]},
    "crates": {
        "core": "engine",
        "game": "engine",
        "mod-api": "platform",
        "cosmetics": "sunrise",
        "launcher": "sunrise",
        "xtask": "tool",
    },
    "allowed": [],
}


def package(name, *deps):
    """A `cargo metadata` package; a dep is a name, or (name, kind) for dev/build."""
    out = []
    for dep in deps:
        dep_name, kind = dep if isinstance(dep, tuple) else (dep, None)
        out.append({"name": dep_name, "kind": kind, "path": f"/x/{dep_name}"})
    return {"name": name, "dependencies": out}


def run(*packages, allowed=(), base=None, crates=None):
    config = dict(CONFIG, allowed=list(allowed), crates=dict(CONFIG["crates"], **(crates or {})))
    return check_layers.check(config, check_layers.edges(packages), base)


class CheckLayersTests(unittest.TestCase):
    def test_downward_and_same_layer_dependencies_pass(self):
        problems, notes = run(package("mod-api", "core"), package("launcher", "mod-api", "cosmetics"))
        self.assertEqual((problems, notes), ([], []))

    def test_engine_may_not_depend_on_platform_or_sunrise(self):
        problems, _ = run(package("game", "mod-api", "cosmetics"))
        self.assertEqual(len(problems), 2)
        self.assertTrue(any("game -> mod-api: engine may not depend on platform" in p for p in problems))
        self.assertTrue(all("docs/layers.md#engine" in p for p in problems))

    def test_platform_may_not_depend_on_sunrise(self):
        problems, _ = run(package("mod-api", "cosmetics"))
        self.assertIn("docs/layers.md#platform", problems[0])

    def test_sunrise_uses_the_platform_not_the_engine(self):
        # The Sunrise layer builds against the platform's public API, so it can live in its own repository.
        problems, _ = run(package("launcher", "game"))
        self.assertIn("launcher -> game: sunrise may not depend on engine", problems[0])
        self.assertIn("docs/layers.md#sunrise", problems[0])

    def test_an_allowed_exception_passes(self):
        self.assertEqual(run(package("game", "cosmetics"), allowed=["game -> cosmetics"]), ([], []))

    def test_build_dependencies_count_and_dev_dependencies_do_not(self):
        self.assertEqual(len(run(package("core", ("mod-api", "build")))[0]), 1)
        self.assertEqual(run(package("core", ("mod-api", "dev"))), ([], []))
        self.assertEqual(len(run(package("core", "mod-api"))[0]), 1)

    def test_an_exception_with_no_dependency_left_is_only_a_note(self):
        problems, notes = run(package("game", "core"), allowed=["game -> cosmetics"])
        self.assertEqual(problems, [])
        self.assertIn("game -> cosmetics", notes[0])

    def test_a_branch_may_not_add_exceptions_between_known_crates(self):
        base = {"crates": CONFIG["crates"], "allowed": []}
        problems, _ = run(package("game", "cosmetics"), allowed=["game -> cosmetics"], base=base)
        self.assertIn("added to 'allowed'", problems[0])
        kept = {"crates": CONFIG["crates"], "allowed": ["game -> cosmetics"]}
        self.assertEqual(run(package("game", "cosmetics"), allowed=["game -> cosmetics"], base=kept), ([], []))

    def test_a_renamed_crate_carries_its_exception(self):
        base = {"crates": CONFIG["crates"], "allowed": ["game -> cosmetics"]}
        problems, _ = run(package("gameplay", "cosmetics"), allowed=["gameplay -> cosmetics"], base=base,
                          crates={"gameplay": "engine"})
        self.assertEqual(problems, [])

    def test_only_witnessed_transfers_keep_their_exception(self):
        old, new = "sunrise-client", "sunrise-client-gameplay"
        source = f"{old} -> sunrise-friends"
        dest = f"{new} -> sunrise-friends"
        old_telemetry = f"{old} -> sunrise-telemetry"
        new_telemetry = f"{new} -> sunrise-telemetry"
        layers = dict(CONFIG["crates"], **{old: "engine", new: "engine",
                                          "sunrise-friends": "sunrise", "sunrise-telemetry": "sunrise"})
        graph = {old: {"sunrise-telemetry"}, new: {"sunrise-friends", "sunrise-telemetry"}}
        base = dict(CONFIG, crates=layers, allowed=[source, old_telemetry])
        config = dict(CONFIG, crates=layers, allowed=[dest, old_telemetry, new_telemetry])
        self.assertEqual(check_layers.check(config, graph, base, {dest}), ([], []))
        self.assertIn("added to 'allowed'", check_layers.check(config, graph, base)[0][0])
        self.assertIn("added to 'allowed'", check_layers.check(
            config, graph, dict(base, allowed=[old_telemetry]), {dest})[0][0])
        graph[old].add("sunrise-friends")
        copied = dict(config, allowed=config["allowed"] + [source])
        self.assertIn("added to 'allowed'", check_layers.check(copied, graph, base, {dest})[0][0])

    def test_a_moved_client_exception_may_become_optional(self):
        old, new = "sunrise-client", "sunrise-client-gameplay"
        for dep in ("sunrise-optifine", "sunrise-theme"):
            with self.subTest(dep=dep):
                source, dest = f"{old} -> {dep}", f"{new} -> {dep}"
                layers = dict(CONFIG["crates"], **{old: "engine", new: "engine", dep: "platform"})
                base = dict(CONFIG, crates=layers, allowed=[source])
                config = dict(CONFIG, crates=layers, optional=[{"edge": dest, "reason": "client-only"}])
                graph = {old: set(), new: {dep}}
                self.assertEqual(check_layers.check(config, graph, base, {dest}), ([], []))
                self.assertIn("added to 'optional'", check_layers.check(
                    config, graph, dict(base, allowed=[]), {dest})[0][0])
                self.assertTrue(any("no longer optional" in problem for problem in
                                    check_layers.check(config, graph, base)[0]))
                for still_has_dependency, still_has_exception in ((True, False), (False, True)):
                    current = dict(config, allowed=[source] if still_has_exception else [])
                    graph[old] = {dep} if still_has_dependency else set()
                    problems, _ = check_layers.check(current, graph, base, {dest})
                    self.assertTrue(any("added to 'optional'" in problem for problem in problems))

    def test_a_portable_gameplay_generator_edge_must_be_build_only(self):
        owner, generator = "sunrise-gameplay-common", "sunrise-mod-api-gen"
        config = dict(CONFIG, crates={owner: "engine", generator: "engine"},
                      portable={"crates": [owner, generator], "forbidden": ["wgpu"]})
        self.assertEqual(check_layers.portable(config, [package(owner, (generator, "build"))]), [])
        self.assertEqual(check_layers.portable(config, [package(owner, generator)]),
                         [f"{owner} -> {generator}: portable gameplay may use the generator only as a build dependency"])
        gpu = {"name": "wgpu", "kind": None, "path": None}
        self.assertIn("takes no window, GPU", check_layers.portable(
            config, [package(owner, (generator, "build")), {"name": generator, "dependencies": [gpu]}])[0])

    def test_a_new_dependency_target_excuses_nothing(self):
        base = {"crates": CONFIG["crates"], "allowed": []}
        problems, _ = run(package("game", "shop"), allowed=["game -> shop"], base=base, crates={"shop": "sunrise"})
        self.assertIn("added to 'allowed'", problems[0])

    def test_unknown_crate_fails(self):
        self.assertIn("no layer", run(package("newcrate", "core"))[0][0])

    def test_tools_may_use_anything_but_nothing_shipped_uses_tools(self):
        self.assertEqual(run(package("xtask", "cosmetics", "game")), ([], []))
        self.assertIn("dev tool", run(package("game", "xtask"))[0][0])

    def test_dependencies_outside_the_layers_are_ignored(self):
        # Vendored crates (third_party/) and test fixtures have no layer.
        self.assertEqual(run(package("game", "naga")), ([], []))

    def test_portable_crates_depend_only_on_each_other_and_no_desktop_library(self):
        config = dict(CONFIG, portable={"crates": ["core"], "forbidden": ["wgpu"]})
        clean = {"name": "core", "dependencies": [
            {"name": "serde", "kind": None, "path": None},
            {"name": "wgpu", "kind": "dev", "path": None},
            {"name": "game", "kind": "dev", "path": "/x/game"},
        ]}
        self.assertEqual(check_layers.portable(config, [clean]), [])
        dirty = {"name": "core", "dependencies": [
            {"name": "game", "kind": None, "path": "/x/game"},
            {"name": "wgpu", "kind": "build", "path": None},
            {"name": "wgpu", "kind": None, "path": None, "target": "cfg(unix)"},
        ]}
        problems = check_layers.portable(config, [dirty])
        self.assertEqual(len(problems), 2, problems)
        self.assertIn("core -> game: a portable crate depends only on the other portable crates", problems[0])
        self.assertIn("core -> wgpu: a portable crate takes no window, GPU", problems[1])

    def test_portable_baseline_only_exempts_the_recorded_optional_edge(self):
        config = dict(CONFIG, portable={"crates": ["core"], "forbidden": []},
                      optional=[{"edge": "core -> mod-api", "reason": "Nonportable optional feature"}])
        dep = {"name": "mod-api", "kind": None, "path": "/x/mod-api", "optional": True}
        pkg = {"name": "core", "dependencies": [dep]}
        self.assertEqual(check_layers.portable(config, [pkg]), [])
        dep["optional"] = False
        self.assertEqual(len(check_layers.portable(config, [pkg])), 1)
        dep.update(name="game", optional=True)
        self.assertEqual(len(check_layers.portable(config, [pkg])), 1)

    def test_a_portable_crate_is_an_engine_crate(self):
        config = dict(CONFIG, portable={"crates": ["mod-api", "gone"], "forbidden": []})
        self.assertEqual(check_layers.portable(config, []), [
            f"gone: listed as portable in {check_layers.CONFIG_PATH}, but not an engine crate there",
            f"mod-api: listed as portable in {check_layers.CONFIG_PATH}, but not an engine crate there",
        ])

    def test_an_optional_exception_holds_only_while_the_dependency_is_optional(self):
        config = dict(CONFIG, optional=[{"edge": "core -> mod-api", "reason": "behind a feature"}])
        optional_dep = {"name": "mod-api", "kind": None, "path": "/x/mod-api", "optional": True}
        packages = [{"name": "core", "dependencies": [optional_dep]}]
        graph = check_layers.edges(packages)
        self.assertEqual(check_layers.check(config, graph, None, check_layers.optional_edges(packages)), ([], []))
        # The same dependency made mandatory (here by a build-dependency next to the optional one) fails.
        packages[0]["dependencies"].append({"name": "mod-api", "kind": "build", "path": "/x/mod-api"})
        problems, _ = check_layers.check(config, graph, None, check_layers.optional_edges(packages))
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("core -> mod-api: listed in 'optional'", problems[0])
        self.assertIn("no longer optional", problems[0])

    def test_an_optional_exception_excuses_no_other_edge(self):
        config = dict(CONFIG, optional=[{"edge": "core -> mod-api", "reason": "behind a feature"}])
        packages = [{"name": "game", "dependencies": [
            {"name": "mod-api", "kind": None, "path": "/x/mod-api", "optional": True}]}]
        problems, _ = check_layers.check(config, check_layers.edges(packages), None,
                                         check_layers.optional_edges(packages))
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("game -> mod-api: engine may not depend on platform", problems[0])

    def test_optional_edges_ignore_dev_and_registry_dependencies(self):
        packages = [{"name": "core", "dependencies": [
            {"name": "mod-api", "kind": None, "path": "/x/mod-api", "optional": True},
            {"name": "mod-api", "kind": "dev", "path": "/x/mod-api"},
            {"name": "serde", "kind": None, "path": None, "optional": True},
        ]}]
        self.assertEqual(check_layers.optional_edges(packages), {"core -> mod-api"})

    def test_an_optional_exception_with_no_dependency_left_is_only_a_note(self):
        config = dict(CONFIG, optional=[{"edge": "core -> mod-api", "reason": "behind a feature"}])
        problems, notes = check_layers.check(config, check_layers.edges([package("core")]))
        self.assertEqual(problems, [])
        self.assertEqual(len(notes), 1)
        self.assertIn("core -> mod-api: no longer a dependency; you can remove it from 'optional'", notes[0])

    def test_a_branch_may_move_an_exception_to_optional_but_not_add_one(self):
        packages = [{"name": "core", "dependencies": [
            {"name": "mod-api", "kind": None, "path": "/x/mod-api", "optional": True}]}]
        graph, optional = check_layers.edges(packages), check_layers.optional_edges(packages)
        config = dict(CONFIG, optional=[{"edge": "core -> mod-api", "reason": "behind a feature"}])
        moved = dict(CONFIG, allowed=["core -> mod-api"])
        self.assertEqual(check_layers.check(config, graph, moved, optional), ([], []))
        self.assertEqual(check_layers.check(config, graph, config, optional), ([], []))
        problems, _ = check_layers.check(config, graph, CONFIG, optional)
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("core -> mod-api: added to 'optional'", problems[0])

    def test_the_repository_config_keeps_its_rows_apart(self):
        config = check_layers.load_config()
        allowed, optional = config["allowed"], check_layers.optional_rows(config)
        self.assertEqual(len(allowed), len(set(allowed)), "an 'allowed' row is listed twice")
        self.assertEqual(optional & set(allowed), set(), "an edge is in both 'allowed' and 'optional'")
        for row in config.get("optional", []):
            self.assertTrue(row.get("reason"), f"{row['edge']}: an 'optional' row needs a reason")
        for workspace in config["workspaces"]:
            self.assertTrue((check_layers.ROOT / workspace).is_file(), workspace)

    def test_a_pending_crate_may_use_only_the_crates_its_entry_lists(self):
        config = dict(CONFIG, portable={"crates": ["core"], "pending": {"game": ["mod-api"]}, "forbidden": ["wgpu"]})
        listed = package("game", "core", "mod-api")
        self.assertEqual(check_layers.portable(config, [listed]), [])
        unlisted = package("game", "core", "mod-api", "cosmetics")
        wgpu = {"name": "wgpu", "kind": None, "path": None}
        problems = check_layers.portable(config, [dict(unlisted, dependencies=unlisted["dependencies"] + [wgpu])])
        self.assertEqual(len(problems), 2, problems)
        self.assertIn("game -> cosmetics: a portable crate depends only on the other portable crates", problems[0])
        self.assertIn("game -> wgpu: a portable crate takes no window, GPU", problems[1])

    def test_a_crate_is_pending_or_portable_never_both(self):
        config = dict(CONFIG, portable={"crates": ["core", "game"], "pending": {"game": []}, "forbidden": []})
        self.assertEqual(check_layers.portable(config, []), [
            f"game: in both portable.crates and portable.pending of {check_layers.CONFIG_PATH}",
        ])

    def test_pending_entries_only_shrink(self):
        def config(*deps):
            return dict(CONFIG, portable={"crates": ["core"], "pending": {"game": list(deps)}, "forbidden": []})
        base = config("mod-api")
        self.assertEqual(check_layers.portable(config("mod-api"), [], base), [])
        self.assertEqual(check_layers.portable(config(), [], base), [])
        problems = check_layers.portable(config("mod-api", "cosmetics"), [], base)
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("game -> cosmetics: added to portable.pending", problems[0])

    def sides(self, *packages):
        config = dict(CONFIG, sides={"crates": ["game", "core"]}, crates=dict(CONFIG["crates"], extra="engine"))
        return check_layers.check(config, check_layers.edges(packages))[0]

    def test_the_two_side_crates_may_not_depend_on_each_other(self):
        problems = self.sides(package("core", "game"), package("game"))
        self.assertEqual(problems, [f"core -> game: gameplay's sides may not depend on each other "
                                    f"({check_layers.CONFIG_PATH} sides)"])

    def test_a_side_dependency_through_a_third_crate_is_refused(self):
        problems = self.sides(package("core", "extra"), package("extra", "game"), package("game"))
        self.assertIn(f"core -> game: gameplay's sides may not depend on each other "
                      f"({check_layers.CONFIG_PATH} sides)", problems)
        self.assertTrue(any(p.startswith("extra -> game: a crate below") for p in problems), problems)

    def test_independent_side_crates_pass(self):
        self.assertEqual(self.sides(package("core", "extra"), package("extra"), package("game", "extra")), [])

    def test_registry_dependencies_are_not_edges(self):
        pkg = {"name": "game", "dependencies": [{"name": "cosmetics", "kind": None, "path": None}]}
        self.assertEqual(check_layers.edges([pkg]), {"game": set()})


class RollupTests(unittest.TestCase):
    def test_a_branch_merging_another_branch_with_a_new_crate_passes_offline(self):
        """A rollup merges pull request heads main lacks; a crate one of them adds gets its layer like any other."""
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            env = isolated_git_env(GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t.invalid",
                                   GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t.invalid")

            def run(*args):
                return subprocess.run(["git", *args], cwd=root, check=True, capture_output=True, env=env,
                                      text=True).stdout.strip()

            (root / "tools").mkdir()
            (root / "tools/layers.json").write_text(json.dumps(CONFIG))
            run("init", "-q", "-b", "main")
            run("add", ".")
            run("commit", "-q", "-m", "base")
            base = run("rev-parse", "HEAD")
            run("checkout", "-q", "-b", "member")
            config = dict(CONFIG, crates=dict(CONFIG["crates"], rule="sunrise"))
            (root / "tools/layers.json").write_text(json.dumps(config))
            run("commit", "-q", "-am", "a new crate")
            run("checkout", "-q", "-b", "rollup", base)
            run("merge", "-q", "--no-ff", "-m", "merge member", "member")
            packages = [package("rule", "cosmetics"), package("cosmetics")]
            with mock.patch.object(check_layers, "ROOT", root), mock.patch.object(merge_base, "ROOT", root), \
                    mock.patch.object(check_layers, "workspace_packages", return_value=packages), \
                    mock.patch.object(check_layers, "load_config", return_value=config), \
                    mock.patch("sys.stdout", new_callable=io.StringIO) as out:
                self.assertEqual(check_layers.main(["--base", base]), 0, out.getvalue())


class PullRequestMergeTests(unittest.TestCase):
    def test_a_pull_request_merge_compares_with_main_as_merged(self):
        """main added an exception after the branch left it: on the pull request's merge into that main, the base
        holds it, so the pull request isn't charged with it; the event's old base does not."""
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            env = isolated_git_env(GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t.invalid",
                                   GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t.invalid")

            def run(*args):
                return subprocess.run(["git", *args], cwd=root, check=True, capture_output=True, env=env,
                                      text=True).stdout.strip()

            (root / "tools").mkdir()
            (root / "tools/layers.json").write_text(json.dumps({"allowed": []}))
            (root / "work.txt").write_text("a\n")
            run("init", "-q", "-b", "main")
            run("add", ".")
            run("commit", "-q", "-m", "base")
            base = run("rev-parse", "HEAD")
            run("checkout", "-q", "-b", "work")
            (root / "work.txt").write_text("b\n")
            run("commit", "-q", "-am", "unrelated change")
            run("checkout", "-q", "main")
            (root / "tools/layers.json").write_text(json.dumps({"allowed": ["core -> sunrise"]}))
            run("commit", "-q", "-am", "main allows an exception")
            run("checkout", "-q", "--detach", "main")
            run("merge", "-q", "--no-ff", "-m", "merge work", "work")
            with mock.patch.object(check_layers, "ROOT", root), mock.patch.object(merge_base, "ROOT", root):
                merged, error = merge_base.pull_request_base(base, True)
                self.assertIsNone(error)
                self.assertEqual(check_layers.base_config(merged), {"allowed": ["core -> sunrise"]})
                self.assertEqual(check_layers.base_config(base), {"allowed": []})
                run("checkout", "-q", "work")
                self.assertEqual(merge_base.pull_request_base(base, True)[1],
                                 "--pull-request-merge needs HEAD to merge the pull request into a descendant of --base.")


class BaseConfigTests(unittest.TestCase):
    def test_the_base_is_where_the_branch_left_main(self):
        """An entry main dropped after the branch left it is still on the branch's base: the branch didn't add it."""
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            env = isolated_git_env(GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t.invalid",
                                   GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t.invalid")

            def run(*args):
                subprocess.run(["git", *args], cwd=root, check=True, capture_output=True, env=env)

            def allow(*entries):
                (root / "tools").mkdir(exist_ok=True)
                (root / "tools" / "layers.json").write_text(json.dumps({"allowed": list(entries)}))

            run("init", "-q", "-b", "main")
            allow("game -> cosmetics")
            run("add", ".")
            run("commit", "-q", "-m", "base")
            run("checkout", "-q", "-b", "work")
            run("commit", "-q", "--allow-empty", "-m", "work")
            run("checkout", "-q", "main")
            allow()
            run("commit", "-q", "-am", "main drops the entry")
            run("checkout", "-q", "work")
            with mock.patch.object(check_layers, "ROOT", root):
                self.assertEqual(check_layers.base_config("main"), {"allowed": ["game -> cosmetics"]})


if __name__ == "__main__":
    unittest.main()
