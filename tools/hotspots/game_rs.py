#!/usr/bin/env python3
"""Measure game.rs against a synced upstream commit, without modifying either tree.

Brace-depth parsing is deliberately approximate: strings, comments, macros and cfg branches
are not Rust-parsed; multiline impl headers and complex generic impl targets are unsupported.
Line counts include signatures and closing braces. This is a sync aid, not a semantic diff.
"""
import argparse
from collections import defaultdict
from dataclasses import dataclass
import difflib
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[2]
GAME = "crates/horizon-client/src/game.rs"
TREES = ("crates/horizon-client/src", "crates/horizon-client-render/src")
FN = re.compile(r"\s*(?:pub(?:\([^)]*\))?\s+)?(?:const |async |unsafe )*fn (\w+)")


@dataclass(frozen=True)
class Function:
    owner: str
    name: str
    path: str
    line: int
    body: str

    @property
    def key(self):
        return self.owner, self.name


def items(source, path=GAME):
    """Record free functions and methods, including functions in inline test modules."""
    depth, current = 0, None
    scopes = []
    lines = source.splitlines()
    result = []
    for i, line in enumerate(lines):
        header = re.match(r"\s*impl(?:<.*?>)?\s+(.+?)\s*\{\s*$", line)
        if header and current is None:
            target = header[1].split(" for ")[-1].split(" where ")[0].strip()
            owner_match = re.search(r"[\w:]+", target)
            owner = owner_match[0].split("::")[-1] if owner_match else target
            scopes.append((depth, owner))
        match = FN.match(line)
        if match and current is None:
            current = (scopes[-1][1] if scopes else "free", match[1], i, depth)
        depth += line.count("{") - line.count("}")
        if current and depth <= current[3] and ("}" in line or line.rstrip().endswith(";")):
            owner, name, start, _ = current
            result.append(Function(owner, name, path, start + 1, "\n".join(lines[start:i + 1])))
            current = None
        while scopes and depth <= scopes[-1][0]:
            scopes.pop()
    return result


def git(repo, *args):
    return subprocess.run(["git", "-C", str(repo), *args], text=True,
                          capture_output=True, check=True).stdout


def tree(repo, ref):
    # One git process for all source bodies, rather than one per file.
    paths = [p for p in git(repo, "ls-tree", "-r", "--name-only", ref, "--", *TREES).splitlines()
             if p.endswith(".rs")]
    request = "".join(f"{ref}:{p}\n" for p in paths).encode()
    data = subprocess.run(["git", "-C", str(repo), "cat-file", "--batch"], input=request,
                          capture_output=True, check=True).stdout
    result = []
    offset = 0
    for path in paths:
        end = data.index(b"\n", offset)
        size = int(data[offset:end].split()[-1])
        source = data[end + 1:end + 1 + size].decode()
        offset = end + size + 2
        result.extend(items(source, path))
    return result


def classifications(path):
    entries = {}
    for number, line in enumerate(path.read_text().splitlines(), 1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        owner, name, upstream_owner, upstream_path, reason = line.split(None, 4)
        key = (owner, name, upstream_owner, upstream_path)
        if key in entries:
            raise ValueError(f"{path}:{number}: duplicate classification")
        entries[key] = reason
    return entries


def report(repo, fork, base, classification):
    ft, bt = tree(repo, fork), tree(repo, base)
    f = {x.key: x for x in ft if x.path == GAME}
    b = {x.key: x for x in bt if x.path == GAME}
    destinations = defaultdict(list)
    for x in bt:
        if x.path != GAME:
            destinations[x.name].append(x)
    entries = classifications(classification)
    used, genuine, classified = set(), [], []
    groups = defaultdict(list)
    changed = []
    for key, x in f.items():
        if key in b:
            same = x.body == b[key].body
            groups["same as base" if same else "in base game.rs, body differs"].append(x)
            if not same:
                diff = list(difflib.ndiff(b[key].body.splitlines(), x.body.splitlines()))
                changed.append((sum(l.startswith("+ ") for l in diff),
                                sum(l.startswith("- ") for l in diff), x))
            continue
        candidates = destinations[x.name]
        if not candidates:
            groups["fork-only (no such fn anywhere in upstream's client crates)"].append(x)
            continue
        # Same-owner moves always win, even if an unrelated destination is classified.
        same_owner = [d for d in candidates if d.owner == x.owner
                      and (x.owner != "free" or "/game/" in d.path)]
        matches = [(d, (x.owner, x.name, d.owner, d.path)) for d in candidates
                   if (x.owner, x.name, d.owner, d.path) in entries]
        if matches and not same_owner:
            used.update(k for _, k in matches)
            classified.append((x, matches))
        else:
            genuine.append((x, same_owner or candidates))
    size = lambda xs: sum(len(x.body.splitlines()) for x in xs)
    print(f"fork game.rs: {len(git(repo, 'show', f'{fork}:{GAME}').splitlines())} lines, "
          f"{len(f)} fns holding {size(f.values())} lines; base: {size(b.values())} fn lines")
    for title, xs in groups.items():
        print(f"\n{title}: {len(xs)} fns, {size(xs)} lines")
    print("\n-- fns in the base's game.rs that the fork's game.rs lacks")
    gone = defaultdict(list)
    fork_keys = {x.key for x in ft if x.path != GAME}
    for key, x in b.items():
        if key not in f:
            gone["fork has it in another file" if key in fork_keys else "fork has it nowhere"].append(x)
    for title, xs in gone.items():
        print(f"{title}: {len(xs)} fns, {size(xs)} lines")
    print(f"\n-- {len(changed)} fns changed in place: +{sum(a for a, _, _ in changed)} "
          f"-{sum(r for _, r, _ in changed)} lines; by added lines:")
    for a, r, x in sorted(changed, key=lambda row: row[0], reverse=True)[:25]:
        print(f"  {x.owner}::{x.name}: +{a} -{r} (now {len(x.body.splitlines())} lines)")
    print("\n-- fns still in the fork's game.rs that the base has in another file")
    print(f"genuine remnants: {len(genuine)} fns, {size(x for x, _ in genuine)} lines")
    for x, ds in genuine:
        print(f"  {x.owner}::{x.name} ({x.line}): " + ", ".join(f"{d.path}:{d.line} ({d.owner})" for d in ds))
    print(f"classified name-only matches: {len(classified)} fns, {size(x for x, _ in classified)} lines")
    for x, matches in classified:
        print(f"  {x.owner}::{x.name}: " + "; ".join(entries[k] for _, k in matches))
    stale = entries.keys() - used
    for key in sorted(stale):
        print(f"stale classification: {' '.join(key)}; delete it (no name-only match)")
    return int(bool(genuine or stale))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fork_ref")
    parser.add_argument("base_ref")
    parser.add_argument("--repo", type=Path, default=ROOT)
    parser.add_argument("--classifications", type=Path, default=Path(__file__).with_suffix(".txt"))
    args = parser.parse_args(argv)
    return report(args.repo, args.fork_ref, args.base_ref, args.classifications)


if __name__ == "__main__":
    raise SystemExit(main())
