#!/usr/bin/env python3
"""Compare host-manifest outputs across the item type boundary change.

Generate each input with horizon-mod-api-gen's host-manifest binary, including
both generated host unions and src/host/*.rs. This deliberately ignores Rust
signatures: ItemStack changes ownership and tag reads change shape. Names,
owner traits, docs and hidden flags remain checked. Explicit changes are listed
in a JSON file mapping 'owner::method' to old/new complete manifest lines; an
unused exception is an error. No broad allow-list masks collateral changes.

Usage: manifest_gate.py BEFORE AFTER [EXCEPTIONS.json]
"""
import argparse
import json
from pathlib import Path


def parse(text):
    methods = {}
    for line in text.splitlines():
        fields = line.split("\t")
        if len(fields) not in (4, 5):
            raise ValueError(f"invalid manifest line: {line!r}")
        owner, name, signature, docs, *hidden = fields
        key = f"{owner}::{name}"
        if key in methods:
            raise ValueError(f"duplicate method: {key}")
        methods[key] = (line, (owner, name, docs, tuple(hidden)))
    return methods


def compare(before, after, exceptions):
    old, new = parse(before), parse(after)
    failures = []
    for key in sorted(old.keys() | new.keys() | exceptions.keys()):
        left, right = old.get(key), new.get(key)
        if key in exceptions:
            actual = {"old": left[0] if left else None,
                      "new": right[0] if right else None}
            if actual != exceptions[key]:
                failures.append(f"{key}: exception does not match exact old/new lines")
            elif left == right:
                failures.append(f"{key}: unused exception")
        elif left is None or right is None or left[1] != right[1]:
            failures.append(f"{key}: name, owner, docs or hidden flag changed")
    return failures


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    parser.add_argument("exceptions", type=Path, nargs="?")
    args = parser.parse_args()
    exceptions = json.loads(args.exceptions.read_text()) if args.exceptions else {}
    failures = compare(args.before.read_text(), args.after.read_text(), exceptions)
    if failures:
        parser.exit(1, "\n".join(failures) + "\n")
    print("Host method names, owners, docs and hidden flags unchanged outside exact exceptions")


if __name__ == "__main__":
    main()
