#!/usr/bin/env python3
"""Compare a mob's live PortalCooldown across summon, tick, chunk reload, and /data edits.

Records the vanilla 26.3 witness and meta.json into .local/witness/<version>/mob_portal_cooldown/
through VanillaCommandExecOracle, then runs the Rust witness test via lab.parity_fixtures() with
SUNRISE_MOB_PORTAL_COOLDOWN_WITNESS.
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
from tools import lab
from tools.command_exec_corpus import cmd, setup

INITIAL_COOLDOWN = 20
MERGED_COOLDOWN = 50
WITNESS_ENV = 'SUNRISE_MOB_PORTAL_COOLDOWN_WITNESS'
WITNESS_NAME = 'mob_portal_cooldown.json'
TEST = 'game::server_side::commands::data::portal_cooldown_tests::a_mob_saves_its_live_portal_cooldown'
ORACLE_PATH = 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCommandExecOracle.java'


def steps():
    # Use a distant chunk beyond player tickets so /forceload exercises real chunk save and load.
    chunk = '4096 4096'
    wait = {'op': 'wait_chunks', 'chunks': [[256, 256]]}
    live = '@e[type=pig,tag=mob_portal_cooldown_live,limit=1]'
    plain = '@e[type=pig,tag=mob_portal_cooldown_plain,limit=1]'
    return [
        *((None, op) for op in setup(f'/forceload add {chunk}')),
        (None, wait),
        (None, cmd(
            f'/summon minecraft:pig 4100.5 300 4104.5 '
            f'{{NoAI:1b,NoGravity:1b,PortalCooldown:{INITIAL_COOLDOWN},Tags:["mob_portal_cooldown_live"]}}'
        )),
        ('after_summon', cmd(f'/data get entity {live} PortalCooldown')),
        (None, {'op': 'tick', 'n': 1}),
        ('after_first_tick', cmd(f'/data get entity {live} PortalCooldown')),
        *((None, op) for op in setup(f'/forceload remove {chunk}')),
        (None, {'op': 'tick', 'n': 60}),
        (None, cmd('/execute unless loaded 4096 300 4096', expect_result=1)),
        (None, cmd(f'/execute unless entity {live}', expect_result=1)),
        *((None, op) for op in setup(f'/forceload add {chunk}')),
        (None, wait),
        (None, cmd(f'/execute if entity {live}', expect_result=1)),
        ('after_chunk_reload', cmd(f'/data get entity {live} PortalCooldown')),
        (None, {'op': 'tick', 'n': 1}),
        ('after_reload_and_tick', cmd(f'/data get entity {live} PortalCooldown')),
        (None, cmd(f'/data merge entity {live} {{PortalCooldown:{MERGED_COOLDOWN}}}')),
        ('after_data_merge', cmd(f'/data get entity {live} PortalCooldown')),
        (None, {'op': 'tick', 'n': 1}),
        ('after_merge_and_tick', cmd(f'/data get entity {live} PortalCooldown')),
        (None, cmd(f'/data remove entity {live} PortalCooldown')),
        ('after_data_remove', cmd(f'/data get entity {live} PortalCooldown')),
        *((None, op) for op in setup(f'/kill {live}', f'/forceload remove {chunk}')),
        (None, cmd(
            '/summon minecraft:pig 12.5 234 8.5 '
            '{NoAI:1b,NoGravity:1b,Tags:["mob_portal_cooldown_plain"]}'
        )),
        ('plain_after_summon', cmd(f'/data get entity {plain} PortalCooldown')),
        (None, {'op': 'tick', 'n': 1}),
        ('plain_after_first_tick', cmd(f'/data get entity {plain} PortalCooldown')),
        *((None, op) for op in setup(f'/kill {plain}')),
    ]


def groups():
    return [('mob portal cooldown', [op for _, op in steps()])]


def extract_witness(vanilla_jsonl):
    recorded_steps = [
        (label, op) for label, op in steps()
        if (op['op'] == 'cmd' and op.get('record', True)) or (op['op'] == 'tick' and op.get('record'))
    ]
    rows = [json.loads(line) for line in vanilla_jsonl.read_text(encoding='utf-8').splitlines() if line.strip()]
    if len(rows) != len(recorded_steps):
        raise SystemExit(f'expected {len(recorded_steps)} recorded vanilla rows, got {len(rows)}')
    expected = {}
    for (label, op), row in zip(recorded_steps, rows):
        if row.get('cmd') != op.get('cmd'):
            raise SystemExit(f'command mismatch: expected {op.get("cmd")!r}, got {row.get("cmd")!r}')
        if label is None:
            continue
        results = row.get('results')
        if not (isinstance(results, list) and len(results) == 1
                and isinstance(results[0], list) and len(results[0]) == 2
                and results[0][0] is True and type(results[0][1]) is int):
            raise SystemExit(f'probe {label!r} did not return a single integer result: {row}')
        expected[label] = results[0][1]
    return {
        'minecraft': lab.VERSION,
        'oracle': 'dev.sunriselab.parity.VanillaCommandExecOracle',
        'harness': 'tools/test_mob_portal_cooldown_parity.py',
        'record_cmd': 'python3 tools/test_mob_portal_cooldown_parity.py --record',
        'inputs': {
            'initial_cooldown': INITIAL_COOLDOWN,
            'merged_cooldown': MERGED_COOLDOWN,
        },
        'expected': expected,
    }


def java_version_string():
    result = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def write_meta(witness_dir, witness_file, cases, command):
    oracle_src = lab.ROOT / ORACLE_PATH
    input_hashes = {
        'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
        ORACLE_PATH: lab.digest(oracle_src, 'sha256'),
        'tools/test_mob_portal_cooldown_parity.py': lab.digest(Path(__file__).resolve(), 'sha256'),
        'groups.json': lab.digest(cases, 'sha256'),
    }
    for jar_name in ('server-game.jar', 'server.jar'):
        jar = lab.GAME / 'jars' / jar_name
        if jar.is_file():
            input_hashes[jar_name] = lab.digest(jar, 'sha256')
            break
    lab.write_json(witness_dir / 'meta.json', {
        'minecraft': lab.VERSION,
        'java_version': java_version_string(),
        'command': [str(part) for part in command],
        'oracle': ORACLE_PATH,
        'harness': 'tools/test_mob_portal_cooldown_parity.py',
        'witness_env': WITNESS_ENV,
        'input_hashes': input_hashes,
        'outputs': {
            WITNESS_NAME: {
                'sha256': lab.digest(witness_file, 'sha256'),
                'bytes': witness_file.stat().st_size,
            },
        },
    })


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument('--reference-only', action='store_true')
    parser.add_argument('--record', action='store_true')
    parser.add_argument('--work-dir', type=Path)
    parser.add_argument('--witness-dir', type=Path)
    parser.add_argument('--resources', type=Path, default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    args = parser.parse_args()
    root = lab.ROOT
    work = (args.work_dir or lab.LOCAL / 'mob-portal-cooldown-parity').resolve()
    work.mkdir(parents=True, exist_ok=True)
    cases = work / 'groups.json'
    cases.write_text(json.dumps(groups()), encoding='utf-8')
    run = [sys.executable, str(root / 'tools/test_command_exec_parity.py'),
           '--groups-file', str(cases), '--work-dir', str(work)]
    if args.binary:
        run.extend(['--binary', str(args.binary.resolve())])
    if args.prepare:
        run.append('--prepare')
    if args.reference_only or args.record:
        run.append('--reference-only')
    result = subprocess.run(run, cwd=root)
    if result.returncode:
        return result.returncode
    witness = extract_witness(work / 'vanilla.jsonl')
    witness_dir = (args.witness_dir or lab.LOCAL / 'witness' / lab.VERSION / 'mob_portal_cooldown').resolve()
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness_file = witness_dir / WITNESS_NAME
    lab.write_json(witness_file, witness)
    write_meta(witness_dir, witness_file, cases, run)
    if args.record or args.reference_only:
        print(f'Recorded {lab.VERSION} witness: {witness_file}')
        return 0
    with tempfile.TemporaryDirectory(prefix='sunrise-mob-portal-cooldown-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env[WITNESS_ENV] = str(witness_file.resolve())
        binary = lab.parity_fixtures()
        log_path = work / 'rust-witness-test.log'
        with log_path.open('w', encoding='utf-8') as log:
            test_result = subprocess.run(
                [str(binary), TEST, '--exact', '--ignored', '--nocapture'],
                cwd=scratch, env=env, timeout=3600, stdout=log, stderr=subprocess.STDOUT,
            )
        output = log_path.read_text(encoding='utf-8', errors='replace')
        if test_result.returncode or '1 passed; 0 failed' not in output:
            tail = output.splitlines()[-40:]
            print('\n'.join(tail), file=sys.stderr, flush=True)
            raise SystemExit(f'{TEST} failed against {witness_file}; see {log_path}')
    print(f'Mob portal cooldown parity passed (witness test against {witness_file})')
    return 0


if __name__ == '__main__':
    sys.exit(main())
