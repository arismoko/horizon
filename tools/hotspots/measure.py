#!/usr/bin/env python3
"""Conflict hotspots: which files many independent PRs change, and where git itself could not merge them.

Reads git history only (git 2.38 or newer for `merge-tree --write-tree`); standard library only.

Units
- A PR is (a) a first-parent merge on main that is neither a rollup nor an upstream sync, or (b) a rollup
  member: the second parent of a merge on a rollup's first-parent chain, unless that parent is a commit of main's
  first-parent chain (main merged into the rollup) or another rollup (see below). A PR's files are
  `diff(merge-base(M^1, M^2), M^2)`: what the PR itself changed. Its id is the PR number whose `refs/pull/<n>/head`
  is that commit or has it on its own first-parent chain (an earlier head), or the first such PR down the commit's
  own first-parent chain or merged into it (a carry branch: our commits on a PR's head), else the commit's short sha. Merge
  subjects are not read for members. Fetch the PR heads first: git fetch origin '+refs/pull/*/head:refs/pull/*/head'.
- An upstream sync riding a rollup (a second parent that brings upstream commits the rollup did not have, with
  `--upstream`) is not a PR: its merge and every merge on its own chain are sync conflicts.
- A rollup merged into the next rollup is not a PR: its merge is replayed once as a rollup conflict, and its own
  chain is walked as a rollup chain, so each member merge is counted and replayed once. A second parent is a rollup when it is on a
  known rollup's chain (merged into main, or `--open-rollup`), or when its own first-parent chain merges the
  current heads of two or more PRs (`refs/pull/*/head`).
- A conflict is a merge replayed with `git merge-tree --write-tree`; the files git could not merge are the
  conflicts. Replayed: main's first-parent merges, every merge on a rollup or sync chain, and every other merge
  with one parent on main's first-parent chain (main merged into a PR branch: "branch"; upstream merged onto
  main's commit: "sync"). Nothing is inferred
  from the committed resolution.
- Fix commits: non-merge commits on a rollup chain ("Rollup resolution", "Carry for", CI fixes): the breakage a
  textual replay cannot see.
- Kind of a main merge: an upstream sync is a merge whose second parent brings upstream commits main did not have
  (with `--upstream`; without it, a branch name with a `sync` word in GitHub's "Merge pull request #N from
  owner/branch" subject). A rollup is a branch named `rollup-<n>` (or `rollup<n>`) in that subject. Every other
  merge is one PR.

The window: `--days N` (back from main's head) or `--since DATE`, and the start of the fork's own history:
`--bound <commit>`, or derived as the last commit of main's first-parent chain that upstream also has
(`--upstream`, default `upstream/main` when that ref exists). One of the four is required.

  python3 tools/hotspots/measure.py --days 3                       # markdown ranking
  python3 tools/hotspots/measure.py --upstream upstream/main --json > raw.json
  python3 tools/hotspots/measure.py --days 3 --report             # the churn report's body
"""
import argparse
import collections
import concurrent.futures as cf
import datetime
import json
import os
import re
import subprocess
import sys

MIN_GIT = (2, 38)
GITHUB_MERGE = re.compile(r"Merge pull request #(\d+) from [^/\s]+/(\S+)")
ROLLUP_BRANCH = re.compile(r"(^|[-/])rollup-?\d+$")
SYNC_BRANCH = re.compile(r"(^|[-/_])sync([-/_]|$)")


class Git:
    def __init__(self, repo):
        self.repo = repo

    def run(self, *args, ok=(0,)):
        p = subprocess.run(["git", "-C", self.repo, *args], capture_output=True, text=True,
                           encoding="utf-8", errors="replace")
        if p.returncode not in ok:
            raise SystemExit(f"git {' '.join(args)}: {p.stderr.strip()}")
        return p.returncode, p.stdout

    def out(self, *args):
        return self.run(*args)[1]

    def rev(self, ref):
        rc, o = self.run("rev-parse", "--verify", "--quiet", ref + "^{commit}", ok=(0, 1))
        return o.strip() if rc == 0 else None


def check_git_version():
    v = subprocess.run(["git", "--version"], capture_output=True, text=True).stdout
    m = re.search(r"(\d+)\.(\d+)", v)
    if not m or (int(m.group(1)), int(m.group(2))) < MIN_GIT:
        raise SystemExit(f"needs git {MIN_GIT[0]}.{MIN_GIT[1]} or newer for merge-tree --write-tree; have {v.strip()}")


