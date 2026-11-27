#!/usr/bin/env python3
"""Compare Fire ticking on eyes, fangs, ominous spawners, clouds and shulker bullets against vanilla 26.3."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, test_rust_parity as base
from tools.command_exec_corpus import cmd, setup


TEST = 'game::server_side::commands::data::fire_tests::fire_changes_on_eyes_fangs_spawners_clouds_and_bullets'
ORACLE = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCommandExecOracle.java'

# (entity_type, extra_nbt) for each of the five #2683 entity types.
ENTITIES = (
    ('minecraft:eye_of_ender', ''),
    ('minecraft:evoker_fangs', 'Warmup:100'),
    ('minecraft:ominous_item_spawner', 'spawn_item_after_ticks:100L,item:{id:"minecraft:stone",count:1}'),
    ('minecraft:area_effect_cloud', 'Duration:200'),
    ('minecraft:shulker_bullet', 'NoGravity:1b'),
)

INITIAL_FIRES = (40, 39, 21, 20, 1, 0, -5)
NUMERIC_FIRE_VARIANTS = (
    ('neg_half_double', 'Fire:-0.5d'),
    ('overflow_pos_double', 'Fire:40000.5d'),
    ('overflow_neg_double', 'Fire:-40000.5d'),
    ('non_numeric_string', 'Fire:"not_a_number"'),
    ('missing_fire', ''),
)
TICK_STEPS = (0, 1, 2)
DATA_SHORT_RE = re.compile(r'has the following entity data: (-?\d+)s$')


def cases():
    out = []
    index = 1
    for kind, extra in ENTITIES:
        for fire in INITIAL_FIRES:
            suffix = f',{extra}' if extra else ''
            nbt = f'{{Fire:{fire}s{suffix}}}'
            uuid_int = f'[I;2683,0,0,{index}]'
            uuid_str = f'00000a7b-0000-0000-0000-{index:012x}'
            out.append((kind, str(fire), extra, nbt, uuid_int, uuid_str, TICK_STEPS))
            index += 1
        for label, fire_field in NUMERIC_FIRE_VARIANTS:
            parts = [p for p in (fire_field, extra) if p]
            nbt = '{' + ','.join(parts) + '}'
            uuid_int = f'[I;2683,0,0,{index}]'
            uuid_str = f'00000a7b-0000-0000-0000-{index:012x}'
            out.append((kind, label, extra, nbt, uuid_int, uuid_str, (0, 1)))
            index += 1
    return out


def groups():
    ops = []
    for kind, _fire, _extra, nbt, uuid_int, uuid_str, steps in cases():
        inner = nbt[1:-1]
        summon_nbt = f'{{UUID:{uuid_int},{inner}}}' if inner else f'{{UUID:{uuid_int}}}'
        ops.append(cmd(f'/summon {kind} 8.5 220 8.5 {summon_nbt}', False))
        prev = 0
        for tick in steps:
            if tick > prev:
                ops.append({'op': 'tick', 'n': tick - prev})
                prev = tick
            ops.append(cmd(f'/execute if entity {uuid_str}'))
            ops.append(cmd(f'/data get entity {uuid_str} Fire'))
        ops.extend(setup(f'/kill {uuid_str}'))
    # Special shulker bullet scenes: swept fire contact with dry endpoint, fresh vs already-ticked lava, /data merge Fire.
    ops.extend(setup(
        '/fill 9 219 7 15 219 9 minecraft:stone',
        '/setblock 10 219 8 minecraft:netherrack',
        '/setblock 10 220 8 minecraft:fire',
        '/fill 13 220 7 15 220 9 minecraft:stone',
        '/setblock 14 220 8 minecraft:lava',
    ))
    # 1. Swept fire contact with dry endpoint (x=8.5 -> x=12.0 through fire at x=10).
    u_sweep = '00000a7b-0000-0001-0000-000000000001'
    ops.append(cmd(f'/summon minecraft:shulker_bullet 8.5 220.2 8.5 {{UUID:[I;2683,1,0,1],NoGravity:1b,Fire:0s,Motion:[3.5d,0.0d,0.0d]}}', False))
    ops.append(cmd(f'/execute if entity {u_sweep}'))
    ops.append(cmd(f'/data get entity {u_sweep} Fire'))
    ops.append({'op': 'tick', 'n': 1})
    ops.append(cmd(f'/execute if entity {u_sweep}'))
    ops.append(cmd(f'/data get entity {u_sweep} Fire'))
    ops.extend(setup(f'/kill {u_sweep}'))
    # 2. Fresh bullet in lava with Fire:40s and HasBeenShot:1b.
    u_fresh_lava = '00000a7b-0000-0001-0000-000000000002'
    ops.append(cmd(f'/summon minecraft:shulker_bullet 14.5 220.2 8.5 {{UUID:[I;2683,1,0,2],NoGravity:1b,Fire:40s,HasBeenShot:1b}}', False))
    ops.append(cmd(f'/execute if entity {u_fresh_lava}'))
    ops.append(cmd(f'/data get entity {u_fresh_lava} Fire'))
    ops.append({'op': 'tick', 'n': 1})
    ops.append(cmd(f'/execute if entity {u_fresh_lava}'))
    ops.append(cmd(f'/data get entity {u_fresh_lava} Fire'))
    ops.extend(setup(f'/kill {u_fresh_lava}'))
    # 3. Already-ticked bullet merged to Fire:40s in air -> destroyed on next tick.
    u_merge_air = '00000a7b-0000-0001-0000-000000000003'
    ops.append(cmd(f'/summon minecraft:shulker_bullet 8.5 220.2 8.5 {{UUID:[I;2683,1,0,3],NoGravity:1b,Fire:0s}}', False))
    ops.append({'op': 'tick', 'n': 1})
    ops.append(cmd(f'/data merge entity {u_merge_air} {{Fire:40s}}', False))
    ops.append(cmd(f'/execute if entity {u_merge_air}'))
    ops.append(cmd(f'/data get entity {u_merge_air} Fire'))
    ops.append({'op': 'tick', 'n': 1})
    ops.append(cmd(f'/execute if entity {u_merge_air}'))
    ops.append(cmd(f'/data get entity {u_merge_air} Fire'))
    ops.extend(setup(f'/kill {u_merge_air}'))
    # 4. Already-ticked bullet teleported into lava and merged to Fire:40s -> destroyed by lavaHurt on next tick.
    u_ticked_lava = '00000a7b-0000-0001-0000-000000000004'
    ops.append(cmd(f'/summon minecraft:shulker_bullet 8.5 220.2 8.5 {{UUID:[I;2683,1,0,4],NoGravity:1b,Fire:0s}}', False))
    ops.append({'op': 'tick', 'n': 1})
    ops.append(cmd(f'/tp {u_ticked_lava} 14.5 220.2 8.5', False))
    ops.append(cmd(f'/data merge entity {u_ticked_lava} {{Fire:40s}}', False))
    ops.append(cmd(f'/execute if entity {u_ticked_lava}'))
    ops.append(cmd(f'/data get entity {u_ticked_lava} Fire'))
    ops.append({'op': 'tick', 'n': 1})
    ops.append(cmd(f'/execute if entity {u_ticked_lava}'))
    ops.append(cmd(f'/data get entity {u_ticked_lava} Fire'))
    ops.extend(setup(
        f'/kill {u_ticked_lava}',
        '/fill 9 219 7 15 220 9 minecraft:air',
    ))
    return [('entity fire ticks 26.3', ops)]


SPECIAL_CASES = (
    ('bullet_swept_fire_dry_end', '00000a7b-0000-0001-0000-000000000001', '{NoGravity:1b,Fire:0s,Motion:[3.5d,0.0d,0.0d]}', (0, 1)),
    ('bullet_fresh_in_lava_40', '00000a7b-0000-0001-0000-000000000002', '{NoGravity:1b,Fire:40s,HasBeenShot:1b}', (0, 1)),
    ('bullet_merge_fire_40_air', '00000a7b-0000-0001-0000-000000000003', '{NoGravity:1b,Fire:0s}+merge40', (0, 1)),
    ('bullet_ticked_in_lava_40', '00000a7b-0000-0001-0000-000000000004', '{NoGravity:1b,Fire:0s}+tp_lava+merge40', (0, 1)),
)


def parse_vanilla_rows(vanilla_jsonl: Path):
    records = [json.loads(line) for line in vanilla_jsonl.read_text(encoding='utf-8').splitlines() if line.strip()]
    case_list = cases()
    expected_records = sum(len(steps) * 2 for *_, steps in case_list) + sum(len(steps) * 2 for *_, steps in SPECIAL_CASES)
    if len(records) != expected_records:
        raise RuntimeError(f'expected {expected_records} records in {vanilla_jsonl}, got {len(records)}')
    rows = []
    cursor = 0

    def consume(kind, initial_fire, nbt, uuid_str, steps):
        nonlocal cursor
        for tick in steps:
            alive_rec = records[cursor]
            fire_rec = records[cursor + 1]
            cursor += 2
            if alive_rec['cmd'] != f'/execute if entity {uuid_str}':
                raise RuntimeError(f'unexpected alive cmd: {alive_rec["cmd"]}')
            if fire_rec['cmd'] != f'/data get entity {uuid_str} Fire':
                raise RuntimeError(f'unexpected fire cmd: {fire_rec["cmd"]}')
            alive_ok, alive_count = alive_rec['results'][0]
            alive = bool(alive_ok and alive_count == 1)
            if alive:
                fire_ok, fire_val = fire_rec['results'][0]
                if not fire_ok:
                    raise RuntimeError(f'alive entity {kind} Fire={initial_fire} tick={tick} failed /data get: {fire_rec}')
                msg = ''.join(span[0] for span in fire_rec['messages'][0])
                match = DATA_SHORT_RE.search(msg)
                if not match:
                    raise RuntimeError(f'could not parse Fire short from {msg!r}')
                parsed = int(match.group(1))
                if parsed != fire_val:
                    raise RuntimeError(f'Fire value mismatch: text={parsed} callback={fire_val}')
                fire_out = str(parsed)
            else:
                fire_out = 'gone'
            rows.append((kind, str(initial_fire), nbt, str(tick), 'alive' if alive else 'gone', fire_out))

    for kind, initial_fire, _extra, nbt, _uuid_int, uuid_str, steps in case_list:
        consume(kind, initial_fire, nbt, uuid_str, steps)
    for label, uuid_str, nbt, steps in SPECIAL_CASES:
        consume('minecraft:shulker_bullet', label, nbt, uuid_str, steps)
    return rows


def java_version_string():
    result = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def format_witness(rows, version='26.3'):
    lines = [
        f'# vanilla {version} witness recorded via tools/test_entity_fire_ticks_parity.py',
        '# oracle: VanillaCommandExecOracle (tools/test_command_exec_parity.py --reference-only)',
        'kind\tinitial_fire\tnbt\tticks\tstate\tfire',
    ]
    for row in rows:
        lines.append('\t'.join(row))
    return '\n'.join(lines) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument('--reference-only', action='store_true')
    parser.add_argument('--record', '--record-only', dest='record', action='store_true',
                        help='run vanilla 26.3 oracle only and write the witness TSV into .local')
    parser.add_argument('--resources', type=Path,
                        default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    parser.add_argument('--work-dir', type=Path)
    args = parser.parse_args()
    root = lab.ROOT
    work = (args.work_dir or lab.LOCAL / 'entity-fire-ticks-parity').resolve()
    work.mkdir(parents=True, exist_ok=True)
    cases_path = work / 'groups.json'
    cases_path.write_text(json.dumps(groups()), encoding='utf-8')
    run = [sys.executable, str(root / 'tools/test_command_exec_parity.py'),
           '--groups-file', str(cases_path), '--work-dir', str(work)]
    if args.binary:
        run.extend(['--binary', str(args.binary.resolve())])
    if args.prepare:
        run.append('--prepare')
    if args.reference_only or args.record:
        run.append('--reference-only')
    rc = subprocess.run(run, cwd=root).returncode
    if rc:
        return rc
    lock, jars, _ = base.verify_artifacts()
    version = lock['minecraft']['version']
    witness_dir = lab.LOCAL / 'witness' / version / 'entity_fire_ticks'
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness_path = witness_dir / 'entity_fire_ticks.tsv'
    rendered = format_witness(parse_vanilla_rows(work / 'vanilla.jsonl'), version)
    witness_path.write_text(rendered, encoding='utf-8')
    harness_path = Path(__file__).resolve()
    lab.write_json(witness_dir / 'meta.json', {
        'game_version': version,
        'java_major': lock['minecraft']['java_major'],
        'java_version': java_version_string(),
        'harness': 'tools/test_entity_fire_ticks_parity.py',
        'oracle': 'dev.sunriselab.parity.VanillaCommandExecOracle',
        'witness_env': 'SUNRISE_ENTITY_FIRE_TICKS_WITNESS',
        'command': run,
        'input_hashes': {
            'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
            'server.jar': lab.digest(jars['server'], 'sha256'),
            ORACLE.relative_to(root).as_posix(): lab.digest(ORACLE, 'sha256'),
            harness_path.relative_to(root).as_posix(): lab.digest(harness_path, 'sha256'),
        },
        'outputs': {
            witness_path.name: {
                'sha256': lab.digest(witness_path, 'sha256'),
                'bytes': witness_path.stat().st_size,
            },
        },
    })
    if args.record or args.reference_only:
        print(f'Recorded {len(rendered.splitlines()) - 3} vanilla {version} witness rows to {witness_path}')
        return 0
    with tempfile.TemporaryDirectory(prefix='sunrise-entity-fire-ticks-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env['SUNRISE_ENTITY_FIRE_TICKS_WITNESS'] = str(witness_path.resolve())
        binary = lab.parity_fixtures()
        log_path = work / 'rust-fixture.log'
        with log_path.open('w', encoding='utf-8') as log:
            ran = subprocess.run(
                [str(binary), TEST, '--exact', '--ignored', '--nocapture'],
                cwd=scratch, env=env, timeout=3600, stdout=log, stderr=subprocess.STDOUT,
            )
        if ran.returncode or '1 passed; 0 failed' not in log_path.read_text(encoding='utf-8'):
            tail = log_path.read_text(encoding='utf-8', errors='replace')[-3000:]
            sys.exit(f'{TEST} failed against {witness_path}; see {log_path}\n{tail}')
    print(f'Entity fire ticks parity passed ({len(rendered.splitlines()) - 3} witness rows against {witness_path})')
    return 0


if __name__ == '__main__':
    sys.exit(main())
