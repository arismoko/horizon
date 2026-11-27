#!/usr/bin/env python3
"""Record the vanilla 26.3 falling-block moving-piston witness into .local and run the Rust witness test (#2363).

Runs tools/test_command_exec_parity.py on the `falling block resting in moving piston` group
against VanillaCommandExecOracle (cached through tools/parity_shim.py when run under
tools/parity.py), writing falling_block_moving_piston.jsonl and meta.json under
.local/witness/<version>/falling_moving_piston/ (gitignored). Unless --record / --record-only
is given, runs the sunrise-client parity fixture against that file via
SUNRISE_FALLING_MOVING_PISTON_WITNESS.

  test_falling_moving_piston_parity.py [--prepare] [--record | --record-only] [--resources DIR]
"""
import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab

GROUP = 'falling block resting in moving piston'
TEST = 'game::server_side::falling_blocks::moving_piston_tests::a_falling_block_resting_in_a_moving_piston_cell_stays'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument('--record', '--record-only', dest='record_only', action='store_true')
    parser.add_argument('--resources', type=Path, default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    args = parser.parse_args()

    build = lab.LOCAL / 'falling-moving-piston-parity'
    build.mkdir(parents=True, exist_ok=True)
    witness_dir = lab.LOCAL / 'witness' / lab.VERSION / 'falling_moving_piston'
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness = witness_dir / 'falling_block_moving_piston.jsonl'

    record_cmd = [
        sys.executable,
        str(lab.ROOT / 'tools/test_command_exec_parity.py'),
        '--only', GROUP,
        '--work-dir', str(build),
        '--record-moving-piston-witness', str(witness),
    ]
    if args.prepare:
        record_cmd.append('--prepare')
    recorded = subprocess.run(record_cmd, cwd=lab.ROOT, stdin=subprocess.DEVNULL, timeout=3600)
    if recorded.returncode:
        sys.exit(recorded.returncode)
    if not witness.is_file() or not (witness_dir / 'meta.json').is_file():
        sys.exit(f'recorder did not write {witness} and {witness_dir / "meta.json"}')

    if args.record_only:
        print(f'Recorded falling-block moving-piston witness into {witness_dir}')
        return

    with tempfile.TemporaryDirectory(prefix='sunrise-falling-moving-piston-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env['SUNRISE_FALLING_MOVING_PISTON_WITNESS'] = str(witness.resolve())
        binary = lab.parity_fixtures()
        log_path = build / 'rust.log'
        with log_path.open('w', encoding='utf-8') as log:
            result = subprocess.run(
                [str(binary), TEST, '--exact', '--ignored', '--nocapture'],
                cwd=scratch, env=env, stdin=subprocess.DEVNULL, timeout=3600,
                stdout=log, stderr=subprocess.STDOUT,
            )
        if result.returncode:
            tail = log_path.read_text(encoding='utf-8', errors='replace').splitlines()[-40:]
            print('\n'.join(tail), file=sys.stderr, flush=True)
            sys.exit(f'{TEST} failed against {witness}: see {log_path}')
        if '1 passed; 0 failed' not in log_path.read_text(encoding='utf-8'):
            sys.exit(f'expected 1 passed; 0 failed for {TEST} in {log_path}')

    print(f'Falling-block moving-piston parity passed ({TEST} against {witness})')


if __name__ == '__main__':
    main()
