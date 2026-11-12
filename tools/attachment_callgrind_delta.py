#!/usr/bin/env python3
"""Subtract two one-event callgrind profiles; report self and call-edge Ir/access."""
import collections
import json
from pathlib import Path
import re
import sys


def read(path):
    names = {}
    own = collections.Counter()
    edges = collections.Counter()
    current = callee = None
    pending = False
    total = 0
    for line in Path(path).read_text().splitlines():
        match = re.match(r"(c?fn)=\((\d+)\)(?: (.*))?$", line)
        if match:
            kind, ident, name = match.groups()
            if name is not None:
                names[ident] = name
            if kind == "fn":
                current = ident
            else:
                callee = ident
        elif line.startswith("summary:"):
            total += int(line.split()[1])
        elif line.startswith("calls="):
            pending = True
        elif re.match(r"^[\d+*-]+ ", line):
            cost = int(line.split()[1])
            if pending:
                edges[current, callee] += cost
                pending = False
            else:
                own[current] += cost
    named_own = collections.Counter()
    named_edges = collections.Counter()
    for key, value in own.items():
        named_own[names[key]] += value
    for (source, target), value in edges.items():
        named_edges[names[source], names[target]] += value
    assert sum(named_own.values()) == total, (sum(named_own.values()), total)
    return total, named_own, named_edges


a, b = map(read, sys.argv[1:3])
normalizer = 14000
result = {"instructions_per_access": (b[0] - a[0]) / normalizer}
for label, slot in [("self", 1), ("edges", 2)]:
    rows = []
    for key in a[slot].keys() | b[slot].keys():
        delta = (b[slot].get(key, 0) - a[slot].get(key, 0)) / normalizer
        if delta:
            rows.append({"function": key, "instructions_per_access": delta})
    result[label] = sorted(rows, key=lambda row: -row["instructions_per_access"])
print(json.dumps(result, indent=2))
