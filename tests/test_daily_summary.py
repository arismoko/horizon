"""The daily summary: the daily-green tag, one issue per failing test, and nothing written outside main."""
import contextlib
import io
import json
import os
import re
import shlex
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tests.git_fixture import git_in
from tests.os_support import needs_runner_shell
from tests.test_ci_concurrency import ROOT
from tests.test_release_branches import MAIN, NIGHTLY, RELEASE, repository, run_gate
from tools import sunrise_mod_binstall
from tools.ci import daily_summary as summary, parity_verdict
from tools.ci import vendored_yaml

yaml = vendored_yaml.load()

COMMIT = "c" * 40
DAILY = yaml.load((ROOT / ".github/workflows/daily.yml").read_text(), Loader=yaml.BaseLoader)
# toJSON(needs) of a green run, as the summary job gets it: every call it needs but `resolve`, whose outputs only
# a parity skip reads.
LEGS = {name: {"result": "success"} for name in DAILY["jobs"]["summary"]["needs"] if name != "resolve"}
PARITY_JOB = "parity / vanilla-parity"
PARITY_PASS = {"passed": ["loot_a"], "failed": [], "complete": True}


def job(name, conclusion="success", failed=()):
    steps = [{"name": "Set up job", "conclusion": "success"}] + [{"name": step, "conclusion": "failure"} for step in failed]
    return {"name": name, "conclusion": conclusion, "steps": steps}


GREEN_JOBS = [job("shards / plan"), job("shards / ignored-tests-shards (sunrise-world)"),
              job("fn-replace-check / route"), job("fn-replace-check / fn-replace-check"),
              job("book-quickstart / route"), job("book-quickstart / quickstart"),
              job("windows-runtime-leases / test"),
              job("windows-runtime-leases / runtime-leases"), job("windows-runtime-leases / launcher-processes"),
              job("cargo-sunrise-mod / plan"),
              job("machine-sample / route"), job("machine-sample / build"), job("exporters / exporters"),
              job("sdk / fresh"), job("sdk / build linux"), job("sdk / build linux-server"),
              job("sdk / fresh"), job("sdk / build windows"), job("sdk / build macos"), job(PARITY_JOB)]
# Every binstall archive the daily builds (cargo-sunrise-mod-build.yml), with no publish job: it never publishes.
GREEN_JOBS += [job(f"cargo-sunrise-mod / build {platform['target']}") for platform in sunrise_mod_binstall.PLATFORMS]
# Every other shard of the daily matrix, green (the summary expects each by name: summary.expected_shards).
GREEN_JOBS += [job(name) for name in summary.expected_shards() if name != "shards / ignored-tests-shards (sunrise-world)"]


def shard_report(problems=(), passed=(), incomplete=()):
    return {"shards": ["sunrise-world"], "problems": list(problems), "passed": list(passed),
            "incomplete": list(incomplete)}


def failure(test, panic="crates/sunrise-world/src/lib.rs:7:3", kind="failed", package="sunrise-world"):
    return {"package": package, "test": test, "kind": kind, "panic": panic, "policy": None}


class Api:
    """GitHub's REST API as the summary sees it: listings by page, and every write recorded."""

    def __init__(self, jobs, issues=(), tag=False, fail_on=None, comments=None, runs=None):
        self.jobs, self.issues, self.tag, self.fail_on, self.writes = jobs, list(issues), tag, fail_on, []
        self.comments = comments if comments is not None else {}
        self.runs = runs or {}

    def __call__(self, method, path, body=None):
        if self.fail_on and self.fail_on in path:
            raise SystemExit("::error::GitHub refused GET: HTTP 403 (rate limit exhausted)")
        if method != "GET":
            self.writes.append((method, path, body))
            if path.endswith("/comments"):
                self.comments.setdefault(int(path.split("/")[1]), []).append(body["body"])
            elif path == "issues":
                self.issues.append({"number": 1000 + len(self.issues), "title": body["title"], "body": body["body"]})
            return {}
        if "matching-refs" in path:
            return [{"ref": "refs/tags/daily-green"}] if self.tag else []
        if re.fullmatch(r"issues/\d+", path):
            number = int(path.split("/")[1])
            return {"body": None, **next(issue for issue in self.issues if issue["number"] == number)}
        if "workflows/daily.yml/runs" in path:
            return {"workflow_runs": list(self.runs.values())} if path.endswith("page=1") else {"workflow_runs": []}
        if re.search(r"actions/runs/(\d+)/jobs", path) and "/runs/77/" not in path:
            ident = int(re.search(r"actions/runs/(\d+)/jobs", path)[1])
            return {"jobs": self.runs[ident]["jobs"]} if path.endswith("page=1") else {"jobs": []}
        page = int(path.rsplit("page=", 1)[1])
        if re.fullmatch(r"issues/\d+/comments\?per_page=100&page=\d+", path):
            found = self.comments.get(int(path.split("/")[1]), [])
            return [{"body": text} for text in found[(page - 1) * 100:page * 100]]
        if "matching-refs" in path:
            return [{"ref": "refs/tags/daily-green"}] if self.tag else []
        page = int(path.rsplit("page=", 1)[1])
        if "/jobs" in path:
            return {"jobs": self.jobs[(page - 1) * 100:page * 100]}
        if path.startswith("issues?"):
            closed = "state=closed" in path
            listed = [issue for issue in self.issues if (issue.get("state") == "closed") == closed]
            return listed[(page - 1) * 100:page * 100]
        raise AssertionError(path)


def closers(fixes):
    """GraphQL as fixes_after sees it: issue number -> the merge commit of the pull request that closed it."""
    def query(text, variables):
        fix = fixes.get(variables["number"])
        nodes = [{"closer": {"mergeCommit": {"oid": fix}} if fix else None}]
        return {"repository": {"issue": {"timelineItems": {"nodes": nodes}}}}
    return query


def git(merged=(), tag=True, ancestors=()):
    def run(*args):
        if args[0] == "rev-parse":
            return "d" * 40 if tag else None
        if args[0] == "merge-base":
            return "" if args[2] in ancestors else None
        return "\n".join(f"Merge pull request #{n} from arismoko/topic" for n in merged) + "\nMerge branch 'x'"
    return run


