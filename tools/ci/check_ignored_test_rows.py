#!/usr/bin/env python3
"""verify-static's guard for tools/ci/ignored-tests/: read every policy file as the ignored-test readers do.

Fails on a malformed row, a test listed in two files, a folder with no rows or with a file other than `*.txt`
and README.md, and a file at the retired single-list path. The shared reader validates both `needs`
and `needs-plain` fixture rows; neither is a known failure. Every `daily` row must name a workspace package and one
of its test targets, and some leg of the daily run's matrix must run it: its package's shard, or the `daily-rows`
leg (tools/ci/ignored_shards.py), so no moved test runs nowhere. Builds nothing.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools.ci import ignored_shards, ignored_tests


def daily_problems(rows, targets, matrix):
    """Why each daily row would not run in the daily matrix `matrix` (shard names), given the workspace's targets."""
    problems = []
    groups = ignored_tests.daily_rows_groups(rows)
    for row in rows:
        where = f"{row.file}: {row.package} {row.test}"
        if row.package not in targets:
            problems.append(f"{where}: {row.package} is not a workspace package")
        elif row.reference not in targets[row.package]:
            problems.append(f"{where}: {row.package} has no target {row.reference}")
        elif ignored_tests.shard_runs(row.package, row.reference):
            if not any(ignored_tests.parse_shard(shard)[0] == row.package for shard in matrix):
                problems.append(f"{where}: no leg runs {row.package}'s shard")
        elif ignored_shards.DAILY_ROWS not in matrix or row.test not in groups.get((row.package, row.reference), ()):
            problems.append(f"{where}: no leg runs it (the {ignored_shards.DAILY_ROWS} leg is missing)")
    return problems


if __name__ == "__main__":
    rows = ignored_tests.listed_entries()
    print(f"{len(rows)} rows in {len({row.file for row in rows})} files of {ignored_tests.POLICY_PATH}/")
    daily = [row for row in rows if row.mark == "daily"]
    problems = daily_problems(daily, ignored_shards.workspace_targets(),
                              [*ignored_shards.expand(ignored_shards.SHARDS), ignored_shards.DAILY_ROWS])
    for problem in problems:
        print(f"::error::{problem}")
    print(f"{len(daily)} daily rows, {len(problems)} that no daily leg runs")
    sys.exit(1 if problems else 0)
