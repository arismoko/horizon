#!/usr/bin/env python3
"""Body token counts of D's checked functions (brace-matched from the check): bodysize.py STAGE dsites.json OUT.json"""
import json, re, sys
from pathlib import Path
stage = Path(sys.argv[1])
sites = json.load(open(sys.argv[2]))
TOK = re.compile(r'"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])\'|//[^\n]*|/\*.*?\*/|[A-Za-z_]\w*|\d[\w.]*|\S', re.S)
cache = {}
for s in sites:
    t = cache.get(s['file'])
    if t is None:
        t = cache[s['file']] = (stage / s['file']).read_text(errors='replace')
    depth, n, calls = 2, 0, 0
    prev = ''
    for m in TOK.finditer(t, s['end']):
        tok = m.group(0)
        if tok.startswith('//') or tok.startswith('/*'):
            continue
        if tok == '{':
            depth += 1
        elif tok == '}':
            depth -= 1
            if depth == 0:
                break
        elif tok == '(' and re.match(r'[A-Za-z_]\w*$', prev) and prev not in ('if', 'while', 'match', 'for', 'return', 'in'):
            calls += 1
        n += 1
        prev = tok
    # the check itself: up to its closing brace
    s['tokens'] = n
    s['calls'] = calls
json.dump(sites, open(sys.argv[3], 'w'))
v = sorted(s['tokens'] for s in sites)
print(len(v), 'median', v[len(v) // 2], 'p25', v[len(v) // 4], 'p75', v[3 * len(v) // 4])
