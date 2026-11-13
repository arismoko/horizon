"""Whether this run gets the heavy checks (everything that compiles), as `heavy=true|false` for $GITHUB_OUTPUT.

Ordinary pull requests get only the cheap checks (verify-static and format). The heavy checks run on main's
pushes, dispatches, and same-repository rollup or sync pull requests, identified by labels or branch names.
Fork pull requests remain cheap even when they use those labels or names.

A reviewer runs the heavy gate on one contributor's pull request with `gh workflow run dyna-ci.yml -f pr=<number>`:
main's workflow checks out that pull request's head commit, runs the heavy jobs on GitHub's runners with a read-only
token, and a job that runs no pull-request code posts their results on that commit.
"""

import json
import os
import re
import subprocess
import sys


def outputs(event: str, same_repository: bool, pr: str, read_pr, *, labels: tuple[str, ...] | list[str] = (), head_ref: str = "") -> dict[str, str]:
    if event == "workflow_dispatch" and pr:
        if not re.fullmatch(r"[1-9][0-9]{0,6}", pr):
            raise SystemExit(f"pr must be a pull request number, not {pr!r}")
        head = read_pr(pr)
        return {"heavy": "true", "pr_sha": head["sha"],
                "trusted": "true" if head["same_repository"] else "false"}
    # A rollup builder session pushes only branches named after itself (`rollup-76`), and a label added after the
    # pull request opens misses its first event, so the branch name alone counts too.
    integration = bool({"rollup", "sync"}.intersection(labels)) or head_ref.startswith(
        ("fork-review-rollup-", "rollup-", "sync-"))
    heavy = event != "pull_request" or (same_repository and integration)
    return {"heavy": "true" if heavy else "false", "pr_sha": "", "trusted": ""}


def read_pr(number: str) -> dict:
    """The pull request's head commit, and whether its branch lives in this repository."""
    repository = os.environ["GITHUB_REPOSITORY"]
    answer = json.loads(subprocess.run(
        ["gh", "api", f"repos/{repository}/pulls/{number}",
         "--jq", "{sha: .head.sha, repo: .head.repo.full_name, state: .state}"],
        check=True, capture_output=True, text=True).stdout)
    if answer["state"] != "open":
        raise SystemExit(f"pull request #{number} is {answer['state']}")
    if not re.fullmatch(r"[0-9a-f]{40}", answer["sha"]):
        raise SystemExit(f"pull request #{number} has no head commit")
    return {"sha": answer["sha"], "same_repository": answer["repo"] == repository}


def main() -> None:
    values = outputs(os.environ.get("EVENT", ""),
                     os.environ.get("HEAD_REPO", "") == os.environ.get("GITHUB_REPOSITORY"),
                     os.environ.get("PR", ""), read_pr,
                     labels=json.loads(os.environ.get("PR_LABELS", "[]")) or [],
                     head_ref=os.environ.get("HEAD_REF", ""))
    for key, value in values.items():
        print(f"{key}={value}")
    print(f"heavy checks: {values['heavy']}", file=sys.stderr)


if __name__ == "__main__":
    main()
