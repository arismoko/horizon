import base64
import contextlib
import datetime as dt
import hashlib
import io
import json
import os
import shutil
import string
import subprocess
import tempfile
import unittest
from unittest import mock
from pathlib import Path

from tests.git_fixture import git_in, isolated_git_env
from tools.ci import nightly
from tools.ci import nightly_set as ns
from tools.ci import vendored_yaml

yaml = vendored_yaml.load()

NOW = "2026-10-28T12:00:00+00:00"
PATTERNS = ns.guarded_patterns((Path(__file__).resolve().parents[1] / ".github/CODEOWNERS").read_text())


def pr(n, head="h", state="open", files=("crates/sunrise-world/src/lib.rs",), base="main"):
    return ns.PullRequest(n, state, f"{head}{n}", f"Fix {n}", f"dev{n}", base, list(files))


def lookup(*prs):
    known = {p.number: p for p in prs}
    return lambda n: known.get(n)


def run(state, text, login="kacpherek", *prs):
    return ns.apply(state, ns.parse(text), ns.Actor(login, 101 if login == "kacpherek" else 9999), NOW,
                    lookup(*prs), PATTERNS)

T = dt.datetime(2026, 10, 28, 12, 0, tzinfo=dt.timezone.utc)


def merged_pr(n, title, labels, issues=(), paths=("crates/sunrise-world/src/lib.rs",)):
    return {"number": n, "title": title, "labels": labels, "issues": list(issues), "author": f"dev{n}",
            "paths": list(paths)}


def notes(chosen=None, picked=(), left_out=(), merged=(), issues=()):
    chosen = chosen or nightly.Ref("main", "b" * 40, T)
    return nightly.notes("2026-10-28", chosen, nightly.Ref("main", "m" * 40, T), list(picked), list(left_out),
                         list(merged), list(issues))


class NightlyTests(unittest.TestCase):
    def test_release_keys_and_publishers_stay_on_github_hosted_runners(self):
        import re
        from tests.test_ci_concurrency import ROOT

        workflows = ROOT / ".github/workflows"
        for path in sorted(workflows.iterdir()):
            if path.suffix not in (".yml", ".yaml"):
                continue
            workflow = yaml.load(path.read_text(), Loader=yaml.BaseLoader)
            for name, job in workflow.get("jobs", {}).items():
                environment = job.get("environment", "")
                if isinstance(environment, dict):
                    environment = environment.get("name", "")
                permissions = job.get("permissions", workflow.get("permissions", {}))
                writes_contents = permissions == "write-all" or (
                    isinstance(permissions, dict) and permissions.get("contents") == "write")
                definition = json.dumps(job)
                publisher = writes_contents and (
                    re.search(r"publish|release", name + " " + job.get("name", ""), re.I)
                    or "gh release " in definition)
                sensitive = (re.search(r"\brelease\b", environment)
                             or "RELEASE_SIGNING_KEY" in definition or publisher)
                if not sensitive:
                    continue
                with self.subTest(workflow=path.name, job=name):
                    if "uses" in job:
                        # Reusable calls have no runner. Their local concrete jobs are checked by this same scan.
                        self.assertTrue(job["uses"].startswith("./.github/workflows/"),
                                        "release publishing must use a local, inspectable workflow")
                        self.assertTrue((ROOT / job["uses"]).is_file())
                        continue
                    self.assertEqual(job.get("runs-on"), "ubuntu-latest",
                                     "release environments, signing keys and publishers must stay on GitHub")

    def test_only_old_dated_nightlies_are_pruned(self):
        tags = [{"tagName": "nightly", "isPrerelease": True},
                {"tagName": "nightly-2026-10-20", "isPrerelease": True},
                {"tagName": "nightly-2026-10-21", "isPrerelease": True},
                {"tagName": "nightly-2026-10-19", "isPrerelease": False},
                {"tagName": "v0.2.80", "isPrerelease": False},
                {"tagName": "nightly-2026-10-15-extra", "isPrerelease": True}]
        self.assertEqual(nightly.stale(tags, dt.date(2026, 10, 28), 7), ["nightly-2026-10-20"])

    def test_notes_list_what_reached_main_as_a_release_did(self):
        merged = [merged_pr(1, "Fix pistons", ["feature"], issues=[7]), merged_pr(2, "Water flows", []),
                  merged_pr(3, "CI only", ["skip-release"]), merged_pr(4, "Docs", [], paths=[])]
        text = notes(merged=merged, issues=["- Known thing"])
        self.assertIn("Nightly 2026-10-28 · bbbbbbb: main (mmmmmmm), built from `main`", text)
        self.assertIn("### New\n- Fix pistons (#1, fixes #7)", text)
        self.assertIn("### Fixed\n- Water flows (#2)", text)
        self.assertNotIn("CI only", text)
        self.assertNotIn("Docs", text)
        self.assertIn("### Known issues\n- Known thing", text)
        self.assertIn("Thanks to @dev1, @dev2.", text)
        self.assertNotIn("### Not merged yet", text)
        self.assertIn("Nothing new for players on main", notes())

    def test_notes_open_with_the_way_to_the_launcher(self):
        # A raw HTML line that launchers skip, as on a game release.
        with mock.patch.object(nightly.train, "REPO", "horizon-rust/horizon"):
            self.assertTrue(notes().startswith(nightly.train.PLAYERS + "\n\nNightly 2026-10-28 · bbbbbbb: "))
            self.assertIn(nightly.PLAYERS, notes())

    def test_fork_notes_use_loaded_intro_and_keep_build_details(self):
        with mock.patch.object(nightly.train, "REPO", "sunrise-org/sunrise"):
            # An older selected checkout may not contain the template. It must already be loaded.
            with mock.patch.object(Path, "read_text", side_effect=AssertionError("late template read")):
                text = notes(merged=[merged_pr(9, "Fix fork install", [])], issues=["- Known issue"])
        self.assertTrue(text.startswith(nightly.SUNRISE_PLAYERS + "\n\n"))
        self.assertNotIn(nightly.PLAYERS, text)
        self.assertIn("Nightly 2026-10-28 · bbbbbbb: main (mmmmmmm)", text)
        self.assertIn("nightly-source/2026-10-28-bbbbbbb", text)
        self.assertIn("### Fixed\n- Fix fork install (#9)", text)
        self.assertIn("### Known issues\n- Known issue", text)
        self.assertTrue(text.endswith(nightly.train.DISCLAIMER + "\n"))

    def test_other_repositories_keep_the_upstream_intro(self):
        with mock.patch.object(nightly.train, "REPO", "another/sunrise"):
            self.assertTrue(notes().startswith(nightly.train.PLAYERS + "\n\n"))
            self.assertIn(nightly.PLAYERS, notes())

    def test_notes_name_the_commit_and_its_permanent_source_tag(self):
        sha = "0c0206714abc" + "0" * 28
        text = notes(chosen=nightly.Ref("main", sha, None))
        self.assertIn(f"Source: commit {sha}, kept for good as the tag `nightly-source/2026-10-28-0c02067`", text)

    def test_the_source_tag_outlives_every_prune(self):
        tag = nightly.source_tag("2026-10-20", "a" * 40)
        self.assertEqual(nightly.stale([{"tagName": tag, "isPrerelease": True}], dt.date(2026, 11, 28), 7), [])
        self.assertIsNone(nightly.DATED.match(tag))