def commits(git, rng, *extra):
    """(sha, parents, commit time, subject) for `git log <extra> <rng>`."""
    rows = []
    for line in git.out("log", *extra, "--format=%H%x09%P%x09%ct%x09%s", *rng).splitlines():
        h, p, t, s = line.split("\t", 3)
        rows.append((h, p.split(), int(t), s))
    return rows


class Measure:
    def __init__(self, git, main, since=None, bound=None, upstream=None, open_rollups=()):
        self.git = git
        self.main = main
        self.since = since
        self.bound = bound
        self.upstream = upstream
        self.open_rollups = list(open_rollups)
        self.main_fp = set(git.out("rev-list", "--first-parent", main).split())
        # Main merged into a PR branch counts from the fork's start only (before it, main was upstream's).
        self.main_own = set(git.out("rev-list", "--first-parent", f"{bound}..{main}").split()) if bound else self.main_fp
        self.pulls = {}
        for line in git.out("for-each-ref", "--format=%(objectname) %(refname)", "refs/pull").splitlines():
            sha, ref = line.split()
            m = re.fullmatch(r"refs/pull/(\d+)/head", ref)
            if m and (sha not in self.pulls or int(m.group(1)) < self.pulls[sha]):
                self.pulls[sha] = int(m.group(1))
        self._pull_commits = None
        self.units = []         # (id, time, merge, p1, p2)
        self.replay = {}        # merge -> (p1, p2, kind, label, time)
        self.fixes = collections.defaultdict(set)   # file -> {fix commit}
        self.fix_rollups = collections.defaultdict(set)
        self.seen = set()       # chain merges already classified
        self.rollup_commits = set()
        self.nested = []        # rollup-into-rollup merges, not replayed

    def in_window(self, t):
        return self.since is None or t >= self.since

    def pull_commits(self):
        """{commit: PR number} for every commit of a PR's own first-parent chain (down to main), nearest head
        first: a rollup may have merged an earlier head than the one `refs/pull/<n>/head` holds now."""
        if self._pull_commits is None:
            def chain(item):
                head, number = item
                own = []
                for c in self.git.out("rev-list", "--first-parent", "--max-count=400", head).split():
                    if c in self.main_fp:
                        break
                    own.append(c)
                return number, own
            found = {}
            with cf.ThreadPoolExecutor(8) as ex:
                for number, own in ex.map(chain, sorted(self.pulls.items())):
                    for depth, c in enumerate(own):
                        if c not in found or depth < found[c][0]:
                            found[c] = (depth, number)
            self._pull_commits = {c: n for c, (_, n) in found.items()}
        return self._pull_commits

    def ident(self, sha):
        """The PR a member merge's second parent belongs to: a PR's current head, then a carry branch (our commits
        on a PR's head or on a merge of it), then an earlier head of a PR."""
        chain = []
        for line in self.git.out("rev-list", "--first-parent", "--parents", "--max-count=50", sha).splitlines():
            c, *ps = line.split()
            if c in self.main_fp:
                break
            chain += [c] + ps[1:]
        n = next((self.pulls[c] for c in chain if c in self.pulls), None)
        if n is None:
            n = next((self.pull_commits()[c] for c in chain if c in self.pull_commits()), None)
        if n is not None:
            return f"#{n}"
        return sha[:12]

    def is_sync(self, p1, p2, branch):
        """Whether merging p2 into p1 brings upstream commits that neither p1 nor the main commits p2's own chain
        started from or merged had (by the branch name without --upstream). A PR based on a newer main than p1
        brings upstream commits only through main, so it is not a sync."""
        if not self.upstream:
            return bool(SYNC_BRANCH.search(branch))
        rc, base = self.git.run("merge-base", p2, self.upstream, ok=(0, 1))
        if rc:
            return False
        base = base.strip()
        mains = [p1]
        for line in self.git.out("rev-list", "--first-parent", "--parents", "--max-count=200", p2).splitlines():
            c, *ps = line.split()
            if c in self.main_fp:
                mains.append(c)
                break
            mains += [x for x in ps[1:] if x in self.main_fp]
        return all(self.git.run("merge-base", "--is-ancestor", base, m, ok=(0, 1))[0] == 1 for m in mains)

    def is_rollup(self, x, base):
        if x in self.rollup_commits:
            return True
        heads = 0
        for _, ps, _, _ in commits(self.git, [f"{base}..{x}"], "--first-parent", "--merges"):
            if len(ps) == 2 and ps[1] in self.pulls and ps[1] not in self.main_fp:
                heads += 1
        return heads >= 2

    def walk(self, base, tip, kind, label):
        """A rollup's or sync's first-parent chain from `base` (excluded) to `tip`."""
        out = self.git.out("log", "--first-parent", "--name-only", "--no-renames",
                           "--format=@%H%x09%P%x09%ct%x09%s", f"{base}..{tip}")
        for block in out.split("\n@"):
            lines = [x for x in block.lstrip("@").split("\n") if x]
            if not lines:
                continue
            h, p, t, _ = lines[0].split("\t", 3)
            ps, t = p.split(), int(t)
            if h in self.seen or h in self.replay or not self.in_window(t):
                continue
            self.seen.add(h)
            if len(ps) == 1:
                if kind == "rollup":
                    for f in lines[1:]:
                        self.fixes[f].add(h)
                        self.fix_rollups[f].add(label)
                continue
            if len(ps) != 2:
                continue
            p1, p2 = ps
            if p2 in self.main_fp:
                self.replay[h] = (p1, p2, kind, label, t)
            elif kind == "rollup" and self.is_rollup(p2, p1):
                # A rollup merged into the next: its merge is replayed once, as a rollup conflict and not a PR;
                # its own chain's members are walked (once: a chain already walked is skipped commit by commit).
                self.replay[h] = (p1, p2, kind, label, t)
                self.nested.append(h)
                self.walk(p1, p2, kind, label)
            elif kind == "sync" or self.is_sync(p1, p2, ""):
                # An upstream sync riding this chain: its merge and its own chain are sync conflicts, not a PR.
                self.replay[h] = (p1, p2, "sync", label, t)
                if kind == "rollup":
                    self.walk(p1, p2, "sync", label)
            else:
                self.replay[h] = (p1, p2, kind, label, t)
                if kind == "rollup":
                    self.units.append((self.ident(p2), t, h, p1, p2))

    def collect(self):
        rng = [f"{self.bound}..{self.main}"] if self.bound else [self.main]
        main_merges = []
        for h, ps, t, s in commits(self.git, rng, "--first-parent", "--merges"):
            if len(ps) != 2 or not self.in_window(t):
                continue
            m = GITHUB_MERGE.match(s)
            number, branch = (m.group(1), m.group(2)) if m else (None, "")
            kind = "rollup" if ROLLUP_BRANCH.search(branch) else "sync" if self.is_sync(*ps, branch) else "pr"
            main_merges.append((h, ps, t, number, kind))
        chains = [(ps[0], ps[1], f"#{n}" if n else h[:12]) for h, ps, t, n, kind in main_merges if kind == "rollup"]
        for b in self.open_rollups:
            tip = self.git.rev(b)
            if not tip:
                raise SystemExit(f"--open-rollup {b}: no such branch")
            chains.append((self.main, tip, b))
        for base, tip, _ in chains:
            self.rollup_commits.update(self.git.out("rev-list", "--first-parent", f"{base}..{tip}").split())
        for h, (p1, p2), t, number, kind in main_merges:
            self.replay[h] = (p1, p2, kind, f"#{number}" if number else h[:12], t)
        for h, (p1, p2), t, number, kind in main_merges:
            if kind == "pr":
                self.units.append((f"#{number}" if number else self.ident(p2), t, h, p1, p2))
            else:
                self.walk(p1, p2, kind, f"#{number}" if number else h[:12])
        for base, tip, label in chains[len(chains) - len(self.open_rollups):]:
            self.walk(base, tip, "rollup", label)
        extra = ["--since", f"@{self.since}"] if self.since is not None else []
        for h, ps, t, _ in commits(self.git, rng, "--merges", *extra):
            if (h not in self.replay and h not in self.seen and len(ps) == 2 and self.in_window(t)
                    and (ps[0] in self.main_own or ps[1] in self.main_own)):
                self.replay[h] = (ps[0], ps[1], "sync" if self.is_sync(ps[0], ps[1], "") else "branch", "", t)

    def pr_files(self, unit):
        _, _, _, p1, p2 = unit
        rc, base = self.git.run("merge-base", p1, p2, ok=(0, 1))
        if rc:
            return []
        return self.git.out("diff", "--name-only", "--no-renames", base.strip(), p2).splitlines()

    def merge_tree(self, merge):
        p1, p2 = self.replay[merge][:2]
        rc, o = self.git.run("-c", "merge.conflictStyle=merge", "merge-tree", "--write-tree", "--name-only", "--no-messages", p1, p2, ok=(0, 1))
        lines = o.split("\n")
        return lines[0], (sorted(set(x for x in lines[1:] if x)) if rc else [])

    def run(self, jobs=8):
        self.collect()
        touch = collections.defaultdict(dict)       # file -> {id: first time}
        with cf.ThreadPoolExecutor(jobs) as ex:
            for u, files in zip(self.units, ex.map(self.pr_files, self.units)):
                for f in files:
                    touch[f].setdefault(u[0], u[1])
        conflicts = collections.defaultdict(list)
        merges = sorted(self.replay)
        with cf.ThreadPoolExecutor(jobs) as ex:
            for h, (tree, files) in zip(merges, ex.map(self.merge_tree, merges)):
                _, _, kind, label, t = self.replay[h]
                for f in files:
                    conflicts[f].append({"merge": h, "kind": kind, "label": label, "time": t, "tree": tree})
        live = set(self.git.out("ls-tree", "-r", "--name-only", self.main).splitlines())
        files = {}
        for f in set(touch) | set(conflicts) | set(self.fixes):
            files[f] = {
                "live": f in live,
                "prs": sorted(touch.get(f, {})),
                "conflicts": sorted(conflicts.get(f, []), key=lambda c: c["time"]),
                "fixes": len(self.fixes.get(f, ())),
                "fix_rollups": sorted(self.fix_rollups.get(f, ())),
            }
        return {
            "main": self.git.rev(self.main),
            "head_time": int(self.git.out("log", "-1", "--format=%ct", self.main)),
            "since": self.since,
            "bound": self.bound,
            "units": len(self.units),
            "prs": len({u[0] for u in self.units}),
            "replayed": len(self.replay),
            "nested_rollups": len(self.nested),
            "files": files,
        }