class DailySummaryTests(unittest.TestCase):
    def run_summary(self, api, reports=(), results=LEGS, ref="refs/heads/main", dry_run="false", merged=(), tag=True,
                    fixes=None, ancestors=(), parity=PARITY_PASS, run="77", parity_scope=None, history=None):
        with tempfile.TemporaryDirectory() as folder:
            for index, report in enumerate(reports):
                path = Path(folder, "reports", f"daily-report-{index}")
                path.mkdir(parents=True)
                (path / "daily-report.json").write_text(json.dumps(report))
            if parity is not None:
                path = Path(folder, "reports", "daily-report-parity-1")
                path.mkdir(parents=True, exist_ok=True)
                scope = {"run_id": run, "run_attempt": "1", "commit": COMMIT} | (parity_scope or {})
                (path / "parity.json").write_text(json.dumps(dict(parity, scope=scope)))
            env = {"GITHUB_REPOSITORY": "arismoko/horizon", "GITHUB_RUN_ID": run, "GITHUB_RUN_ATTEMPT": "1", "GITHUB_REF": ref,
                   "GITHUB_SERVER_URL": "https://github.com", "COMMIT": COMMIT, "DRY_RUN": dry_run,
                   "RESULTS": json.dumps(results), "REPORTS": str(Path(folder, "reports")),
                   "GITHUB_STEP_SUMMARY": str(Path(folder, "summary")), "GITHUB_OUTPUT": str(Path(folder, "out"))}
            with mock.patch.dict(os.environ, env, clear=True), contextlib.redirect_stdout(io.StringIO()):
                summary.main(api, history or git(merged, tag, ancestors), closers(fixes or {}))
            return Path(folder, "summary").read_text(), Path(folder, "out").read_text()

    def run_after(self, api, result, ref="refs/heads/main", run="77", fixes=None):
        env = {"GITHUB_REPOSITORY": "arismoko/horizon", "GITHUB_RUN_ID": run, "GITHUB_REF": ref,
               "GITHUB_SERVER_URL": "https://github.com", "COMMIT": COMMIT, "DRY_RUN": "false"}
        with mock.patch.dict(os.environ, env, clear=True), contextlib.redirect_stdout(io.StringIO()):
            summary.after("sdk-publish", result, api, git((), True), closers(fixes or {}))

    def test_a_failed_sdk_publish_files_one_issue_and_its_next_success_closes_it(self):
        failed = [job("sdk-publish / index"), job("sdk-publish / publish", "failure", ["Release sdk-<sha>"])]
        api = Api(failed)
        self.run_after(api, "failure")
        created = [body for method, path, body in api.writes if method == "POST" and path == "issues"]
        self.assertEqual([body["title"] for body in created], ["Daily: sdk-publish did not complete"])
        self.assertIn("`sdk-publish / publish: Release sdk-<sha>`", created[0]["body"])
        # A later run's failure comments on that issue instead of filing another.
        api.writes.clear()
        api.runs = {78: {"jobs": failed}}
        self.run_after(api, "failure", run="78")
        self.assertEqual([(method, path) for method, path, _ in api.writes], [("POST", "issues/1000/comments")])
        api.writes.clear()
        self.run_after(api, "success", run="79")
        self.assertIn(("PATCH", "issues/1000", {"state": "closed", "state_reason": "completed"}), api.writes)
        # A failure on a commit that predates the fix which closed the issue comments on it and leaves it closed.
        api.issues[0]["state"] = "closed"
        api.runs[80] = {"jobs": failed}
        api.writes.clear()
        self.run_after(api, "failure", run="80", fixes={1000: "f" * 40})
        self.assertEqual([(method, path) for method, path, _ in api.writes], [("POST", "issues/1000/comments")])
        self.assertIn("which predates the fix `fffffffff`", api.writes[0][2]["body"])
        # Off main nothing is written.
        quiet = Api(failed)
        self.run_after(quiet, "failure", ref="refs/heads/topic")
        self.assertEqual(quiet.writes, [])

    def run_parity(self, api, result, log, expected=(), run="77", merged=(2101, 2102)):
        api.jobs = [j for j in GREEN_JOBS if j["name"] != PARITY_JOB] + api.jobs
        if run != "77":
            api.runs = {int(run): {"jobs": api.jobs}}
        verdict = None if log is None else parity_verdict.verdict(log, set(expected), "1" if "FAIL " in log else "0")[0]
        return self.run_summary(api, results=dict(LEGS, parity={"result": result}), parity=verdict, run=run, merged=merged)

    def test_a_failing_harness_files_one_issue_with_the_suspects_and_its_pass_closes_it(self):
        api = Api([job(PARITY_JOB, "failure", ["Run every automatic harness"])])
        log = ("PASS loot_a 3s\nFAIL boat_rider 884s  (log)\nPASS sculk#1 2s\nFAIL sculk#2 4s\n"
               "RETRIED sculk#2: failed, then failed again on its one retry\n2 passed, 2 failed in 300.2 min.\n")
        _, out = self.run_parity(api, "failure", log, ["loot_a", "boat_rider", "sculk#1", "sculk#2"])
        self.assertEqual(out.splitlines(), ["green=false", "sdk_green=true"])
        self.assertFalse(any(path.startswith("git/") for _, path, _ in api.writes))
        created = [body for method, path, body in api.writes if method == "POST" and path == "issues"]
        self.assertEqual([body["title"] for body in created], ["Daily: parity boat_rider", "Daily: parity sculk"])
        self.assertIn("#2101", created[0]["body"])
        self.assertIn("#2102", created[0]["body"])
        self.assertNotIn("884", created[0]["body"], "a harness's log text stays on the runner")
        # The next run: boat_rider still fails, but sculk passes in every shard and closes.
        next_run = Api([job(PARITY_JOB, "failure")], issues=api.issues)
        self.run_parity(next_run, "failure", "FAIL boat_rider 9s\nPASS sculk#1 1s\nPASS sculk#2 1s\n"
                        "2 passed, 1 failed in 290.0 min.\n", ["boat_rider", "sculk#1", "sculk#2"], run="78")
        self.assertEqual([(method, path) for method, path, _ in next_run.writes],
                         [("POST", "issues/1000/comments"), ("POST", "issues/1001/comments"), ("PATCH", "issues/1001")])

    def test_a_failing_harness_uses_the_prior_green_and_names_rollup_members(self):
        api = Api([j for j in GREEN_JOBS if j["name"] != PARITY_JOB] + [job(PARITY_JOB, "failure")], tag=True)
        previous_green = "d" * 40
        calls = []

        def history(*args):
            calls.append(args)
            self.assertEqual(api.writes, [], "read suspects before applying the summary's writes")
            if args[0] == "rev-parse":
                self.assertEqual(args[-1], "refs/tags/daily-green^{commit}")
                return previous_green
            self.assertEqual(args, ("log", "--merges", "--format=%s", f"{previous_green}..{COMMIT}"))
            return ("Merge pull request #2110 from org/rollup\nMerge #2105 at abcdef0\n"
                    "Merge #2103 at fedcba0\nMerge #2105 at abcdef0\nMerge branch 'topic'\n")

        self.run_summary(api, results=dict(LEGS, parity={"result": "failure"}), history=history,
                         parity={"passed": [], "failed": ["loot_a"], "complete": True})
        self.assertEqual([args[0] for args in calls], ["rev-parse", "log"])
        self.assertEqual([path for _, path, _ in api.writes], ["issues"])
        self.assertIn(f"merged since the last green daily (`daily-green`..`{COMMIT[:9]}`): #2103, #2105, #2110",
                      api.issues[-1]["body"])

    def test_a_required_parity_call_with_no_verdict_holds_green_but_not_the_sdk(self):
        for status in ("skipped", "cancelled", "failure", "success"):
            with self.subTest(status=status):
                api = Api([job(PARITY_JOB, status, ["Run every automatic harness"])])
                _, out = self.run_parity(api, status, None)
                self.assertEqual(out.splitlines(), ["green=false", "sdk_green=true"])
                self.assertEqual([body["title"] for method, path, body in api.writes if path == "issues"],
                                 ["Daily: parity did not complete"])
                self.assertFalse(any(path.startswith("git/") for _, path, _ in api.writes))
                self.assertIn("Run every automatic harness", api.issues[-1]["body"])

    def test_a_parity_call_cancelled_before_start_holds_green_but_files_nothing(self):
        api = Api([{"name": PARITY_JOB, "conclusion": "cancelled", "steps": []}])
        _, out = self.run_parity(api, "cancelled", None)
        self.assertEqual(out.splitlines(), ["green=false", "sdk_green=true"])
        self.assertEqual(api.writes, [])

    def test_an_inapplicable_parity_skip_leaves_its_issues_alone_and_other_legs_decide_green(self):
        issues = [{"number": 7, "title": "Daily: parity did not complete"},
                  {"number": 8, "title": "Daily: parity loot_a"}]
        jobs = [j for j in GREEN_JOBS if j["name"] != PARITY_JOB]
        # The resolve job supplies one explicit distinction: a skipped required call is still a failure.
        results = dict(LEGS, parity={"result": "skipped"}, resolve={"outputs": {"parity_required": "false"}})
        for dry_run, ref in (("true", "refs/heads/main"), ("true", "refs/heads/topic")):
            with self.subTest(dry_run=dry_run, ref=ref):
                api = Api(jobs, issues=issues)
                _, out = self.run_summary(api, results=results, parity=None, ref=ref, dry_run=dry_run)
                self.assertEqual(out.splitlines(), ["green=true", "sdk_green=true"])
                self.assertEqual(api.writes, [])
        # A writing run never leaves parity out, whatever resolve said: no tag and no Nightly without it.
        api = Api(jobs, issues=issues)
        _, out = self.run_summary(api, results=results, parity=None, ref="refs/heads/main", dry_run="false")
        self.assertEqual(out.splitlines(), ["green=false", "sdk_green=true"])
        self.assertNotIn("git/refs", [path for _, path, _ in api.writes])

    def test_an_incomplete_parity_verdict_reports_no_passes(self):
        log = "PASS loot_a#1 3s\n"
        report, _ = parity_verdict.verdict(log, {"loot_a#1", "loot_a#2"}, "0")
        self.assertEqual(report, {"passed": [], "failed": [], "complete": False})
        log += "PASS loot_a#2 4s\n2 passed, 0 failed in 0.1 min.\n"
        report, _ = parity_verdict.verdict(log, {"loot_a#1", "loot_a#2"}, "0")
        self.assertEqual(report, {"passed": ["loot_a"], "failed": [], "complete": True})

    def test_merged_share_verdicts_report_one_run(self):
        scope = {"run_id": "77", "run_attempt": "2", "commit": COMMIT}

        def share(index, selected, passed=(), failed=(), complete=True, run=None):
            return {"bucket": index, "of": 2, "selected": list(selected), "passed": list(passed),
                    "failed": list(failed), "complete": complete, "scope": run or scope}
        plan = ["loot_a", "loot_b"]
        first, second = share(0, ["loot_a"], passed=["loot_a"]), share(1, ["loot_b"], failed=["loot_b"])
        merged = parity_verdict.merge([first, second], 2, plan, scope)
        self.assertEqual((merged["passed"], merged["failed"], merged["complete"], merged["problems"]),
                         (["loot_a"], ["loot_b"], True, []))
        self.assertEqual(merged["scope"], scope)
        # A share that never reported leaves the run unfinished, and an unfinished run proves no pass.
        missing = parity_verdict.merge([first], 2, plan, scope)
        self.assertEqual((missing["passed"], missing["complete"]), ([], False))
        self.assertIn("shares [0] reported, not 0..1", missing["problems"])
        # The same share twice is not a run.
        duplicate = parity_verdict.merge([first, dict(second, bucket=0)], 2, plan, scope)
        self.assertIn("share 0 reported twice", duplicate["problems"])
        # Two attempts' verdicts never merge into one run, even for the same commit and run ID.
        older = share(0, ["loot_a"], failed=["loot_a"], run=scope | {"run_attempt": "1"})
        foreign = parity_verdict.merge([older, second], 2, plan, scope)
        self.assertEqual((foreign["passed"], foreign["failed"], foreign["complete"], foreign["scope"]),
                         ([], ["loot_b"], False, scope))
        # Two shares that ran the same entries and left others to nobody are not a run either.
        overlapping = parity_verdict.merge([first, dict(first, bucket=1)], 2, plan, scope)
        self.assertFalse(overlapping["complete"])
        self.assertIn("two shares ran the same harness", overlapping["problems"])
        self.assertIn("1 planned harnesses ran in no share, 0 ran outside the plan", overlapping["problems"])
        # A share that did not finish keeps the failures it saw, and proves no pass.
        partial = parity_verdict.merge([first, dict(second, complete=False)], 2, plan, scope)
        self.assertEqual((partial["passed"], partial["failed"], partial["complete"]), ([], ["loot_b"], False))

    def test_malformed_share_evidence_does_not_erase_a_current_failure(self):
        scope = {"run_id": "77", "run_attempt": "1", "commit": COMMIT}
        current = {"bucket": 0, "of": 2, "selected": ["loot_a#2"], "passed": [], "failed": ["loot_a"],
                   "complete": True, "scope": scope}
        other = dict(current, bucket=1, selected=["loot_b"], failed=["loot_b"])
        malformed = [None, [], dict(other, scope=None), dict(other, bucket=True), dict(other, of=3),
                     dict(other, complete="true"), dict(other, selected="loot_b"), dict(other, selected=[["loot_b"]]),
                     dict(other, selected=["loot_b", "loot_b"]), dict(other, failed="loot_b"),
                     dict(other, failed=["loot_a"]), dict(other, failed=[])]
        for report in malformed:
            with self.subTest(report=report):
                merged = parity_verdict.merge([current, report], 2, ["loot_a#2", "loot_b"], scope)
                self.assertEqual((merged["passed"], merged["failed"], merged["complete"], merged["scope"]),
                                 ([], ["loot_a"], False, scope))

    def test_malformed_plan_cannot_prove_complete_coverage(self):
        scope = {"run_id": "77", "run_attempt": "1", "commit": COMMIT}
        report = {"bucket": 0, "of": 1, "selected": ["loot_a#1"], "passed": [], "failed": ["loot_a"],
                  "complete": True, "scope": scope}
        for plan in (None, [], "loot_a#1", {"loot_a#1": True}, [["loot_a#1"]], ["loot_a#1", "loot_a#1"]):
            with self.subTest(plan=plan):
                merged = parity_verdict.merge([report], 1, plan, scope)
                self.assertEqual((merged["passed"], merged["failed"], merged["complete"], merged["scope"]),
                                 ([], ["loot_a"], False, scope))

    def test_a_red_share_files_its_harness_and_no_infrastructure_issue(self):
        api = Api([job("parity / vanilla-parity (0/2)", "success", ["Run every automatic harness"]),
                   job("parity / vanilla-parity (1/2)", "failure", ["Run every automatic harness"]),
                   job("parity / verdict", "failure", ["Fail unless every share reported and no harness failed"])])
        _, out = self.run_parity(api, "failure", "PASS loot_a 3s\nFAIL boat_rider 9s\n1 passed, 1 failed in 1.0 min.\n",
                                 ["loot_a", "boat_rider"])
        self.assertIn("green=false", out)
        self.assertEqual([body["title"] for method, path, body in api.writes if path == "issues"],
                         ["Daily: parity boat_rider"])

    def test_a_share_that_dies_after_its_verdict_stays_visible_beside_a_red_harness(self):
        api = Api([job("parity / vanilla-parity (0/2)", "failure", ["Upload the harnesses' verdict"]),
                   job("parity / vanilla-parity (1/2)", "failure", ["Run every automatic harness"]),
                   job("parity / verdict", "failure", ["Fail unless every share reported and no harness failed"])],
                  issues=[{"number": 7, "title": "Daily: parity did not complete"}])
        _, out = self.run_parity(api, "failure", "PASS loot_a 3s\nFAIL boat_rider 9s\n1 passed, 1 failed in 1.0 min.\n",
                                 ["loot_a", "boat_rider"])
        self.assertIn("green=false", out)
        self.assertEqual([body["title"] for method, path, body in api.writes if path == "issues"],
                         ["Daily: parity boat_rider"])
        self.assertIn(("POST", "issues/7/comments"),
                      [(method, path) for method, path, _ in api.writes])

    def test_an_incomplete_parity_run_files_failures_but_closes_nothing(self):
        api = Api([job(PARITY_JOB, "failure")],
                  issues=[{"number": 7, "title": "Daily: parity sculk"}])
        _, out = self.run_parity(api, "failure", "PASS sculk#1 3s\nFAIL loot_c 2s\n1 passed, 1 failed in 1.0 min.\n",
                                 ["sculk#1", "sculk#2", "loot_c"])
        self.assertIn("green=false", out)
        self.assertEqual([body["title"] for method, path, body in api.writes if path == "issues"],
                         ["Daily: parity loot_c", "Daily: parity did not complete"])
        self.assertFalse(any(method == "PATCH" for method, _, _ in api.writes))

    def test_a_cancelled_parity_call_with_a_finished_red_verdict_still_did_not_complete(self):
        api = Api([job(PARITY_JOB, "cancelled")], issues=[{"number": 7, "title": "Daily: parity did not complete"}])
        _, out = self.run_parity(api, "cancelled", "FAIL loot_a 3s\n0 passed, 1 failed in 1.0 min.\n", ["loot_a"])
        self.assertIn("green=false", out)
        self.assertEqual([(method, path) for method, path, _ in api.writes],
                         [("POST", "issues"), ("POST", "issues/7/comments")])

    def test_parity_verdict_must_match_run_attempt_and_commit(self):
        for mismatched in ({"run_id": "76"}, {"run_attempt": "0"}, {"commit": "old"}):
            with self.subTest(scope=mismatched):
                api = Api(GREEN_JOBS, issues=[{"number": 7, "title": "Daily: parity loot_a"}])
                _, out = self.run_summary(api, parity_scope=mismatched)
                self.assertEqual(out.splitlines(), ["green=false", "sdk_green=true"])
                self.assertEqual([body["title"] for _, path, body in api.writes if path == "issues"],
                                 ["Daily: parity did not complete"])
                self.assertFalse(any(method == "PATCH" for method, _, _ in api.writes))

    def test_all_green_moves_the_tag_and_closes_issues_that_went_green(self):
        api = Api(GREEN_JOBS, issues=[{"number": 5, "title": "Daily: sunrise-world a::b"},
                                      {"number": 6, "title": "Daily: fn-replace-check / fn-replace-check did not complete"},
                                      {"number": 7, "title": "Daily: sunrise-world still::unseen"},
                                      {"number": 8, "title": "Something else"}], tag=True)
        text, out = self.run_summary(api, [shard_report(passed=[{"package": "sunrise-world", "test": "a::b"}])])
        self.assertIn("green=true", out)
        self.assertIn(("PATCH", "git/refs/tags/daily-green", {"sha": COMMIT, "force": True}), api.writes)
        closed = [path for method, path, body in api.writes if method == "PATCH" and path.startswith("issues/")]
        self.assertCountEqual(closed, ["issues/5", "issues/6"])
        self.assertFalse(any(path.startswith("issues/7") for _, path, _ in api.writes))
        self.assertIn("Move `daily-green`", text)

    def test_a_skipped_sdk_build_or_publish_is_fine_but_a_skipped_shard_is_not(self):
        jobs = [job for job in GREEN_JOBS if not job["name"].startswith("sdk")] + [
            job("sdk / fresh"), job("sdk / build linux", "skipped"), job("sdk / publish", "skipped")]
        _, out = self.run_summary(Api(jobs), [shard_report()])
        self.assertIn("green=true", out, "sdk-<sha> already exists, and a dry run publishes nothing")
        jobs = [job("shards / ignored-tests-shards (sunrise-world)", "skipped")] + GREEN_JOBS[2:]
        api = Api(jobs)
        _, out = self.run_summary(api, [])
        self.assertIn("green=false", out, "a skipped shard is missing coverage")

    def test_a_failed_sdk_build_is_red(self):
        jobs = [job for job in GREEN_JOBS if not job["name"].startswith("sdk")] + [
            job("sdk / fresh"), job("sdk / build windows", "failure", ["Build and collect"])]
        api = Api(jobs)
        _, out = self.run_summary(api, [shard_report()], results=dict(LEGS, sdk={"result": "failure"}))
        self.assertIn("green=false", out)
        self.assertEqual([body["title"] for _, path, body in api.writes if path == "issues"],
                         ["Daily: sdk / build windows did not complete"])

    def test_a_failed_probe_in_the_second_sdk_call_blocks_publication_and_names_the_step(self):
        # Both daily.yml calls are named `sdk`, including the Windows job in the second call.
        step = "Two mod libraries share the game's exact Rust state"
        jobs = [j for j in GREEN_JOBS if j["name"] != "sdk / build windows"] + [
            job("sdk / build windows", "failure", [step])]
        api = Api(jobs)
        _, out = self.run_summary(api, [shard_report()], results=dict(LEGS, **{"sdk-windows-macos": {"result": "failure"}}))
        self.assertEqual(out.splitlines(), ["green=false", "sdk_green=false"])
        created = [body for _, path, body in api.writes if path == "issues"]
        self.assertEqual([body["title"] for body in created], ["Daily: sdk native linkage probe (windows)"])
        self.assertIn(f"fails at the step `{step}`", created[0]["body"])
        self.assertNotIn("fn-replace-check", created[0]["body"], "fn-replace-check reads only the Linux legs")
        # A second call that was cancelled before any of its jobs ran leaves the sdk leg not green either.
        jobs = [job for job in GREEN_JOBS if not job["name"].startswith("sdk")] + [job("sdk / fresh"), job("sdk / build linux")]
        _, out = self.run_summary(Api(jobs), [shard_report()], results=dict(LEGS, **{"sdk-windows-macos": {"result": "cancelled"}}))
        self.assertIn("sdk_green=false", out)

    def test_a_linux_sdk_check_red_is_one_issue_and_its_readers_missing_their_input_file_none(self):
        probe = "Two mod libraries share the game's exact Rust state"
        readers = (summary.FN_REPLACE_JOB, summary.BOOK_QUICKSTART_JOB)
        jobs = [j for j in GREEN_JOBS if j["name"] not in ("sdk / build linux", *readers)] + [
            job("sdk / build linux", "failure", [probe]),
            job(summary.FN_REPLACE_JOB, "failure", ["The Linux legs are here"]),
            job(summary.BOOK_QUICKSTART_JOB, "failure", ["Run actions/download-artifact@v4", "The Linux legs are here"])]
        api = Api(jobs, issues=[{"number": 6, "title": "Daily: fn-replace-check / fn-replace-check did not complete"},
                                {"number": 7, "title": "Daily: book-quickstart / quickstart did not complete"}])
        _, out = self.run_summary(api, [shard_report()], results=dict(
            LEGS, sdk={"result": "failure"}, **{"fn-replace-check": {"result": "failure"},
                                                "book-quickstart": {"result": "failure"}}))
        self.assertEqual(out.splitlines(), ["green=false", "sdk_green=false"])
        created = [body for _, path, body in api.writes if path == "issues"]
        self.assertEqual([body["title"] for body in created], ["Daily: sdk native linkage probe (linux)"])
        self.assertIn("`fn-replace-check` and `book-quickstart` could not run: they need this leg's SDK",
                      created[0]["body"])
        self.assertFalse(any(path.startswith(("issues/6", "issues/7")) for _, path, _ in api.writes),
                         "a blocked reader neither comments on nor closes its own issue")

    def test_a_failed_linux_fresh_check_with_no_published_sdk_files_one_issue(self):
        # The Linux call's `fresh` failed, so no Linux build job ran and fn-replace-check found no input.
        linux = GREEN_JOBS.index(job("sdk / fresh"))
        jobs = [j for at, j in enumerate(GREEN_JOBS) if at != linux and j["name"] not in (
            "sdk / build linux", "sdk / build linux-server", summary.FN_REPLACE_JOB)] + [
            job("sdk / fresh", "failure", ["Is sdk-<sha> published?"]), job("sdk / build linux", "skipped"),
            job("sdk / build linux-server", "skipped"), job(summary.FN_REPLACE_JOB, "failure", ["The Linux legs are here"])]
        api = Api(jobs)
        _, out = self.run_summary(api, [shard_report()], results=dict(
            LEGS, sdk={"result": "failure"}, **{"fn-replace-check": {"result": "failure"}}))
        self.assertEqual(out.splitlines(), ["green=false", "sdk_green=false"])
        created = [body for _, path, body in api.writes if path == "issues"]
        self.assertEqual([body["title"] for body in created], ["Daily: sdk did not complete"])
        self.assertIn("`fn-replace-check` could not run: it needs this leg's SDK", created[0]["body"])

    def test_fn_replace_check_missing_its_input_from_a_green_linux_sdk_is_its_own_red(self):
        jobs = [j for j in GREEN_JOBS if j["name"] != summary.FN_REPLACE_JOB] + [
            job(summary.FN_REPLACE_JOB, "failure", ["The Linux legs are here"])]
        api = Api(jobs)
        _, out = self.run_summary(api, [shard_report()], results=dict(LEGS, **{"fn-replace-check": {"result": "failure"}}))
        self.assertEqual(out.splitlines(), ["green=false", "sdk_green=false"])
        self.assertEqual([body["title"] for _, path, body in api.writes if path == "issues"],
                         ["Daily: fn-replace-check / fn-replace-check did not complete"])

    def test_a_genuine_fn_replace_red_beside_a_linux_sdk_red_is_filed_too(self):
        jobs = [j for j in GREEN_JOBS if j["name"] not in ("sdk / build linux-server", summary.FN_REPLACE_JOB)] + [
            job("sdk / build linux-server", "failure", ["Build and collect"]),
            job(summary.FN_REPLACE_JOB, "failure", ["Refusals by name"])]
        api = Api(jobs)
        self.run_summary(api, [shard_report()], results=dict(
            LEGS, sdk={"result": "failure"}, **{"fn-replace-check": {"result": "failure"}}))
        self.assertCountEqual([body["title"] for _, path, body in api.writes if path == "issues"],
                              ["Daily: fn-replace-check / fn-replace-check did not complete",
                               "Daily: sdk / build linux-server did not complete"])

    def test_a_source_stage_failure_is_its_own_issue_and_holds_the_sdk(self):
        step = "The source stages with this commit's build data"
        jobs = [j for j in GREEN_JOBS if j["name"] != "sdk / build linux"] + [job("sdk / build linux", "failure", [step])]
        api = Api(jobs)
        _, out = self.run_summary(api, [shard_report()], results=dict(LEGS, sdk={"result": "failure"}))
        self.assertEqual(out.splitlines(), ["green=false", "sdk_green=false"])
        created = [body for _, path, body in api.writes if path == "issues"]
        self.assertEqual([body["title"] for body in created], ["Daily: sdk source stage (linux)"])
        self.assertIn(step, created[0]["body"])

    def test_every_sdk_check_is_a_step_of_the_sdk_build(self):
        workflow = (Path(__file__).resolve().parents[1] / ".github/workflows/prebuilt-game-build.yml").read_text()
        # A cross-built macOS or Windows leg repeats the checks in `cross-check`; the `build` job keeps each once.
        build = workflow.split("\n  build:\n", 1)[1].split("\n  cross-check:\n", 1)[0]
        for step in summary.SDK_CHECKS:
            self.assertEqual(build.count(f"- name: {step}\n"), 1, step)

    def test_an_sdk_check_issue_closes_only_when_its_step_passed(self):
        title = "Daily: sdk API-source check (linux)"
        step = "The starter mod checks against the API source alone"
        # sdk-<sha> already existed: the build skipped, so the check never ran and its issue stays open.
        jobs = [j for j in GREEN_JOBS if not j["name"].startswith("sdk / build ")] + [
            job(name, "skipped") for name in ("sdk / build linux", "sdk / build linux-server",
                                              "sdk / build windows", "sdk / build macos")]
        api = Api(jobs, issues=[{"number": 9, "title": title}])
        _, out = self.run_summary(api, [shard_report()])
        self.assertIn("green=true", out)
        self.assertFalse(any(path.startswith("issues/9") for _, path, _ in api.writes))
        ran = dict(job("sdk / build linux"))
        ran["steps"] = ran["steps"] + [{"name": step, "conclusion": "success"}]
        api = Api([j for j in GREEN_JOBS if j["name"] != "sdk / build linux"] + [ran], issues=[{"number": 9, "title": title}])
        self.run_summary(api, [shard_report()])
        self.assertIn("issues/9", [path for method, path, _ in api.writes if method == "PATCH"])

    def test_the_sdk_legs_alone_decide_sdk_green(self):
        jobs = [job("shards / ignored-tests-shards (sunrise-world)", "failure", ["Run"])] + GREEN_JOBS[2:]
        _, out = self.run_summary(Api(jobs), [], results=dict(LEGS, shards={"result": "failure"}))
        self.assertIn("green=false", out)
        self.assertIn("sdk_green=true", out, "a red shard holds the nightly, not the SDK")
        jobs = [j for j in GREEN_JOBS if not j["name"].startswith("machine-sample")]
        _, out = self.run_summary(Api(jobs), [shard_report()], results=dict(LEGS, **{"machine-sample": {"result": "skipped"}}))
        self.assertIn("sdk_green=false", out, "an SDK leg that never ran is never green")

    def test_the_tag_is_created_the_first_time(self):
        api = Api(GREEN_JOBS, tag=False)
        self.run_summary(api, [shard_report()])
        self.assertIn(("POST", "git/refs", {"ref": "refs/tags/daily-green", "sha": COMMIT}), api.writes)

    def test_a_failing_test_gets_one_issue_with_location_run_and_suspects(self):
        jobs = GREEN_JOBS[:1] + [job("shards / ignored-tests-shards (sunrise-world)", "failure",
                                     ["Run the ignored tests that need prepared data"])] + GREEN_JOBS[2:]
        api = Api(jobs)
        report = shard_report([failure("game::x::tests::broken"), failure("game::x::tests::broken")])
        _, out = self.run_summary(api, [report], results=dict(LEGS, shards={"result": "failure"}), merged=[12, 15])
        self.assertIn("green=false", out)
        created = [body for method, path, body in api.writes if (method, path) == ("POST", "issues")]
        self.assertEqual(len(created), 1, "one issue per failing test")
        issue = created[0]
        self.assertEqual(issue["title"], "Daily: sunrise-world game::x::tests::broken")
        self.assertEqual(issue["labels"], ["bug"])
        self.assertTrue(issue["body"].startswith(summary.DISCLOSURE + "\n"))
        for part in ("`crates/sunrise-world/src/lib.rs:7:3`", "https://github.com/arismoko/horizon/actions/runs/77",
                     "#12, #15"):
            self.assertIn(part, issue["body"])
        self.assertFalse(any("tags" in path for _, path, _ in api.writes), "a red run never moves the tag")

    def test_a_failed_windows_row_files_a_named_issue_and_a_passing_row_closes_its_issue(self):
        name = "windows-runtime-leases / runtime-leases"
        jobs = [item for item in GREEN_JOBS if item["name"] != name] + [
            job(name, "failure", ["The Windows-only daily rows"])]
        source = "crates/sunrise-prepare/tests/exporters.rs:8:3"
        report = {"windows_rows": True, "problems": [failure("broken", source, package="sunrise-prepare")],
                  "passed": [{"package": "sunrise-prepare", "test": "fixed"}], "incomplete": []}
        api = Api(jobs, issues=[{"number": 9, "title": "Daily: sunrise-prepare fixed"}])
        _, out = self.run_summary(api, [report], results=dict(LEGS, **{"windows-runtime-leases": {"result": "failure"}}),
                                  merged=[15])
        self.assertIn("green=false", out)
        created = [body for method, path, body in api.writes if (method, path) == ("POST", "issues")]
        self.assertEqual([body["title"] for body in created], ["Daily: sunrise-prepare broken"])
        for part in (f"`{source}`", "#15", f"`{name}`"):
            self.assertIn(part, created[0]["body"])
        self.assertIn(("PATCH", "issues/9", {"state": "closed", "state_reason": "completed"}), api.writes)

    def test_a_windows_build_failure_outside_the_rows_files_a_job_issue(self):
        name = "windows-runtime-leases / runtime-leases"
        jobs = [item for item in GREEN_JOBS if item["name"] != name] + [job(name, "failure", ["Build the actor"])]
        api = Api(jobs)
        _, out = self.run_summary(api, results=dict(LEGS, **{"windows-runtime-leases": {"result": "failure"}}))
        self.assertIn("green=false", out)
        created = [body for method, path, body in api.writes if (method, path) == ("POST", "issues")]
        self.assertEqual([body["title"] for body in created], [f"Daily: {name} did not complete"])
        self.assertIn("`Build the actor`", created[0]["body"])

    def test_a_windows_row_build_error_keeps_its_exit_status_in_the_job_issue(self):
        name = "windows-runtime-leases / runtime-leases"
        jobs = [item for item in GREEN_JOBS if item["name"] != name] + [
            job(name, "failure", ["The Windows-only daily rows"])]
        report = {"windows_rows": True, "problems": [], "passed": [], "incomplete": [
            {"shard": "sunrise-prepare", "reason": "failed without a failed test", "cargo_exit": 101}]}
        api = Api(jobs)
        self.run_summary(api, [report], results=dict(LEGS, **{"windows-runtime-leases": {"result": "failure"}}))
        created = [body for method, path, body in api.writes if (method, path) == ("POST", "issues")]
        self.assertEqual([body["title"] for body in created], [f"Daily: {name} did not complete"])
        self.assertIn("(cargo exit 101)", created[0]["body"])

    def test_an_open_issue_gets_a_comment_not_a_duplicate(self):
        jobs = [job("shards / ignored-tests-shards (sunrise-world)", "failure", ["Run"])] + GREEN_JOBS[2:]
        issues = [{"number": n, "title": f"filler {n}"} for n in range(150)]
        issues.append({"number": 900, "title": "Daily: sunrise-world game::x::tests::broken", "body": "> filed"})
        api = Api(jobs, issues=issues)
        self.run_summary(api, [shard_report([failure("game::x::tests::broken")])],
                         results=dict(LEGS, shards={"result": "failure"}))
        self.assertEqual([(m, p) for m, p, _ in api.writes], [("POST", "issues/900/comments")],
                         "found past the first page of open issues")
        self.assertTrue(api.writes[0][2]["body"].startswith("🤖 Agent: "))

    def test_a_red_whose_fix_merged_after_the_tested_commit_comments_on_the_closed_issue(self):
        jobs = [job("shards / ignored-tests-shards (sunrise-world)", "failure", ["Run"])] + GREEN_JOBS[2:]
        late, early = "f" * 40, "e" * 40
        issues = [{"number": 901, "title": "Daily: sunrise-world a::fixed_later", "state": "closed"},
                  {"number": 902, "title": "Daily: sunrise-world a::fixed_before", "state": "closed"},
                  {"number": 903, "title": "Daily: sunrise-world a::closed_by_hand", "state": "closed"}]
        api = Api(jobs, issues=issues)
        report = shard_report([failure("a::fixed_later"), failure("a::fixed_before"), failure("a::closed_by_hand")])
        self.run_summary(api, [report], results=dict(LEGS, shards={"result": "failure"}),
                         fixes={901: late, 902: early}, ancestors=(early,))
        comments = [(path, body["body"]) for method, path, body in api.writes if path.endswith("/comments")]
        self.assertEqual([path for path, _ in comments], ["issues/901/comments"])
        self.assertIn(f"on `{COMMIT[:9]}`, which predates the fix `{late[:9]}`", comments[0][1])
        self.assertFalse(any(path == "issues/901" for _, path, _ in api.writes), "it stays closed")
        # A fix the tested commit holds did not fix it, and a hand close names no fix: both are filed anew.
        self.assertCountEqual([body["title"] for _, path, body in api.writes if path == "issues"],
                              ["Daily: sunrise-world a::fixed_before", "Daily: sunrise-world a::closed_by_hand"])

    def test_a_leg_that_failed_without_test_names_gets_one_issue_per_leg(self):
        jobs = [job("shards / ignored-tests-shards (sunrise-world)", "cancelled", ["Prepare checkout-keyed vanilla data"]),
                job("fn-replace-check / fn-replace-check", "failure", ["Refusals by name"])] + GREEN_JOBS[4:]
        api = Api(jobs)
        self.run_summary(api, [], results=dict(LEGS, shards={"result": "cancelled"},
                                               **{"fn-replace-check": {"result": "failure"}}), tag=False)
        titles = sorted(body["title"] for method, path, body in api.writes if path == "issues")
        self.assertEqual(titles, ["Daily: fn-replace-check / fn-replace-check did not complete",
                                  "Daily: shards / ignored-tests-shards (sunrise-world) did not complete"])
        bodies = " ".join(body["body"] for _, path, body in api.writes if path == "issues")
        self.assertIn("`Refusals by name`", bodies)
        self.assertIn("no daily run has been green yet", bodies)

    def test_every_mod_tool_archive_green_is_green_and_one_that_fails_holds_the_nightly(self):
        _, out = self.run_summary(Api(GREEN_JOBS))
        self.assertEqual(out.splitlines(), ["green=true", "sdk_green=true"])
        failed = "cargo-sunrise-mod / build aarch64-apple-darwin"
        api = Api([job(failed, "failure", ["Build"]) if entry["name"] == failed else entry for entry in GREEN_JOBS])
        _, out = self.run_summary(api, results=dict(LEGS, **{"cargo-sunrise-mod": {"result": "failure"}}))
        self.assertEqual(out.splitlines(), ["green=false", "sdk_green=true"])
        self.assertEqual([body["title"] for _, path, body in api.writes if path == "issues"],
                         [f"Daily: {failed} did not complete"])

    def test_a_job_skipped_beside_another_leg_jobs_failure_is_still_its_own_issue(self):
        arm = "cargo-sunrise-mod / build aarch64-apple-darwin"
        windows = "cargo-sunrise-mod / build x86_64-pc-windows-msvc"
        jobs = [job(arm, "failure", ["Build tool"]) if entry["name"] == arm
                else job(windows, "skipped") if entry["name"] == windows else entry for entry in GREEN_JOBS]
        api = Api(jobs)
        _, out = self.run_summary(api, results=dict(LEGS, **{"cargo-sunrise-mod": {"result": "failure"}}))
        self.assertEqual(sorted(body["title"] for _, path, body in api.writes if path == "issues"),
                         [f"Daily: {arm} did not complete", f"Daily: {windows} did not complete"])
        self.assertEqual(out.splitlines(), ["green=false", "sdk_green=true"])

    def test_a_shard_killed_for_memory_says_by_what_with_its_memory_time_and_running_tests(self):
        stuck = [f"game::slow::case_{n}" for n in range(7)]
        incomplete = {"shard": "sunrise-world", "reason": "a test process crashed", "ended_by": "signal 9 (SIGKILL)",
                      "cargo_exit": 101, "wall_seconds": 4454, "memory_peak_bytes": 34359738368,
                      "memory_max_bytes": 34359738368, "oom_kills": 1, "still_running": stuck,
                      "still_running_more": 0, "never_reported": None}
        jobs = [job("shards / ignored-tests-shards (sunrise-world)", "failure",
                    ["Run the ignored tests that need prepared data"])] + GREEN_JOBS[2:]
        api = Api(jobs)
        self.run_summary(api, [shard_report(incomplete=[incomplete])], results=dict(LEGS, shards={"result": "failure"}))
        body = next(body["body"] for _, path, body in api.writes if path == "issues")
        self.assertIn("a test run did not complete: `sunrise-world` killed by signal 9 (SIGKILL), out of memory "
                      "(1 kill by the kernel); job memory peak 32.0 GiB of 32.0 GiB; after 74 min; still running past "
                      "60 s: `game::slow::case_0`, `game::slow::case_1`, `game::slow::case_2`, `game::slow::case_3`, "
                      "`game::slow::case_4` and 2 more.", body)

    def test_what_a_stopped_run_reports_is_shown_only_as_names_and_numbers(self):
        self.assertEqual(summary.stopped_text({"shard": "sunrise-client#2of2", "in_progress": True}),
                         "`sunrise-client#2of2` was still running when the leg ended (its time limit, a cancel, or a "
                         "lost runner); which tests were running and the process's end are not known")
        self.assertEqual(summary.stopped_text({"shard": "x", "reason": "listed tests never reported a result",
                                               "cargo_exit": 0, "wall_seconds": 30, "never_reported": 2}),
                         "`x`: listed tests never reported a result (cargo exit 0); after 30 s; "
                         "2 listed tests never reported")
        text = summary.stopped_text({"shard": "x`<b>", "reason": "PRIVATE: payload", "ended_by": "signal 9; rm",
                                     "still_running": ["ok::name", "not a name", "ghp_x y"], "wall_seconds": "9",
                                     "memory_peak_bytes": True, "oom_kills": -1})
        self.assertEqual(text, "a run did not complete; still running past 60 s: `ok::name`")
        self.assertEqual(summary.stopped_text({"shard": "x"}), "`x` did not complete")

    def test_every_shard_of_the_matrix_must_report_by_name(self):
        launcher = "shards / ignored-tests-shards (daily-rows)"
        self.assertIn(launcher, summary.expected_shards())
        _, out = self.run_summary(Api(GREEN_JOBS), [shard_report()])
        self.assertIn("green=true", out)
        # Every other job green, one shard absent: red, with one leg issue naming it.
        api = Api([entry for entry in GREEN_JOBS if entry["name"] != launcher])
        _, out = self.run_summary(api, [shard_report()])
        self.assertIn("green=false", out)
        self.assertEqual([body["title"] for _, path, body in api.writes if path == "issues"],
                         [f"Daily: {launcher} did not complete"])
        # A shard renamed in the workflow but not in the matrix's source: the expected name is missing.
        renamed = [job("shards / ignored-tests-shards (daily-rows-tools)") if entry["name"] == launcher else entry
                   for entry in GREEN_JOBS]
        _, out = self.run_summary(Api(renamed), [shard_report()])
        self.assertIn("green=false", out)

    def test_a_stopped_runs_test_names_are_bounded_in_length(self):
        long = "a" * 301
        text = summary.stopped_text({"shard": "x", "still_running": [long, "ok::name"]})
        self.assertEqual(text, "`x` did not complete; still running past 60 s: `ok::name`")
        self.assertIn("a" * 300, summary.stopped_text({"shard": "x", "still_running": ["a" * 300]}))

    def test_a_skipped_leg_with_no_jobs_is_not_green(self):
        api = Api([job for job in GREEN_JOBS if not job["name"].startswith("shards")])
        _, out = self.run_summary(api, [], results=dict(LEGS, shards={"result": "skipped"}))
        self.assertIn("green=false", out)
        self.assertIn("Daily: shards did not complete", [body["title"] for _, path, body in api.writes if path == "issues"])

    def test_a_failing_test_names_the_pull_requests_that_changed_its_package_since_the_previous_run(self):
        shard = "shards / ignored-tests-shards (sunrise-world)"
        jobs = [job(shard, "failure", ["Run"])] + GREEN_JOBS[2:]
        before, change, other = "b" * 40, "1" * 40, "2" * 40
        previous = {"id": 70, "conclusion": "failure", "html_url": "https://github.com/arismoko/horizon/actions/runs/70",
                    "jobs": [job(f"summary {before} (dry run)"), job(shard)]}

        def history(*args):
            if args[:2] == ("merge-base", "--is-ancestor"):
                return "" if args[2] == before else None
            if args[0] == "rev-parse":
                return None
            if args[:2] == ("log", "--no-merges"):
                self.assertEqual(args[3:], (f"{before}..{COMMIT}", "--", "crates/sunrise-world"))
                return f"{change}\n{other}"
            if args[:2] == ("log", "--merges"):
                # Each change's merges on its way in, newest first: its own merge is the oldest naming a PR.
                return {change: "Merge pull request #40 from org/rollup\nMerge #31 at 1234",
                        other: "Merge pull request #41 from org/fix"}[args[4].split("..")[0]]
            return ""

        def run(runs):
            api = Api(jobs, runs=runs)
            env = {"GITHUB_REPOSITORY": "arismoko/horizon", "GITHUB_RUN_ID": "77", "GITHUB_RUN_ATTEMPT": "1", "GITHUB_REF": "refs/heads/main",
                   "GITHUB_SERVER_URL": "https://github.com", "COMMIT": COMMIT, "DRY_RUN": "false",
                   "RESULTS": json.dumps(dict(LEGS, shards={"result": "failure"})), "REPORTS": "/nonexistent"}
            with tempfile.TemporaryDirectory() as folder, \
                    mock.patch.dict(os.environ, dict(env, REPORTS=folder), clear=True), \
                    contextlib.redirect_stdout(io.StringIO()):
                Path(folder, "shard").mkdir()
                Path(folder, "shard", "r.json").write_text(json.dumps(shard_report([failure("a::red")])))
                summary.main(api, history, closers({}))
            return next(body["body"] for _, path, body in api.writes if path == "issues")

        body = run({70: previous})
        self.assertIn(f"it passed in the previous daily run (https://github.com/arismoko/horizon/actions/runs/70, "
                      f"`{before[:9]}`)", body)
        self.assertIn("that changed `crates/sunrise-world`: #31, #41.", body)
        # Its job failed there: nothing says the test passed, so earlier merges stay suspects.
        previous["jobs"][1] = job(shard, "failure")
        body = run({70: previous})
        self.assertIn(f"does not show this test passing (its job `{shard}` ended `failure`)", body)
        # No earlier run: the old line.
        self.assertIn("every merge before", run({}))

    def test_a_package_built_from_another_folder_counts_that_folder(self):
        self.assertEqual(summary.package_folders("sunrise-client-lib"),
                         ["crates/sunrise-client", "crates/sunrise-client-lib"])
        self.assertEqual(summary.package_folders("sunrise-world"), ["crates/sunrise-world"])

    def test_a_second_invocation_for_one_run_posts_nothing_twice(self):
        jobs = [job("shards / ignored-tests-shards (sunrise-world)", "failure", ["Run"])] + GREEN_JOBS[2:]
        red = dict(LEGS, shards={"result": "failure"})
        issues = [{"number": 900, "title": "Daily: sunrise-world old::red", "body": "> filed earlier"},
                  {"number": 901, "title": "Daily: sunrise-world was::red", "body": "> filed earlier"}]
        report = shard_report([failure("old::red"), failure("new::red")],
                              passed=[{"package": "sunrise-world", "test": "was::red"}])
        api = Api(jobs, issues=issues)
        self.run_summary(api, [report], results=red)
        first = list(api.writes)
        self.assertEqual(sorted((m, p) for m, p, _ in first),
                         [("PATCH", "issues/901"), ("POST", "issues"), ("POST", "issues/900/comments"),
                          ("POST", "issues/901/comments")])
        api.writes.clear()
        # The rerun sees the issue it filed (its body names this run) and the comments it posted.
        self.run_summary(api, [report], results=red)
        self.assertEqual([(m, p) for m, p, _ in api.writes], [("PATCH", "issues/901")],
                         "only the idempotent close is repeated")

    def test_a_partial_first_application_completes_on_the_second(self):
        jobs = [job("shards / ignored-tests-shards (sunrise-world)", "failure", ["Run"])] + GREEN_JOBS[2:]
        run_url = "https://github.com/arismoko/horizon/actions/runs/77"
        # The first attempt filed the new issue, then stopped before commenting on the old one.
        issues = [{"number": 900, "title": "Daily: sunrise-world old::red", "body": "> filed earlier"},
                  {"number": 950, "title": "Daily: sunrise-world new::red", "body": f"> filed\nFirst daily run that saw it: {run_url} on `ccc`."}]
        api = Api(jobs, issues=issues)
        self.run_summary(api, [shard_report([failure("old::red"), failure("new::red")])],
                         results=dict(LEGS, shards={"result": "failure"}))
        self.assertEqual([(m, p) for m, p, _ in api.writes], [("POST", "issues/900/comments")])

    def test_a_failed_helper_files_one_leg_issue_naming_it(self):
        for helper, leg, skipped in (("sdk / fresh", "sdk", ["sdk / build linux"]),
                                     ("fn-replace-check / route", "fn-replace-check", ["fn-replace-check / fn-replace-check"])):
            with self.subTest(helper=helper):
                jobs = [j for j in GREEN_JOBS if not j["name"].startswith(leg + " / ")]
                jobs += [job(helper, "failure", ["Set up job"])] + [job(name, "skipped") for name in skipped]
                api = Api(jobs)
                _, out = self.run_summary(api, [shard_report()], results=dict(LEGS, **{leg: {"result": "failure"}}))
                self.assertIn("green=false", out)
                created = [body for _, path, body in api.writes if path == "issues"]
                self.assertEqual([body["title"] for body in created], [f"Daily: {leg} did not complete"])
                self.assertIn(f"`{helper}` failed", created[0]["body"])

    def test_a_red_machine_sample_files_one_action_issue_naming_the_step_and_closes_when_green(self):
        step = "The machine sample builds against this API"
        jobs = [j for j in GREEN_JOBS if j["name"] != "machine-sample / build"] + [
            job("machine-sample / build", "failure", [step])]
        api = Api(jobs)
        _, out = self.run_summary(api, [shard_report()], results=dict(LEGS, **{"machine-sample": {"result": "failure"}}),
                                  merged=[4100, 4101])
        self.assertIn("green=false", out)
        created = [body for _, path, body in api.writes if path == "issues"]
        self.assertEqual([body["title"] for body in created], ["Daily: update the machine sample for the mod API change"])
        self.assertEqual(created[0]["labels"], ["bug"])
        self.assertIn(f"`{step}`", created[0]["body"])
        self.assertIn("#4100, #4101", created[0]["body"])
        self.assertIn("check_machine_sample.py", created[0]["body"])
        # Still red the next day: a comment on the same issue. Green: closed.
        issue = [{"number": 77, "title": "Daily: update the machine sample for the mod API change", "body": "> filed"}]
        api = Api(jobs, issues=issue)
        self.run_summary(api, [shard_report()], results=dict(LEGS, **{"machine-sample": {"result": "failure"}}))
        self.assertEqual([(m, p) for m, p, _ in api.writes], [("POST", "issues/77/comments")])
        api = Api(GREEN_JOBS, issues=issue)
        _, out = self.run_summary(api, [shard_report()])
        self.assertIn("green=true", out)
        self.assertIn(("PATCH", "issues/77"), [(m, p) for m, p, _ in api.writes])

    def test_a_leg_cancelled_before_it_started_files_and_closes_nothing(self):
        # Run 43900023409: every leg job still waited for a runner when the cancel came, with no step run.
        def waiting(name):
            return {"name": name, "conclusion": "cancelled", "steps": []}
        jobs = [waiting(j["name"]) if j["name"].startswith("machine-sample / ") else j for j in GREEN_JOBS]
        issue = [{"number": 77, "title": "Daily: update the machine sample for the mod API change", "body": "> filed"}]
        api = Api(jobs, issues=issue)
        _, out = self.run_summary(api, [shard_report()], results=dict(LEGS, **{"machine-sample": {"result": "cancelled"}}))
        self.assertIn("green=false", out, "a leg that never ran is not green either")
        self.assertEqual(api.writes, [])
        # A leg cancelled after it started (a time limit, a lost runner) did run: it is still red.
        jobs = [j for j in GREEN_JOBS if j["name"] != "machine-sample / build"] + [
            job("machine-sample / build", "cancelled", ["The machine sample builds against this API"])]
        api = Api(jobs)
        self.run_summary(api, [shard_report()], results=dict(LEGS, **{"machine-sample": {"result": "cancelled"}}))
        self.assertEqual([body["title"] for _, path, body in api.writes if path == "issues"],
                         ["Daily: update the machine sample for the mod API change"])

    def test_a_skipped_sdk_build_is_green_only_when_fresh_found_the_release(self):
        jobs = [j for j in GREEN_JOBS if not j["name"].startswith("sdk / ")] + [
            job("sdk / fresh", "cancelled"), job("sdk / build linux", "skipped")]
        api = Api(jobs)
        _, out = self.run_summary(api, [shard_report()], results=dict(LEGS, sdk={"result": "cancelled"}))
        self.assertIn("green=false", out)
        self.assertEqual([body["title"] for _, path, body in api.writes if path == "issues"],
                         ["Daily: sdk did not complete"])

    def test_a_rerun_that_turns_a_test_green_still_posts_its_close_comment(self):
        red_jobs = [job("shards / ignored-tests-shards (sunrise-world)", "failure", ["Run"])] + GREEN_JOBS[2:]
        issues = [{"number": 900, "title": "Daily: sunrise-world flaky::one", "body": "> filed earlier"}]
        api = Api(red_jobs, issues=issues)
        self.run_summary(api, [shard_report([failure("flaky::one")])], results=dict(LEGS, shards={"result": "failure"}))
        self.assertEqual([(m, p) for m, p, _ in api.writes], [("POST", "issues/900/comments")])
        api.writes.clear()
        api.jobs = GREEN_JOBS
        self.run_summary(api, [shard_report(passed=[{"package": "sunrise-world", "test": "flaky::one"}])])
        self.assertEqual([(m, p) for m, p, _ in api.writes if p.startswith("issues/")],
                         [("POST", "issues/900/comments"), ("PATCH", "issues/900")],
                         "the red comment of this run does not hide its green-again comment")
        comment = next(body for _, path, body in api.writes if path == "issues/900/comments")
        self.assertIn("Green again in the daily run", comment["body"])

    def test_a_leg_level_issue_closes_once_the_leg_passes_whole(self):
        failed = [j for j in GREEN_JOBS if not j["name"].startswith("sdk / ")] + [
            job("sdk / fresh", "failure", ["Set up job"]), job("sdk / build linux", "skipped")]
        api = Api(failed)
        self.run_summary(api, [shard_report()], results=dict(LEGS, sdk={"result": "failure"}))
        created = [body["title"] for _, path, body in api.writes if path == "issues"]
        self.assertEqual(created, ["Daily: sdk did not complete"])
        number = api.issues[-1]["number"]
        api.writes.clear()
        api.jobs = [j for j in GREEN_JOBS if not j["name"].startswith("sdk / ")]
        self.run_summary(api, [shard_report()], results=dict(LEGS, sdk={"result": "success"}))
        self.assertNotIn(("PATCH", f"issues/{number}"), [(m, p) for m, p, _ in api.writes],
                         "a run where the leg did not report leaves it open")
        api.writes.clear()
        api.jobs = GREEN_JOBS
        self.run_summary(api, [shard_report()])
        self.assertIn(("PATCH", f"issues/{number}"), [(m, p) for m, p, _ in api.writes])
        comment = next(body for _, path, body in api.writes if path == f"issues/{number}/comments")
        self.assertIn("Green again in the daily run", comment["body"])

    def test_an_unknown_job_or_a_leg_with_no_job_is_red(self):
        for jobs, title in (
                (GREEN_JOBS + [job("surprise / something")], "Daily: surprise / something did not complete"),
                ([j for j in GREEN_JOBS if not j["name"].startswith("fn-replace-check / ")],
                 "Daily: fn-replace-check did not complete")):
            with self.subTest(title=title):
                api = Api(jobs)
                _, out = self.run_summary(api, [shard_report()])
                self.assertIn("green=false", out, "never silently green")
                self.assertEqual([body["title"] for _, path, body in api.writes if path == "issues"], [title])
                self.assertIn("did not report", api.writes[0][2]["body"])

    def test_seen_finds_a_completed_real_run_by_its_summary_job(self):
        sha = "e" * 40
        runs = {5: {"id": 5, "conclusion": "failure", "html_url": "u5", "jobs": [job(f"summary {sha}", "failure")]},
                6: {"id": 6, "conclusion": "success", "html_url": "u6", "jobs": [job(f"summary {sha} (dry run)")]},
                7: {"id": 7, "conclusion": "cancelled", "html_url": "u7", "jobs": [job(f"summary {sha}")]}}
        with mock.patch.dict(os.environ, {"GITHUB_RUN_ID": "77", "GITHUB_REPOSITORY": "arismoko/horizon"}):
            self.assertEqual(summary.seen(sha, Api([], runs=runs)), "u5", "a failed real run counts")
            del runs[5]
            self.assertEqual(summary.seen(sha, Api([], runs=runs)), "", "dry and cancelled runs do not")

    def test_nothing_is_written_off_main_or_in_a_dry_run(self):
        jobs = [job("shards / ignored-tests-shards (sunrise-world)", "failure", ["Run"])] + GREEN_JOBS[2:]
        for ref, dry_run in (("refs/heads/hotspots-daily", "false"), ("refs/heads/main", "true")):
            with self.subTest(ref=ref, dry_run=dry_run):
                api = Api(jobs)
                text, _ = self.run_summary(api, [shard_report([failure("a::b")])], ref=ref, dry_run=dry_run,
                                           results=dict(LEGS, shards={"result": "failure"}))
                self.assertEqual(api.writes, [])
                self.assertIn("(dry run: nothing written)", text)
                self.assertIn("File **Daily: sunrise-world a::b**", text)

    def test_a_failed_lookup_writes_nothing(self):
        jobs = [job("shards / ignored-tests-shards (sunrise-world)", "failure", ["Run"])] + GREEN_JOBS[2:]
        api = Api(jobs, fail_on="issues?")
        with self.assertRaises(SystemExit):
            self.run_summary(api, [shard_report([failure("a::b")])], results=dict(LEGS, shards={"result": "failure"}))
        self.assertEqual(api.writes, [])