class ScheduledTests(unittest.TestCase):
    def test_the_scheduled_nightly_drops_merged_closed_and_refused_ones(self):
        state = run({}, "/nightly 1 2 3 4", "kacpherek", pr(1), pr(2), pr(3), pr(4)).state
        get = lookup(pr(1, head="new"), pr(2, state="merged"), pr(3, state="closed"),
                     pr(4, files=[".github/workflows/x.yml"]))
        kept, left_out = nightly.refresh(state["prs"], get, PATTERNS)
        self.assertEqual([(e["number"], e["head"]) for e in kept], [(1, "h1")])
        self.assertEqual(len(left_out), 3)
        self.assertIn("merged", left_out[0])

    def test_the_scheduled_nightly_builds_the_head_that_was_picked(self):
        state = run({}, "/nightly 1", "kacpherek", pr(1)).state
        kept, left_out = nightly.refresh(state["prs"], lookup(pr(1, head="new")), PATTERNS)
        self.assertEqual([(e["number"], e["head"]) for e in kept], [(1, "h1")])
        self.assertEqual(left_out, [])

    def test_an_unreviewed_pull_request_stays_in_the_scheduled_set(self):
        # No review is looked at: a pull request nobody reviewed is kept as long as nothing else refuses it.
        state = run({}, "/nightly 1", "kacpherek", pr(1)).state
        kept, left_out = nightly.refresh(state["prs"], lookup(pr(1)), PATTERNS)
        self.assertEqual([e["number"] for e in kept], [1])
        self.assertEqual(left_out, [])

    def test_notes_list_each_pull_request_and_who_picked_it(self):
        prs = run({}, "/nightly 1 2", "kacpherek", pr(1), pr(2)).state["prs"]
        text = notes(chosen=nightly.Ref("nightly/build-2026-10-28-1", "b" * 40, None), picked=prs,
                     left_out=["#3 Fix 3: closed, left out"])
        self.assertIn("main (mmmmmmm) with 2 pull requests merged in", text)
        self.assertIn("### Not merged yet (2)\n- Fix 1 (#1) by dev1, at h1, picked by kacpherek", text)
        self.assertIn("### Left out (1)", text)

WORKFLOWS = Path(__file__).resolve().parents[1] / ".github" / "workflows"


class FirstNightlyTests(unittest.TestCase):
    def test_fetch_succeeds_with_or_without_an_existing_nightly_tag(self):
        class ReachedGreenBase(Exception):
            pass

        for existing in (False, True):
            with self.subTest(existing=existing):
                calls = []
                def git(*args):
                    calls.append(args)
                    if args[0] == "ls-remote":
                        return "abc\\trefs/tags/nightly" if existing else ""
                    return ""
                with mock.patch.object(nightly, "git", side_effect=git), mock.patch.object(nightly, "green_base", side_effect=ReachedGreenBase), \
                        mock.patch.dict(os.environ, {"GITHUB_REF": "refs/heads/main", "DRY_RUN": "false"}):
                    with self.assertRaises(ReachedGreenBase):
                        nightly.pick(False)
                fetched = next(args for args in calls if args[0] == "fetch")
                self.assertIn("+refs/heads/main:refs/remotes/origin/main", fetched)
                self.assertEqual("+refs/tags/nightly:refs/tags/nightly" in fetched, existing)


