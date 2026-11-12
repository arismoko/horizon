#!/usr/bin/env python3
"""Recover same-repository boat startup outages without retrying test failures."""
import os
import subprocess
import time
from datetime import datetime, timezone

from boat_health import api, recovery_reason, user_step_started


def main():
    repo = os.environ["GITHUB_REPOSITORY"]
    run = os.environ["GITHUB_RUN_ID"]
    attempt = os.environ["GITHUB_RUN_ATTEMPT"]
    deadline = time.monotonic() + 900
    while time.monotonic() < deadline:
        jobs = api(f"repos/{repo}/actions/runs/{run}/attempts/{attempt}/jobs?per_page=100")["jobs"]
        targets = [job for job in jobs if job["name"] in ("lint", "verify", "verify-static")]
        reason = recovery_reason(jobs, datetime.now(timezone.utc))
        if reason:
            # Dispatch before cancelling: a refused dispatch must not destroy this run.
            subprocess.run([
                "gh", "api", "--method", "POST",
                f"repos/{repo}/actions/workflows/dyna-ci.yml/dispatches",
                "-f", f"ref={os.environ['RECOVERY_REF']}",
                "-F", "inputs[github-fallback]=true",
            ], check=True)
            with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as summary:
                summary.write(f"### Boat fallback\n\n{reason}. Dispatched GitHub-hosted recovery on the source branch; cancelling this run.\n")
            subprocess.run(["gh", "api", "--method", "POST", f"repos/{repo}/actions/runs/{run}/cancel"], check=True)
            return
        if len(targets) == 3 and all(
            job["status"] == "completed" or user_step_started(job) for job in targets
        ):
            return
        time.sleep(30)
    raise SystemExit("Boat jobs did not appear within 15 minutes; no recovery dispatched")


if __name__ == "__main__":
    main()
