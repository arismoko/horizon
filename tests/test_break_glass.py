"""Break-glass stable patch releases (docs/release-flow.md "Break-glass patch releases"): the legs a patch's changes
select, the rules never waived, the two approvals, and how the workflows hold to them."""
import contextlib
import io
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tests import test_daily_summary as daily
from tests.git_fixture import git_in, isolated_git_env
from tests.test_release_branches import RELEASE, REPO, daily_run, daily_runs, repository, workflow
from tools import release_train as train
from tools.ci import daily_summary as summary
from tools.ci import nightly

PUBLISHED = train.github_time("2026-11-01T00:00:00Z")

ALWAYS = sorted(train.BREAK_GLASS_ALWAYS)
CRATES = ["cargo-sunrise-mod", "parity", "shards", "windows-runtime-leases"]
PARENT_TAG = "26.3-0.1.0"


def write_commit(folder, files):
    """A commit writing `files` ({path: text}) on the current one (a new repository's first, when it has none)."""
    repo = Path(folder)
    git = git_in(repo)
    if not (repo / ".git").exists():
        git("init", "-q", "-b", "release")
        git("config", "user.name", "Test")
        git("config", "user.email", "test@example.invalid")
    for path, text in files.items():
        (repo / path).parent.mkdir(parents=True, exist_ok=True)
        (repo / path).write_text(text)
        git("add", "--", path)
    git("commit", "-q", "--allow-empty", "-m", "change")
    return git("rev-parse", "HEAD")


def history(folder, *changes):
    """26.3-0.1.0 at `parent`, a cherry-pick above it writing each (path, kind): a file, or a symlink."""
    repo = Path(folder)
    git = git_in(repo)
    git("init", "-q", "-b", "release")
    git("config", "user.name", "Test")
    git("config", "user.email", "test@example.invalid")
    (repo / "docs").mkdir()
    (repo / "docs" / "modding").mkdir()
    (repo / "docs" / "modding" / "a.md").write_text("book\n")
    git("add", "-A")
    git("commit", "-q", "-m", "0.1.0")
    parent = git("rev-parse", "HEAD")
    # Each entry goes straight into the index, with nothing on disk: a symbolic link is the entry git stores for one
    # (mode 120000, its target as content), and a file name may hold a line break no Windows file system allows,
    # which Git for Windows takes into the index only with core.protectNTFS off.
    for path, kind in changes:
        mode, content = ("120000", "a.md") if kind == "link" else ("100644", "fix\n")
        blob = subprocess.run(["git", "hash-object", "-w", "--stdin"], input=content, cwd=repo, text=True,
                              env=isolated_git_env(), capture_output=True, check=True).stdout.strip()
        git("-c", "core.protectNTFS=false", "update-index", "--add", "--cacheinfo", f"{mode},{blob},{path}")
    git("commit", "-q", "--allow-empty", "-m", "cherry-pick")
    return parent, git("rev-parse", "HEAD")


