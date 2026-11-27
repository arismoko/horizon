#!/usr/bin/env python3
"""Compare EvokerFangs fluid-current Motion against vanilla 26.3 (issue #2362)."""
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import sys
import tempfile
import uuid

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import harness_args, test_command_exec_parity as driver

MINECRAFT_VERSION = '26.3'
TEST = 'game::server_side::fangs::current_tests::fangs_in_a_current_are_pushed_and_fangs_in_still_water_are_not'
ORACLE = driver.lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCommandExecOracle.java'
TICKS = (0, 1, 2)
MOTION_RE = re.compile(r'entity data: \[([^,\]]+)d, ([^,\]]+)d, ([^,\]]+)d\]$')

# (name, uuid_tail, dimension, summon_pos, extra_nbt)
CASES = (
    ('lava_north', 1, 'minecraft:overworld', '2.5 200 3.5', ''),
    ('water_north', 2, 'minecraft:overworld', '6.5 200 3.5', ''),
    ('water_diagonal', 3, 'minecraft:overworld', '10.5 200 3.5', ''),
    ('still_water', 4, 'minecraft:overworld', '2.5 200 10.5', ''),
    ('still_water_loaded', 5, 'minecraft:overworld', '6.5 200 10.5', ',Motion:[0.125d,-0.25d,0.5d]'),
    ('dry_air_loaded', 6, 'minecraft:overworld', '10.5 200 10.5', ',Motion:[-0.0625d,0.03125d,0.25d]'),
    ('water_north_loaded', 7, 'minecraft:overworld', '6.5 200 3.5', ',Motion:[0.05d,0.0d,-0.02d]'),
    ('lava_north_loaded', 8, 'minecraft:overworld', '2.5 200 3.5', ',Motion:[0.05d,0.0d,-0.02d]'),
    ('upper_cell_float_height', 9, 'minecraft:overworld', '13.5 200.20099999 3.5', ''),
    ('upper_cell_just_below', 10, 'minecraft:overworld', '13.5 200.20099998 3.5', ''),
    ('nether_fast_lava_north', 11, 'minecraft:the_nether', '2.5 200 3.5', ''),
    ('nether_fast_lava_loaded', 12, 'minecraft:the_nether', '2.5 200 3.5', ',Motion:[0.05d,0.0d,-0.02d]'),
)

# Fluid arena at y=200 inside chunk (0, 0), well clear of terrain and chunk borders.
ARENA_COMMANDS = (
    ('floor', '/fill 0 199 0 15 199 15 minecraft:stone', 256),
    ('clear-fill', '/fill 0 200 0 15 202 15 minecraft:barrier', 768),
    ('clear-air', '/fill 0 200 0 15 202 15 minecraft:air', 768),
    ('lava-source', '/setblock 2 200 4 minecraft:lava', 1),
    ('lava-flow', '/setblock 2 200 3 minecraft:lava[level=2]', 1),
    ('water-source', '/setblock 6 200 4 minecraft:water', 1),
    ('water-flow', '/setblock 6 200 3 minecraft:water[level=1]', 1),
    ('diag-south', '/setblock 10 200 4 minecraft:water', 1),
    ('diag-east', '/setblock 11 200 3 minecraft:water[level=3]', 1),
    ('diag-center', '/setblock 10 200 3 minecraft:water[level=1]', 1),
    ('still-wall', '/fill 1 200 9 3 200 11 minecraft:stone', 9),
    ('still-water', '/setblock 2 200 10 minecraft:water', 1),
    ('loaded-wall', '/fill 5 200 9 7 200 11 minecraft:stone', 9),
    ('loaded-water', '/setblock 6 200 10 minecraft:water', 1),
    # Upper-cell-only flowing water at y=201 (y=200 under it is air), testing widened 0.8F height.
    ('upper-floor-south', '/setblock 13 200 4 minecraft:stone', 1),
    ('upper-floor-north', '/setblock 13 200 2 minecraft:stone', 1),
    ('upper-wall-west', '/setblock 12 201 3 minecraft:stone', 1),
    ('upper-wall-east', '/setblock 14 201 3 minecraft:stone', 1),
    ('upper-water-source', '/setblock 13 201 4 minecraft:water', 1),
    ('upper-water-flow', '/setblock 13 201 3 minecraft:water[level=1]', 1),
    # Fast lava (Nether) channel at y=200 inside Nether chunk (0, 0).
    ('nether-floor', '/execute in minecraft:the_nether run fill 1 199 1 4 199 5 minecraft:stone', 20),
    ('nether-clear-fill', '/execute in minecraft:the_nether run fill 1 200 1 4 202 5 minecraft:barrier', 60),
    ('nether-clear-air', '/execute in minecraft:the_nether run fill 1 200 1 4 202 5 minecraft:air', 60),
    ('nether-lava-source', '/execute in minecraft:the_nether run setblock 2 200 4 minecraft:lava', 1),
    ('nether-lava-flow', '/execute in minecraft:the_nether run setblock 2 200 3 minecraft:lava[level=1]', 1),
)

