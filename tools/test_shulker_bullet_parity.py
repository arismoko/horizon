#!/usr/bin/env python3
"""Compare shulker bullet origin, stand-on checks, and 26.3 tick rotation against vanilla."""
import argparse
import json
import os
from pathlib import Path
import struct
import subprocess

import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness as harness, test_model_baking_parity as baking, test_rust_parity as base


WITNESS = lab.LOCAL / 'witness' / lab.VERSION / 'shulker_bullet_turn' / 'shulker_bullet_turn.tsv'
TURN_WITNESS_TEST = 'projectiles::tests::a_shulker_bullet_turns_halfway_towards_its_motion_26_3'


def f32(value):
    return struct.unpack('<f', struct.pack('<f', value))[0]


def f32_hex(value):
    return f'{struct.unpack("<I", struct.pack("<f", value))[0]:08x}'


def f64_hex(value):
    return f'{struct.unpack("<Q", struct.pack("<d", value))[0]:016x}'


def peeks():
    """Shulker.updatePeekAmount opening the lid: 0, then +0.05F a tick up to 1 (float sums)."""
    peek, out = 0.0, [0.0]
    for _ in range(20):
        peek = min(f32(peek + f32(0.05)), 1.0)
        out.append(peek)
    return out


# Full cubes, partial tops, hollow tops, context shapes (scaffolding, powder snow, liquids) and empty shapes.
BLOCKS = ['air', 'stone', 'glass', 'oak_leaves', 'barrier', 'mangrove_roots', 'soul_sand', 'mud',
          'honey_block', 'farmland', 'dirt_path', 'cactus', 'chest', 'end_portal_frame', 'hopper',
          'cauldron', 'composter', 'lectern', 'daylight_detector', 'oak_fence', 'cobblestone_wall',
          'iron_bars', 'bamboo', 'big_dripleaf', 'powder_snow', 'water', 'lava', 'light',
          'oak_slab[type=bottom]', 'oak_slab[type=top]', 'oak_slab[type=double]',
          'oak_stairs[half=bottom]', 'oak_stairs[half=top,facing=north]',
          'oak_trapdoor[half=bottom]', 'oak_trapdoor[half=top]', 'oak_trapdoor[half=top,open=true]',
          'snow[layers=1]', 'snow[layers=8]', 'scaffolding[distance=0,bottom=false]',
          'scaffolding[distance=7,bottom=true]']


def turn_cases():
    """Deterministic ShulkerBullet.tick rotation scenarios covering free flight, gravity, wrap and breeze deflection."""
    specs = [
        ('south_no_gravity', True, [0.0, 0.0, 0.5], 0.0, 0.0, None, None, 4),
        ('still_no_gravity', True, [0.0, 0.0, 0.0], -170.5, 35.25, None, None, 3),
        ('falling_from_rest', False, [0.0, 0.0, 0.0], 45.0, 0.0, None, None, 4),
        ('diagonal_gravity', False, [0.3, 0.2, -0.4], 10.0, -20.0, None, None, 4),
        ('east_from_zero', True, [0.5, 0.0, 0.0], 0.0, 0.0, None, None, 3),
        ('west_wrap_pos', True, [-0.5, 0.0, 0.0], 175.0, -45.0, None, None, 3),
        ('north_wrap_neg', True, [0.0, 0.0, -0.5], -175.0, 45.0, None, None, 3),
        ('octant_pos_neg_pos', True, [0.3, -0.2, 0.4], -170.0, 15.0, None, None, 3),
        ('octant_neg_pos_neg', True, [-0.25, 0.35, -0.15], 170.0, -30.0, None, None, 3),
        ('multi_turn_yaw', True, [0.15, 0.0, 0.0], -270.0, 0.0, None, None, 3),
        ('straight_up', True, [0.0, 0.5, 0.0], 30.0, -45.0, None, None, 3),
        ('straight_down', True, [0.0, -0.5, 0.0], -120.0, 45.0, None, None, 3),
        ('breeze_deflect_south', True, [0.0, 0.0, 0.5], 0.0, 0.0, 1, [0.5, 109.5, 1.0], 3),
        ('breeze_deflect_diagonal_gravity', False, [0.4, -0.1, 0.4], 20.0, 10.0, 42, [0.9, 109.4, 0.9], 3),
    ]
    rows = []
    for name, no_gravity, velocity, yaw, pitch, deflect_seed, breeze, ticks in specs:
        row = dict(
            mode='turn', seed=1, cells=[], name=name, no_gravity=no_gravity,
            velocity=[f64_hex(v) for v in velocity], yaw=f32_hex(yaw), pitch=f32_hex(pitch),
            deflect_seed=deflect_seed, ticks=ticks,
        )
        if breeze is not None:
            row['breeze'] = [f64_hex(v) for v in breeze]
        rows.append(row)
    return rows


