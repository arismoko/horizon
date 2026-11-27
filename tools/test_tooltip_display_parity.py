#!/usr/bin/env python3
"""Compare item tooltip display visibility, styled names, and tooltip_style sprites with vanilla 26.3.

Also records the 26.3 tooltip_style sprite witness and meta.json into
.local/witness/<version>/tooltip_display/ and runs the sunrise-client
render_tooltip_texture witness fixture against it through
SUNRISE_TOOLTIP_STYLE_SPRITES_WITNESS."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness as harness, test_model_baking_parity as baking, test_rust_parity as base

STYLE_TEST = (
    'game::native_hooks::render_tooltip_texture::render_tooltip_texture_tests::'
    'an_items_tooltip_style_draws_its_own_sprites'
)

STYLE_CASES = [
    ('custom_namespace', 'minecraft:stone', 'lobby:gold', (40, 30, 60, 8)),
    ('minecraft_namespace', 'minecraft:stick', 'minecraft:fancy', (40, 30, 60, 8)),
    ('nested_path', 'minecraft:stone', 'lobby:themes/royal', (20, 20, 50, 8)),
    ('bare_default_namespace', 'minecraft:stick', 'fancy', (12, 16, 48, 18)),
]


def java_version_string():
    result = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def cases(build):
    values = [
        (False, False, False),
        (False, False, True),
        (False, True, False),
        (False, True, True),
        (True, False, False),
        (True, False, True),
        (True, True, False),
        (True, True, True),
    ]
    path = build / 'cases.txt'
    # Lowercase: Rust's bool parser takes only true/false (Java's parseBoolean ignores case).
    names = [
        'Plain name',
        {'text': '', 'extra': [
            {'text': 'Red', 'color': '#ff0000', 'italic': False},
            {'text': 'Blue', 'color': '#0000ff'}]},
        {'text': 'Gold', 'color': 'gold', 'bold': True, 'italic': False},
        [{'text': 'List ', 'bold': True}, {'text': 'tail', 'bold': False}],
    ]
    styles = [
        f'style\t{x}\t{y}\t{w}\t{h}\t{{id:"{item}",count:1,components:{{"minecraft:tooltip_style":"{style}"}}}}\n'
        for _, item, style, (x, y, w, h) in STYLE_CASES
    ]
    path.write_text('\n'.join('\t'.join(str(v).lower() for v in row) for row in values) + '\n'
                    + ''.join('name\t' + json.dumps(name, ensure_ascii=False) + '\n' for name in names)
                    + ''.join(styles), encoding='utf-8')
    return path, len(values) + len(names) + len(STYLE_CASES)


def write_style_witness(lock, jars, oracle_path, input_path, client_out, output_path=None):
    """Write tooltip_style sprite witness JSON and meta.json from VanillaTooltipDisplayOracle output."""
    version = lock['minecraft']['version']
    style_lines = [line.removeprefix('style\t')
                   for line in client_out.read_text(encoding='utf-8').splitlines()
                   if line.startswith('style\t')]
    if len(style_lines) != len(STYLE_CASES):
        raise AssertionError(f'Expected {len(STYLE_CASES)} style lines, got {len(style_lines)}')
    recorded = []
    for (label, item, style_input, box), raw in zip(STYLE_CASES, style_lines):
        draws = []
        for entry in raw.split('|'):
            sprite, x, y, w, h, color = entry.split(',')
            draws.append({
                'sprite': sprite,
                'x': int(x),
                'y': int(y),
                'w': int(w),
                'h': int(h),
                'color': int(color),
            })
        recorded.append({
            'label': label,
            'item': item,
            'tooltip_style': style_input,
            'box': list(box),
            'draws': draws,
        })
    dest = Path(output_path) if output_path is not None else (
        lab.LOCAL / 'witness' / version / 'tooltip_display' / 'tooltip_style_sprites.json'
    )
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(json.dumps({
        'minecraft_version': version,
        'oracle': 'VanillaTooltipDisplayOracle',
        'cases': recorded,
    }, indent=2) + '\n', encoding='utf-8')
    oracle_rel = oracle_path.relative_to(lab.ROOT).as_posix()
    lab.write_json(dest.parent / 'meta.json', {
        'game_version': version,
        'java_major': lock['minecraft']['java_major'],
        'java_version': java_version_string(),
        'harness': 'tools/test_tooltip_display_parity.py',
        'command': 'python3 tools/test_tooltip_display_parity.py',
        'oracle': oracle_rel,
        'witness_env': 'SUNRISE_TOOLTIP_STYLE_SPRITES_WITNESS',
        'input_hashes': {
            'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
            'client.jar': lab.digest(jars['client'], 'sha256'),
            oracle_rel: lab.digest(oracle_path, 'sha256'),
            'cases.txt': lab.digest(input_path, 'sha256'),
        },
        'outputs': {
            dest.name: {'sha256': lab.digest(dest, 'sha256'), 'bytes': dest.stat().st_size},
        },
    })
    return dest


def record_style_witness(output_path=None):
    """Run VanillaTooltipDisplayOracle on pinned 26.3 and record tooltip_style sprite draws."""
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'tooltip-display-parity'
    build.mkdir(parents=True, exist_ok=True)
    input_path, _ = cases(build)
    classes, dependencies, oracle_path = harness.compile_oracle(
        build, 'VanillaTooltipDisplayOracle', jars, libraries, lock['minecraft']['java_major'])
    client_out = build / 'client.txt'
    harness.java_run(build, classes, dependencies, 'VanillaTooltipDisplayOracle',
                     [input_path, client_out], 'client')
    dest = write_style_witness(lock, jars, oracle_path, input_path, client_out, output_path)
    print(f'Recorded {len(STYLE_CASES)} tooltip_style sprite cases for {lock["minecraft"]["version"]}: {dest}')
    return dest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record-only', action='store_true',
                        help='Record tooltip_style sprite witness into .local/witness/<version>/tooltip_display/ using Java only')
    parser.add_argument('--record', nargs='?', const='default', default=None,
                        help='Record tooltip_style sprite witness JSON using Java only (defaults to .local/witness/<version>/tooltip_display/tooltip_style_sprites.json)')
    parser.add_argument('--resources', type=Path,
                        default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    args = parser.parse_args()
    if args.record_only or args.record is not None:
        dest = None if args.record in (None, 'default') else Path(args.record)
        record_style_witness(dest)
        return
    harness.differential(
        'tooltip-display-parity', 'VanillaTooltipDisplayOracle', 'tooltip_display_fixture', cases,
        'Tooltip visibility, styled custom-name inheritance, and 26.3 tooltip_style sprite selection',
        sources=[
            lab.ROOT / 'crates/sunrise-client-gameplay/src/game/blocks/stations.rs',
            lab.ROOT / 'crates/sunrise-client/src/item_tooltip.rs',
            lab.ROOT / 'crates/sunrise-client-render/src/ui.rs',
            lab.ROOT / 'crates/sunrise-items/src/item_data.rs',
        ],
        expected_lines=12 + len(STYLE_CASES))
    lock, jars, _ = base.verify_artifacts()
    build = lab.LOCAL / 'tooltip-display-parity'
    oracle_path = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaTooltipDisplayOracle.java'
    witness = write_style_witness(lock, jars, oracle_path, build / 'cases.txt', build / 'client.txt')
    with tempfile.TemporaryDirectory(prefix='sunrise-tooltip-style-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env['SUNRISE_TOOLTIP_STYLE_SPRITES_WITNESS'] = str(witness.resolve())
        binary = lab.parity_fixtures()
        log = build / 'style-witness-test.log'
        with log.open('w', encoding='utf-8') as stream:
            result = subprocess.run(
                [str(binary), STYLE_TEST, '--exact', '--ignored', '--nocapture'],
                cwd=scratch, env=env, timeout=3600, stdout=stream, stderr=subprocess.STDOUT,
            )
        if result.returncode != 0 or '1 passed; 0 failed' not in log.read_text(encoding='utf-8'):
            tail = log.read_text(encoding='utf-8', errors='replace').splitlines()[-40:]
            print('\n'.join(tail), file=sys.stderr, flush=True)
            raise SystemExit(f'{STYLE_TEST} failed against {witness}; see {log}')
    print(f'Tooltip style sprite witness test passed against {witness}', flush=True)


if __name__ == '__main__':
    main()