class LegsTests(unittest.TestCase):
    def test_each_path_selects_its_legs_and_an_unmapped_path_needs_the_full_daily(self):
        for paths, legs in (
                (["docs/modding/src/quickstart.md"], ALWAYS + ["book-quickstart"]),
                (["docs/modding"], ALWAYS + ["book-quickstart"]),  # the folder replaced by a file
                (["tools/book_quickstart.py"], ALWAYS + ["book-quickstart"]),
                (["docs/hosting.md"], ALWAYS),
                (["docs"], ALWAYS),
                (["crates/sunrise-world/src/lib.rs", "Cargo.lock"], ALWAYS + CRATES),
                # World generation: its parity harness runs before the release, never only after.
                (["crates/sunrise-world/src/worldgen/noise.rs"], ALWAYS + CRATES),
                (["Cargo.toml"], ALWAYS + CRATES),
                ([], ALWAYS)):
            with self.subTest(paths=paths):
                self.assertEqual(train.break_glass_legs(paths), (sorted(legs), []))
        for path in (".github/workflows/daily.yml", "tools/ci/nightly.py", "packaging/server/install.sh",
                     "cratesx/a.rs", "tools/book_quickstart.py.bak", "README.md"):
            with self.subTest(path=path):
                self.assertEqual(train.break_glass_legs(["docs/hosting.md", path])[1], [path])

    def test_the_code_that_judges_a_release_and_its_manifest_redirects_need_the_full_daily(self):
        # The judge's own crates, what they build from (a path dependency, one through [workspace.dependencies], a
        # crate [patch] puts in place of a registry one, the crates.io versions Cargo.lock gives them) and a change to
        # [patch] or to a workspace dependency they take: none may ride a break-glass release. A dev-dependency is not
        # built into the judge, and a game crate, a workspace dependency or a locked package outside it is an
        # ordinary change.
        def root(serde="1.0", other="2"):
            return (f'[workspace.dependencies]\nshared = {{ path = "crates/shared" }}\nserde = "{serde}"\n'
                    f'other = "{other}"\n[patch.crates-io]\nsyn = {{ path = "third_party/syn" }}\n')

        def lock(serde="1.0.1", rand="0.8.0"):
            return ('version = 4\n[[package]]\nname = "sunrise-mod-api-gen"\nversion = "0.1.0"\n'
                    'dependencies = ["serde", "shared"]\n[[package]]\nname = "shared"\nversion = "0.1.0"\n'
                    f'[[package]]\nname = "serde"\nversion = "{serde}"\nsource = "registry+x"\n'
                    f'[[package]]\nname = "rand"\nversion = "{rand}"\nsource = "registry+x"\n')

        files = {"Cargo.toml": root(), "Cargo.lock": lock(),
                 "crates/sunrise-mod-api-gen/Cargo.toml":
                     '[package]\nname = "sunrise-mod-api-gen"\n[dependencies]\n'
                     'sunrise-mod-source = { path = "../sunrise-mod-source" }\nshared = { workspace = true }\n'
                     'serde = { workspace = true }\n[dev-dependencies]\ntestonly = { path = "../testonly" }\n',
                 "crates/sunrise-mod-source/Cargo.toml": '[target.\'cfg(unix)\'.dependencies]\ndeep = { path = "../deep" }\n',
                 "third_party/syn/Cargo.toml": "", "crates/deep/Cargo.toml": "", "crates/shared/Cargo.toml": ""}
        with tempfile.TemporaryDirectory() as folder:
            parent = write_commit(folder, files)
            for path, content, judged in (
                    ("crates/sunrise-mod-source/src/lib.rs", "fix\n", True), ("crates/deep/src/lib.rs", "fix\n", True),
                    ("crates/shared/src/lib.rs", "fix\n", True), ("crates/sunrise-mod-api-gen/src/main.rs", "fix\n", True),
                    ("crates/sunrise-release-sign/src/lib.rs", "fix\n", True),
                    ("crates/testonly/src/lib.rs", "fix\n", False), ("crates/sunrise-world/src/lib.rs", "fix\n", False),
                    ("Cargo.toml", root() + "# a comment\n", False), ("Cargo.toml", root(other="3"), False),
                    ("Cargo.toml", root(serde="1.1"), True),
                    # What the root sets for every crate: debug-assertions in a release build, say, turns on
                    # sunrise-release-sign's developer key.
                    ("Cargo.toml", root() + "[profile.release]\ndebug-assertions = true\n", True),
                    ("Cargo.toml", root() + "[profile.release.package.sunrise-release-sign]\nopt-level = 0\n", True),
                    ("Cargo.toml", root() + "[workspace.lints.rust]\nunsafe_code = 'allow'\n", True),
                    ("Cargo.toml", root().replace("[workspace.dependencies]", "[workspace.package]\nedition = '2021'\n"
                                                  "[workspace.dependencies]"), True),
                    ("Cargo.toml", root().replace("[workspace.dependencies]", "[workspace]\nresolver = '3'\n"
                                                  "[workspace.dependencies]"), True),
                    ("Cargo.lock", lock(rand="0.8.1"), False), ("Cargo.lock", lock(serde="1.0.2"), True),
                    # The judge's `serde` no longer names one package: nothing to compare, so it is a change.
                    ("Cargo.lock", lock() + '[[package]]\nname = "serde"\nversion = "1.0.1"\nsource = "git+y#1"\n',
                     True)):
                with self.subTest(path=path, content=content):
                    git_in(Path(folder))("checkout", "-q", parent)
                    sha = write_commit(folder, {path: content})
                    with mock.patch.object(train, "ROOT", Path(folder)):
                        plain, special = train.changed_since(parent, sha)
                    self.assertEqual((special, plain), ([path], []) if judged else ([], [path]))
            # A redirect added by the candidate: the manifest and the crate it points to both need the full daily.
            git_in(Path(folder))("checkout", "-q", parent)
            sha = write_commit(folder, {"Cargo.toml": root() + 'serde = { path = "crates/evil" }\n',
                                        "crates/evil/src/lib.rs": "fix\n", "crates/evil/Cargo.toml": ""})
            with mock.patch.object(train, "ROOT", Path(folder)):
                plain, special = train.changed_since(parent, sha)
            self.assertEqual((sorted(special), plain), (["Cargo.toml", "crates/evil/Cargo.toml", "crates/evil/src/lib.rs"], []))

    def test_a_judges_locked_graph_is_compared_by_the_packages_its_selectors_name(self):
        # `a 1.0.0` locked three times, as Cargo writes it (checked against `cargo generate-lockfile`): a path
        # package, which has no source, and two git ones, whose sources carry the locked commit and whose selectors
        # don't.
        git_a, git_b = "git+https://example.com/a?branch=main", "git+https://example.com/b"

        def closure(judge, a=("local", git_a, git_b), commit="1111", b=()):
            sources = {"local": None, git_a: f"{git_a}#{commit}", git_b: f"{git_b}#2222"}
            packages = [("sunrise-mod-api-gen", None, judge), *(("a", sources[name], ()) for name in a),
                        ("b", "registry+x", b),
                        # A registry package of the judge's own name: never where the judge's closure starts.
                        ("sunrise-mod-api-gen", "registry+x", ())]
            lock = {"package": [{"name": name, "version": "1.0.0", **({"source": source} if source else {}),
                                 "dependencies": list(dependencies)} for name, source, dependencies in packages]}
            with mock.patch.object(train, "manifest", lambda sha, path: lock):
                return train.locked_closure("sha", ["sunrise-mod-api-gen"])

        def linked(found):
            return {(name, source) for name, _, source, *_ in found}

        self.assertEqual(linked(closure([f"a 1.0.0 ({git_a})"])),
                         {("sunrise-mod-api-gen", ""), ("a", f"{git_a}#1111")}, "the git package, by its commit")
        self.assertNotEqual(closure([f"a 1.0.0 ({git_a})"]), closure([f"a 1.0.0 ({git_a})"], commit="3333"),
                            "the same git package moved to another commit")
        self.assertEqual(linked(closure(["a 1.0.0"])), {("sunrise-mod-api-gen", ""), ("a", "")},
                         "an unqualified version among several is the path package")
        self.assertNotEqual(closure(["a 1.0.0"]), closure([f"a 1.0.0 ({git_b})"]))
        # The same packages reached, but the judge now links `a` itself and not only through `b`.
        self.assertNotEqual(closure([f"a 1.0.0 ({git_a})", "b"], b=[f"a 1.0.0 ({git_a})"]),
                            closure(["b"], b=[f"a 1.0.0 ({git_a})"]))
        # Cargo qualifies a selector only once another `a` is locked: the same package, spelled out, is no change.
        self.assertEqual(closure([f"a 1.0.0 ({git_a})"]), closure(["a"], a=(git_a,)))
        # A selector naming no entry or more than one: nothing to compare, so the caller fails closed.
        for judge, a in (([f"a 1.0.0 (git+https://example.com/c)"], ("local", git_a)), (["a"], ("local", git_a)),
                         (["a 1.0.0"], (git_a, git_b)), (["a 2.0.0"], ("local",))):
            with self.subTest(judge=judge, a=a):
                self.assertIsNone(closure(judge, a))

    def test_every_selected_leg_is_a_daily_leg(self):
        legs = set(train.BREAK_GLASS_ALWAYS).union(*(selected or () for _, selected in train.BREAK_GLASS_LEGS))
        self.assertLessEqual(legs, set(workflow("daily.yml")["jobs"]["summary"]["needs"]))

    def test_each_legs_job_name_fits_githubs_limit(self):
        # A longer name is cut by the Jobs API, and legs_passed would never find the leg it names.
        job = workflow("daily.yml")["jobs"]["legs"]
        self.assertEqual(job["name"], "legs ${{ needs.resolve.outputs.commit }} ${{ matrix.leg }}")
        self.assertIn("needs.resolve.outputs.legs", job["strategy"]["matrix"]["leg"])
        every = set(ALWAYS).union(*(selected or () for _, selected in train.BREAK_GLASS_LEGS))
        self.assertLessEqual(max(len(f"legs {'f' * 40} {leg}") for leg in every), 100)

    def test_changed_paths_are_read_raw_and_links_or_line_breaks_are_never_mapped(self):
        with tempfile.TemporaryDirectory() as folder:
            parent, sha = history(folder, ("docs/ü.md", "file"), ("docs/modding/link.md", "link"),
                                  ("docs/Cargo.toml\nCargo.lock", "file"))
            with mock.patch.object(train, "ROOT", Path(folder)):
                plain, special = train.changed_since(parent, sha)
        self.assertEqual(plain, ["docs/ü.md"], "unquoted, so it maps")
        self.assertEqual(sorted(special), ["docs/Cargo.toml\nCargo.lock", "docs/modding/link.md"])