class DailyWorkflowTests(unittest.TestCase):
    def setUp(self):
        from tests.test_ci_concurrency import ROOT
        self.workflow = yaml.load((ROOT / ".github/workflows/daily.yml").read_text(), Loader=yaml.BaseLoader)

    def test_the_schedule_only_dispatches_a_real_run_on_main(self):
        jobs = self.workflow["jobs"]
        self.assertEqual(self.workflow["on"]["schedule"], [{"cron": "0 9 * * *"}])
        self.assertIn("github.event_name == 'schedule'", jobs["schedule-dispatch"]["if"])
        self.assertEqual(jobs["schedule-dispatch"]["runs-on"], "ubuntu-latest")
        self.assertIn("--ref main -f dry_run=false -f scheduled=true", jobs["schedule-dispatch"]["steps"][0]["run"])
        resolve = jobs["resolve"]["steps"][-1]
        self.assertIn('if [ "$SCHEDULED" = true ] && [ "$DRY_RUN" = false ]', resolve["run"],
                      "only the schedule stops on a commit already tested; a hand dispatch retries")
        self.assertEqual(self.workflow["on"]["workflow_dispatch"]["inputs"]["scheduled"]["default"], "false")
        self.assertEqual(jobs["resolve"]["if"], "github.event_name == 'workflow_dispatch'")
        self.assertEqual(self.workflow["on"]["workflow_dispatch"]["inputs"]["dry_run"]["default"], "true")

    def test_only_main_without_dry_run_writes(self):
        jobs = self.workflow["jobs"]
        step = jobs["resolve"]["steps"][-1]
        # Exactly main or release/26.3, dispatched in this repository, decide it (release_train.py authorize).
        self.assertEqual(step["env"]["DRY_RUN"], "${{ steps.auth.outputs.dry_run }}")
        self.assertIn("authorize --publish-kind daily", jobs["resolve"]["steps"][-2]["run"])
        # A writing run takes only a commit of its branch: an unmerged commit is never tagged, signed or published.
        self.assertIn('git merge-base --is-ancestor "$sha" "refs/remotes/origin/$branch"', step["run"])
        self.assertEqual(jobs["summary"]["steps"][-1]["env"]["DRY_RUN"], "${{ needs.resolve.outputs.dry_run }}")
        self.assertEqual(jobs["nightly"]["if"],
                         "!cancelled() && needs.green.result == 'success' && needs.resolve.outputs.branch == 'main'"
                         " && needs.nightly-branch.result == 'success'")
        self.assertEqual(self.workflow["concurrency"]["cancel-in-progress"], "false")
        self.assertEqual(jobs["sdk"]["with"]["commit"], "${{ needs.resolve.outputs.commit }}")
        self.assertNotIn("permissions", jobs["sdk"], "the build leg publishes nothing")
        # The SDK is two calls named `sdk`: the Linux legs, which fn-replace-check needs alone, and the others.
        self.assertEqual(jobs["sdk"]["with"]["legs"], "linux")
        # Only this call runs the C-source test on Windows and macOS; a Dev SDK build never does.
        self.assertEqual(jobs["sdk-windows-macos"]["with"], {"commit": "${{ needs.resolve.outputs.commit }}",
                                                             "legs": "windows-macos", "c_source_test": "true"})
        self.assertEqual(jobs["sdk-windows-macos"]["name"], "sdk")
        self.assertEqual(summary.PART_OF, {"sdk-windows-macos": "sdk"})
        self.assertEqual(jobs["fn-replace-check"]["needs"], ["resolve", "sdk"])
        # The book's quickstart is an ordinary leg that tests this run's own SDK, never the previous Nightly's.
        self.assertEqual(jobs["book-quickstart"]["needs"], ["resolve", "sdk"])
        self.assertEqual(jobs["book-quickstart"]["with"]["own_sdk"], "true")
        self.assertIn("sdk-windows-macos", jobs["summary"]["needs"])
        publish = jobs["sdk-publish"]
        self.assertEqual(publish["needs"], ["resolve", "summary", "sdk", "sdk-windows-macos"])
        self.assertEqual(publish["if"], "!cancelled() && needs.resolve.result == 'success' && needs.summary.result == 'success'"
                                        " && needs.sdk.result == 'success' && needs.sdk-windows-macos.result == 'success'"
                                        " && needs.summary.outputs.sdk_green == 'true'"
                                        " && needs.resolve.outputs.dry_run == 'false' && needs.sdk.outputs.needed == 'true'")
        self.assertEqual(jobs["summary"]["outputs"]["sdk_green"], "${{ steps.summary.outputs.sdk_green }}")
        self.assertIn("needs.summary.outputs.sdk_green == 'true'", jobs["sdk-publish-report"]["if"])
        for name in ("summary", "sdk-publish-report"):
            self.assertTrue(jobs[name]["if"].startswith("${{ !cancelled() && "),
                            f"{name} reports nothing from a run cancelled by hand")
        # The one publish step, signed only on main through the called workflow's `release` environment.
        self.assertEqual(publish["uses"], "./.github/workflows/sdk-publish.yml")
        self.assertEqual(publish["with"], {"commit": "${{ needs.resolve.outputs.commit }}", "publish": "true"})
        # The nightly waits for the SDK through `green`, which needs it published (or already out).
        self.assertIn("sdk-publish", jobs["green"]["needs"])
        self.assertIn("green", jobs["nightly"]["needs"])
        report = jobs["sdk-publish-report"]
        self.assertIn("sdk-publish", report["needs"])
        self.assertIn("daily_summary.py --after sdk-publish", report["steps"][-1]["run"])
        self.assertEqual(report["steps"][-1]["env"]["DRY_RUN"], "${{ needs.resolve.outputs.dry_run }}")

        self.assertEqual(jobs["nightly"]["with"]["daily"], "true")
        writer = jobs["summary"]["steps"][-1]
        self.assertEqual(writer["run"], "python3 tools/ci/daily_summary.py")
        self.assertEqual(writer["env"]["DRY_RUN"], "${{ needs.resolve.outputs.dry_run }}")
        self.assertTrue(jobs["summary"]["name"].startswith("summary ${{ needs.resolve.outputs.commit }}"))

    def test_daily_checks_linkage_on_the_linux_and_windows_client_sdks_before_pack(self):
        from tests.test_ci_concurrency import ROOT, expression_value
        jobs = self.workflow["jobs"]
        self.assertNotIn("linkage", jobs)
        self.assertNotIn("linkage", jobs["summary"]["needs"])
        sdk = "./.github/workflows/prebuilt-game-build.yml"
        self.assertEqual(jobs["sdk"]["uses"], sdk)
        self.assertEqual(jobs["sdk-windows-macos"]["uses"], sdk)
        workflow = yaml.load((ROOT / sdk).read_text(), Loader=yaml.BaseLoader)
        self.assertEqual(set(workflow["on"]), {"workflow_call"})
        legs = next(step for step in workflow["jobs"]["fresh"]["steps"] if step.get("id") == "legs")
        steps = workflow["jobs"]["build"]["steps"]
        toolchain = next(step for step in steps if step.get("uses", "").startswith("dtolnay/rust-toolchain@"))
        probe = next(step for step in steps if step.get("name") == "Two mod libraries share the game's exact Rust state")
        api = next(step for step in steps if step.get("name") ==
                   "The starter mod checks against the API source alone")
        for leg in json.loads(legs["env"]["ALL"]):
            with self.subTest(leg=leg["name"]):
                wants_probe = leg["name"] in ("linux", "windows")
                self.assertEqual(bool(expression_value(probe["if"], {}, matrix=leg)), wants_probe)
                self.assertEqual(expression_value(toolchain["with"]["components"][3:-2], {}, matrix=leg),
                                 "clippy" if wants_probe else "")
                self.assertEqual(bool(expression_value(api["if"], {}, matrix=leg)), leg["name"] == "linux")
        self.assertEqual(shlex.split(probe["run"]), ["python", "tools/native_dylib_probe.py", "--sdk", "prebuilt",
                                                     "--output", "$RUNNER_TEMP/native-dylib-probe"])
        commands = {step["run"].split()[2]: i for i, step in enumerate(steps)
                    if step.get("run", "").startswith("python tools/prebuilt_game.py ")}
        self.assertLess(max(commands[command] for command in ("build", "smoke", "start", "exports")), steps.index(probe))
        self.assertLess(steps.index(probe), steps.index(api))
        self.assertLess(steps.index(api), commands["pack"])
        api_commands = api["run"].splitlines()
        build = 'python tools/sdk_api_source.py build "$RUNNER_TEMP/api-source"'
        check = ('python tools/sdk_api_source.py check "$RUNNER_TEMP/api-source" '
                 '--scratch "$RUNNER_TEMP/sdk-api-source-check"')
        self.assertLess(api_commands.index(build), api_commands.index(check))

    def test_the_sdk_build_packs_exactly_the_archives_a_release_needs(self):
        from tests.test_ci_concurrency import ROOT
        from tools.ci import nightly
        workflow = yaml.load((ROOT / ".github/workflows/prebuilt-game-build.yml").read_text(), Loader=yaml.BaseLoader)
        build = workflow["jobs"]["build"]
        # Each leg packs its three slices and a client leg its target's tool; build data and surface are gathered once.
        choose = next(step for step in workflow["jobs"]["fresh"]["steps"] if step.get("id") == "legs")
        legs = [{key: str(value).lower() for key, value in leg.items()} for leg in json.loads(choose["env"]["ALL"])]
        self.assertEqual(build["strategy"]["matrix"], "${{ fromJSON(needs.fresh.outputs.matrix) }}")
        names = {f"{kind}-{entry['target']}{'-server' if entry.get('server') == 'true' else ''}.tar.xz"
                 for entry in legs for kind in ("runtime", "api", "engine")}
        names |= {f"tool-{entry['target']}.tar.xz" for entry in legs if entry.get("server") != "true"}
        self.assertEqual(names | {"api-source.tar.xz", "build-data.tar.xz", "surface.json"}, set(nightly.SDK_ARCHIVES))
        self.assertEqual(len(nightly.SDK_ARCHIVES), len(set(nightly.SDK_ARCHIVES)))
        steps = {step.get("name") or step.get("uses"): step for step in build["steps"]}
        self.assertEqual(steps["Pack the SDK's assets"]["run"],
                         "python tools/prebuilt_game.py pack prebuilt dist --stage-tool "
                         "target/prebuilt/tools/release/sunrise-stage${{ runner.os == 'Windows' && '.exe' || '' }}")
        self.assertEqual(steps["actions/upload-artifact@v4"]["with"]["name"], "sdk-leg-${{ matrix.name }}")
        self.assertEqual(set(build.get("env", {})), {"CROSS", "SUNRISE_PREBUILT_TARGET"}, "no hand-set archive name")

        self.assertEqual(set(nightly.SDK_ASSETS) - set(nightly.SDK_ARCHIVES), {"sdk.json", "SHA256SUMS", "SHA256SUMS.sig"})

    def test_the_sdk_is_signed_and_published_only_on_main(self):
        from tests.test_ci_concurrency import ROOT
        workflow = yaml.load((ROOT / ".github/workflows/sdk-publish.yml").read_text(), Loader=yaml.BaseLoader)
        jobs = workflow["jobs"]
        on_main = "needs.authorize.outputs.publish == 'true'"
        self.assertEqual(jobs["sign"]["environment"], f"${{{{ {on_main} && 'release' || '' }}}}")
        sign = jobs["sign"]["steps"][-3]
        self.assertEqual(sign["run"], 'bash tools/ci/sign_release.sh "$TAG" dist/*')
        self.assertEqual(sign["env"]["EPHEMERAL"], f"${{{{ !({on_main}) && 'yes' || '' }}}}")
        for key in ("RELEASE_SIGNING_KEY", "RELEASE_SIGNING_KEY_NEXT"):
            self.assertEqual(sign["env"][key], f"${{{{ {on_main} && secrets.{key} || '' }}}}")
        self.assertEqual(workflow["env"]["TAG"], "sdk-${{ inputs.commit }}")
        # Only the sign job sees the key, and it checks out nothing but the signing script; the key signs only a
        # commit of main, whatever commit the caller names.
        self.assertEqual(jobs["sign"]["steps"][0]["with"]["sparse-checkout"], "tools/ci/sign_release.sh")
        on_main_check = jobs["sign"]["steps"][1]
        self.assertEqual(on_main_check["if"], f"${{{{ {on_main} }}}}")
        self.assertIn('git merge-base --is-ancestor "$COMMIT" "refs/remotes/origin/$GITHUB_REF_NAME"', on_main_check["run"])
        # The jobs that read the legs' output cannot write; the legs' artifacts only, not an earlier attempt's own.
        for job in ("index", "sign"):
            self.assertEqual(jobs[job]["permissions"], {"contents": "read"}, job)
        self.assertEqual(jobs["publish"]["permissions"], {"contents": "write"})
        self.assertEqual(jobs["index"]["steps"][4]["with"]["pattern"], "sdk-leg-*")
        self.assertNotIn("environment", jobs["index"])
        self.assertNotIn("environment", jobs["publish"])
        publish = {step.get("name"): step for step in jobs["publish"]["steps"]}
        self.assertEqual(jobs["publish"]["env"]["PUBLISH"], "${{ needs.authorize.outputs.publish }}")
        self.assertEqual(jobs["authorize"]["steps"][-1]["env"]["DRY_INPUT"], "${{ !inputs.publish }}")
        self.assertEqual(publish["Release sdk-<sha>"]["if"], "env.PUBLISH == 'true'")
        order = [step.get("name") for step in jobs["publish"]["steps"]]
        self.assertLess(order.index("The signature, checked as the launcher checks it"), order.index("Release sdk-<sha>"))
        self.assertLess(order.index("Exactly the assets a Nightly needs"), order.index("Release sdk-<sha>"))
        hand = yaml.load((ROOT / ".github/workflows/prebuilt-game.yml").read_text(), Loader=yaml.BaseLoader)
        self.assertEqual(hand["jobs"]["publish"]["with"]["publish"], "${{ github.ref == 'refs/heads/main' }}")

    def test_the_windows_report_is_uploaded_even_on_failure_and_downloaded_by_the_summary(self):
        import fnmatch

        from tests.test_ci_concurrency import ROOT
        daily_job, windows_job = summary.WINDOWS_ROWS_JOB.split(" / ")
        workflow_path = ROOT / self.workflow["jobs"][daily_job]["uses"]
        workflow = yaml.safe_load(workflow_path.read_text())
        steps = workflow["jobs"][windows_job]["steps"]
        upload = next(step for step in steps if step.get("uses", "").startswith("actions/upload-artifact@"))
        self.assertEqual(upload["if"], "always()")
        filename = upload["with"]["path"].removeprefix("${{ runner.temp }}/")
        self.assertTrue(filename.endswith(".json"))
        run = next(step for step in steps if step.get("name") == summary.WINDOWS_ROWS_STEP)
        self.assertIn(f'--report "$RUNNER_TEMP/{filename}"', run["run"])
        download = next(step for step in self.workflow["jobs"]["summary"]["steps"]
                        if step.get("uses", "").startswith("actions/download-artifact@"))
        self.assertTrue(fnmatch.fnmatchcase(upload["with"]["name"], download["with"]["pattern"]))

    def test_parity_holds_the_nightly_through_the_summary_but_not_sdk_publish(self):
        jobs = self.workflow["jobs"]
        parity = jobs["parity"]
        self.assertEqual(parity["uses"], "./.github/workflows/parity.yml")
        self.assertEqual(parity["with"], {"commit": "${{ needs.resolve.outputs.commit }}"})
        self.assertIn("shards", parity["needs"])
        self.assertIn("parity", jobs["summary"]["needs"])
        self.assertIn("sdk-windows-macos", jobs["summary"]["needs"])
        self.assertIn("!cancelled()", jobs["summary"]["if"])
        self.assertIn("needs.summary.outputs.green == 'true'", jobs["green"]["if"])
        self.assertIn("needs.parity.result == 'success'", jobs["green"]["if"])
        self.assertIn("needs.green.result == 'success'", jobs["nightly"]["if"])
        self.assertIn("needs.summary.outputs.sdk_green == 'true'", jobs["sdk-publish"]["if"])
        self.assertNotIn("parity-report", jobs)

    def test_the_parity_call_runs_when_the_resolve_job_requires_it(self):
        jobs = self.workflow["jobs"]
        self.assertEqual(jobs["resolve"]["outputs"]["parity_required"], "${{ steps.commit.outputs.parity_required }}")
        self.assertIn("needs.resolve.outputs.parity_required == 'true'", jobs["parity"]["if"])

    def resolve_commit(self, folder, ref, source, commit, *, dry_input="false", scheduled="false", break_glass="false"):
        steps = self.workflow["jobs"]["resolve"]["steps"]
        auth = run_gate(steps[-2]["run"], DRY_INPUT=dry_input, GITHUB_REF=ref)
        out = Path(folder, "out")
        out.write_text("")
        env = {**os.environ, "COMMIT": commit, "DRY_RUN": auth["dry_run"], "SCHEDULED": scheduled,
               "BREAK_GLASS": break_glass, "GITHUB_SHA": source, "GITHUB_REF": ref, "GITHUB_OUTPUT": str(out),
               "GITHUB_STEP_SUMMARY": str(Path(folder, "summary")), "GIT_ALLOW_PROTOCOL": "file"}
        result = subprocess.run(["bash", "-e", "-c", steps[-1]["run"]], cwd=Path(folder, "clone"), env=env,
                                capture_output=True, text=True)
        return result, out.read_text()

    @needs_runner_shell
    def test_every_writing_run_requires_parity_and_only_a_dry_run_leaves_it_out(self):
        with tempfile.TemporaryDirectory() as folder:
            git, (older, source, later) = repository(folder)
            git("push", "-q", "origin", f"{later}:{RELEASE}")
            for ref, dispatch, tested, dry_input, writes in (
                    (MAIN, later, later, "false", True), (MAIN, later, older, "false", True),
                    (RELEASE, later, later, "false", True), (RELEASE, later, older, "false", True),
                    (MAIN, source, later, "true", False), (NIGHTLY, later, later, "false", False),
                    ("refs/heads/topic", later, later, "false", False)):
                with self.subTest(ref=ref, dispatch=dispatch, tested=tested, dry_input=dry_input):
                    result, out = self.resolve_commit(folder, ref, dispatch, tested[:12], dry_input=dry_input)
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    self.assertEqual(dict(line.split("=", 1) for line in out.splitlines()),
                                     {"commit": tested, "skip": "false", "dry_run": str(not writes).lower(),
                                      "parity_required": str(writes).lower(),
                                      "branch": ref.removeprefix("refs/heads/") if writes else ""})

    @needs_runner_shell
    def test_writing_resolution_refuses_commits_outside_the_dispatch_graph(self):
        with tempfile.TemporaryDirectory() as folder:
            git, (_, source, later) = repository(folder)
            git("push", "-q", "origin", f"{later}:{RELEASE}")
            # The checkout and both live branches contain the later commit; the immutable dispatch SHA does not.
            # Neither --seen nor break-glass-legs can run here: these helpers aren't in the scratch checkout.
            for ref, scheduled, break_glass in ((MAIN, "true", "false"), (RELEASE, "false", "true")):
                with self.subTest(ref=ref):
                    result, out = self.resolve_commit(folder, ref, source, later, scheduled=scheduled,
                                                      break_glass=break_glass)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(f"::error::{later} is not in the dispatched workflow's history ({source})", result.stdout)
                    self.assertEqual(result.stderr, "")
                    self.assertEqual(out, "")

    @needs_runner_shell
    def test_writing_resolution_still_requires_the_live_branch(self):
        with tempfile.TemporaryDirectory() as folder:
            git, (older, tested, source) = repository(folder)
            git("push", "-q", "origin", f"{source}:{RELEASE}")
            # The clone and the dispatch still contain tested, but the refreshed branch does not.
            git_in(Path(folder, "origin"))("update-ref", RELEASE, older)
            result, out = self.resolve_commit(folder, RELEASE, source, tested)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(f"::error::{tested} is not on release/26.3", result.stdout)
            self.assertEqual(out, "")

    def test_every_leg_is_a_called_workflow_with_the_commit(self):
        # The summary's needs are the legs it judges (daily_summary.daily_legs): each a called workflow given
        # the resolved commit. Publishing and the independent fuzz report are not legs it judges.
        jobs = self.workflow["jobs"]
        needs = jobs["summary"]["needs"]
        self.assertEqual(set(needs), {"resolve"} | {name for name, job in jobs.items() if "uses" in job
                                                     and name not in ("sdk-publish", "nightly", "fuzz")})
        for leg in needs:
            if leg == "resolve":
                continue
            with self.subTest(leg=leg):
                self.assertTrue(jobs[leg]["uses"].startswith("./.github/workflows/"))
                self.assertEqual(jobs[leg]["with"]["commit"], "${{ needs.resolve.outputs.commit }}")
        self.assertEqual(summary.daily_legs({name: {} for name in needs}),
                         tuple(name for name in needs if name not in ("resolve", *summary.PART_OF)))


if __name__ == "__main__":
    unittest.main()