BLOCK_GUARDS = (
    ('guard-lava-source', '/execute if block 2 200 4 minecraft:lava[level=0]'),
    ('guard-lava-flow', '/execute if block 2 200 3 minecraft:lava[level=2]'),
    ('guard-lava-north-air', '/execute if block 2 200 2 minecraft:air'),
    ('guard-water-source', '/execute if block 6 200 4 minecraft:water[level=0]'),
    ('guard-water-flow', '/execute if block 6 200 3 minecraft:water[level=1]'),
    ('guard-water-north-air', '/execute if block 6 200 2 minecraft:air'),
    ('guard-diag-south', '/execute if block 10 200 4 minecraft:water[level=0]'),
    ('guard-diag-east', '/execute if block 11 200 3 minecraft:water[level=3]'),
    ('guard-diag-center', '/execute if block 10 200 3 minecraft:water[level=1]'),
    ('guard-still-water', '/execute if block 2 200 10 minecraft:water[level=0]'),
    ('guard-loaded-water', '/execute if block 6 200 10 minecraft:water[level=0]'),
    ('guard-dry-air', '/execute if block 10 200 10 minecraft:air'),
    ('guard-upper-air', '/execute if block 13 200 3 minecraft:air'),
    ('guard-upper-water-source', '/execute if block 13 201 4 minecraft:water[level=0]'),
    ('guard-upper-water-flow', '/execute if block 13 201 3 minecraft:water[level=1]'),
    ('guard-nether-lava-source', '/execute in minecraft:the_nether if block 2 200 4 minecraft:lava[level=0]'),
    ('guard-nether-lava-flow', '/execute in minecraft:the_nether if block 2 200 3 minecraft:lava[level=1]'),
)


def fangs_uuid(tail):
    return str(uuid.UUID(int=(2362 << 96) | tail))