def run_record(id, head, created="2026-11-20T12:00:00Z", branch=train.RELEASE_BRANCH, conclusion="success"):
    return daily_run(id, head, branch=branch, conclusion=conclusion) | {"created_at": created}


def legs_jobs(sha, legs):
    return {f"legs {sha} {leg}": "success" for leg in legs}


def full(sha, green=True):
    return {f"summary {sha}": "success" if green else "failure", f"green {sha}": "success" if green else "skipped"}


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        folder = self.enterContext(tempfile.TemporaryDirectory())
        git, (self.older, self.sha, self.tip) = repository(folder)
        git("push", "-q", "origin", f"{self.tip}:{RELEASE}")
        self.enterContext(mock.patch.object(train, "ROOT", Path(folder, "clone")))
        self.enterContext(mock.patch.object(train, "REPO", REPO))

    def admitted(self, *runs):
        records = [(run, [{"name": name, "conclusion": conclusion, "status": "completed",
                           "run_id": run["id"], "run_attempt": run["run_attempt"]} for name, conclusion in jobs.items()])
                   for run, jobs in runs]
        with mock.patch.object(train, "gh_json", side_effect=daily_runs(*records)):
            return train.break_glass_runs()

    def test_a_break_glass_run_counts_for_the_legs_it_passed_and_no_fewer(self):
        sha = self.sha
        legs = ALWAYS + ["book-quickstart"]
        run = run_record(1, self.tip)
        jobs = legs_jobs(sha, legs)
        self.assertEqual(train.legs_passed(sha, legs, self.admitted((run, jobs))), run["html_url"])
        self.assertEqual(train.legs_passed(sha, legs, self.admitted((run, legs_jobs(sha, legs + ["shards"])))),
                         run["html_url"], "more legs than needed")
        for evidence in ((run, legs_jobs(sha, ALWAYS)),
                         (run, jobs | {f"legs {sha} sdk": "failure"}),
                         (run, legs_jobs(self.tip, legs)),
                         (run_record(2, self.tip, branch="main"), jobs),
                         (run_record(2, self.older), jobs),
                         (run | {"conclusion": "failure"}, jobs),
                         (run | {"conclusion": "cancelled"}, jobs)):
            with self.subTest(evidence=evidence):
                self.assertEqual(train.legs_passed(sha, legs, self.admitted(evidence)), "")
        # Two runs that each passed some of the legs are not one run that passed them all.
        split = self.admitted((run, legs_jobs(sha, ALWAYS)), (run_record(3, self.tip), legs_jobs(sha, ["book-quickstart"])))
        self.assertEqual(train.legs_passed(sha, legs, split), "")

    def test_the_newest_admitted_full_daily_of_exactly_the_commit_decides(self):
        sha = self.sha
        green = run_record(1, self.tip, "2026-11-19T09:00:00Z")
        red = run_record(2, self.tip, "2026-11-20T09:00:00Z", conclusion="failure")
        skipped = run_record(3, self.tip, "2026-11-21T09:00:00Z")  # a scheduled run that found the commit already done
        partial = run_record(4, self.tip, "2026-11-21T10:00:00Z")
        inconclusive = [(partial, {f"summary {sha} (break-glass legs)": "success", **legs_jobs(sha, ALWAYS)}),
                        (partial, {f"summary {sha} (dry run)": "success"}),
                        (skipped, {f"summary {sha}": "skipped"}),
                        (red | {"conclusion": "cancelled"}, full(sha, green=False))]
        for evidence in inconclusive:
            with self.subTest(evidence=evidence):
                self.assertIsNone(train.newest_full_daily(sha, self.admitted(evidence)))
        for verdict in (full(sha, green=False), full(sha)):
            runs = self.admitted((red, verdict), (green, full(sha)))
            self.assertEqual(train.newest_full_daily(sha, runs), (red["html_url"], False))
        # A failed summary is red even if the run and its green marker somehow succeeded.
        self.assertEqual(train.newest_full_daily(sha, self.admitted((green, full(sha) | {f"summary {sha}": "failure"}))),
                         (green["html_url"], False))
        self.assertEqual(train.newest_full_daily(sha, self.admitted((green, full(sha)))), (green["html_url"], True))
        self.assertIsNone(train.newest_full_daily(sha, self.admitted((green, full(self.tip)))))

    def test_a_full_daily_without_provenance_or_tested_ancestry_supplies_no_verdict(self):
        sha = self.sha
        old = run_record(1, self.older)
        bad_ref = run_record(2, self.tip)
        sdk, = bad_ref["referenced_workflows"]
        bad_ref["referenced_workflows"] = [sdk | {"ref": "refs/tags/release/26.3"}]
        for run in (old, bad_ref):
            with self.subTest(run=run):
                self.assertIsNone(train.newest_full_daily(sha, self.admitted((run, full(sha)))))
                failed = run | {"conclusion": "failure"}
                self.assertIsNone(train.newest_full_daily(sha, self.admitted((failed, full(sha, green=False)))))

    def test_the_full_daily_cutoff_is_strict_and_compares_instants(self):
        since = train.github_time("2026-11-20T00:00:00Z")
        for created, accepted in (("2026-11-19T09:00:00Z", False), ("2026-11-20T00:00:00Z", False),
                                  ("2026-11-20T01:00:00+01:00", False), ("2026-11-20T00:00:00.5+00:00", True)):
            with self.subTest(created=created):
                run = run_record(1, self.tip, created)
                runs = self.admitted((run, full(self.sha)))
                self.assertEqual(train.newest_full_daily(self.sha, runs, since=since),
                                 (run["html_url"], True) if accepted else None)


