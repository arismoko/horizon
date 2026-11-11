#!/usr/bin/env python3
"""Variant F from the staged D tree: keep a replacement check only where the rule places one.
make_f.py STAGE_D STAGE_F dsizes.json rules2-placed.json RULE
Kept: every function the rule places a check on, every check outside horizon-client, and every check
the call graph could not name (conservative). Everything else loses its first-line check; its
reader and installer stay (dead code)."""
import json, re, shutil, sys, subprocess
from pathlib import Path
from collections import defaultdict
src, dst = Path(sys.argv[1]), Path(sys.argv[2])
sites = json.load(open(sys.argv[3]))
placed = set(json.load(open(sys.argv[4]))[sys.argv[5]])
assert not dst.exists()
shutil.copytree(src, dst, symlinks=True)
subprocess.run(['git', 'init', '-q'], cwd=dst, check=True)
CHECK = re.compile(r'if let ::std::option::Option::Some\(__fnr\) = [^;]*?__fnr_read_slot_\d+\(\) \{ return __fnr\([^;]*\); \}')
drop = defaultdict(list)
kept = removed = 0
keep_list = []
for s in sites:
    if s['crate'] != 'horizon-client' or not s.get('node') or s['node'] in placed:
        kept += 1
        keep_list.append(f"{s['file']}:{s['line']}\t{s.get('node') or s['fn']}")
        continue
    drop[s['file']].append(s['start'])
for f, starts in drop.items():
    p = dst / f
    t = p.read_text()
    for st in sorted(starts, reverse=True):
        m = CHECK.match(t, st)
        assert m, (f, st, t[st:st + 200])
        t = t[:st] + t[m.end():]
        removed += 1
    p.write_text(t)
(dst / 'fnr-coarse-kept.txt').write_text('\n'.join(sorted(keep_list)) + '\n')
print('kept', kept, 'removed', removed)