class UntrustedBuildTests(unittest.TestCase):
    """A nightly can hold pull requests nobody reviewed: nothing its build leaves may reach a release (game-build.yml)."""

    def setUp(self):
        self.game = (WORKFLOWS / "game-build.yml").read_text()
        self.exporters = (WORKFLOWS / "exporters.yml").read_text()

    def test_no_nightly_build_is_trusted(self):
        # The train only starts `nightly build` (Nightly only since 2026-10-28); it builds nothing itself.
        release = (WORKFLOWS / "release.yml").read_text()
        self.assertNotIn("/game-build.yml", release)
        self.assertIn("gh workflow run nightly-build.yml", release)
        nightly_build = (WORKFLOWS / "nightly-build.yml").read_text()
        self.assertIn("trusted: false", nightly_build)
        self.assertNotIn("trusted: true", nightly_build)
        self.assertIn("default: false", self.game.split("trusted:", 1)[1].split("developer_id:", 1)[0])

    def test_an_untrusted_build_neither_shares_nor_saves_the_release_cache(self):
        self.assertIn("shared-key: ${{ inputs.trusted && 'release' || 'nightly' }}-${{ matrix.name }}", self.game)
        self.assertIn("save-if: ${{ inputs.trusted }}", self.game)
        self.assertEqual(self.game.count("Swatinem/rust-cache"), 1)

    def test_no_build_job_names_a_self_hosted_mac(self):
        # Sunrise has no Mac runner of its own: the Macs build on Blacksmith's (or GitHub's) machines, and exporters
        # run on Dyna's runner for an untrusted build only, never sharing a trusted build with pull requests' jobs.
        self.assertNotIn("MAC_RUNNER", self.game)
        self.assertNotIn('"macOS"', self.game)
        self.assertNotIn('"macOS"', self.exporters)
        exporters = next(line for line in self.exporters.splitlines() if "runs-on:" in line and "NIGHTLY_EXPORTERS_RUNNER" in line)
        self.assertIn("!inputs.trusted && vars.DYNA_CI_SELF_HOSTED == 'true' && vars.NIGHTLY_EXPORTERS_RUNNER", exporters)

    def test_a_trusted_build_never_reads_the_shared_sccache(self):
        both = self.game + self.exporters
        calls = both.count("bash tools/ci/runner-env.sh")
        self.assertEqual(both.count("SUNRISE_NO_SCCACHE: ${{ inputs.trusted && '1' || '' }}"), calls)
        env = (Path(__file__).resolve().parents[1] / "tools/ci/runner-env.sh").read_text()
        self.assertEqual(env.count('-z "${SUNRISE_NO_SCCACHE:-}"'), 2)


class PruneBuildsTests(unittest.TestCase):
    TODAY = dt.date(2026, 11, 10)

    def test_only_old_unpublished_builds_go(self):
        branches = {
            "nightly/build-2026-10-27-1": "a",   # old: goes
            "nightly/build-2026-10-27-2": "pub",  # old, but a published nightly-<date> points to it: stays
            "nightly/build-2026-11-02-1": "c",   # old enough by one day: goes
            "nightly/build-2026-11-03-1": "d",   # exactly 7 days: stays
            "nightly/build-2026-11-09-3": "e",   # recent: stays
        }
        self.assertEqual(nightly.stale_builds(branches, {"pub"}, self.TODAY, 7),
                         ["nightly/build-2026-10-27-1", "nightly/build-2026-11-02-1"])

    def test_the_newest_build_always_stays(self):
        branches = {"nightly/build-2026-10-01-1": "a", "nightly/build-2026-10-01-2": "b"}
        self.assertEqual(nightly.stale_builds(branches, set(), self.TODAY, 7), ["nightly/build-2026-10-01-1"])
        self.assertEqual(nightly.stale_builds({"nightly/build-2026-10-01-1": "a"}, set(), self.TODAY, 7), [])
        # the newest by number, not by text: -10 is newer than -9
        branches = {"nightly/build-2026-10-01-9": "a", "nightly/build-2026-10-01-10": "b"}
        self.assertEqual(nightly.stale_builds(branches, set(), self.TODAY, 7), ["nightly/build-2026-10-01-9"])

    def test_nothing_else_is_touched(self):
        branches = {"nightly-state": "s", "nightly/build-x": "x", "main": "m", "nightly/build-2026-10-01-1-fix": "f",
                    "feature/nightly/build-2026-10-01-1": "g", "nightly/build-2026-10-02-1": "n"}
        self.assertEqual(nightly.stale_builds(branches, set(), self.TODAY, 7), [])

    def test_the_source_is_archived_from_git_signed_attached_and_tagged(self):
        build = (WORKFLOWS / "nightly-build.yml").read_text()
        pick = build.split("\n  build:", 1)[0]
        self.assertIn('git archive --format=tar.gz --prefix="sunrise-${SHA:0:7}/" -o sunrise-source.tar.gz "$SHA"',
                      pick)
        sign = build.split("\n  sign:", 1)[1].split("\n  publish:", 1)[0]
        self.assertIn("dist/sunrise-source.tar.gz", sign.split('sign_release.sh "$SIGN_FOR"', 1)[1])
        publish = build.split("\n  publish:", 1)[1].split("\n  answer:", 1)[0]
        self.assertIn('"dist/sunrise-source.tar.gz#Source of this Nightly', publish)
        self.assertIn('nightly.py keep-source "$SHA" "$DATE"', publish)

    def test_the_workflow_prunes_on_real_runs_only(self):
        build = (WORKFLOWS / "nightly-build.yml").read_text()
        step = build.split("Old nightly/build-* branches deleted", 1)[1].split("- uses:", 1)[0]
        self.assertIn("if: steps.mode.outputs.publish == 'true'", step)
        self.assertIn("prune-builds --days 7", step)
        self.assertIn("GITHUB_STEP_SUMMARY", step)



