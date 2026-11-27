#!/usr/bin/env python3
"""Record the vanilla 26.3 fangs/ominous-spawner ticking-gate witness into .local and run its Rust tests.

Runs VanillaCommandExecOracle (via tools/test_command_exec_parity.py --record-fangs-ticking-witness)
to record fangs_ominous_spawner_ticking_gate.json and meta.json into
.local/witness/<version>/fangs_ticking_gate/ (gitignored). Unless --record-only / --record is given,
runs the two sunrise-client witness tests through lab.parity_fixtures() with
SUNRISE_FANGS_TICKING_GATE_WITNESS pointing at the recorded file.

  test_fangs_ticking_gate_parity.py [--prepare] [--record-only|--record] [--filter SUBSTRING] [--resources DIR]
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

TESTS = [
    'game::server_side::fangs::ticking_tests::fangs_and_an_ominous_item_spawner_outside_the_ticking_chunks_do_not_tick',
    'game::server_side::fangs::ticking_tests::fangs_and_an_ominous_item_spawner_pause_on_scheduled_ticket_removal_and_resume_26_3',
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument('--record-only', '--record', dest='record_only', action='store_true',
                        help='record the vanilla witness and meta.json into .local without running Rust tests')
    parser.add_argument('--filter', default='')
    parser.add_argument('--resources', type=Path, default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    args = parser.parse_args()

    build = lab.LOCAL / 'fangs-ticking-gate-parity'
    build.mkdir(parents=True, exist_ok=True)
    witness_dir = lab.LOCAL / 'witness' / lab.VERSION / 'fangs_ticking_gate'
    witness = witness_dir / 'fangs_ominous_spawner_ticking_gate.json'

    record_cmd = [
        sys.executable,
        str(lab.ROOT / 'tools/test_command_exec_parity.py'),
        '--record-fangs-ticking-witness', str(witness),
        '--work-dir', str(build),
    ]
    if args.prepare:
        record_cmd.append('--prepare')
    recorded = subprocess.run(record_cmd, cwd=lab.ROOT)
    if recorded.returncode != 0:
        sys.exit(recorded.returncode)
    if not witness.is_file():
        sys.exit(f'recorder did not write {witness}')
    if not (witness_dir / 'meta.json').is_file():
        sys.exit(f'recorder did not write {witness_dir / "meta.json"}')

    if args.record_only:
        return

    chosen = [name for name in TESTS if args.filter in name]
    if not chosen:
        sys.exit(f'no fangs ticking-gate witness tests match --filter {args.filter!r}')

    def run(command, name, cwd, env):
        path = build / f'{name}.log'
        with path.open('w', encoding='utf-8') as log:
            result = subprocess.run(list(map(str, command)), cwd=cwd, env=env, timeout=3600,
                                    stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            tail = path.read_text(encoding='utf-8', errors='replace').splitlines()[-40:]
            print('\n'.join(tail), file=sys.stderr, flush=True)
            sys.exit(f'{name} failed: see {path}')
        return path

    with tempfile.TemporaryDirectory(prefix='sunrise-fangs-ticking-gate-') as tmp:
        scratch = Path(tmp)
        binary = lab.parity_fixtures()
        for idx, test in enumerate(chosen):
            test_dir = scratch / f'test-{idx}'
            test_dir.mkdir()
            homes = {k: test_dir / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
            env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
            env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
            env['SUNRISE_FANGS_TICKING_GATE_WITNESS'] = str(witness.resolve())
            # Run each witness test in a fresh process so process-global registries stay isolated.
            log = run([binary, test, '--exact', '--ignored', '--nocapture'], f'rust-{idx}', cwd=test_dir, env=env)
            if '1 passed; 0 failed' not in log.read_text(encoding='utf-8'):
                sys.exit(f'expected 1 passed; 0 failed for {test} in {log}')

    print(f'Fangs ticking-gate parity passed ({len(chosen)} tests against {witness})')


if __name__ == '__main__':
    main()