def make_input(build):
    rows = []
    for position in ([0.5, 64.0, 0.5], [-7.5, -3.0, 12.5]):
        target = [position[0] + 6, position[1] + 2, position[2] - 4]
        for face in range(6):
            for peek in peeks():
                for scale in (1.0, 0.5, 2.5, 4.0):
                    rows.append(dict(mode='origin', seed=1, cells=[], position=position, target=target, face=face,
                                     peek=struct.unpack('<I', struct.pack('<f', peek))[0], scale=scale))
    for pos in ([0, 64, 0], [-17, -20, 31]):
        for block in BLOCKS:
            for above in (-0.5, 0.5, 1.0, 1.5):
                rows.append(dict(mode='stand', seed=1, cells=[], pos=pos, block='minecraft:' + block,
                                 y=pos[1] + above))
    rows.extend(turn_cases())
    path = build / 'inputs.jsonl'
    path.write_text(''.join(json.dumps(row) + '\n' for row in rows))
    return path, len(rows)


def java_version_string():
    result = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def format_turn_witness(version, rows, results):
    if len(results) != len(rows):
        raise AssertionError(f'Expected {len(rows)} oracle lines, got {len(results)}')
    lines = [
        f'# Vanilla {version} ShulkerBullet.tick rotation witness (recorded by tools/test_shulker_bullet_parity.py)',
        '# Columns: name\tno_gravity\tvx\tvy\tvz\tyaw0\tpitch0\tdeflect_seed\tbreeze\tticks\tsteps',
        '# steps: comma-separated per-tick yaw_bits:pitch_bits:prev_yaw_bits:prev_pitch_bits (raw IEEE-754 f32 hex)',
    ]
    for row, steps in zip(rows, results):
        seed = '-' if row['deflect_seed'] is None else str(row['deflect_seed'])
        breeze = '-' if 'breeze' not in row else ','.join(row['breeze'])
        lines.append('\t'.join([
            row['name'], '1' if row['no_gravity'] else '0',
            *row['velocity'], row['yaw'], row['pitch'],
            seed, breeze, str(row['ticks']), steps,
        ]))
    return '\n'.join(lines) + '\n'


def write_turn_witness(output_path, lock, jars, input_path, rows, results, command):
    version = lock['minecraft']['version']
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(format_turn_witness(version, rows, results), encoding='utf-8')
    oracle_sources = [
        lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaShulkerBulletOracle.java',
        lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaBlockUpdateOracle.java',
        lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/ParityData.java',
    ]
    input_hashes = {
        'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
        'client.jar': lab.digest(jars['client'], 'sha256'),
        input_path.name: lab.digest(input_path, 'sha256'),
        **{src.relative_to(lab.ROOT).as_posix(): lab.digest(src, 'sha256') for src in oracle_sources},
    }
    lab.write_json(output_path.parent / 'meta.json', {
        'game_version': version,
        'java_major': lock['minecraft']['java_major'],
        'java_version': java_version_string(),
        'harness': 'tools/test_shulker_bullet_parity.py',
        'witness_env': 'SUNRISE_SHULKER_BULLET_TURN_WITNESS',
        'command': command,
        'input_hashes': input_hashes,
        'outputs': {
            output_path.name: {
                'sha256': lab.digest(output_path, 'sha256'),
                'bytes': output_path.stat().st_size,
            },
        },
    })


