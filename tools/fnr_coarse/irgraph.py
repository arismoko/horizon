#!/usr/bin/env python3
"""Call graph from opt-level-0 LLVM IR (v0 mangling): irgraph.py OUT.json FILE.ll...

Nodes are defined functions. For each: direct callees, whether it makes indirect calls,
and whether its address is taken (vtable entry, fn pointer stored or passed).
Names are demangled with c++filt and crate hashes are stripped."""
import json, re, subprocess, sys
from collections import defaultdict

SYM = r'(?:"(_R[^"]+)"|(_R[0-9A-Za-z_$.]+))'
define_re = re.compile(r'^define [^@]*@' + SYM + r'\(')
call_re = re.compile(r'\b(?:call|invoke|tail call)\b[^@%]*?(?:@' + SYM + r'|(%[\w.]+))\(')
ref_re = re.compile(r'@' + SYM)

out, files = sys.argv[1], sys.argv[2:]
callees = defaultdict(set)
indirect = defaultdict(int)
taken = defaultdict(set)  # symbol -> set of where (vtable / function names)
defined = set()
size = defaultdict(int)
for f in files:
    cur = None
    for line in open(f, errors='replace'):
        if cur is None:
            m = define_re.match(line)
            if m:
                cur = m.group(1) or m.group(2)
                defined.add(cur)
                continue
            if line.startswith('@') and ' = ' in line:
                # global: vtable or other constant; every function it names is address-taken
                gname = line.split(' = ', 1)[0]
                for r in ref_re.finditer(line.split(' = ', 1)[1]):
                    taken[r.group(1) or r.group(2)].add('global:' + gname[:40])
            continue
        if line.startswith('}'):
            cur = None
            continue
        size[cur] += 1
        m = call_re.search(line)
        target = None
        if m:
            target = m.group(1) or m.group(2)
            if target:
                callees[cur].add(target)
            elif m.group(3) and 'asm' not in line:
                indirect[cur] += 1
        for r in ref_re.finditer(line):
            s = r.group(1) or r.group(2)
            if s != target:
                taken[s].add('fn')

syms = sorted(defined | set(taken) | {c for cs in callees.values() for c in cs})
dem = subprocess.run(['c++filt'], input='\n'.join(syms), capture_output=True, text=True).stdout.split('\n')
clean = lambda s: re.sub(r'\[[0-9a-f]+\]', '', s)
name = {s: clean(d) for s, d in zip(syms, dem)}
json.dump({
    'defined': sorted(defined),
    'name': name,
    'callees': {k: sorted(v) for k, v in callees.items()},
    'indirect': indirect,
    'taken': {k: sorted(v) for k, v in taken.items()},
    'size': size,
}, open(out, 'w'))
print(len(defined), 'defined', sum(len(v) for v in callees.values()), 'edges',
      sum(1 for v in indirect.values() if v), 'fns with indirect calls', len(taken), 'address-taken')
