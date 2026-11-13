#!/usr/bin/env python3
"""The open PRs whose own diff touches a path: run before a switch PR, whose body carries the list.

A PR's own diff is `diff(merge-base(main, head), head)`, read from the fetched `refs/pull/<n>/head`. The open PR
numbers come from stdin (one per line; the first number on each line counts, so `gh pr list` output works), or from
`gh pr list` with --from-gh. A path matches a changed file when it is the file, a folder above it, or a glob
(`launcher/lang/*.json`). Prints `<number>\t<file>` for every match; exits 2 when a PR's head is not fetched.

    git fetch origin '+refs/pull/*/head:refs/pull/*/head'
    python3 tools/hotspots/open_prs.py --from-gh crates/horizon-mod-vocab/src/exports/part_table.rs
    gh pr list --state open --limit 500 | python3 tools/hotspots/open_prs.py tools/ci/
"""
import argparse
import fnmatch
import json
import re
import subprocess
import sys


def git(repo, *args, ok=(0,)):
    p = subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True)
    if p.returncode not in ok:
        raise SystemExit(f"git {' '.join(args)}: {p.stderr.strip()}")
    return p.returncode, p.stdout


def matches(path, file):
    path = path.rstrip("/")
    return file == path or file.startswith(path + "/") or fnmatch.fnmatchcase(file, path)


def changed_files(repo, main, number):
    """The files PR `number` changed against main, or None when its head is not fetched."""
    rc, head = git(repo, "rev-parse", "--verify", "--quiet", f"refs/pull/{number}/head^{{commit}}", ok=(0, 1))
    if rc:
        return None
    rc, base = git(repo, "merge-base", main, head.strip(), ok=(0, 1))
    if rc:
        return None
    return git(repo, "diff", "--name-only", "--no-renames", base.strip(), head.strip())[1].splitlines()


def open_numbers(from_gh, stdin):
    if from_gh:
        listed = subprocess.run(["gh", "pr", "list", "--state", "open", "--limit", "1000", "--json", "number"],
                                capture_output=True, text=True, check=True).stdout
        return sorted(item["number"] for item in json.loads(listed))
    numbers = []
    for line in stdin:
        m = re.search(r"\d+", line)
        if m:
            numbers.append(int(m.group(0)))
    return numbers


def main(argv=None, stdin=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("paths", nargs="+", help="a file, a folder or a glob")
    ap.add_argument("--repo", default=".")
    ap.add_argument("--main", default="origin/main")
    ap.add_argument("--from-gh", action="store_true", help="read the open PR numbers with `gh pr list`")
    args = ap.parse_args(argv)
    numbers = open_numbers(args.from_gh, sys.stdin if stdin is None else stdin)
    missing = []
    for number in numbers:
        files = changed_files(args.repo, args.main, number)
        if files is None:
            missing.append(number)
            continue
        for f in files:
            if any(matches(p, f) for p in args.paths):
                print(f"{number}\t{f}")
    if missing:
        print("not fetched (git fetch origin '+refs/pull/*/head:refs/pull/*/head'): "
              + " ".join(f"#{n}" for n in missing), file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
