#!/usr/bin/env python3
"""Compare the items block entity renderers draw with vanilla 26.3.

The Java oracle (VanillaBlockEntityItemOracle) runs the real CampfireRenderer,
BrushableBlockRenderer, ShelfRenderer and VaultRenderer, with a real ItemModelResolver over
the baked item models, under the pose LevelRenderer gives them. The game test
`game::server_side::campfire::block_entity_items_tests::block_entity_items_match_the_vanilla_cases` runs the
same cases through `campfire_items`, `brushable_items`, `shelf_items` and `vault_items`, the
items `campfire_geometry`, `brushable_geometry`, `shelf_geometry` and `vault_geometry` draw.
Each item handed to the item renderer is compared in order, exactly: the stack, the display
context and seed it was resolved with, the pose's matrix as raw float bits, and the light.

Cases: every facing; one to four items in different slots (block items, flat items, special
models); each dusted level and hit direction; shelves aligned to the bottom or not; vaults with
and without a display item, clusters of each size and a few spin alphas; blocks near the origin,
below zero, at y 319 and near the world border; and camera offsets at chunk and reach boundaries.

Only the pass and fail counts are printed; each mismatch is written to
.local/block-entity-item-geometry-parity/report.txt.

  test_block_entity_item_geometry_parity.py [--filter SUBSTRING]
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import (
    lab,
    test_rust_parity as base,
    test_model_baking_parity as baking,
    test_item_model_parity as item_model,
)

PARITY = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity'
TEST = 'game::server_side::campfire::block_entity_items_tests::block_entity_items_match_the_vanilla_cases'
FACINGS = ['north', 'south', 'west', 'east']
# Near the origin, below zero, at the top of the world, near the world border.
PLACES = [(3, 64, 5), (-7, -40, -13), (12, 319, -3), (29_999_983, 70, -29_999_985)]
CAMERA = (2.37, 1.62, -3.11)
ITEM_SETS = [
    [(0, 'minecraft:beef', 1)],
    [(0, 'minecraft:beef', 1), (1, 'minecraft:stone', 1), (2, 'minecraft:shield', 1), (3, 'minecraft:stick', 1)],
    [(2, 'minecraft:chest', 1), (3, 'minecraft:diamond', 5)],
    [(1, 'minecraft:decorated_pot', 1), (2, 'minecraft:oak_fence', 1)],
]


def spec(name, props):
    return name + ('[' + ','.join(f'{k}={v}' for k, v in props.items()) + ']' if props else '')


def case(name, kind, block, props, pos, light, **extra):
    camera = [pos[0] + CAMERA[0], pos[1] + CAMERA[1], pos[2] + CAMERA[2]]
    return {'name': name, 'kind': kind, 'block': spec(block, props), 'blockName': block, 'props': props,
            'pos': list(pos), 'camera': camera, 'light': list(light), **extra}


def cases():
    out = []
    for p, pos in enumerate(PLACES):
        for f, facing in enumerate(FACINGS):
            for block, emission in (('minecraft:campfire', 15), ('minecraft:soul_campfire', 10)):
                for lit in ('true', 'false'):
                    # A lit campfire's own light is at least its emission, as the light engine leaves it.
                    light = (15 - f, emission) if lit == 'true' else (f * 3, p * 4)
                    items = ITEM_SETS[(f + p) % len(ITEM_SETS)]
                    out.append(case(f'campfire {block} {facing} lit={lit} at {pos}', 'campfire', block,
                                    {'facing': facing, 'lit': lit}, pos, light, items=[list(i) for i in items]))
            for align in (False, True):
                items = ITEM_SETS[(f + p + 1) % len(ITEM_SETS)]
                items = [(slot % 3, item, count) for slot, item, count in items][:3]
                out.append(case(f'shelf {facing} align={align} at {pos}', 'shelf', 'minecraft:oak_shelf',
                                {'facing': facing}, pos, (f * 4, 15 - p), items=[list(i) for i in items], align=align))
        for block in ('minecraft:suspicious_sand', 'minecraft:suspicious_gravel'):
            for dusted in (1, 2, 3):
                for hit in range(6):
                    item = ['minecraft:emerald', 'minecraft:brick', 'minecraft:stone', 'minecraft:shield'][(hit + dusted) % 4]
                    out.append(case(f'brushable {block} dusted={dusted} hit={hit} at {pos}', 'brushable', block,
                                    {'dusted': str(dusted)}, pos, (hit * 2, dusted * 4), item=[item, 1], hit=hit))
        for f, facing in enumerate(FACINGS):
            for ominous in ('false', 'true'):
                out.append(case(f'vault {facing} ominous={ominous} empty at {pos}', 'vault', 'minecraft:vault',
                                {'facing': facing, 'ominous': ominous, 'vault_state': 'inactive'}, pos, (15, 15),
                                spin=[0.0, 10.0, 0.5]))
        for i, (item, count) in enumerate([('minecraft:diamond', 1), ('minecraft:trial_key', 2), ('minecraft:stone', 17),
                                           ('minecraft:emerald', 33), ('minecraft:oak_log', 64), ('minecraft:shield', 1)]):
            for spin in ([0.0, 10.0, 0.0], [350.0, 0.0, 0.5], [-170.0, 175.0, 0.25], [100.0, 110.0, 0.999]):
                out.append(case(f'vault {item}x{count} spin={spin} at {pos}', 'vault', 'minecraft:vault',
                                {'facing': FACINGS[i % 4], 'ominous': 'false', 'vault_state': 'active'}, pos, (13, 15),
                                item=[item, count], spin=spin))

    # Exercise item submission on both sides of chunk boundaries and at the strict renderer reach.
    pos = (15, 64, -1)
    fixtures = [
        case('campfire', 'campfire', 'minecraft:campfire', {'facing': 'north', 'lit': 'false'},
             pos, (15, 0), items=[[0, 'minecraft:beef', 1]]),
        case('shelf', 'shelf', 'minecraft:oak_shelf', {'facing': 'north'},
             pos, (15, 0), items=[[0, 'minecraft:stone', 1]], align=False),
        case('brushable', 'brushable', 'minecraft:suspicious_sand', {'dusted': '1'},
             pos, (15, 0), item=['minecraft:emerald', 1], hit=1),
        case('vault', 'vault', 'minecraft:vault',
             {'facing': 'north', 'ominous': 'false', 'vault_state': 'active'},
             pos, (15, 0), item=['minecraft:diamond', 1], spin=[0.0, 10.0, 0.5]),
    ]
    offsets = [((63.75, 0, 0), True), ((0, 0, 63.75), True), ((-63.75, 0, 0), True),
               ((64, 0, 0), False), ((64.25, 0, 0), False), ((0, 64.25, 0), False),
               ((48, 40, 32), False)]
    for fixture in fixtures:
        for offset, visible in offsets:
            out.append({**fixture, 'name': f'{fixture["name"]} reach {offset}',
                        'camera': [p + 0.5 + d for p, d in zip(pos, offset)],
                        'expect_items': visible})
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--filter', default='')
    parser.add_argument('--resources', type=Path, default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'block-entity-item-geometry-parity'
    build.mkdir(parents=True, exist_ok=True)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    resources = build / 'resources'
    item_model.extract(jars['client'], resources)
    defaults = lab.LOCAL / 'sunrise-client' / lock['minecraft']['version'] / 'items.json'
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))
    chosen = [c for c in cases() if args.filter in c['name']]
    inputs = build / 'inputs.jsonl'
    inputs.write_text(''.join(json.dumps(c) + '\n' for c in chosen))
    report = build / 'report.txt'

    def run(command, name, cwd=lab.ROOT, env=None):
        path = build / (name + '.log')
        with path.open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=cwd, env=env, timeout=3600, stdout=log,
                                    stderr=subprocess.STDOUT)
        if result.returncode:
            sys.exit(f'{name} failed: see {path}')
        return path

    oracles = [PARITY / n for n in ('ParityData.java', 'ParityFont.java', 'VanillaTextDisplayRenderOracle.java',
                                    'VanillaBlockEntityRenderOracle.java', 'VanillaItemModelOracle.java',
                                    'VanillaBlockEntityItemOracle.java')]
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn', '-cp', dependencies, '-d', classes,
         *oracles], 'compile-java')
    run(['java', '-Xmx2G', '-Djava.awt.headless=true', '-cp', str(classes) + os.pathsep + dependencies,
         'dev.sunriselab.parity.VanillaBlockEntityItemOracle', resources, defaults, inputs, build / 'vanilla.jsonl'], 'vanilla')
    with tempfile.TemporaryDirectory(prefix='sunrise-block-entity-items-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        binaries = [lab.parity_fixtures()]
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env['SUNRISE_BLOCK_ENTITY_ITEM_CASES'] = str(inputs)
        env['SUNRISE_BLOCK_ENTITY_ITEM_OUT'] = str(build / 'rust.jsonl')
        log = run([binaries[0], TEST, '--exact', '--ignored', '--nocapture'], 'game', cwd=scratch, env=env)
        assert '1 passed; 0 failed' in log.read_text(encoding='utf-8'), 'the game test did not run'
    vanilla = [json.loads(l) for l in (build / 'vanilla.jsonl').read_text().splitlines()]
    rust = [json.loads(l) for l in (build / 'rust.jsonl').read_text().splitlines()]
    assert len(vanilla) == len(rust) == len(chosen), (len(vanilla), len(rust), len(chosen))
    lines, failed = [], 0
    for c, v, r in zip(chosen, vanilla, rust):
        if 'error' in v:
            lines.append(f'ORACLE ERROR {c["name"]}: {v["error"]}')
        elif bool(v['submits']) != c.get('expect_items', bool(c.get('item') or c.get('items'))):
            lines.append(f'UNEXPECTED VANILLA VISIBILITY {c["name"]}')
        elif v['submits'] != r['submits']:
            lines.append(f'MISMATCH {c["name"]}\n  vanilla {json.dumps(v["submits"])}\n  sunrise {json.dumps(r["submits"])}')
        else:
            continue
        failed += 1
    report.write_text(''.join(l + '\n' for l in lines))
    print(f'block entity item geometry: {len(chosen) - failed} passed, {failed} failed')
    if failed:
        sys.exit(1)


if __name__ == '__main__':
    main()
