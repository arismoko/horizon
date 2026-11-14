#!/usr/bin/env python3
"""Move ingredient tests mechanically; preserve every function verbatim before rustfmt.

--source accepts an archived monolithic source when porting a merged author head.
"""
import argparse
import collections
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[2]
BASE = ROOT / 'crates/horizon-client/src/crafting/ingredient'
OLD = BASE / 'builtin_kinds_tests.rs'
DEST = BASE / 'builtin_kinds_tests'


def family(name):
    if any(x in name for x in ('color', 'colour', 'collar', 'dye')):
        return 'colour'
    if any(x in name for x in ('fuel', 'compostable', 'food')):
        return 'fuel'
    if any(x in name for x in ('variant',)):
        return 'enums'
    if any(x in name for x in ('damage', 'map_id', 'amplifier', 'duration_scale', 'attack_charge', 'cooldown')):
        return 'numeric'
    return 'rest'


def split(source):
    starts = list(re.finditer(r'(?m)^((?:#\[[^\n]*\]\n)*)fn (\w+)\(', source))
    header = source[:starts[0].start()]
    groups = collections.defaultdict(list)
    selectors = {}
    for i, match in enumerate(starts):
        name = match[2]
        body = source[match.start():starts[i + 1].start() if i + 1 < len(starts) else len(source)]
        target = 'shared' if name in ('load', 'parse', 'resources') else family(name)
        groups[target].append(body)
        if '#[test]' in match[1].splitlines():
            selectors['crafting::ingredient::builtin_kinds_tests::' + name] = 'crafting::ingredient::builtin_kinds_tests::families::' + target + '::' + name
    (DEST / 'families').mkdir(parents=True, exist_ok=True)
    (DEST / 'mod.rs').write_text(header + '\n' + ''.join(groups.pop('shared')) + '\nmod families {\n    use super::*;\n    include!(concat!(env!("OUT_DIR"), "/builtin_ingredient_test_modules.rs"));\n}\n')
    for target, bodies in groups.items():
        (DEST / 'families' / (target + '.rs')).write_text('use super::*;\n\n' + ''.join(bodies))
    if OLD.exists():
        OLD.unlink()
    parent = BASE.parent / 'ingredient.rs'
    parent.write_text(parent.read_text().replace('ingredient/builtin_kinds_tests.rs', 'ingredient/builtin_kinds_tests/mod.rs'))
    # Rows use exact Rust selectors; update only rows containing moved names.
    for row in (ROOT / 'tools/ci/ignored-tests').rglob('*'):
        if row.is_file():
            text = row.read_text()
            updated = text
            for before, after in selectors.items():
                updated = updated.replace(before, after)
            if updated != text:
                row.write_text(updated)
    return selectors


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, default=OLD)
    parser.add_argument('--map', type=Path, required=True)
    args = parser.parse_args()
    args.map.write_text(json.dumps(split(args.source.read_text()), indent=2) + '\n')