def counts(entry):
    k = collections.Counter(c["kind"] for c in entry["conflicts"])
    return {"prs": len(entry["prs"]), "rollup": k["rollup"], "sync": k["sync"], "branch": k["branch"],
            "pr": k["pr"], "fixes": entry["fixes"], "fix_rollups": len(entry["fix_rollups"])}


def markdown(raw, top):
    rows = [(f, counts(e)) for f, e in raw["files"].items() if e["live"]]
    rows.sort(key=lambda r: (-r[1]["prs"], -r[1]["rollup"], r[0]))
    out = [f"Main at {raw['main'][:11]}; {raw['prs']} PRs ({raw['units']} merges of them) and {raw['replayed']} merges "
           f"replayed; {window(raw)}.", "",
           "| PRs | rollup | sync | branch | fixes (rollups) | file |", "|---:|---:|---:|---:|---:|---|"]
    for f, c in rows[:top]:
        out.append(f"| {c['prs']} | {c['rollup']} | {c['sync']} | {c['branch']} | {c['fixes']} ({c['fix_rollups']}) | `{f}` |")
    return "\n".join(out) + "\n"


def window(raw):
    def day(t):
        return datetime.datetime.fromtimestamp(t, datetime.timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    start = f"from {day(raw['since'])}" if raw["since"] is not None else f"from {raw['bound'][:11]}"
    return f"{start} to {day(raw['head_time'])}"


def construct(text, start):
    """The first line of the construct a conflict at line `start` (the `<<<<<<<` marker) sits in."""
    lines = text.split("\n")
    first = next((x for x in lines[start + 1:] if x.strip() and not x.startswith(("=======", ">>>>>>>"))), "")
    if first and not first[0].isspace():
        return first.strip()
    for x in reversed(lines[:start]):
        if x and not x[0].isspace() and not x.startswith((">>>>>>>", "}", ")", "]")):
            return x.strip()
    return first.strip()


def constructs(git, path, conflict_list):
    found = collections.Counter()
    for c in conflict_list:
        rc, text = git.run("cat-file", "-p", f"{c['tree']}:{path}", ok=(0, 128))
        marks = [i for i, x in enumerate(text.split("\n")) if x.startswith("<<<<<<< ")] if rc == 0 else []
        if not marks:
            found["(the whole file: added, deleted, renamed or binary on one side)"] += 1
        for i in marks:
            found[construct(text, i)[:120]] += 1
    return found


def report(git, raw, min_rollup, min_prs):
    rows = []
    for f, e in raw["files"].items():
        c = counts(e)
        if e["live"] and (c["rollup"] >= min_rollup or c["prs"] >= min_prs):
            rows.append((f, e, c))
    rows.sort(key=lambda r: (-r[2]["rollup"], -r[2]["prs"], r[0]))
    out = [f"Main at `{raw['main'][:11]}`, {window(raw)}: {raw['prs']} PRs, {raw['replayed']} merges replayed with "
           f"`git merge-tree`. Listed: every file with {min_rollup} or more replayed rollup conflicts or {min_prs} or "
           "more distinct PRs. Made by `tools/hotspots/measure.py --report`; whether a file is a list that should "
           "become discovered is decided by a person, not by this report.", ""]
    if not rows:
        out.append("No file reached either threshold.")
    for f, e, c in rows:
        out.append(f"### `{f}`")
        out.append(f"{c['prs']} PRs; replayed conflicts: {c['rollup']} rollup, {c['sync']} sync, {c['branch']} on PR "
                   f"branches; {c['fixes']} rollup fix commits.")
        found = constructs(git, f, e["conflicts"])
        if found:
            out.append("")
            out.append("Conflicting constructs (first line, times):")
            for text, n in found.most_common(10):
                out.append(f"- `{text.replace('`', chr(39))}` ({n})")
        out.append("")
    return "\n".join(out)


def parse_args(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("--repo", default=".")
    ap.add_argument("--main", default="origin/main", help="main's ref (default origin/main)")
    w = ap.add_mutually_exclusive_group()
    w.add_argument("--days", type=float, help="only merges in the last N days before main's head")
    w.add_argument("--since", help="only merges at or after this date (YYYY-MM-DD or ISO time, UTC)")
    ap.add_argument("--bound", help="the last commit before the fork's own history (excluded)")
    ap.add_argument("--upstream", help="upstream's main ref, to derive --bound (default upstream/main if it exists)")
    ap.add_argument("--open-rollup", action="append", default=[], metavar="BRANCH",
                    help="a rollup branch not merged yet (repeatable)")
    out = ap.add_mutually_exclusive_group()
    out.add_argument("--json", action="store_true", help="every file's PRs and replayed conflicts")
    out.add_argument("--markdown", action="store_true", help="a ranking table (the default)")
    out.add_argument("--report", action="store_true", help="the churn report: files over the thresholds")
    ap.add_argument("--top", type=int, default=60, help="rows of the ranking (default 60)")
    ap.add_argument("--min-rollup", type=int, default=3)
    ap.add_argument("--min-prs", type=int, default=15)
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 4)
    return ap.parse_args(argv)


def measure(args):
    check_git_version()
    git = Git(args.repo)
    main = git.rev(args.main)
    if not main:
        raise SystemExit(f"--main {args.main}: no such commit")
    since = None
    if args.days is not None:
        since = int(git.out("log", "-1", "--format=%ct", main)) - int(args.days * 86400)
    elif args.since:
        when = datetime.datetime.fromisoformat(args.since)
        since = int((when if when.tzinfo else when.replace(tzinfo=datetime.timezone.utc)).timestamp())
    bound = git.rev(args.bound) if args.bound else None
    if args.bound and not bound:
        raise SystemExit(f"--bound {args.bound}: no such commit")
    upstream = args.upstream or ("upstream/main" if git.rev("upstream/main") else None)
    if upstream and not git.rev(upstream):
        raise SystemExit(f"--upstream {upstream}: no such ref")
    if not bound and upstream:
        own = git.out("rev-list", "--first-parent", main, f"^{upstream}").split()
        bound = git.out("rev-parse", f"{own[-1]}^1").strip() if own else main
    if not bound and since is None:
        raise SystemExit("say where to start: --days, --since, --bound or --upstream "
                         "(git fetch https://github.com/horizon-rust/horizon.git main:refs/remotes/upstream/main)")
    m = Measure(git, main, since=since, bound=bound, upstream=git.rev(upstream) if upstream else None,
                open_rollups=args.open_rollup)
    return git, m.run(args.jobs)


def main(argv=None):
    args = parse_args(sys.argv[1:] if argv is None else argv)
    git, raw = measure(args)
    if args.json:
        json.dump(raw, sys.stdout)
        print()
    elif args.report:
        print(report(git, raw, args.min_rollup, args.min_prs))
    else:
        print(markdown(raw, args.top), end="")


if __name__ == "__main__":
    main()
