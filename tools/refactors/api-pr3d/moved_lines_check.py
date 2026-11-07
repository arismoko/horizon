#!/usr/bin/env python3
"""Prove moved renderer lines and report the remaining hand-written change size."""
import argparse
from collections import Counter
import difflib
import json
import hashlib
from pathlib import Path
import subprocess

from move import EXPORT, SOURCE, rewrite


def git(root, *args):
    return subprocess.check_output(["git", *args], cwd=root)


def old_file(root, base, name):
    names = git(root, "ls-tree", "--name-only", base, "--", name).decode()
    return git(root, "show", f"{base}:{name}").decode() if names else ""


def lines(text):
    # Only module/import declarations are exempt; comments and whitespace are not.
    return Counter(
        line for line in text.splitlines()
        if not line.lstrip().startswith(("use ", "pub use ", "mod ", "pub mod "))
    )


def apply_seam(text, entries):
    for entry in entries:
        new = entry["new"]
        if "start" in entry:
            start = text.index(entry["start"])
            end = text.index(entry["end"], start)
            old, count = text[start:end], 1
            if hashlib.sha256(old.encode()).hexdigest() != entry["sha256"]:
                raise ValueError("declared seam source hash mismatch")
        else:
            old, count = entry["old"], entry["count"]
        if not old or text.count(old) != count:
            raise ValueError(f"declared seam occurrence mismatch: {old!r}")
        text = text.replace(old, new)
    return text


def hand_lines(before, after):
    return sum(
        (end - start) + (new_end - new_start)
        for kind, start, end, new_start, new_end in difflib.SequenceMatcher(
            None, before.splitlines(), after.splitlines(), autojunk=False
        ).get_opcodes()
        if kind != "equal"
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--destination-crate", required=True)
    parser.add_argument("--destination-lib", required=True)
    parser.add_argument("--seam", type=Path, default=Path(__file__).with_name("seam.json"))
    args = parser.parse_args()
    root = args.root
    destination = f"crates/{args.destination_crate}/src/render.rs"
    original = old_file(root, args.base, SOURCE)
    if not original:
        raise ValueError("base has no original renderer")
    expected = rewrite(apply_seam(original, json.loads(args.seam.read_text())), args.destination_lib)
    actual = (root / destination).read_text()
    removed, added = lines(expected) - lines(actual), lines(actual) - lines(expected)
    if removed or added:
        raise ValueError(f"moved-lines mismatch: missing {dict(removed)}, extra {dict(added)}")
    if (root / SOURCE).exists() or (root / EXPORT).exists():
        raise ValueError("old API render source or exports still exists")
    # A seam inside the moved file counts, not the entire 700-line move.
    total = hand_lines(original, expected)
    names = set(git(root, "diff", "--name-only", args.base).decode().splitlines())
    names.update(git(root, "ls-files", "--others", "--exclude-standard").decode().splitlines())
    for name in sorted(names - {SOURCE, destination}):
        if name.endswith("Cargo.lock"):
            continue
        before = old_file(root, args.base, name)
        expected = rewrite(before, args.destination_lib) if name.endswith((".rs", ".md", ".txt")) else before
        path = root / name
        after = path.read_text() if path.exists() else ""
        count = hand_lines(expected, after)
        if count:
            print(f"hand-written {count:4} {name}")
        total += count
    print(f"Moved-lines multiset matches; hand-written added + removed: {total}")
    if total > 500:
        raise ValueError("hand-written changes exceed the 500-line cap; split a complete layer")


if __name__ == "__main__":
    main()