class SdkPairTests(unittest.TestCase):
    def test_only_complete_published_exact_pair_is_eligible(self):
        # Written out, not taken from the module: the release a Nightly needs, signed index and sums included.
        assets = [{"name": name} for name in (
            "sdk.json", "SHA256SUMS", "SHA256SUMS.sig", "api-source.tar.xz", "build-data.tar.xz", "surface.json",
            *(f"{kind}-{leg}.tar.xz" for kind in ("runtime", "api", "engine") for leg in (
                "x86_64-unknown-linux-gnu", "x86_64-unknown-linux-gnu-server", "x86_64-pc-windows-gnu",
                "aarch64-apple-darwin")),
            *(f"tool-{target}.tar.xz" for target in (
                "x86_64-unknown-linux-gnu", "x86_64-pc-windows-gnu", "aarch64-apple-darwin")))]
        release = {"draft": False, "tag_name": "sdk-" + "a" * 40, "assets": assets}
        for changes, expected in [({}, True), ({"draft": True}, False),
                                  ({"tag_name": "sdk-" + "b" * 40}, False),
                                  # The old prebuilt-<sha> release is not an SDK.
                                  ({"tag_name": "prebuilt-" + "a" * 40}, False)]:
            with mock.patch.object(nightly.train, "gh_json", return_value=[[], [{**release, **changes}]]):
                self.assertEqual(nightly.sdk_ready("a" * 40), expected)
        for index in range(len(assets)):
            with mock.patch.object(nightly.train, "gh_json", return_value=[[{
                    **release, "assets": assets[:index] + assets[index + 1:]}]]):
                self.assertFalse(nightly.sdk_ready("a" * 40))
        with mock.patch.object(nightly.train, "gh_json", return_value=[[]]):
            self.assertFalse(nightly.sdk_ready("a" * 40))
        with mock.patch.object(nightly.train, "gh_json", side_effect=SystemExit("API failed")):
            with self.assertRaises(SystemExit):
                nightly.sdk_ready("a" * 40)

    def test_nightly_selection_reuses_the_main_push_channel_gate(self):
        for explicit in ("", "a" * 40):
            with self.subTest(explicit=explicit), \
                    mock.patch.dict(nightly.os.environ, {"NIGHTLY_BASE": explicit}), \
                    mock.patch.object(nightly.train, "channel_head", return_value="a" * 40) as gate, \
                    mock.patch.object(nightly, "git", return_value=NOW), \
                    mock.patch.object(nightly, "sdk_ready", side_effect=AssertionError("build_set checks the SDK")):
                self.assertEqual(nightly.green_base("sunrise-org/sunrise", "").sha, "a" * 40)
                gate.assert_called_once_with("nightly", explicit, "", "")
        with mock.patch.object(nightly.train, "channel_head", side_effect=SystemExit("gate failed")):
            with self.assertRaisesRegex(SystemExit, "gate failed"):
                nightly.green_base("sunrise-org/sunrise", "")

    def test_final_sha_change_stops_before_pushing(self):
        with mock.patch.dict(nightly.os.environ, {"GITHUB_REPOSITORY": "sunrise-org/sunrise"}), \
                mock.patch.object(nightly.ns, "fetch_heads", return_value=[]), \
                mock.patch.object(nightly.ns, "assemble", return_value="b" * 40), \
                mock.patch.object(nightly.ns, "push") as push:
            with self.assertRaises(SystemExit):
                nightly.build_set(nightly.Ref("main", "a" * 40, T), [], [], "2026-11-11", True, False)
            push.assert_not_called()

    def test_a_real_nightly_without_its_sdk_stops_before_pushing(self):
        for ready, dry_run in ((False, False), (False, True), (True, False)):
            with self.subTest(ready=ready, dry_run=dry_run), \
                    mock.patch.dict(nightly.os.environ, {"GITHUB_REPOSITORY": "sunrise-org/sunrise",
                                                         "GITHUB_RUN_ID": "1", "GITHUB_RUN_ATTEMPT": "1"}), \
                    mock.patch.object(nightly.ns, "fetch_heads", return_value=[]), \
                    mock.patch.object(nightly.ns, "assemble", return_value="a" * 40), \
                    mock.patch.object(nightly.ns, "builds_today", return_value=[]), \
                    mock.patch.object(nightly.ns, "next_build", return_value="nightly/build-2026-11-11-1"), \
                    mock.patch.object(nightly.ns, "update_state"), \
                    mock.patch.object(nightly.ns, "push", return_value=True) as push, \
                    mock.patch.object(nightly, "sdk_ready", return_value=ready), \
                    mock.patch.object(nightly, "git", return_value="b" * 40), \
                    mock.patch.object(nightly, "merged_since", return_value=[]), \
                    mock.patch.object(nightly.train, "known_issues", return_value=[]), \
                    mock.patch("builtins.open", mock.mock_open()), \
                    mock.patch.object(nightly, "output") as output:
                if not ready and not dry_run:
                    with self.assertRaises(SystemExit):
                        nightly.build_set(nightly.Ref("main", "a" * 40, T), [], [], "2026-11-11", False, dry_run)
                    push.assert_not_called()
                    output.assert_not_called()
                else:
                    nightly.build_set(nightly.Ref("main", "a" * 40, T), [], [], "2026-11-11", False, dry_run)
                    push.assert_called_once()
                    self.assertEqual(output.call_args.kwargs["sdk"], str(ready).lower())

    def test_forced_rebuild_of_the_published_commit_lists_the_changes_since_the_nightly_before(self):
        with tempfile.TemporaryDirectory() as repo:
            sh = git_in(repo)
            sh("init", "-q")
            commits = []
            for n in range(3):
                sh("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", str(n))
                commits.append(sh("rev-parse", "HEAD"))
            first, second, published = commits
            sh("remote", "add", "origin", repo)
            sh("tag", "nightly-source/2026-11-15-aaaaaaa", first)
            sh("tag", "nightly-source/2026-11-16-bbbbbbb", second)
            sh("tag", "nightly-source/2026-11-17-ccccccc", published)
            sh("tag", "nightly", published)
            cwd = os.getcwd()
            os.chdir(repo)
            try:
                with mock.patch.dict(nightly.os.environ, {"GITHUB_REPOSITORY": "sunrise-org/sunrise"}), \
                        mock.patch.object(nightly.ns, "fetch_heads", return_value=[]), \
                        mock.patch.object(nightly.ns, "assemble", return_value=published), \
                        mock.patch.object(nightly.ns, "builds_today", return_value=["nightly/build-2026-11-17-1"]), \
                        mock.patch.object(nightly.ns, "push", return_value=True), \
                        mock.patch.object(nightly.ns, "update_state"), \
                        mock.patch.object(nightly, "sdk_ready", return_value=True), \
                        mock.patch.object(nightly, "REPOSITORY", repo), \
                        mock.patch.object(nightly, "merged_since", return_value=[]) as since, \
                        mock.patch.object(nightly.train, "known_issues", return_value=[]), \
                        mock.patch("builtins.open", return_value=mock.MagicMock(__enter__=lambda _: io.StringIO())), \
                        mock.patch.object(nightly, "output"):
                    nightly.build_set(nightly.Ref("main", published, T), [], [], "2026-11-17", True, False)
            finally:
                os.chdir(cwd)
        since.assert_called_once_with(second, published)

    def test_publication_checks_fresh_inventory_and_leaves_upstream_unchanged(self):
        with mock.patch.dict(nightly.os.environ, {"GITHUB_REPOSITORY": "sunrise-org/sunrise"}), \
                mock.patch.object(nightly, "ready_sdk_shas", side_effect=[{"a" * 40}, set()]) as inventory, \
                mock.patch.object(nightly, "published_index") as index:
            nightly.require_sdk("a" * 40)
            index.assert_called_once_with("a" * 40)
            with self.assertRaisesRegex(SystemExit, "publication refused"):
                nightly.require_sdk("a" * 40)
            self.assertEqual(inventory.call_count, 2)
        with mock.patch.dict(nightly.os.environ, {"GITHUB_REPOSITORY": "horizon-rust/horizon"}), \
                mock.patch.object(nightly, "sdk_ready") as sdk:
            nightly.require_sdk("a" * 40)
            sdk.assert_not_called()

    def test_the_sdk_index_must_name_this_build_format_and_assets(self):
        sha = "a" * 40
        good = {"build": {"commit": sha}, "package_format": nightly.package_format(),
                "assets": {name: {} for name in nightly.SDK_ARCHIVES}}
        self.assertEqual(nightly.checked_index(sha, json.dumps(good).encode()), good)
        for why, document in (
                ("for build", {**good, "build": {"commit": "b" * 40}}),
                ("package format", {**good, "package_format": 0}),
                ("does not list surface.json", {**good, "assets": {name: {} for name in nightly.SDK_ARCHIVES[:-1]}}),
                ("not an SDK index", {"build": {}})):
            with self.subTest(why), self.assertRaisesRegex(SystemExit, why):
                nightly.checked_index(sha, json.dumps(document).encode())

    def test_the_previous_sdk_is_the_nearest_published_one_behind_the_build(self):
        a, b, c, d = ("a" * 40, "b" * 40, "c" * 40, "d" * 40)
        # c is not on d's history; b is nearer than a.
        with mock.patch.object(nightly, "ready_sdk_shas", return_value={a, b, c, d}), \
                mock.patch.object(nightly, "is_ancestor", side_effect=lambda x, y: x in (a, b)), \
                mock.patch.object(nightly, "git", side_effect=lambda *args: {f"{a}..{d}": "9", f"{b}..{d}": "2"}[args[-1]]):
            self.assertEqual(nightly.previous_sdk(d), b)
        with mock.patch.object(nightly, "ready_sdk_shas", return_value={d}):
            self.assertEqual(nightly.previous_sdk(d), "")

    def test_fork_pick_refuses_explicit_prs_ignores_saved_prs_and_upstream_keeps_both(self):
        main = nightly.Ref("main", "a" * 40, T)
        saved = [{"number": 17, "head": "b" * 40}]
        explicit = [{"number": 19, "head": "c" * 40}]
        for repo in ("sunrise-org/sunrise", "horizon-rust/horizon"):
            for spec in ("", "19"):
                with self.subTest(repo=repo, spec=spec), \
                        mock.patch.dict(nightly.os.environ, {"GITHUB_REPOSITORY": repo, "NIGHTLY_PRS": spec,
                                                            "GITHUB_EVENT_NAME": "workflow_dispatch", "DRY_RUN": "true"}), \
                        mock.patch.object(nightly, "git", return_value=""), \
                        mock.patch.object(nightly, "green_base", return_value=main), \
                        mock.patch.object(nightly.ns, "load_state", return_value=({"prs": saved}, "", "")) as state, \
                        mock.patch.object(nightly.ns, "lookup"), \
                        mock.patch.object(nightly, "requested", return_value=explicit) as requested, \
                        mock.patch.object(nightly, "build_set") as build:
                    if repo == "sunrise-org/sunrise" and spec:
                        with self.assertRaises(SystemExit):
                            nightly.pick(False)
                        build.assert_not_called()
                        requested.assert_not_called()
                        state.assert_not_called()
                    else:
                        nightly.pick(False)
                        expected = [] if repo == "sunrise-org/sunrise" else explicit if spec else saved
                        self.assertEqual(build.call_args.args[:3], (main, expected, []))
                        if repo == "sunrise-org/sunrise":
                            state.assert_not_called()
                            requested.assert_not_called()