class ProblemsTests(unittest.TestCase):
    """break_glass_problems on a real history."""

    def problems(self, folder, sha, parent, tag="26.3-0.1.1", runs=None, gate=True):
        with mock.patch.object(train, "ROOT", Path(folder)), \
                mock.patch.object(train, "break_glass_runs", return_value=runs if runs is not None else []), \
                mock.patch.object(train, "fork_gate_passed", return_value=gate) as gated:
            found = train.break_glass_problems(sha, tag, parent)
        if gated.called:
            gated.assert_called_once_with(sha, train.RELEASE_BRANCH)
        return found

    @staticmethod
    def evidence(sha, parent_sha, legs=("book-quickstart",)):
        return [(run_record(2, sha), legs_jobs(sha, ALWAYS + list(legs))),
                (run_record(1, sha, "2026-11-19T09:00:00Z"), full(parent_sha))]

    def test_a_patch_whose_legs_gate_and_parent_daily_passed_may_ship(self):
        with tempfile.TemporaryDirectory() as folder:
            parent, sha = history(folder, ("docs/modding/src/a.md", "file"))
            release = train.Release(PARENT_TAG, parent, train.github_time("2026-11-18T00:00:00Z"))
            found = self.problems(folder, sha, release, runs=self.evidence(sha, parent))
        self.assertEqual(found, ([], sorted(ALWAYS + ["book-quickstart"])))

    def test_each_rule_refuses_on_its_own(self):
        with tempfile.TemporaryDirectory() as folder:
            parent, sha = history(folder, ("crates/sunrise-world/src/lib.rs", "file"))
            release = train.Release(PARENT_TAG, parent, train.github_time("2026-11-18T00:00:00Z"))
            good = self.evidence(sha, parent, CRATES)
            own_red = [(run_record(3, sha, "2026-11-20T13:00:00Z"), full(sha, green=False))]
            for change, reason in (
                    ({"tag": "26.3-0.2.0"}, "is not a patch release of 26.3-0.1.0"),
                    ({"tag": "26.3-1.0.0"}, "is not a patch release of 26.3-0.1.0"),
                    ({"runs": good[1:]}, "has no real daily run that passed the legs"),
                    ({"runs": self.evidence(sha, parent, CRATES[:2])}, "passed the legs"),
                    ({"runs": good[:1]}, "the newest full daily run of 26.3-0.1.0"),
                    ({"runs": good[:1] + [(run_record(1, sha), full(parent, green=False))]}, "failed"),
                    ({"runs": own_red + good}, f"the newest full daily run of {sha} failed"),
                    ({"gate": False}, "gate jobs (lint, verify, verify-static) did not all pass")):
                with self.subTest(change=change):
                    problems, _ = self.problems(folder, sha, release, **{"runs": good, **change})
                    self.assertEqual(len(problems), 1, problems)
                    self.assertIn(reason, problems[0])

    def test_the_parent_needs_a_full_daily_created_after_its_publication(self):
        # Whatever kind of release the parent was: a green run from before its publication never stands in for the
        # one after it, and the newest one after it decides.
        with tempfile.TemporaryDirectory() as folder:
            parent, sha = history(folder, ("docs/a.md", "file"))
            release = train.Release(PARENT_TAG, parent, train.github_time("2026-11-20T00:00:00Z"))
            legs = (run_record(2, sha), legs_jobs(sha, ALWAYS))
            green_before = (run_record(1, sha, "2026-11-19T09:00:00Z"), full(parent))
            green_after = (run_record(3, sha, "2026-11-20T05:00:00Z"), full(parent))
            red_after = (run_record(4, sha, "2026-11-20T09:00:00Z", conclusion="failure"), full(parent, green=False))
            for runs, problem in (([legs], "is missing"), ([legs, green_before], "is missing"),
                                  ([legs, green_after, green_before], None), ([legs, red_after, green_after], "failed"),
                                  ([legs, red_after, green_before], "failed")):
                with self.subTest(runs=runs):
                    problems, _ = self.problems(folder, sha, release, runs=runs)
                    if problem:
                        self.assertIn(f"since its publication (2026-11-20T00:00:00Z) {problem}", problems[0])
                        self.assertIn(f"-f commit={parent}", problems[0])
                    else:
                        self.assertEqual(problems, [])

    def test_an_unmapped_or_special_path_a_foreign_parent_or_no_parent_refuses(self):
        with tempfile.TemporaryDirectory() as folder:
            parent, sha = history(folder, ("docs/a.md", "file"), (".github/workflows/release.yml", "file"))
            release = train.Release(PARENT_TAG, parent, PUBLISHED)
            problems, legs = self.problems(folder, sha, release, runs=self.evidence(sha, parent, ()))
            self.assertIn("'.github/workflows/release.yml'", problems[0])
            other = git_in(Path(folder))("commit-tree", "-m", "elsewhere", "4b825dc642cb6eb9a060e54bf8d69288fbee4904")
            problems, _ = self.problems(folder, sha, release._replace(sha=other), runs=self.evidence(sha, other, ()))
            self.assertIn("is not a new commit descending from 26.3-0.1.0", problems[0])
            problems, _ = self.problems(folder, parent, release, runs=self.evidence(parent, parent, ()))
            self.assertIn("is not a new commit descending from 26.3-0.1.0", problems[0])
            self.assertEqual(self.problems(folder, sha, None, tag=PARENT_TAG)[1], [])
            self.assertIn("no published stable release on 26.3", self.problems(folder, sha, None, tag=PARENT_TAG)[0][0])
        with tempfile.TemporaryDirectory() as folder:
            parent, sha = history(folder, ("docs/modding/link.md", "link"))
            problems, _ = self.problems(folder, sha, train.Release(PARENT_TAG, parent, PUBLISHED),
                                        runs=self.evidence(sha, parent))
            self.assertIn("'docs/modding/link.md'", problems[0])


