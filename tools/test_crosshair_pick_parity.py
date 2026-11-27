#!/usr/bin/env python3
"""Compare crosshair targeting and hit results against vanilla 26.3.

Compiles and runs VanillaCrosshairPickOracle against the pinned 26.3 client jar on JDK 25,
writing witness.json and meta.json into .local/witness/<version>/crosshair_pick/, then runs
each sunrise-client crosshair parity test in its own fresh process against it via
SUNRISE_CROSSHAIR_PICK_WITNESS."""
import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness as harness, test_rust_parity as base, test_model_baking_parity as baking

TESTS = [
    'game::client_side::crosshair_pick_tests::primed_tnt_and_falling_blocks_match_26_3_witness',
    'game::client_side::crosshair_pick_tests::nested_ride_root_vehicle_exclusion_matches_26_3_witness',
    'game::client_side::crosshair_pick_tests::review_parity_cases_match_26_3_witness',
    'game::client_side::crosshair_pick_tests::singleplayer_and_unstored_pick_share_unified_query_and_root_exclusion_26_3',
    'game::client_side::crosshair_pick_tests::middle_click_sends_pick_item_from_entity_for_tnt_and_falling_block',
    'game::client_side::crosshair_pick_tests::dynamic_redirectable_tag_controls_arrow_pick_and_in_ground_stops_it',
]


def record(lock, jars, libraries, build, witness_dir=None):
    version = lock['minecraft']['version']
    if witness_dir is None:
        witness_dir = lab.LOCAL / 'witness' / version / 'crosshair_pick'
    witness_dir = Path(witness_dir).resolve()
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness = witness_dir / 'witness.json'
    classes, dependencies, oracle_path = harness.compile_oracle(
        build, 'VanillaCrosshairPickOracle', jars, libraries, lock['minecraft']['java_major']
    )
    harness.java_run(build, classes, dependencies, 'VanillaCrosshairPickOracle', [witness], 'client')
    java_ver = subprocess.check_output(['java', '--version'], text=True).splitlines()[0].strip()
    lab.write_json(witness_dir / 'meta.json', dict(
        minecraft=version,
        client_jar_sha256=lab.digest(jars['client'], 'sha256'),
        java_version=java_ver,
        command='python3 tools/test_crosshair_pick_parity.py',
        oracle='mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCrosshairPickOracle.java',
        inputs_sha256={
            'oracle': lab.digest(oracle_path, 'sha256'),
        },
    ))
    return witness


def prepare_witness(work, env_file=None):
    """Record the vanilla 26.3 crosshair witness into work and optionally export it to env_file."""
    work = Path(work).resolve()
    work.mkdir(parents=True, exist_ok=True)
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = work / 'build'
    build.mkdir(parents=True, exist_ok=True)
    witness = record(lock, jars, libraries, build, witness_dir=work)
    if env_file is not None:
        with Path(env_file).open('a', encoding='utf-8') as out:
            out.write(f'SUNRISE_CROSSHAIR_PICK_WITNESS={witness}\n')
    return witness


def run_logged(command, build, name, cwd=lab.ROOT, env=None):
    path = build / f'{name}.log'
    with path.open('w', encoding='utf-8') as log:
        result = subprocess.run([str(c) for c in command], cwd=cwd, env=env, timeout=3600,
                                stdout=log, stderr=subprocess.STDOUT)
    if result.returncode:
        tail = path.read_text(encoding='utf-8', errors='replace').splitlines()[-40:]
        print('\n'.join(tail), file=sys.stderr, flush=True)
        sys.exit(f'{name} failed: see {path}')
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--resources', type=Path, default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    parser.add_argument('--record-only', action='store_true')
    parser.add_argument('--prepare-witness', type=Path, default=None)
    parser.add_argument('--env-file', type=Path, default=None)
    args = parser.parse_args()
    if args.prepare_witness is not None:
        witness = prepare_witness(args.prepare_witness, args.env_file)
        print(f'prepared {witness} ({witness.stat().st_size} bytes)')
        return
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'crosshair-pick-parity'
    build.mkdir(parents=True, exist_ok=True)
    witness = record(lock, jars, libraries, build)
    if args.record_only:
        print(f'wrote {witness} ({witness.stat().st_size} bytes)')
        return
    binary = lab.parity_fixtures()
    for index, test in enumerate(TESTS):
        with tempfile.TemporaryDirectory(prefix=f'sunrise-crosshair-pick-{index}-') as tmp:
            scratch = Path(tmp)
            homes = {
                k: scratch / k
                for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')
            }
            env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
            env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
            env['SUNRISE_CROSSHAIR_PICK_WITNESS'] = str(witness.resolve())
            log = run_logged([binary, '--ignored', '--exact', test], build, f'game-{index}', cwd=scratch, env=env)
            expected = '1 passed; 0 failed'
            assert expected in log.read_text(encoding='utf-8'), f'expected "{expected}" in {log}'
    print(f'Crosshair pick parity passed ({len(TESTS)} isolated tests against {witness})')


if __name__ == '__main__':
    main()
