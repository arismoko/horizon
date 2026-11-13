"""Whether this run gets the heavy checks (everything that compiles), as `heavy=true|false` for $GITHUB_OUTPUT.

A pull request from a contributor's fork gets only the cheap checks (verify-static and the format job): reviews read
the diff, and a rollup carries the PR into one heavy run. The heavy checks run on main's pushes, on dispatches, and
on every pull request whose head branch is in this repository: ours (rollups, syncs, fast-path fixes, our sessions'
work); contributors' heads live in their forks. Workflows without this job spell the same rule as an expression
(tests/test_pr_gate.py keeps them equal).

A reviewer runs the heavy gate on one contributor's pull request with `gh workflow run dyna-ci.yml -f pr=<number>`:
main's workflow checks out that pull request's head commit, runs the heavy jobs on GitHub's runners with a read-only
token, and a job that runs no pull-request code posts their results on that commit.
"""

import json
import os
import re
import subprocess
import sys


def outputs(event: str, same_repository: bool, pr: str, read_pr) -> dict[str, str]:
    if event == "workflow_dispatch" and pr:
        if not re.fullmatch(r"[1-9][0-9]{0,6}", pr):
            raise SystemExit(f"pr must be a pull request number, not {pr!r}")
        head = read_pr(pr)
        return {"heavy": "true", "pr_sha": head["sha"],
                "trusted": "true" if head["same_repository"] else "false"}
    heavy = event != "pull_request" or same_repository
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
                     os.environ.get("PR", ""), read_pr)
    for key, value in values.items():
        print(f"{key}={value}")
    print(f"heavy checks: {values['heavy']}", file=sys.stderr)


if __name__ == "__main__":
    main()