if __name__ == "__main__":
    unittest.main()


@unittest.skipUnless(shutil.which("openssl"), "needs the runner's OpenSSL")
class VerifySumsTests(unittest.TestCase):
    """verify_sums: a release folder checked as launchers check it, with OpenSSL and no code of the checkout."""

    def setUp(self):
        self.work = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.key, self.public = self.keypair("release")
        _, self.other = self.keypair("other")

    def keypair(self, name):
        private = self.work / f"{name}.pem"
        subprocess.run(["openssl", "genpkey", "-algorithm", "ed25519", "-out", str(private)], check=True,
                       capture_output=True)
        der = subprocess.run(["openssl", "pkey", "-in", str(private), "-pubout", "-outform", "DER"], check=True,
                             capture_output=True).stdout
        return private, base64.b64encode(der[-32:]).decode()

    def signature(self, tag, sums, key=None):
        """The base64 signature of `sums` for `tag`, as sign_release.sh writes it."""
        message = self.work / "message"
        message.write_bytes(f"horizon release {tag}\n".encode() + sums)
        signature = subprocess.run(["openssl", "pkeyutl", "-sign", "-inkey", str(key or self.key), "-rawin", "-in",
                                    str(message)], check=True, capture_output=True).stdout
        return base64.b64encode(signature).decode()

    def release(self, tag, files, signed_for=None, key=None, sums=None):
        """A folder with `files`, their SHA256SUMS (or `sums`), and SHA256SUMS.sig signed for `signed_for` (default
        `tag`)."""
        folder = self.work / f"release-{len(list(self.work.glob('release-*')))}"
        folder.mkdir()
        for name, data in files.items():
            (folder / name).write_bytes(data)
        if sums is None:
            sums = "".join(f"{hashlib.sha256(data).hexdigest()}  {name}\n" for name, data in files.items()).encode()
        (folder / "SHA256SUMS").write_bytes(sums)
        (folder / "SHA256SUMS.sig").write_text(self.signature(signed_for or tag, sums, key) + "\n")
        return folder

    def verify(self, folder, tag, **kwargs):
        with contextlib.redirect_stdout(io.StringIO()) as out, contextlib.redirect_stderr(io.StringIO()):
            nightly.verify_sums(folder, tag, **{"keys": [self.public], **kwargs})
        self.assertEqual(out.getvalue(), "", "stdout is its callers' result")

    def test_a_folder_signed_for_its_tag_by_a_release_key_passes(self):
        files = {"sdk.json": b"index", "surface.json": b"surface"}
        folder = self.release("26.3-0.1.0", files)
        self.verify(folder, "26.3-0.1.0")
        self.verify(folder, "26.3-0.1.0", only=["sdk.json"])
        self.verify(folder, "26.3-0.1.0", keys=[self.other, self.public])

    def test_a_tampered_folder_another_tag_or_another_key_refuses(self):
        files = {"sdk.json": b"index", "surface.json": b"surface"}
        folder = self.release("26.3-0.1.0", files)
        cases = (("26.3-0.1.1", {}, "isn't signed for `horizon release 26.3-0.1.1`"),
                 ("26.3-0.1.0", {"keys": [self.other]}, "isn't signed"),
                 ("26.3-0.1.0", {"only": ["missing.json"]}, "missing.json is not in 26.3-0.1.0's SHA256SUMS"),
                 ("26.3-0.1.0", {"keys": []}, "no key to check the signature with"),
                 ("26.3-0.1.0", {"signature": "SHA256SUMS.v2.sig"}, "has no SHA256SUMS.v2.sig"))
        for tag, kwargs, error in cases:
            with self.subTest(error=error), self.assertRaisesRegex(SystemExit, error):
                self.verify(folder, tag, **kwargs)
        # A file changed after signing: the signed list holds, its bytes don't.
        (folder / "surface.json").write_bytes(b"tampered")
        with self.assertRaisesRegex(SystemExit, "26.3-0.1.0's surface.json does not match its SHA256SUMS"):
            self.verify(folder, "26.3-0.1.0")
        self.verify(folder, "26.3-0.1.0", only=["sdk.json"])
        # A list rewritten after signing: the signature no longer covers it.
        sums = (folder / "SHA256SUMS").read_text().replace(hashlib.sha256(b"surface").hexdigest(),
                                                           hashlib.sha256(b"tampered").hexdigest())
        (folder / "SHA256SUMS").write_text(sums)
        with self.assertRaisesRegex(SystemExit, "isn't signed"):
            self.verify(folder, "26.3-0.1.0")

    def test_checksum_rows_are_read_as_the_native_verifier_reads_them(self):
        # Every list here is signed: only its format decides.
        a = hashlib.sha256(b"a").hexdigest()
        for sums in (f"{a}  a.tar.gz\n", f"{a} *a.tar.gz\n", f"{a} a.tar.gz\n", f"{a.upper()}  a.tar.gz\r\n",
                     f"\n{a}  a.tar.gz\n  \n"):
            with self.subTest(sums=sums):
                self.verify(self.release("26.3-0.1.0", {"a.tar.gz": b"a"}, sums=sums.encode()), "26.3-0.1.0")
        for sums, error in ((b"not a row\n", "row 'not a row' is not"),
                            (f"{a[1:]}  a.tar.gz\n".encode(), "is not `<sha256>  <file>`"),
                            (f"{a}\ta.tar.gz\n".encode(), "is not `<sha256>  <file>`"),
                            (f"{a}  \n".encode(), "is not `<sha256>  <file>`"),
                            (f"{a}  ../a.tar.gz\n".encode(), "row .*\\.\\./a.tar.gz.* is not"),
                            (f"{a}  sub\\a.tar.gz\n".encode(), "is not `<sha256>  <file>`"),
                            (f"{a}  ..\n".encode(), "is not `<sha256>  <file>`"),
                            (f"{a}  a\xff.tar.gz\n".encode("latin-1"), "is not UTF-8 text"),
                            # One marker is stripped, not two: the file ` a.tar.gz` is not in the folder.
                            (f"{a}   a.tar.gz\n".encode(), "No such file"),
                            # A bare carriage return ends no line: here it is part of the file's name.
                            (f"{a}  a.tar.gz\r".encode(), "No such file"),
                            (f"\n \n".encode(), "lists no file"),
                            # Rust's trim keeps U+001F, so this line is a row, and no row.
                            (f"{a}  a.tar.gz\n\x1f\n".encode(), "is not `<sha256>  <file>`"),
                            # A repeated name is checked against each of its rows.
                            (f"{a}  a.tar.gz\n{'0' * 64}  a.tar.gz\n".encode(), "a.tar.gz does not match")):
            with self.subTest(sums=sums), self.assertRaisesRegex((SystemExit, FileNotFoundError), error):
                self.verify(self.release("26.3-0.1.0", {"a.tar.gz": b"a"}, sums=sums), "26.3-0.1.0")

    def test_one_signature_line_that_verifies_is_enough_as_for_the_native_verifier(self):
        folder = self.release("26.3-0.1.0", {"a.tar.gz": b"a"})
        good = (folder / "SHA256SUMS.sig").read_text().strip()
        other = self.signature("26.3-0.1.1", (folder / "SHA256SUMS").read_bytes())
        for text in (f"not-base64!\n{good}\n", f"{other}\n{good}\n", f"  {good}\r\n", f"{good}"):
            with self.subTest(text=text):
                (folder / "SHA256SUMS.sig").write_text(text)
                self.verify(folder, "26.3-0.1.0")
        # Two signatures on one line, or the good one with the unused bits of its last digit set (the same bytes,
        # which the native verifier's strict decoder refuses), are not signed.
        alphabet = string.ascii_uppercase + string.ascii_lowercase + string.digits + "+/"
        loose = good[:-3] + alphabet[alphabet.index(good[-3]) ^ 1] + good[-2:]
        self.assertEqual(base64.b64decode(loose), base64.b64decode(good))
        for text in (f"{other} {good}\n", f"{loose}\n", "not-base64!\n", f"{other}\r{good}\n", f"\x1c{good}\n"):
            with self.subTest(text=text), self.assertRaisesRegex(SystemExit, "isn't signed"):
                (folder / "SHA256SUMS.sig").write_text(text)
                self.verify(folder, "26.3-0.1.0")
        for name in ("../SHA256SUMS.sig", ".", "..", ""):
            with self.subTest(signature=name), self.assertRaisesRegex(SystemExit, "names a file of the folder"):
                self.verify(folder, "26.3-0.1.0", signature=name)

    def test_sdk_previous_prints_only_the_build_it_found(self):
        # sdk-publish.yml reads its stdout as one SHA: the verifier's own report goes to stderr. The release is a
        # real signed one, checked by the real verifier; only the download and main's keys are stubbed.
        build = "b" * 40
        published = self.release(f"sdk-{build}", {nightly.SDK_SURFACE: b'{"schema": 3}'})

        def download(*args):
            self.assertEqual(args[:3], ("release", "download", f"sdk-{build}"))
            directory = Path(args[args.index("--dir") + 1])
            for path in published.iterdir():
                shutil.copy(path, directory / path.name)
        with mock.patch.object(nightly, "previous_sdk", return_value=build), \
                mock.patch.object(nightly, "gh", side_effect=download), \
                mock.patch.object(nightly, "release_keys", return_value=[self.public]), \
                mock.patch("sys.argv", ["nightly.py", "sdk-previous", "c" * 40, str(self.work / "previous.json")]), \
                contextlib.redirect_stdout(io.StringIO()) as out, contextlib.redirect_stderr(io.StringIO()):
            nightly.main()
        self.assertEqual(out.getvalue(), build + "\n")
        self.assertEqual((self.work / "previous.json").read_bytes(), b'{"schema": 3}')

    def test_publishing_before_any_key_warns_and_unsigned_never_passes_once_one_is_listed(self):
        folder = self.release("nightly", {"a.tar.gz": b"a"})
        (folder / "SHA256SUMS.sig").unlink()
        with contextlib.redirect_stderr(io.StringIO()) as out:
            nightly.verify_sums(folder, "nightly", keys=[], publishing=True)
        self.assertIn("::warning::nightly: published with SHA256SUMS but unsigned", out.getvalue())
        with self.assertRaisesRegex(SystemExit, "has no SHA256SUMS.sig"):
            self.verify(folder, "nightly", publishing=True)
        # Only Sunrise's own releases: a fork has no keys either, and publishes nothing unchecked, signed or not.
        signed = self.release("nightly", {"a.tar.gz": b"a"})
        for release in (folder, signed):
            with self.subTest(release=release.name), \
                    mock.patch.dict(nightly.os.environ, {"GITHUB_REPOSITORY": "someone/sunrise"}), \
                    self.assertRaisesRegex(SystemExit, "has no SHA256SUMS.sig|no key to check"):
                nightly.verify_sums(release, "nightly", keys=[], publishing=True)
        with mock.patch.dict(nightly.os.environ, {"GITHUB_REPOSITORY": "sunrise-org/sunrise"}), \
                contextlib.redirect_stderr(io.StringIO()) as out:
            nightly.verify_sums(signed, "nightly", keys=[], publishing=True)
        self.assertIn("signed, but release-keys.txt lists no key yet", out.getvalue())

    def test_the_keys_are_mains_never_the_checkouts(self):
        origin, clone = self.work / "origin", self.work / "clone"
        env = isolated_git_env()
        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], env=env, check=True)
        subprocess.run(["git", "clone", "-q", str(origin), str(clone)], env=env, check=True, capture_output=True)
        keys = clone / nightly.KEYS_FILE
        keys.parent.mkdir(parents=True)
        keys.write_text(f"# main's\n{self.public}\n")
        for command in (["add", "-A"], ["-c", "user.name=T", "-c", "user.email=t@example.invalid", "commit", "-q", "-m",
                                        "keys"], ["push", "-q", "origin", "main"]):
            subprocess.run(["git", *command], cwd=clone, env=env, check=True)
        keys.write_text(f"{self.public}\n{self.other}\n")  # a candidate's own copy adds a key
        with mock.patch.object(nightly, "REPOSITORY", clone), \
                mock.patch.dict(os.environ, {"GITHUB_REPOSITORY": "sunrise-org/sunrise"}):
            self.assertEqual(nightly.release_keys(), [self.public])
        with mock.patch.dict(os.environ, {"GITHUB_REPOSITORY": "someone/fork"}):
            self.assertEqual(nightly.release_keys(), [], "another repository's releases are trusted by no key")
        # A line the native verifier can't read makes launchers trust no key at all: refused, never skipped. Its
        # whitespace and comments are read as it reads them.
        alphabet = string.ascii_uppercase + string.ascii_lowercase + string.digits + "+/"
        loose = self.public[:-2] + alphabet[alphabet.index(self.public[-2]) ^ 1] + "="  # an unused bit set
        for text, keys in ((f" {self.public}\u3000\r\n\n# a comment\n", [self.public]),
                           (f"{self.public}\nnot-a-key\n", None), (f"{self.public[:-4]}\n", None),
                           (f"{base64.b64encode(bytes(31)).decode()}\n", None), (f"{loose}\n", None),
                           (f"\x1c{self.public}\n", None),
                           # A bare carriage return ends no line for the native reader: one line, and no key.
                           (f"{self.public}\r{self.public}\n", None)):
            subprocess.run(["git", "checkout", "-q", "main"], cwd=clone, env=env, check=True)
            keys_file = clone / nightly.KEYS_FILE
            keys_file.write_text(text)
            for command in (["add", "-A"], ["-c", "user.name=T", "-c", "user.email=t@example.invalid", "commit",
                                            "-q", "--allow-empty", "-m", "keys"], ["push", "-q", "origin", "main"]):
                subprocess.run(["git", *command], cwd=clone, env=env, check=True)
            with self.subTest(text=text), mock.patch.object(nightly, "REPOSITORY", clone), \
                    mock.patch.dict(os.environ, {"GITHUB_REPOSITORY": "sunrise-org/sunrise"}):
                if keys:
                    self.assertEqual(nightly.release_keys(), keys)
                else:
                    with self.assertRaisesRegex(SystemExit, "is no Ed25519 public key"):
                        nightly.release_keys()


