#!/usr/bin/env python3
"""Owner-level call graph: ograph.py graph.json ograph.json"""
import json, sys
from collections import defaultdict, deque
sys.path.insert(0, __import__('os').path.dirname(__file__))
from owners import load

g, name, own = load(sys.argv[1])
defined = set(g['defined'])
rev = defaultdict(set)
for s, cs in g['callees'].items():
    for c in cs:
        if c in defined:
            rev[c].add(s)
owners = defaultdict(list)
for s, o in own.items():
    if o:
        owners[o].append(s)
callers = {}
via_std = defaultdict(set)
for o, syms in owners.items():
    out = set()
    seen = set(syms)
    q = deque(syms)
    while q:
        s = q.popleft()
        for p in rev[s]:
            if p in seen:
                continue
            seen.add(p)
            po = own.get(p)
            if po is None:
                q.append(p)
            elif po != o:
                out.add(po)
                if s not in syms:
                    via_std[o].add(po)
    callers[o] = sorted(out)
taken = g['taken']
fn_taken, closure_taken, indirect_calls, size = {}, {}, {}, {}
for o, syms in owners.items():
    ft = [s for s in syms if s in taken and '{closure' not in name[s]]
    ct = [s for s in syms if s in taken and '{closure' in name[s]]
    if ft:
        fn_taken[o] = sorted({w for s in ft for w in taken[s]})
    if ct:
        closure_taken[o] = len(ct)
    n = sum(g['indirect'].get(s, 0) for s in syms)
    if n:
        indirect_calls[o] = n
    size[o] = sum(g['size'].get(s, 0) for s in syms)
json.dump({'callers': callers, 'fn_taken': fn_taken, 'closure_taken': closure_taken,
           'indirect_calls': indirect_calls, 'size': size,
           'via_std': {k: sorted(v) for k, v in via_std.items()}}, open(sys.argv[2], 'w'))
print(len(owners), 'owners;', sum(1 for c in callers.values() if not c), 'with no game caller;',
      len(fn_taken), 'address-taken;', len(closure_taken), 'with address-taken closures;',
      len(indirect_calls), 'making indirect calls')