def record_witness(output_path):
    """Run only the pinned vanilla 26.3 Java oracle on turn_cases() and write the local witness TSV and meta.json."""
    lock, jars, _ = base.verify_artifacts()
    version = lock['minecraft']['version']
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'rust-shulker-bullet-parity'
    build.mkdir(parents=True, exist_ok=True)
    rows = turn_cases()
    input_path = build / 'turn_inputs.jsonl'
    input_path.write_text(''.join(json.dumps(row) + '\n' for row in rows), encoding='utf-8')
    classes, dependencies, _ = harness.compile_oracle(
        build, 'VanillaShulkerBulletOracle', jars, libraries, lock['minecraft']['java_major'])
    raw_out = build / 'turn_client.txt'
    harness.java_run(build, classes, dependencies, 'VanillaShulkerBulletOracle', [input_path, raw_out], 'client-record')
    results = raw_out.read_text(encoding='utf-8').splitlines()
    write_turn_witness(
        output_path, lock, jars, input_path, rows, results,
        'python3 tools/test_shulker_bullet_parity.py --record',
    )
    print(f'Recorded {len(rows)} vanilla {version} shulker bullet turn cases to {output_path}', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true', help='record the 26.3 turn witness from the Java oracle only')
    parser.add_argument('--witness', type=Path, default=WITNESS, help='local witness TSV output path')
    args = parser.parse_args()
    if args.record:
        record_witness(args.witness)
        return
    harness.differential(
        'rust-shulker-bullet-parity', 'VanillaShulkerBulletOracle', 'shulker_bullet_probe', make_input,
        'The real 26.3 ShulkerBullet(level, shulker, target, axis) constructor, Level.loadedAndEntityCanStandOn, '
        'and ShulkerBullet.tick rotation via ProjectileUtil.rotateTowardsMovement(this, 0.5F) including Breeze deflection.',
        sources=[lab.ROOT / 'crates/sunrise-sim/src/mobs/shulker.rs',
                 lab.ROOT / 'crates/sunrise-sim/src/projectiles.rs',
                 lab.ROOT / 'crates/sunrise-level/src/collisions.rs',
                 lab.ROOT / 'crates/sunrise-world/src/block_collision.rs',
                 lab.ROOT / 'crates/sunrise-world/src/collision_context.rs',
                 lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaBlockUpdateOracle.java',
                 lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/ParityData.java'])
    lock, jars, _ = base.verify_artifacts()
    build = lab.LOCAL / 'rust-shulker-bullet-parity'
    input_path = build / 'inputs.jsonl'
    all_inputs = [json.loads(line) for line in input_path.read_text(encoding='utf-8').splitlines()]
    all_results = (build / 'client.txt').read_text(encoding='utf-8').splitlines()
    if len(all_inputs) != len(all_results):
        raise AssertionError(f'Expected {len(all_inputs)} oracle lines, got {len(all_results)}')
    turn_pairs = [(row, res) for row, res in zip(all_inputs, all_results) if row.get('mode') == 'turn']
    turn_rows = [row for row, _ in turn_pairs]
    turn_results = [res for _, res in turn_pairs]
    if turn_rows != turn_cases():
        raise AssertionError('Turn cases in inputs.jsonl diverged from turn_cases()')
    # The recorded turn witness stays in .local; the Rust test reads it from SUNRISE_SHULKER_BULLET_TURN_WITNESS.
    write_turn_witness(
        args.witness, lock, jars, input_path, turn_rows, turn_results,
        'python3 tools/test_shulker_bullet_parity.py',
    )
    log_path = build / 'turn-witness-test.log'
    with log_path.open('w', encoding='utf-8') as log:
        result = subprocess.run(
            ['cargo', 'test', '--locked', '-p', 'sunrise-client-lib', '--lib',
             '--', '--ignored', '--exact', TURN_WITNESS_TEST],
            cwd=lab.ROOT, timeout=3600, stdout=log, stderr=subprocess.STDOUT,
            env={**os.environ, 'SUNRISE_SHULKER_BULLET_TURN_WITNESS': str(args.witness.resolve())},
        )
    if result.returncode != 0 or '1 passed; 0 failed' not in log_path.read_text(encoding='utf-8'):
        raise SystemExit(f'{TURN_WITNESS_TEST} failed against the recorded witness; see {log_path}')
    print(f'Shulker bullet turn witness test passed ({len(turn_rows)} cases against {args.witness})', flush=True)


if __name__ == '__main__':
    main()
