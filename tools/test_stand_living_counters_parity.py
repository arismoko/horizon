#!/usr/bin/env python3
"""Record vanilla 26.3 armor stand living-counter witness into .local and run the Rust witness test.

Runs VanillaCommandExecOracle for the `armor stand living counters` group through
tools/test_command_exec_parity.py (cached by tools/parity_shim.py), writes the witness JSON
and meta.json into .local/witness/<version>/stand_living_counters/, then runs
game::server_side::commands::data::stand_living_tests::an_armor_stand_ticks_its_living_counters through
lab.parity_fixtures() with SUNRISE_STAND_LIVING_COUNTERS_WITNESS.
"""
import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import command_exec_corpus as corpus_module, lab

WITNESS_ENV = 'SUNRISE_STAND_LIVING_COUNTERS_WITNESS'
WITNESS_NAME = 'stand_living_counters.json'
TEST = 'game::server_side::commands::data::stand_living_tests::an_armor_stand_ticks_its_living_counters'
ORACLE_PATH = 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCommandExecOracle.java'


def java_version_string():
    result = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def write_meta(witness_dir, witness_file, command):
    input_hashes = {
        'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
        ORACLE_PATH: lab.digest(lab.ROOT / ORACLE_PATH, 'sha256'),
        'tools/command_exec_corpus.py': lab.digest(lab.ROOT / 'tools/command_exec_corpus.py', 'sha256'),
        'tools/test_command_exec_parity.py': lab.digest(lab.ROOT / 'tools/test_command_exec_parity.py', 'sha256'),
        'tools/test_stand_living_counters_parity.py': lab.digest(Path(__file__).resolve(), 'sha256'),
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
        'harness': 'tools/test_stand_living_counters_parity.py',
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
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument('--reference-only', action='store_true')
    parser.add_argument('--record', action='store_true')
    parser.add_argument('--work-dir', type=Path)
    parser.add_argument('--witness-dir', type=Path)
    parser.add_argument('--resources', type=Path, default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    args = parser.parse_args()
    work = (args.work_dir or lab.LOCAL / 'stand-living-counters-parity').resolve()
    work.mkdir(parents=True, exist_ok=True)
    witness_dir = (args.witness_dir or lab.LOCAL / 'witness' / lab.VERSION / 'stand_living_counters').resolve()
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness_file = witness_dir / WITNESS_NAME
    run = [
        sys.executable, str(lab.ROOT / 'tools/test_command_exec_parity.py'),
        '--only', corpus_module.STAND_LIVING_GROUP,
        '--reference-only',
        '--work-dir', str(work),
        '--record-stand-living-witness', str(witness_file),
    ]
    if args.prepare:
        run.append('--prepare')
    result = subprocess.run(run, cwd=lab.ROOT)
    if result.returncode:
        return result.returncode
    if not witness_file.is_file():
        raise SystemExit(f'recorder did not write {witness_file}')
    write_meta(witness_dir, witness_file, run)
    if args.record or args.reference_only:
        print(f'Recorded {lab.VERSION} witness: {witness_file}')
        return 0
    with tempfile.TemporaryDirectory(prefix='sunrise-stand-living-counters-') as tmp:
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
    print(f'Armor stand living counters parity passed (witness test against {witness_file})')
    return 0


if __name__ == '__main__':
    sys.exit(main())
