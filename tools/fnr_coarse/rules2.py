#!/usr/bin/env python3
"""Rules restricted to the stager's eligible functions (D's checks), with stored-closure edges.
rules2.py graph.json ograph.json targets.json lostC.json dsites.json OUT.json"""
import json, sys, statistics as st
from collections import defaultdict, deque
argv = sys.argv
sys.argv = [argv[0], argv[2]]
exec(open(__file__.replace('rules2.py', 'reach.py')).read().split("if __name__")[0])
sys.path.insert(0, __file__.rsplit('/', 1)[0])
from owners import owner
g = json.load(open(argv[1]))
targets = json.load(open(argv[3]))
lost = json.load(open(argv[4]))
dsites = json.load(open(argv[5]))
eligible = {s['node'] for s in dsites if s.get('node')}
name = g['name']
defined = set(g['defined'])
own = {s: owner(name[s]) for s in defined}
# stored-closure edges: owner o's address-taken closure reaching owner t (through std nodes)
closure_edges = set()
for s in defined:
    if s in g['taken'] and '{closure' in name[s] and own.get(s):
        o0 = own[s]
        seen = {s}; q = deque([s])
        while q:
            x = q.popleft()
            for c in g['callees'].get(x, ()):
                if c in seen or c not in defined:
                    continue
                seen.add(c)
                oc = own.get(c)
                if oc is None:
                    q.append(c)
                elif oc != o0:
                    closure_edges.add((o0, oc))
lost_nodes = {}
for n, t in lost:
    r = resolve(t)
    if len(r) == 1:
        lost_nodes[next(iter(r))] = n
size = o['size']
fanin = {k: len(v) for k, v in callers.items()}
base = phase | root | indirect


def evaluate(label, wanted):
    global entries
    placed = wanted & eligible
    gaps = wanted - eligible
    # a wanted entry the stager can't check: a phase/root one is replaced by walking further up
    # (it stays a pass-through); an address-taken one can't be covered at all
    uncoverable = {x for x in gaps if x in indirect}
    entries = placed | uncoverable
    res = {'rule': label, 'wanted': len(wanted), 'checks': len(placed), 'uncoverable_indirect': len(uncoverable),
           'lost_inline_kept': sum(1 for k in lost_nodes if k in placed),
           'lost_inline_sites_kept': sum(v for k, v in lost_nodes.items() if k in placed),
           'lost_inline_matched': len(lost_nodes), 'groups': {}}
    for gname, ts in targets.items():
        rows = []
        for t in ts:
            r = resolve(t)
            if len(r) != 1:
                continue
            f = next(iter(r))
            if f in placed:
                copy, reached, capped, np_, ni = {f}, set(), False, 1, 0
            else:
                copy, reached, capped = up(f)
                np_, ni = paths(f, copy, reached)
            bad = sorted(e for e in reached if e in uncoverable)
            cl = sum(1 for x in copy for p in callers.get(x, ()) if (p, x) in closure_edges and p in copy | reached)
            rows.append({'target': t, 'node': f, 'own_check': f in placed, 'copy_fns': len(copy),
                         'copy_ir_lines': sum(size.get(x, 0) for x in copy), 'entries': len(reached),
                         'paths': np_, 'paths_via_indirect_entry': ni, 'uncoverable_entries': bad,
                         'stored_closure_edges': cl, 'capped': capped})
        def q(k):
            v = sorted(x[k] for x in rows)
            return {'median': st.median(v), 'p90': v[int(.9 * len(v))], 'max': v[-1]}
        res['groups'][gname] = {'n': len(rows), 'own_check': sum(x['own_check'] for x in rows),
                                'copy_fns': q('copy_fns'), 'entries': q('entries'), 'paths': q('paths'),
                                'with_indirect_entry_paths': sum(1 for x in rows if x['paths_via_indirect_entry']),
                                'refused_uncoverable': sum(1 for x in rows if x['uncoverable_entries']),
                                'with_stored_closure_edge': sum(1 for x in rows if x['stored_closure_edges']),
                                'rows': rows}
    return res, placed


tok = {x['node']: x['tokens'] for x in json.load(open('dsizes.json')) if x.get('node')}
rules = {
    'F': ('R1 + every fn whose body exceeds 120 tokens', base | {k for k, v in tok.items() if v > 120}),
    'FT': ('F + every trait-impl method', base | {k for k, v in tok.items() if v > 120} | {k for k in callers if ' as ' in k}),
    'F80': ('R1 + body > 80 tokens', base | {k for k, v in tok.items() if v > 80}),
    'F240': ('R1 + body > 240 tokens', base | {k for k, v in tok.items() if v > 240}),
    'R1': ('coarse: phase+root+address-taken', base),
    'R2k4': ('R1 + fan-in>=4', base | {x for x, n in fanin.items() if n >= 4}),
    'R2k8': ('R1 + fan-in>=8', base | {x for x, n in fanin.items() if n >= 8}),
    'R5': ('R1 + (fan-in>=4 and opt0 IR > 150 lines)', base | {x for x, n in fanin.items() if n >= 4 and size.get(x, 0) > 150}),
    'R3s150': ('root+address-taken+every fn with opt0 IR > 150 lines', root | indirect | {x for x in callers if size.get(x, 0) > 150}),
    'R3s300': ('root+address-taken+every fn with opt0 IR > 300 lines', root | indirect | {x for x in callers if size.get(x, 0) > 300}),
    'ALL': ('every eligible fn (D)', set(callers)),
}
out = {'eligible_mapped': len(eligible), 'closure_edges': len(closure_edges), 'rules': {}}
placed_sets = {}
for k, (label, wanted) in rules.items():
    r, placed = evaluate(label, wanted)
    out['rules'][k] = r
    placed_sets[k] = sorted(placed)
    print(f"{k} {label}: wanted {r['wanted']} checks {r['checks']} uncoverable {r['uncoverable_indirect']} "
          f"lost-inline kept {r['lost_inline_kept']}/{r['lost_inline_matched']} sites {r['lost_inline_sites_kept']}")
    for gname, v in r['groups'].items():
        print(f"   {gname}: n {v['n']} own {v['own_check']} copy {v['copy_fns']} entries med {v['entries']['median']} max {v['entries']['max']} "
              f"paths med {v['paths']['median']} max {v['paths']['max']} via-indirect {v['with_indirect_entry_paths']} "
              f"refused {v['refused_uncoverable']} stored-closure {v['with_stored_closure_edge']}")
json.dump(out, open(argv[6], 'w'), indent=1)
json.dump(placed_sets, open(argv[6].replace('.json', '-placed.json'), 'w'))
