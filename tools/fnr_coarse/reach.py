#!/usr/bin/env python3
"""Reach of coarse entries: reach.py ograph.json OUT.json
Entries: phase (drivers and their direct callees), root (no game caller), indirect (address taken).
For every target: the functions on all call paths up to the nearest entries, the entries reached,
the number of distinct paths, and how many of them end at an indirect-only entry."""
import json, re, sys, csv
from collections import defaultdict

o = json.load(open(sys.argv[1]))
callers = o['callers']
callees = defaultdict(set)
for k, v in callers.items():
    for p in v:
        callees[p].add(k)
G = '<horizon_client::game::Game>::'
DRIVERS = [G + n for n in ('tick', 'tick_world', 'tick_server', 'tick_server_body', 'tick_guests',
                           'handle_packet', 'as_guest', 'tick_lan_packets', 'tick_lan')]
DRIVERS = [d for d in DRIVERS if d in callers]
phase = set(DRIVERS)
for d in DRIVERS:
    phase |= callees[d]
root = {k for k, v in callers.items() if not v}
indirect = set(o['fn_taken'])
entries = phase | root | indirect


def kind(e):
    return 'phase' if e in phase else 'root' if e in root else 'indirect' if e in indirect else 'checked'


# match targets written as rows write them
idx = defaultdict(set)
for k in callers:
    m = re.match(r'^<([^<>]*?)(?: as ([^<>]*))?>::(\w+)$', k)
    if m:
        ty = m.group(1).split('::')[-1]
        tr = (m.group(2) or '').split('::')[-1]
        idx[(ty, tr, m.group(3))].add(k)
    else:
        idx[('', '', k)].add(k)
        parts = k.split('::')
        idx[('', '', '::'.join(parts[-2:]))].add(k)


def resolve(t):
    t = re.sub(r"<'[_a-z]+>", '', t).replace("'_", '')
    m = re.match(r'^(.*)::<\s*([\w:]+)\s+as\s+([\w:]+)\s*>::(\w+)$', t)
    if m:
        return idx.get((m.group(2).split('::')[-1], m.group(3).split('::')[-1], m.group(4)), set())
    parts = t.split('::')
    if len(parts) >= 2 and parts[-2][:1].isupper():
        return idx.get((parts[-2], '', parts[-1]), set())
    return idx.get(('', '', t), set()) or idx.get(('', '', '::'.join(parts[-2:])), set())


CAP = 40000


def up(f):
    """Up-closure from f, stopping at entries (f itself is walked even if an entry)."""
    seen = {f}
    stack = [f]
    reached = set()
    while stack and len(seen) < CAP:
        x = stack.pop()
        if x != f and x in entries:
            reached.add(x)
            continue
        for p in callers.get(x, ()):
            if p not in seen:
                seen.add(p)
                stack.append(p)
    copy = {x for x in seen if x not in reached}
    return copy, reached, len(seen) >= CAP


def paths(f, copy, reached):
    """Distinct simple paths from any reached entry down to f, cycles condensed (counted once)."""
    sub = copy | reached
    memo = {}
    onstack = set()

    def n(x):
        if x in reached:
            return 1
        if x in memo:
            return memo[x]
        if x in onstack:
            return 0
        onstack.add(x)
        t = sum(n(p) for p in callers.get(x, ()) if p in sub)
        onstack.discard(x)
        memo[x] = t
        return t
    sys.setrecursionlimit(100000)
    total = n(f)
    # paths ending at indirect-kind entries
    ind = {e for e in reached if kind(e) == 'indirect'}
    memo.clear()

    def m(x):
        if x in reached:
            return 1 if x in ind else 0
        if x in memo:
            return memo[x]
        if x in onstack:
            return 0
        onstack.add(x)
        t = sum(m(p) for p in callers.get(x, ()) if p in sub)
        onstack.discard(x)
        memo[x] = t
        return t
    return total, m(f)


def report(target):
    found = resolve(target)
    if len(found) != 1:
        return {'target': target, 'status': 'missing' if not found else 'ambiguous', 'matches': sorted(found)}
    f = next(iter(found))
    copy, reached, capped = up(f)
    total, ind = paths(f, copy, reached)
    k = defaultdict(int)
    for e in reached:
        k[kind(e)] += 1
    return {'target': target, 'node': f, 'status': 'ok', 'self_entry': kind(f) if f in entries else None,
            'self_address_taken': f in indirect, 'callers': len(callers[f]), 'copy_fns': len(copy),
            'copy_ir_lines': sum(o['size'].get(x, 0) for x in copy), 'entries': len(reached),
            'entry_kinds': dict(k), 'paths': total, 'paths_via_indirect': ind, 'capped': capped,
            'entries_list': sorted(reached)[:40]}


if __name__ == '__main__':
    groups = json.load(open(sys.argv[2]))
    out = {'drivers': DRIVERS, 'n_phase': len(phase), 'n_root': len(root), 'n_indirect': len(indirect),
           'n_entries': len(entries), 'n_owners': len(callers), 'groups': {}}
    for gname, targets in groups.items():
        out['groups'][gname] = [report(t) for t in targets]
    json.dump(out, open(sys.argv[3], 'w'), indent=1)
    print({k: v for k, v in out.items() if k != 'groups'})