class ApprovalTests(unittest.TestCase):
    UMCEKO, ARIA = (f"break-glass-{name}" for name in ("umceko", "aria"))

    def approvals(self, *records, attempt="1", pages=None):
        with mock.patch.object(train, "gh_json", return_value=pages or [list(records)]) as call:
            lines = train.break_glass_approvals("55", attempt)
        call.assert_called_once_with("api", "--paginate", f"repos/{train.REPO}/actions/runs/55/approvals?per_page=100")
        return lines

    @staticmethod
    def record(login, user_id, *environments, state="approved", comment="the save fix", kind="User"):
        return {"state": state, "comment": comment, "user": {"login": login, "id": user_id, "type": kind},
                "environments": [{"name": name} for name in environments]}

    def umceko(self, **kwargs):
        return self.record("UMCEKO", 11256132, self.UMCEKO, **kwargs)

    def aria(self, **kwargs):
        return self.record("arismoko", 19542195, self.ARIA, **kwargs)

    def test_each_environment_approved_once_by_its_own_approver(self):
        self.assertEqual(self.approvals(self.umceko(), self.aria(comment=" two\nlines ")),
                         [f"@UMCEKO ({self.UMCEKO}): the save fix", f"@arismoko ({self.ARIA}): two lines"])
        # The approvals come over several pages.
        self.assertEqual(len(self.approvals(pages=[[self.umceko()], [self.aria(comment=None)]])), 2)

    def test_a_missing_rejected_repeated_or_foreign_approval_refuses(self):
        for records, reason in (
                ([self.umceko()], f"{self.ARIA} needs the approval of user 19542195, not nobody"),
                ([], f"{self.UMCEKO} needs the approval of user 11256132, not nobody"),
                ([self.umceko(), self.aria(state="rejected")], f"{self.ARIA} was rejected by @arismoko"),
                ([self.aria(state="rejected"), self.umceko(), self.aria()], f"{self.ARIA} was rejected"),
                ([self.umceko(), self.aria(), self.aria()], f"{self.ARIA} was approved by @arismoko after another"),
                ([self.record("UMCEKO", 11256132, self.UMCEKO, self.ARIA)], "needs the approval of user 19542195"),
                ([self.umceko(), self.record("someone", 42, self.ARIA)], "not @someone"),
                ([self.umceko(), self.record("arismoko", 19542195, self.ARIA, kind="Bot")], "Bot 19542195")):
            with self.subTest(records=records), self.assertRaisesRegex(SystemExit, reason):
                self.approvals(*records)

    def test_only_the_first_attempt_counts(self):
        with mock.patch.object(train, "gh_json") as call, self.assertRaisesRegex(SystemExit, "not attempt 2's"):
            train.break_glass_approvals("55", "2")
        call.assert_not_called()

    def test_a_dry_run_reports_and_verifies_nothing(self):
        with tempfile.TemporaryDirectory() as folder, mock.patch.dict(os.environ, {
                "PUBLISH": "false", "BREAK_GLASS_REASON": "a\nsave  fix", "GITHUB_STEP_SUMMARY": f"{folder}/s",
                "GITHUB_OUTPUT": f"{folder}/o"}), mock.patch.object(train, "break_glass_approvals") as verify, \
                mock.patch("sys.argv", ["release_train.py", "break-glass-approvals"]), \
                contextlib.redirect_stdout(io.StringIO()):
            Path(folder, "o").touch()
            self.assertEqual(train.main(), 0)
            verify.assert_not_called()
            self.assertIn("Why: a save fix\n", Path(folder, "s").read_text())
            self.assertEqual(Path(folder, "o").read_text(), "", "no note: publishing needs a real run's")


