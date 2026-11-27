#!/usr/bin/env python3
"""Record the vanilla 26.3 summon lightning_bolt NBT witness into .local and run the Rust witness test.

Runs VanillaCommandExecOracle (with SeedAgent) through tools/test_command_exec_parity.py
for the `summon lightning_bolt NBT` corpus group (#2690), writing summon_bolt_nbt.json and
meta.json into .local/witness/<version>/summon_bolt_nbt/ (gitignored). Unless --record /
--record-only is given, runs game::server_side::commands::summon_bolt_tests::a_bolt_summoned_with_nbt_strikes_and_goes
through lab.parity_fixtures() with SUNRISE_SUMMON_BOLT_NBT_WITNESS pointing at the recorded witness.

  test_summon_bolt_nbt_parity.py [--prepare] [--record | --record-only] [--resources DIR]
"""
import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, seed_agent, test_rust_parity as base

GROUP = 'summon lightning_bolt NBT'
ORACLE = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCommandExecOracle.java'
TEST = 'game::server_side::commands::summon_bolt_tests::a_bolt_summoned_with_nbt_strikes_and_goes'


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
            timeout=3600,
            stdout=log,
            stderr=subprocess.STDOUT,
        )
    if result.returncode:
        tail = path.read_text(encoding='utf-8', errors='replace').splitlines()[-40:]
        print('\n'.join(tail), file=sys.stderr, flush=True)
        sys.exit(f'{name} failed ({result.returncode}): see {path}')
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument(
        '--record',
        '--record-only',
        action='store_true',
        dest='record_only',
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

    build = lab.LOCAL / 'summon-bolt-nbt-parity'
    build.mkdir(parents=True, exist_ok=True)
    witness_dir = lab.LOCAL / 'witness' / version / 'summon_bolt_nbt'
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness = witness_dir / 'summon_bolt_nbt.json'

    record_cmd = [
        sys.executable,
        str(lab.ROOT / 'tools/test_command_exec_parity.py'),
        '--only',
        GROUP,
        '--record',
        str(witness),
        '--work-dir',
        str(build),
    ]
    run_logged(record_cmd, build, 'record', cwd=lab.ROOT)
    if not witness.is_file():
        sys.exit(f'recorder did not write {witness}')

    input_sources = [
        lab.LOCK,
        ORACLE,
        seed_agent.SOURCE,
        lab.ROOT / 'tools/command_exec_corpus.py',
        lab.ROOT / 'tools/test_command_exec_parity.py',
    ]
    input_hashes = {'server.jar': lab.digest(jars['server'], 'sha256')}
    for src in input_sources:
        input_hashes[src.relative_to(lab.ROOT).as_posix()] = lab.digest(src, 'sha256')
    lab.write_json(
        witness_dir / 'meta.json',
        {
            'game_version': version,
            'java_major': lock['minecraft']['java_major'],
            'java_version': java_version_string(),
            'harness': 'tools/test_summon_bolt_nbt_parity.py',
            'witness_env': 'SUNRISE_SUMMON_BOLT_NBT_WITNESS',
            'oracle': ORACLE.relative_to(lab.ROOT).as_posix(),
            'group': GROUP,
            'command': [str(part) for part in record_cmd],
            'input_hashes': input_hashes,
            'outputs': {
                witness.name: {
                    'sha256': lab.digest(witness, 'sha256'),
                    'bytes': witness.stat().st_size,
                }
            },
        },
    )

    if args.record_only:
        print(f'Recorded summon_bolt_nbt witness into {witness_dir}')
        return

    with tempfile.TemporaryDirectory(prefix='sunrise-summon-bolt-nbt-') as tmp:
        scratch = Path(tmp)
        homes = {
            k: scratch / k
            for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')
        }
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env['SUNRISE_SUMMON_BOLT_NBT_WITNESS'] = str(witness.resolve())
        env['SUNRISE_TEST_SEEDS'] = '1'
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

    print(f'Summon bolt NBT parity passed ({TEST} against {witness})')


if __name__ == '__main__':
    main()
