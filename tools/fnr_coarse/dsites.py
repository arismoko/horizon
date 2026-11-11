#!/usr/bin/env python3
"""The replacement checks of a staged D tree, mapped to IR owner names: dsites.py STAGE ograph.json OUT.json"""
import json, re, sys, os
from pathlib import Path
argv = sys.argv
sys.argv = [argv[0], argv[2]]
exec(open(os.path.join(os.path.dirname(__file__), 'reach.py')).read().split("if __name__")[0])
stage = Path(argv[1])
CHECK = re.compile(r'if let ::std::option::Option::Some\(__fnr\) = ([^;]*?)__fnr_read_slot_(\d+)\(\) \{ return __fnr\(')
READER = re.compile(r'(?:impl\s*(<[^{]*?>)?\s*([^{]*?)\s*\{|\})\s*# \[doc \(hidden\)\] # \[inline \(always\)\] # \[allow \(unsafe_code , dead_code , clippy :: all\)\] pub fn __fnr_read_slot_(\d+) \(\).*?pub fn (__fnr_\w+) \(', re.S)
sites = []
for crate_dir in sorted((stage / 'crates').iterdir()):
    src = crate_dir / 'src'
    if not src.is_dir():
        continue
    readers = {}
    files = list(src.rglob('*.rs'))
    texts = {}
    for f in files:
        t = f.read_text(errors='replace')
        if '__fnr_read_slot_' not in t:
            continue
        texts[f] = t
        for m in READER.finditer(t):
            ty = (m.group(2) or '').strip()
            ty = re.sub(r'\s+', '', ty)
            ty = re.sub(r'<.*', '', ty).split('::')[-1]
            readers[int(m.group(3))] = (ty, m.group(4), f)
    for f, t in texts.items():
        for m in CHECK.finditer(t):
            slot = int(m.group(2))
            head = t[:m.start()]
            fns = re.findall(r'\bfn\s+(r#)?(\w+)', head[-4000:])
            name = fns[-1][1] if fns else None
            ty, inst, rf = readers.get(slot, ('', None, None))
            tr = ''
            if inst and inst.startswith('__fnr_as_'):
                tr = inst[len('__fnr_as_'):-len(name) - 1] if name and inst.endswith('_' + name) else '?'
            line = t.count('\n', 0, m.start()) + 1
            sites.append({'crate': crate_dir.name, 'file': str(f.relative_to(stage)), 'line': line, 'slot': slot,
                          'fn': name, 'type': ty, 'trait': tr, 'start': m.start(), 'end': m.end()})
# map
for s in sites:
    if s['crate'] != 'horizon-client':
        s['node'] = None
        continue
    if s['type']:
        cand = idx.get((s['type'], s['trait'], s['fn']), set())
    else:
        p = Path(s['file'])
        mod = p.parent.name if p.stem == 'mod' else p.stem
        cand = {k for k in idx.get(('', '', f"{mod}::{s['fn']}"), set())}
        if not cand:
            cand = {k for k in callers if k.endswith('::' + s['fn']) and not k.startswith('<')}
            if len(cand) > 1:
                cand = {k for k in cand if f'::{mod}::' in k} or cand
    s['node'] = next(iter(cand)) if len(cand) == 1 else None
    s['cands'] = len(cand)
json.dump(sites, open(argv[3], 'w'))
from collections import Counter
print(len(sites), 'sites', Counter(s['crate'] for s in sites))
hc = [s for s in sites if s['crate'] == 'horizon-client']
print('horizon-client mapped', sum(1 for s in hc if s['node']), 'unmapped', sum(1 for s in hc if not s['node']),
      'ambiguous', sum(1 for s in hc if s.get('cands', 0) > 1))
