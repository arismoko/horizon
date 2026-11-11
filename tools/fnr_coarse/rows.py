#!/usr/bin/env python3
"""IR owner names -> `[[native.mixin]]` row targets, by finding the defining file in the source.
rows.py SRC proto-sets.json KEY -> prints `target\trole` lines"""
import json, re, subprocess, sys
from pathlib import Path
src = Path(sys.argv[1])
sets = json.load(open(sys.argv[2]))[sys.argv[3]]
game = src / 'crates/horizon-client/src'


def module_of(f: Path):
    rel = f.relative_to(game)
    parts = list(rel.parts)
    if parts[0] == 'game.rs':
        return 'horizon_client::game'
    if parts[0] == 'game':
        parts = parts[1:]
        # area dir with .game-area marker: logical module is a direct child of game
        if len(parts) > 1 and (game / 'game' / parts[0] / '.game-area').exists():
            parts = parts[1:]
        mods = [p[:-3] if p.endswith('.rs') else p for p in parts]
        if mods[-1] == 'mod':
            mods = mods[:-1]
        return 'horizon_client::game::' + '::'.join(mods)
    mods = [p[:-3] if p.endswith('.rs') else p for p in parts]
    if mods[-1] == 'mod':
        mods = mods[:-1]
    return 'horizon_client::' + '::'.join(mods)


def find(name, ty, tr):
    out = subprocess.run(['grep', '-rlE', rf'\bfn\s+{name}\b', str(game)], capture_output=True, text=True).stdout.split()
    hits = []
    for f in out:
        t = Path(f).read_text()
        for m in re.finditer(rf'\bfn\s+{name}\b', t):
            # nearest preceding impl header
            head = t[:m.start()]
            imps = list(re.finditer(r'^impl\b([^{]*)\{', head, re.M))
            if ty is None:
                if not imps or head.rfind('\n}') > imps[-1].start() if imps else True:
                    hits.append((Path(f), None))
                continue
            if not imps:
                continue
            h = imps[-1].group(1)
            if re.search(rf'\b{ty}\b', h) and (not tr or re.search(rf'\b{tr}\b', h)) and (tr or ' for ' not in h):
                hits.append((Path(f), h.strip()))
    return hits


for role in ('copy', 'entries'):
    for n in sets[role]:
        m = re.match(r'^<([\w:]+)(?: as ([\w:]+))?>::(\w+)$', n)
        if m:
            ty = m.group(1).split('::')[-1]
            tr = (m.group(2) or '').split('::')[-1]
            hits = find(m.group(3), ty, tr)
            if len(hits) != 1:
                print(f'#AMBIGUOUS {n} {[(str(h[0]), h[1]) for h in hits]}')
                continue
            mod = module_of(hits[0][0])
            if tr:
                print(f"{mod}::<{ty}<'_> as {tr}>::{m.group(3)}\t{role}\t{n}")
            else:
                lt = "<'_>" if ty in ('Game', 'GameLevel', 'Containers') else ''
                print(f"{mod}::{ty}{lt}::{m.group(3)}\t{role}\t{n}")
        else:
            print(f'{n}\t{role}\t{n}')