class NotesTests(unittest.TestCase):
    ENV = {"TAG": "26.3-0.1.1", "SHA": "c" * 40, "PARENT_TAG": PARENT_TAG, "LEGS": "sdk book-quickstart",
           "BREAK_GLASS_REASON": "the save\nfix & \\1", "BREAK_GLASS_APPROVALS": "@UMCEKO (break-glass-umceko): ok &"}

    def test_the_section_goes_above_the_closing_disclaimer_as_written(self):
        with tempfile.TemporaryDirectory() as folder:
            notes = Path(folder, "notes.md")
            # A commit subject may hold anything, even the section's title or the disclaimer itself.
            body = f"- {nightly.BREAK_GLASS_SECTION} <!-- break-glass approvals --> {train.DISCLAIMER}\n\n"
            notes.write_text(body + train.DISCLAIMER + "\n")
            with mock.patch.dict(os.environ, self.ENV):
                nightly.break_glass_notes(notes)
            text = notes.read_text()
        self.assertTrue(text.startswith(body + nightly.BREAK_GLASS_SECTION + "\n26.3-0.1.1 (commit " + "c" * 40))
        self.assertTrue(text.endswith("Why: the save fix & \\1\n\nApproved by: @UMCEKO (break-glass-umceko): ok &\n\n"
                                      + train.DISCLAIMER + "\n"))

    def test_no_verified_approvals_no_release(self):
        with tempfile.TemporaryDirectory() as folder:
            notes = Path(folder, "notes.md")
            notes.write_text(train.DISCLAIMER)
            with mock.patch.dict(os.environ, {**self.ENV, "BREAK_GLASS_APPROVALS": ""}), \
                    self.assertRaisesRegex(SystemExit, "without its verified approvals"):
                nightly.break_glass_notes(notes)


class ReserveTests(unittest.TestCase):
    SHA = "c" * 40
    PARENT = train.Release(PARENT_TAG, "a" * 40, PUBLISHED)

    def reserve(self, parent, problems=()):
        env = {"GITHUB_REF": f"refs/heads/{train.RELEASE_BRANCH}", "GITHUB_REPOSITORY": REPO, "BREAK_GLASS": "true",
               "PARENT_SHA": self.PARENT.sha, "PARENT_TAG": self.PARENT.tag, "GITHUB_RUN_ID": "55", "GITHUB_RUN_ATTEMPT": "1"}
        with mock.patch.dict(os.environ, env), \
                mock.patch.object(nightly.train, "release_head", return_value=self.SHA) as head, \
                mock.patch.object(nightly.train, "origin_tags", return_value={PARENT_TAG: self.PARENT.sha,
                                                                              "26.3-0.1.1": self.SHA}), \
                mock.patch.object(nightly, "stable_parent", return_value=parent), \
                mock.patch.object(nightly.train, "break_glass_problems", return_value=(list(problems), [])) as rules, \
                mock.patch.object(nightly.train, "break_glass_approvals") as approvals, \
                contextlib.redirect_stdout(io.StringIO()):
            nightly.reserve_stable_tag(self.SHA, "26.3-0.1.1")
        head.assert_called_once_with(self.SHA, daily=False)
        rules.assert_called_once_with(self.SHA, "26.3-0.1.1", parent)
        approvals.assert_called_once_with("55", "1")

    def test_the_tag_waits_for_the_same_parent_its_rules_and_both_approvals_again(self):
        self.reserve(self.PARENT)
        for changed in (self.PARENT._replace(sha="b" * 40), self.PARENT._replace(tag="26.3-0.1.2")):
            with self.subTest(changed=changed), \
                    self.assertRaisesRegex(SystemExit, "the parent release changed since the pick"):
                self.reserve(changed)
        with self.assertRaisesRegex(SystemExit, "the gate failed"):
            self.reserve(self.PARENT, ["the gate failed"])


class PickTests(unittest.TestCase):
    SHA = "c" * 40
    PARENT = train.Release(PARENT_TAG, "a" * 40, PUBLISHED)

    def pick(self, parent=PARENT, **env):
        """pick_stable for a break-glass patch of `parent` (a Release, or an exception stable_parent raises): its
        outputs, run summary and stop() calls."""
        with tempfile.TemporaryDirectory() as folder:
            env = {"GITHUB_REPOSITORY": REPO, "GITHUB_OUTPUT": f"{folder}/out", "GITHUB_REF": "refs/heads/release/26.3",
                   "DRY_RUN": "false", "NIGHTLY_BASE": self.SHA, "RELEASE_VERSION": "0.1.1", "RELEASES_PAUSED": "",
                   "BREAK_GLASS": "the save fix", "GITHUB_STEP_SUMMARY": f"{folder}/summary", "GITHUB_RUN_ATTEMPT": "1",
                   **env}
            Path(folder, "summary").touch()
            back = os.getcwd()
            os.chdir(folder)
            try:
                with mock.patch.dict(os.environ, env), mock.patch.object(nightly, "git", return_value="") as git, \
                        mock.patch.object(nightly.train, "origin_tags", return_value={PARENT_TAG: self.PARENT.sha}), \
                        mock.patch.object(nightly.train, "release_head", return_value=self.SHA), \
                        mock.patch.object(nightly.train, "open_blockers", return_value=[]), \
                        mock.patch.object(nightly.train, "known_issues", return_value=[]), \
                        mock.patch.object(nightly, "sdk_ready", return_value=True), \
                        mock.patch.object(nightly, "stable_parent", side_effect=[parent]), \
                        mock.patch.object(nightly.train, "break_glass_problems", return_value=([], ALWAYS)), \
                        mock.patch.object(nightly, "stop", side_effect=SystemExit("stopped")) as stop, \
                        contextlib.redirect_stdout(io.StringIO()) as out:
                    nightly.pick_stable()
            finally:
                os.chdir(back)
                summary_text = Path(folder, "summary").read_text()
            outputs = dict(line.split("=", 1) for line in Path(folder, "out").read_text().splitlines())
            notes = Path(folder, "nightly-notes.md").read_text()
        return outputs, summary_text, notes, git, out.getvalue(), stop

    def test_the_approvers_see_the_parent_and_legs_before_they_approve(self):
        outputs, summary_text, notes, git, _, _ = self.pick()
        for part in (PARENT_TAG, self.PARENT.sha, "26.3-0.1.1", self.SHA, *ALWAYS):
            self.assertIn(part, summary_text)
        self.assertEqual((outputs["parent_tag"], outputs["parent_sha"], outputs["break_glass_legs"]),
                         (PARENT_TAG, self.PARENT.sha, " ".join(ALWAYS)))
        # Its notes list what changed since the same parent.
        self.assertIn(f"### Since {PARENT_TAG}", notes)
        git.assert_any_call("log", "--reverse", "--format=- %s", f"{self.PARENT.sha}..{self.SHA}")

    def test_a_rerun_is_refused_before_anyone_is_asked(self):
        with self.assertRaisesRegex(SystemExit, "stopped"):
            self.pick(GITHUB_RUN_ATTEMPT="2")

    def test_an_orphan_tag_stops_a_real_release_at_the_pick_and_a_dry_run_says_so(self):
        orphan = SystemExit("::error::refused: 26.3-0.1.5 is a tag with no published release above 26.3-0.1.0")
        with self.assertRaisesRegex(SystemExit, "26.3-0.1.5 is a tag with no published release"):
            self.pick(parent=orphan, BREAK_GLASS="")
        outputs, _, notes, _, out, _ = self.pick(parent=orphan, BREAK_GLASS="", DRY_RUN="true")
        self.assertIn("::warning::a real release would stop here: ::error::refused: 26.3-0.1.5", out)
        self.assertEqual(outputs["parent_tag"], "")


