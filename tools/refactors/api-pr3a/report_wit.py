#!/usr/bin/env python3
"""Report the 3a type delta and reject reordered existing enum/variant cases.

Usage: report_wit.py BASE.wit HEAD.wit
Function changes use the existing CI reporter; type blocks use PR 1's parser.
"""
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "api-pr1"))
from wit_order import canonical_wit


def types(path):
    _, groups = canonical_wit(Path(path).read_text())
    return {
        re.match(r"  (?:type|record|variant|enum|flags|resource) ([\w%-]+)", block)[1]: block
        for block in groups[("types", "types")]
    }


before, after = map(types, sys.argv[1:])
print("### WIT type delta\n")
for label, names in [
    ("Added", after.keys() - before.keys()),
    ("Changed", {name for name in before.keys() & after.keys() if before[name] != after[name]}),
    ("Removed", before.keys() - after.keys()),
]:
    print(f"**{label} types** ({len(names)}): " + (", ".join(f"`{name}`" for name in sorted(names)) or "none") + ".\n")
    for kind in ["record", "enum", "variant"]:
        matching = [name for name in sorted(names) if (after.get(name) or before[name]).startswith(f"  {kind} ")]
        print(f"- {kind}s: " + (", ".join(f"`{name}`" for name in matching) or "none"))
    print()

def cases(block):
    body = block.partition("{")[2].rpartition("}")[0]
    depth, start, result = 0, 0, []
    for index, char in enumerate(body + ","):
        if char in "(<":
            depth += 1
        elif char in ")>":
            depth -= 1
        elif char == "," and depth == 0:
            value = body[start:index].strip()
            match = re.fullmatch(r"([\w%-]+)(?:\(.*\))?", value, re.S)
            if not match:
                raise ValueError(f"Unrecognized WIT case: {value!r}")
            result.append(match[1])
            start = index + 1
    if depth or not result:
        raise ValueError("Invalid or empty WIT case list")
    return result


checked = 0
for name in sorted(before.keys() & after.keys()):
    if not re.match(r"  (enum|variant) ", before[name]):
        continue
    old, new = cases(before[name]), cases(after[name])
    if new[:len(old)] != old:
        raise SystemExit(f"Existing cases reordered or removed: {name}: {old} -> {new}")
    checked += 1
print(f"Existing enum/variant case order preserved in all {checked} common definitions (new cases, if any, only append).")
