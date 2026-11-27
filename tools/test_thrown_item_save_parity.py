#!/usr/bin/env python3
"""Record the vanilla 26.3 moved thrown-item save witness into .local and run its Rust fixture.

Runs the 'moved thrown item save' group of tools/test_command_exec_parity.py against
VanillaCommandExecOracle in --reference-only mode, writes moved_thrown_item_save.json and
meta.json into .local/witness/<version>/thrown_item_save/ (gitignored), and unless --record /
--record-only is passed, runs sunrise-parity-fixtures on
game::server_side::persist::thrown_item_tests::a_moved_thrown_item_projectile_saves_its_loaded_item
with SUNRISE_THROWN_ITEM_SAVE_WITNESS pointing at the recorded witness.

  test_thrown_item_save_parity.py [--prepare] [--record | --record-only] [--resources DIR]
"""
import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, test_rust_parity as base

GROUP = 'moved thrown item save'
ORACLE = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCommandExecOracle.java'
CORPUS = lab.ROOT / 'tools/command_exec_corpus.py'
DRIVER = lab.ROOT / 'tools/test_command_exec_parity.py'
TEST = 'game::server_side::persist::thrown_item_tests::a_moved_thrown_item_projectile_saves_its_loaded_item'
ENV_VAR = 'SUNRISE_THROWN_ITEM_SAVE_WITNESS'


def java_version_string():
    result = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def run_logged(command, build, name, cwd=lab.ROOT, env=None):
    path = build / f'{name}.log'
    with path.open('w', encoding='utf-8') as log:
        result = subprocess.run(
            [str(part) for part in command],
            cwd=cwd,
            env=env,
            stdin=subprocess.DEVNULL,
            timeout=3600,
            stdout=log,
            stderr=subprocess.STDOUT,
        )
    if result.returncode:
        tail = path.read_text(encoding='utf-8', errors='replace').splitlines()[-40:]
        print('\n'.join(tail), file=sys.stderr, flush=True)
        sys.exit(f'{name} failed: see {path}')
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument(
        '--record',
        '--record-only',
        dest='record_only',
        action='store_true',
        help='record the vanilla witness and meta.json into .local without running the Rust test',
    )
    parser.add_argument(
        '--resources',
        type=Path,
        default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)),
    )
    args = parser.parse_args()

    if args.prepare:
        lab.prepare()
    lock, jars, _ = base.verify_artifacts()
    version = lock['minecraft']['version']

    build = lab.LOCAL / 'thrown-item-save-parity'
    build.mkdir(parents=True, exist_ok=True)
    witness_dir = lab.LOCAL / 'witness' / version / 'thrown_item_save'
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness = witness_dir / 'moved_thrown_item_save.json'

    record_cmd = [
        sys.executable,
        str(DRIVER),
        '--only',
        GROUP,
        '--reference-only',
        '--work-dir',
        str(build),
        '--record-thrown-item-witness',
        str(witness),
    ]
    run_logged(record_cmd, build, 'record-witness')
    if not witness.is_file():
        sys.exit(f'recorder did not write {witness}')

    lab.write_json(
        witness_dir / 'meta.json',
        {
            'game_version': version,
            'java_major': lock['minecraft']['java_major'],
            'java_version': java_version_string(),
            'harness': 'tools/test_thrown_item_save_parity.py',
            'oracle': 'dev.sunriselab.parity.VanillaCommandExecOracle',
            'witness_env': ENV_VAR,
            'command': [str(part) for part in record_cmd],
            'input_hashes': {
                'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
                'server.jar': lab.digest(jars['server'], 'sha256'),
                ORACLE.relative_to(lab.ROOT).as_posix(): lab.digest(ORACLE, 'sha256'),
                CORPUS.relative_to(lab.ROOT).as_posix(): lab.digest(CORPUS, 'sha256'),
                DRIVER.relative_to(lab.ROOT).as_posix(): lab.digest(DRIVER, 'sha256'),
            },
            'outputs': {
                witness.name: {
                    'sha256': lab.digest(witness, 'sha256'),
                    'bytes': witness.stat().st_size,
                },
            },
        },
    )

    if args.record_only:
        print(f'Recorded thrown-item save witness into {witness}')
        return

    with tempfile.TemporaryDirectory(prefix='sunrise-thrown-item-save-') as tmp:
        scratch = Path(tmp)
        homes = {
            key: scratch / key
            for key in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')
        }
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env[ENV_VAR] = str(witness.resolve())
        binary = lab.parity_fixtures()
        log = run_logged(
            [binary, TEST, '--exact', '--ignored', '--nocapture'],
            build,
            'rust',
            cwd=scratch,
            env=env,
        )
        if '1 passed; 0 failed' not in log.read_text(encoding='utf-8'):
            sys.exit(f'expected 1 passed; 0 failed for {TEST} in {log}')

    print(f'Thrown-item save parity passed ({TEST} against {witness})')


if __name__ == '__main__':
    main()