class PartialSummaryTests(unittest.TestCase):
    run_summary = daily.DailySummaryTests.run_summary
    CHOSEN = ALWAYS + ["book-quickstart"]
    PREFIX = summary.WRITING_REFS[f"refs/heads/{train.RELEASE_BRANCH}"]

    def results(self, **overrides):
        results = {name: {"result": "success" if name in self.CHOSEN or name == "sdk-windows-macos" else "skipped"}
                   for name in daily.LEGS}
        return {**results, **overrides, "resolve": {"outputs": {"legs": json.dumps(self.CHOSEN)}}}

    def jobs(self, failing=""):
        chosen = [daily.job(entry["name"], "failure", ["Run the quickstart"]) if entry["name"] == failing else entry
                  for entry in daily.GREEN_JOBS if entry["name"].partition(" / ")[0] in self.CHOSEN]
        return chosen + [daily.job("shards", "skipped"), daily.job("parity", "skipped")]

    def test_a_break_glass_run_is_judged_on_its_own_legs_and_files_or_closes_nothing(self):
        issue = {"number": 9, "title": summary.leg_title("book-quickstart", self.PREFIX), "body": ""}
        api = daily.Api(self.jobs(), issues=[issue])
        _, out = self.run_summary(api, results=self.results(), ref=f"refs/heads/{train.RELEASE_BRANCH}", parity=None)
        self.assertIn("green=true", out.splitlines())
        self.assertEqual(api.writes, [], "its green closes no issue")
        api = daily.Api(self.jobs("book-quickstart / quickstart"))
        _, out = self.run_summary(api, results=self.results(**{"book-quickstart": {"result": "failure"}}),
                                  ref=f"refs/heads/{train.RELEASE_BRANCH}", parity=None)
        self.assertIn("green=false", out.splitlines())
        self.assertEqual(api.writes, [], "its red files no issue")

    def test_its_sdk_publish_report_writes_nothing(self):
        issue = {"number": 9, "title": summary.leg_title("sdk-publish", self.PREFIX), "body": ""}
        for partial, writes in (("true", 0), ("false", 1)):
            api = daily.Api([], issues=[issue])
            env = {"GITHUB_REPOSITORY": REPO, "GITHUB_RUN_ID": "77", "GITHUB_REF": f"refs/heads/{train.RELEASE_BRANCH}",
                   "GITHUB_SERVER_URL": "https://github.com", "COMMIT": daily.COMMIT, "DRY_RUN": "false",
                   "PARTIAL": partial}
            with self.subTest(partial=partial), mock.patch.dict(os.environ, env, clear=True), \
                    contextlib.redirect_stdout(io.StringIO()):
                summary.after("sdk-publish", "success", api, daily.git((), True), daily.closers({}))
                self.assertEqual(len([write for write in api.writes if write[0] == "PATCH"]), writes)


class ReleaseLineIssueTests(unittest.TestCase):
    run_summary = daily.DailySummaryTests.run_summary

    def test_the_release_lines_reds_file_their_own_issues_and_never_move_the_tag(self):
        red = [daily.job("shards / ignored-tests-shards (sunrise-world)", "failure", ["Run"])] + daily.GREEN_JOBS[2:]
        results = dict(daily.LEGS, shards={"result": "failure"})
        main_issue = {"number": 5, "title": "Daily: sunrise-world a::b", "body": ""}
        api = daily.Api(red, issues=[main_issue])
        self.run_summary(api, [daily.shard_report([daily.failure("a::b")])], ref="refs/heads/release/26.3",
                         results=results)
        created = [body["title"] for method, path, body in api.writes if method == "POST" and path == "issues"]
        self.assertEqual(created, ["Daily (release/26.3): sunrise-world a::b"], "main's open issue is not the line's")
        api = daily.Api(daily.GREEN_JOBS, issues=[main_issue])
        self.run_summary(api, ref="refs/heads/release/26.3")
        self.assertEqual(api.writes, [], "a green release-line run neither moves daily-green nor closes main's issue")


