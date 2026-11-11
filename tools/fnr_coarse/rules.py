#!/usr/bin/env python3
"""Compare check-placement rules: rules.py ograph.json targets.json kept-out-of-line-C.json OUT.json"""
import json, sys, statistics as st
argv = sys.argv
sys.argv = [argv[0], argv[1]]
exec(open(__file__.replace('rules.py', 'reach.py')).read().split("if __name__")[0])
targets = json.load(open(argv[2]))
lost = json.load(open(argv[3]))
lost_nodes = {}
for n, t in lost:
    r = resolve(t)
    if len(r) == 1:
        lost_nodes[next(iter(r))] = n
size = o['size']
fanin = {k: len(v) for k, v in callers.items()}
base = phase | root | indirect


def evaluate(label, checked):
    global entries
    entries = checked
    res = {'rule': label, 'checks': len(checked),
           'lost_inline_kept': sum(1 for k in lost_nodes if k in checked),
           'lost_inline_sites_kept': sum(v for k, v in lost_nodes.items() if k in checked),
           'lost_inline_matched': len(lost_nodes), 'groups': {}}
    for g, ts in targets.items():
        rows = []
        for t in ts:
            r = resolve(t)
            if len(r) != 1:
                continue
            f = next(iter(r))
            copy, reached, capped = up(f)
            np_, ni = paths(f, copy, reached)
            rows.append({'target': t, 'node': f, 'own_check': f in checked, 'copy_fns': len(copy),
                         'copy_ir_lines': sum(size.get(x, 0) for x in copy), 'entries': len(reached),
                         'paths': np_, 'paths_via_indirect': ni,
                         'entries_indirect_only': sum(1 for e in reached if e in indirect and e not in phase and e not in root)})
        def q(k):
            v = sorted(x[k] for x in rows)
            return {'median': st.median(v), 'p90': v[int(.9 * len(v))], 'max': v[-1]}
        res['groups'][g] = {'n': len(rows), 'own_check': sum(x['own_check'] for x in rows),
                            'copy_fns': q('copy_fns'), 'entries': q('entries'), 'paths': q('paths'),
                            'with_indirect_paths': sum(1 for x in rows if x['paths_via_indirect']),
                            'worst': sorted(rows, key=lambda x: -x['copy_fns'])[:5], 'rows': rows}
    return res


out = []
out.append(evaluate('R1 coarse: phase+root+address-taken', base))
for k in (4, 8, 16):
    out.append(evaluate(f'R2 coarse + fan-in>={k}', base | {x for x, n in fanin.items() if n >= k}))
for s in (40, 80, 150, 300):
    out.append(evaluate(f'R3 root+address-taken+opt0-IR-size>{s}', root | indirect | {x for x in callers if size.get(x, 0) > s}))
json.dump(out, open(argv[4], 'w'), indent=1)
for r in out:
    print(f"{r['rule']}: checks {r['checks']}, lost-inline fns kept {r['lost_inline_kept']}/{r['lost_inline_matched']} "
          f"(sites {r['lost_inline_sites_kept']})")
    for g, v in r['groups'].items():
        print(f"   {g}: n {v['n']} own_check {v['own_check']} copy {v['copy_fns']} entries {v['entries']['median']}/{v['entries']['max']} "
              f"paths med {v['paths']['median']} max {v['paths']['max']} indirect {v['with_indirect_paths']}")