class SdkBodyTests(unittest.TestCase):
    def index(self, previous="p" * 40, changes=(), notes=(), source="breaking"):
        surface = {"hash": "h"}
        if previous:
            surface.update(previous=previous, source=source, changes=list(changes), notes=list(notes))
        return {"build": {"commit": "c" * 40}, "surface": surface}

    def test_notes_naming_the_earliest_change_come_first(self):
        notes = [{"ids": ["item:b"], "why": "WHY-LATE"},
                 {"ids": ["item:z", "hook:a"], "why": "WHY-BREAKING", "migrate": "use the new hook"}]
        body = nightly.sdk_body(self.index(changes=["hook:a", "item:b", "item:z"], notes=notes))
        self.assertLess(body.index("WHY-BREAKING"), body.index("WHY-LATE"))
        self.assertIn("Migrate: use the new hook", body)
        self.assertIn("`hook:a`", body)
        self.assertIn("Changes since sdk-" + "p" * 40, body)
        self.assertIn("breaking", body)

    def test_no_previous_says_there_is_no_comparison(self):
        body = nightly.sdk_body(self.index(previous=None))
        self.assertIn("c" * 40, body)
        self.assertIn("no comparison", body)
        self.assertNotIn("Changes since", body)

    def test_the_body_stops_at_a_whole_note_under_the_cap(self):
        notes = [{"ids": [f"item:{n}"], "why": f"NOTE-{n} " + "x" * 100} for n in range(10)]
        changes = [f"item:{n}" for n in range(10)]
        with mock.patch.object(nightly, "GITHUB_RELEASE_BODY_LIMIT", 800):
            body = nightly.sdk_body(self.index(changes=changes, notes=notes))
        self.assertLessEqual(len(body), 800)
        self.assertIn("NOTE-0 ", body)
        self.assertNotIn("NOTE-9 ", body)
        shown = body.count("NOTE-")
        self.assertIn(f"The remaining notes ({10 - shown}) are in sdk.json's `surface.notes`", body)
        for n in range(shown):
            self.assertIn("x" * 100, body.split(f"NOTE-{n} ")[1].split("\n")[0])

    def test_changes_no_note_names_are_counted_not_listed(self):
        changes = [f"item:{n}" for n in range(300)]
        notes = [{"ids": ["item:0"], "why": "WHY"}]
        body = nightly.sdk_body(self.index(changes=changes, notes=notes))
        self.assertIn("299 changed records", body)
        self.assertIn("cargo sunrise-mod sdk diff", body)
        self.assertNotIn("item:1`", body)
        self.assertNotIn("item:299", body)
