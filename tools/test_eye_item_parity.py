#!/usr/bin/env python3
"""Record the vanilla 26.3 eye_of_ender Item witness into .local and run the Rust witness test.

Runs VanillaCommandExecOracle on the `summoned eye of ender` command group via
tools/test_command_exec_parity.py --record-eye-item-witness (going through tools/parity_shim.py
when invoked by tools/parity.py), writing eye_item.tsv and meta.json into
.local/witness/<version>/eye_item/, then runs the sunrise-client parity fixture against that
witness via SUNRISE_EYE_ITEM_WITNESS (#2357).

  test_eye_item_parity.py [--prepare] [--record | --record-only] [--work-dir DIR] [--resources DIR]
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

ORACLE = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCommandExecOracle.java'
DRIVER = lab.ROOT / 'tools/test_command_exec_parity.py'
CORPUS = lab.ROOT / 'tools/command_exec_corpus.py'
TEST = 'game::server_side::commands::data::eye_item_tests::a_summoned_eye_saves_its_normalised_item_live_and_after_a_reload'


def java_version_string():
    result = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def run_logged(command, build, name, cwd=lab.ROOT, env=None):
    path = build / (name + '.log')
    with path.open('w', encoding='utf-8') as log:
        result = subprocess.run(list(map(str, command)), cwd=cwd, env=env, timeout=3600,
                                stdout=log, stderr=subprocess.STDOUT)
    if result.returncode:
        tail = path.read_text(encoding='utf-8', errors='replace').splitlines()[-40:]
        print('\n'.join(tail), file=sys.stderr, flush=True)
        sys.exit(f'{name} failed: see {path}')
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument('--record', '--record-only', dest='record_only', action='store_true',
                        help='only record the vanilla witness and meta.json into .local')
    parser.add_argument('--work-dir', type=Path, help='isolated scratch output directory')
    parser.add_argument('--resources', type=Path,
                        default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    args = parser.parse_args()

    if args.prepare:
        lab.prepare()
    lock, jars, _ = base.verify_artifacts()
    version = lock['minecraft']['version']

    build = args.work_dir.resolve() if args.work_dir else lab.LOCAL / 'eye-item-parity'
    build.mkdir(parents=True, exist_ok=True)
    witness_dir = lab.LOCAL / 'witness' / version / 'eye_item'
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness = witness_dir / 'eye_item.tsv'

    record_cmd = [
        sys.executable,
        str(DRIVER),
        '--record-eye-item-witness',
        str(witness),
        '--work-dir',
        str(build),
    ]
    run_logged(record_cmd, build, 'record-witness')
    if not witness.is_file():
        sys.exit(f'recorder did not write {witness}')

    lab.write_json(witness_dir / 'meta.json', {
        'game_version': version,
        'java_major': lock['minecraft']['java_major'],
        'java_version': java_version_string(),
        'harness': 'tools/test_eye_item_parity.py',
        'oracle': 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCommandExecOracle.java',
        'witness_env': 'SUNRISE_EYE_ITEM_WITNESS',
        'command': [str(part) for part in record_cmd],
        'input_hashes': {
            'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
            'server.jar': lab.digest(jars['server'], 'sha256'),
            'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCommandExecOracle.java': lab.digest(ORACLE, 'sha256'),
            'tools/command_exec_corpus.py': lab.digest(CORPUS, 'sha256'),
            'tools/test_command_exec_parity.py': lab.digest(DRIVER, 'sha256'),
            'tools/test_eye_item_parity.py': lab.digest(Path(__file__).resolve(), 'sha256'),
        },
        'outputs': {
            'eye_item.tsv': {'sha256': lab.digest(witness, 'sha256'), 'bytes': witness.stat().st_size},
        },
    })

    if args.record_only:
        print(f'Recorded eye_item witness into {witness_dir}')
        return

    with tempfile.TemporaryDirectory(prefix='sunrise-eye-item-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env['SUNRISE_EYE_ITEM_WITNESS'] = str(witness.resolve())
        binary = lab.parity_fixtures()
        log = run_logged([binary, TEST, '--exact', '--ignored', '--nocapture'], build, 'rust', cwd=scratch, env=env)
        if '1 passed; 0 failed' not in log.read_text(encoding='utf-8'):
            sys.exit(f'expected 1 passed; 0 failed for {TEST} in {log}')

    print(f'Eye item parity passed ({TEST} against {witness})')


if __name__ == '__main__':
    main()