def program():
    ops = []

    def cmd(text, label, expected=None, admission=True, record=True):
        op = dict(op='cmd', cmd=text, label=label, admission=admission)
        if expected is not None:
            op['expect_result'] = expected
        if not record:
            op['record'] = False
        ops.append(op)

    cmd('/tick freeze', 'freeze')
    for key in ('spawn_mobs', 'advance_time', 'advance_weather', 'mob_griefing'):
        cmd(f'/gamerule {key} false', f'rule-{key}')
    cmd('/gamerule random_tick_speed 0', 'rule-random-tick-speed')
    cmd('/gamerule show_advancement_messages false', 'rule-advancement-messages', record=False)
    cmd('/gamemode spectator', 'spectator', record=False)
    cmd('/forceload add 0 0 15 15', 'forceload', 1)
    cmd('/execute in minecraft:the_nether run forceload add -16 -16 31 31', 'nether-forceload', 9)
    # `settle` only waits for `player.level()`'s chunks to finish async loading and
    # reach `ENTITY_TICKING` (`areEntitiesActuallyLoadedAndTicking`). Visit the
    # Nether and settle there before returning to the Overworld so chunk (0, 0) in
    # `minecraft:the_nether` is deterministically accessible and entity-ticking
    # before summoning the Nether fangs.
    cmd('/execute in minecraft:the_nether run tp @s 8.5 230 8.5', 'enter-nether', record=False)
    ops.append(dict(op='settle', required_chunks=[[0, 0]]))
    cmd('/execute in minecraft:overworld run tp @s 8.5 230 8.5', 'return-overworld', record=False)
    ops.append(dict(op='settle', required_chunks=[[0, 0]]))
    for label, text, expected in ARENA_COMMANDS:
        cmd(text, label, expected)
    for name, tail, dim, pos, extra in CASES:
        prefix = f'/execute in {dim} run ' if dim != 'minecraft:overworld' else '/'
        cmd(
            f'{prefix}summon minecraft:evoker_fangs {pos} {{UUID:[I;2362,0,0,{tail}],Warmup:100{extra}}}',
            f'{name}-summon',
            1,
        )
    previous = 0
    for tick in TICKS:
        if tick:
            delta = tick - previous
            cmd(f'/tick step {delta}', f'step-{tick}')
            ops.append(dict(op='tick', n=delta))
        previous = tick
        for name, tail, dim, _, _ in CASES:
            prefix = f'/execute in {dim} run ' if dim != 'minecraft:overworld' else '/'
            cmd(f'{prefix}data get entity {fangs_uuid(tail)} Motion', f'{name}-t{tick}-motion', 3, admission=False)
    for label, text in BLOCK_GUARDS:
        cmd(text, label, 1)
    return ops


def double_bits(text):
    return struct.pack('>d', float(text)).hex()


def extract_rows(recorded, records):
    by_label = {op['label']: row for op, row in zip(recorded, records)}
    rows = []
    for name, _, _, _, _ in CASES:
        for tick in TICKS:
            row = by_label[f'{name}-t{tick}-motion']
            messages = row.get('messages') or []
            if len(messages) != 1:
                raise AssertionError(f'expected one Motion message for {name} tick {tick}: {row}')
            text = ''.join(span[0] for span in messages[0])
            match = MOTION_RE.search(text)
            if not match:
                raise AssertionError(f'unexpected Motion reply for {name} tick {tick}: {text!r}')
            mx, my, mz = match.groups()
            rows.append((name, tick, double_bits(mx), double_bits(my), double_bits(mz), mx, my, mz))
    return rows


def java_version_string():
    result = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def format_witness(rows, version=MINECRAFT_VERSION):
    lines = [
        f'# minecraft {version} evoker_fangs fluid current Motion witness',
        '# recorded by: python3 tools/test_fangs_fluid_current_parity.py',
        '# case\ttick\tmx_bits\tmy_bits\tmz_bits\tmx\tmy\tmz',
    ]
    for name, tick, bx, by, bz, mx, my, mz in rows:
        lines.append(f'{name}\t{tick}\t{bx}\t{by}\t{bz}\t{mx}\t{my}\t{mz}')
    return '\n'.join(lines) + '\n'


def pop_resources(argv):
    resources = Path(os.environ.get('SUNRISE_TEST_RESOURCES', driver.lab.RESOURCES))
    kept = [argv[0]]
    index = 1
    while index < len(argv):
        arg = argv[index]
        if arg == '--resources' and index + 1 < len(argv):
            resources = Path(argv[index + 1])
            index += 2
        elif arg.startswith('--resources='):
            resources = Path(arg[len('--resources='):])
            index += 1
        else:
            kept.append(arg)
            index += 1
    argv[:] = kept
    return resources