class WorkflowTests(unittest.TestCase):
    def test_nothing_is_built_signed_or_published_before_both_approvals(self):
        jobs = workflow("nightly-build.yml")["jobs"]
        bound = {name: job for name, job in jobs.items() if "break-glass-" in job.get("environment", "")}
        self.assertEqual({job["environment"] for job in bound.values()},
                         {f"${{{{ needs.pick.outputs.publish == 'true' && '{name}' || '' }}}}"
                          for name in train.BREAK_GLASS_APPROVERS})
        self.assertEqual(set(jobs["break-glass"]["needs"]), {"pick", *bound})
        self.assertIn("break-glass-approvals", jobs["break-glass"]["steps"][-1]["run"])
        self.assertIn("needs.pick.result == 'success'", jobs["exporters"]["if"])
        # A re-run, of failed jobs too, never re-enters an approval, a build or a signing of a break-glass release.
        for name in ("break-glass", *bound):
            with self.subTest(job=name):
                self.assertIn("github.run_attempt == '1'", jobs[name]["if"])
        self.assertIn("(needs.pick.outputs.break_glass == 'false' || github.run_attempt == '1')",
                      " ".join(jobs["build"]["if"].split()))
        for name in ("exporters", "sign", "publish"):
            with self.subTest(job=name):
                self.assertIn("break-glass", jobs[name]["needs"])
                self.assertIn("(needs.break-glass.result == 'success' && github.run_attempt == '1')", jobs[name]["if"])
        self.assertIn("exporters", jobs["build"]["needs"])

    def test_the_surface_is_checked_at_the_pick_and_again_before_the_tag(self):
        jobs = workflow("nightly-build.yml")["jobs"]
        # Every stable release's mod API is judged at the pick, before any approval, build or signature.
        pick = next(step for step in jobs["pick"]["steps"] if "stable-mod-api" in step.get("run", ""))
        self.assertEqual(pick["env"]["BREAK_GLASS"], "${{ steps.pick.outputs.break_glass }}")
        self.assertIn("inputs.kind == 'stable'", pick["if"])
        self.assertNotIn("break_glass", pick["if"])
        steps = jobs["pick"]["steps"]
        rust = next(step for step in steps if step.get("name", "").startswith("The pinned Rust"))
        self.assertLess(steps.index(rust), steps.index(pick), "the parent's sunrise-stage is built with cargo")
        publish = jobs["publish"]
        self.assertEqual(publish["steps"][0]["with"]["fetch-depth"], "0")
        check = next(step for step in publish["steps"] if "stable-mod-api" in step.get("run", ""))
        self.assertEqual(check["env"]["BREAK_GLASS"], "${{ needs.pick.outputs.break_glass }}")
        stable = next(step for step in publish["steps"] if step.get("name", "").startswith("Publish as the stable"))
        self.assertEqual(stable["env"]["BREAK_GLASS_APPROVALS"], "${{ needs.break-glass.outputs.note }}")
        self.assertEqual(stable["env"]["PARENT_SHA"], "${{ needs.pick.outputs.parent_sha }}")
        self.assertLess(stable["run"].index("break-glass-notes"), stable["run"].index("reserve-stable-tag"))
        self.assertLess(publish["steps"].index(check), publish["steps"].index(stable))

    def test_no_release_job_runs_cargo_in_the_checkout(self):
        # The checkout is the candidate: its .cargo/config.toml, [patch] or build scripts would run. Signatures are
        # checked with OpenSSL (nightly.py verify-sums); the only build is the parent's sunrise-stage, from the
        # parent's own tree (nightly.parent_stage, whose cargo run test_stable_mod_api.JudgeBuildTests checks).
        # The SDK's and the mod tool's signing, checking and publishing jobs build nothing either.
        for file, names in (("nightly-build.yml", ("pick", "break-glass", "sign", "publish")),
                            ("sdk-publish.yml", ("sign", "publish")), ("mod-tool.yml", ("sign", "verify", "publish"))):
            jobs = workflow(file)["jobs"]
            for name in names:
                for step in jobs[name]["steps"]:
                    with self.subTest(workflow=file, job=name, step=step.get("name", step.get("uses"))):
                        self.assertNotRegex(step.get("run", ""), r"(^|\s)cargo\s")

    def test_the_full_daily_follows_every_published_stable_release(self):
        full_daily = workflow("nightly-build.yml")["jobs"]["full-daily"]
        self.assertIn("needs.publish.result == 'success'", full_daily["if"])
        self.assertIn("inputs.kind == 'stable'", full_daily["if"])
        self.assertNotIn("break_glass", full_daily["if"])
        run = full_daily["steps"][0]["run"]
        self.assertIn('gh workflow run daily.yml', run)
        self.assertIn('-f dry_run=false -f commit="$SHA"', run)
        self.assertNotIn("break_glass", run)

    def test_a_break_glass_daily_is_never_a_full_daily(self):
        daily_yml = workflow("daily.yml")
        jobs = daily_yml["jobs"]
        self.assertIn("needs.resolve.outputs.legs == ''", jobs["green"]["if"])
        self.assertTrue(jobs["legs"]["name"].startswith("legs ${{ needs.resolve.outputs.commit }}"))
        self.assertIn("needs.resolve.outputs.branch == 'release/26.3'", jobs["legs"]["if"])
        self.assertIn("(break-glass legs)", jobs["summary"]["name"])
        for leg in jobs["summary"]["needs"]:
            if leg != "resolve":
                with self.subTest(leg=leg):
                    self.assertIn("needs.resolve.outputs.legs == ''", jobs[leg]["if"])
        report = jobs["sdk-publish-report"]["steps"][-1]["env"]
        self.assertEqual(report["PARTIAL"], "${{ needs.resolve.outputs.legs != '' }}")

    def test_each_release_line_commit_has_its_own_daily_queue(self):
        # A release's follow-up full daily waits for nobody else's and is never replaced by another commit's.
        group = workflow("daily.yml")["concurrency"]["group"]
        self.assertIn("format('-release-{0}', inputs.commit || github.sha)", group)

    def test_the_release_branch_pushes_run_the_gate(self):
        on = workflow("dyna-ci.yml")["on"]
        self.assertEqual(on["push"]["branches"], ["main", train.RELEASE_BRANCH])


if __name__ == "__main__":
    unittest.main()
