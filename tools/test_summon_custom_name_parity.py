#!/usr/bin/env python3
"""Record vanilla 26.3 /summon CustomName replies into .local and run the Rust witness test.

Runs VanillaCommandExecOracle on the `summon custom name replies` corpus group through
tools/test_command_exec_parity.py (--record-summon-name-witness), writing
summon_custom_name_replies.json and meta.json into .local/witness/<version>/summon_custom_name/
(gitignored). Unless --record / --record-only is given, runs the sunrise-client fixture
through lab.parity_fixtures() with SUNRISE_SUMMON_CUSTOM_NAME_WITNESS pointing at the witness.

  test_summon_custom_name_parity.py [--prepare] [--record | --record-only] [--resources DIR]
"""
import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, test_command_exec_parity as driver, test_rust_parity as base

GROUP = 'summon custom name replies'
TEST = 'game::server_side::commands::summon_item_orb_tests::summon_replies_with_the_custom_name_of_every_kind'
ORACLE = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCommandExecOracle.java'


def java_version_string():
    result = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def record(lock, jars, build, witness_dir):
    version = lock['minecraft']['version']
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness = witness_dir / 'summon_custom_name_replies.json'
    record_argv = [
        str(lab.ROOT / 'tools/test_command_exec_parity.py'),
        '--only', GROUP,
        '--record-summon-name-witness', str(witness),
        '--work-dir', str(build),
    ]
    saved_argv = sys.argv
    try:
        sys.argv = record_argv
        driver.main()
    finally:
        sys.argv = saved_argv
    if not witness.is_file():
        sys.exit(f'VanillaCommandExecOracle did not write {witness}')
    input_hashes = {
        'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
        'server.jar': lab.digest(jars['server'], 'sha256'),
        ORACLE.relative_to(lab.ROOT).as_posix(): lab.digest(ORACLE, 'sha256'),
        'tools/command_exec_corpus.py': lab.digest(lab.ROOT / 'tools/command_exec_corpus.py', 'sha256'),
        'tools/test_command_exec_parity.py': lab.digest(lab.ROOT / 'tools/test_command_exec_parity.py', 'sha256'),
        'script.jsonl': lab.digest(build / 'script.jsonl', 'sha256'),
    }
    lab.write_json(witness_dir / 'meta.json', {
        'game_version': version,
        'java_major': lock['minecraft']['java_major'],
        'java_version': java_version_string(),
        'harness': 'tools/test_summon_custom_name_parity.py',
        'witness_env': 'SUNRISE_SUMMON_CUSTOM_NAME_WITNESS',
        'oracle': ORACLE.relative_to(lab.ROOT).as_posix(),
        'group': GROUP,
        'command': [
            'python3', 'tools/test_command_exec_parity.py',
            '--only', GROUP,
            '--record-summon-name-witness', witness.relative_to(lab.ROOT).as_posix(),
            '--work-dir', build.relative_to(lab.ROOT).as_posix(),
        ],
        'input_hashes': input_hashes,
        'outputs': {
            witness.name: {
                'sha256': lab.digest(witness, 'sha256'),
                'bytes': witness.stat().st_size,
            },
        },
    })
    return witness


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument('--record', '--record-only', dest='record_only', action='store_true',
                        help='record only the vanilla witness into .local/witness/<version>/summon_custom_name/')
    parser.add_argument('--resources', type=Path, default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    args = parser.parse_args()

    if args.prepare:
        lab.prepare()
    lock, jars, _ = base.verify_artifacts()
    version = lock['minecraft']['version']

    build = lab.LOCAL / 'summon-custom-name-parity'
    build.mkdir(parents=True, exist_ok=True)
    witness_dir = lab.LOCAL / 'witness' / version / 'summon_custom_name'
    witness = record(lock, jars, build, witness_dir)

    if args.record_only:
        print(f'Recorded summon custom name witness into {witness}')
        return

    with tempfile.TemporaryDirectory(prefix='sunrise-summon-custom-name-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env['SUNRISE_SUMMON_CUSTOM_NAME_WITNESS'] = str(witness.resolve())
        binary = lab.parity_fixtures()
        log = build / 'rust.log'
        with log.open('w', encoding='utf-8') as out:
            result = subprocess.run(
                [str(binary), TEST, '--exact', '--ignored', '--nocapture'],
                cwd=scratch, env=env, timeout=3600, stdout=out, stderr=subprocess.STDOUT,
            )
        if result.returncode:
            tail = log.read_text(encoding='utf-8', errors='replace').splitlines()[-40:]
            print('\n'.join(tail), file=sys.stderr, flush=True)
            sys.exit(f'{TEST} failed against the recorded witness; see {log}')
        if '1 passed; 0 failed' not in log.read_text(encoding='utf-8'):
            sys.exit(f'expected 1 passed; 0 failed for {TEST} in {log}')

    print(f'Summon custom name parity passed ({TEST} against {witness})')


if __name__ == '__main__':
    main()
