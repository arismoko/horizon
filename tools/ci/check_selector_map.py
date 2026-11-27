#!/usr/bin/env python3
"""Check destination selectors against staged functions, the SDK surface and the Rust test inventory."""

import argparse
from collections import Counter
import json
from pathlib import Path


def problems(selections, functions, surface, tests):
    replaceable = functions["functions"]
    refused = functions["refused"]
    hook_targets = Counter((hook["id"], hook["engine"]["target"]) for hook in surface["hooks"])
    test_names = Counter((test["package"], test["target"], test["name"]) for test in tests)
    errors = []
    for key in selections["replaceable"]:
        if key not in replaceable or key in refused:
            errors.append(f"{key}: selector has no replaceable function in the staged index")
    for hook in selections["hooks"]:
        identity = (hook["id"], hook["target"])
        if hook_targets[identity] != 1:
            errors.append(f"{identity[0]} -> {identity[1]}: expected one engine binding, found {hook_targets[identity]}")
    for test in selections["ignored_tests"]:
        identity = (test["package"], test["target"], test["name"])
        if test_names[identity] != 1:
            errors.append(f"{' '.join(identity)}: expected one test, found {test_names[identity]}")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("selections", type=Path, help="JSON with replaceable keys, hooks and ignored_tests")
    parser.add_argument("functions", type=Path, help="staged functions.json")
    parser.add_argument("surface", type=Path, help="SDK surface.json")
    parser.add_argument("tests", type=Path, help="Rust test inventory (JSON lines from sunrise-game-discovery)")
    args = parser.parse_args()
    functions = json.loads(args.functions.read_text(encoding="utf-8"))
    surface = json.loads(args.surface.read_text(encoding="utf-8"))
    selections = json.loads(args.selections.read_text(encoding="utf-8"))
    tests = [json.loads(line) for line in args.tests.read_text(encoding="utf-8").splitlines() if line.strip()]
    errors = problems(selections, functions, surface, tests)
    for error in errors:
        print(error)
    return int(bool(errors))


if __name__ == "__main__":
    raise SystemExit(main())