def main():
    resources = pop_resources(sys.argv)
    record = False
    for flag in ('--record', '--record-only'):
        while flag in sys.argv:
            sys.argv.remove(flag)
            record = True
    if record and '--reference-only' not in sys.argv:
        sys.argv.append('--reference-only')
    build = harness_args.ensure_work_dir(driver.lab.LOCAL / 'fangs-fluid-current-parity').resolve()
    ops = program()
    driver.corpus_module.groups = lambda: [('evoker fangs fluid current', ops)]
    if '--allow-gaps' not in sys.argv:
        sys.argv.append('--allow-gaps')
    driver.main()
    lock, jars, _ = driver.base.verify_artifacts()
    version = lock['minecraft']['version']
    recorded = [op for op in ops if op['op'] == 'cmd' and op.get('record', True)]
    java_bytes = (build / 'vanilla.jsonl').read_bytes()
    java = [json.loads(line) for line in java_bytes.splitlines()]
    assert len(recorded) == len(java)
    witness_dir = driver.lab.LOCAL / 'witness' / version / 'fangs_fluid_current'
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness_path = witness_dir / 'fangs_fluid_current.tsv'
    witness_text = format_witness(extract_rows(recorded, java), version)
    witness_path.write_text(witness_text, encoding='utf-8')
    harness_path = Path(__file__).resolve()
    driver.lab.write_json(witness_dir / 'meta.json', {
        'game_version': version,
        'java_major': lock['minecraft']['java_major'],
        'java_version': java_version_string(),
        'harness': 'tools/test_fangs_fluid_current_parity.py',
        'oracle': 'dev.sunriselab.parity.VanillaCommandExecOracle',
        'witness_env': 'SUNRISE_FANGS_FLUID_CURRENT_WITNESS',
        'command': [sys.executable, 'tools/test_fangs_fluid_current_parity.py', '--record'],
        'input_hashes': {
            'research-lock.json': driver.lab.digest(driver.lab.LOCK, 'sha256'),
            'server.jar': driver.lab.digest(jars['server'], 'sha256'),
            ORACLE.relative_to(driver.lab.ROOT).as_posix(): driver.lab.digest(ORACLE, 'sha256'),
            harness_path.relative_to(driver.lab.ROOT).as_posix(): driver.lab.digest(harness_path, 'sha256'),
        },
        'outputs': {
            witness_path.name: {
                'sha256': driver.lab.digest(witness_path, 'sha256'),
                'bytes': witness_path.stat().st_size,
            },
        },
    })
    if record:
        print(f'Recorded {version} witness: {witness_path}')
        return
    if '--reference-only' in sys.argv:
        print(f'Recorded vanilla {version} reference: {witness_path}')
        return
    native = [json.loads(line) for line in (build / 'rust.jsonl').read_text(encoding='utf-8').splitlines()]
    assert len(recorded) == len(native)
    differences = [
        dict(label=op['label'], expected=v, actual=r)
        for op, v, r in zip(recorded, java, native)
        if v != r
    ]
    report = dict(
        minecraft=version,
        records=len(native),
        passed=len(native) - len(differences),
        differences=differences,
        reference_sha256=hashlib.sha256(java_bytes).hexdigest(),
        comparison='exact raw records and IEEE-754 double bits; no tolerances',
    )
    (build / 'strict-report.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    print(f'Strict raw records: {report["passed"]}/{report["records"]}')
    if differences:
        raise SystemExit(1)
    with tempfile.TemporaryDirectory(prefix='sunrise-fangs-fluid-current-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = driver.lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        env['SUNRISE_TEST_RESOURCES'] = str(resources.resolve())
        env['SUNRISE_FANGS_FLUID_CURRENT_WITNESS'] = str(witness_path.resolve())
        binary = driver.lab.parity_fixtures()
        log_path = build / 'rust-fixture.log'
        with log_path.open('w', encoding='utf-8') as log:
            ran = subprocess.run(
                [str(binary), TEST, '--exact', '--ignored', '--nocapture'],
                cwd=scratch, env=env, timeout=3600, stdout=log, stderr=subprocess.STDOUT,
            )
        if ran.returncode or '1 passed; 0 failed' not in log_path.read_text(encoding='utf-8'):
            tail = log_path.read_text(encoding='utf-8', errors='replace')[-3000:]
            sys.exit(f'{TEST} failed against {witness_path}; see {log_path}\n{tail}')
    print(f'Fangs fluid current parity passed ({len(CASES)} cases against {witness_path})')


if __name__ == '__main__':
    main()
