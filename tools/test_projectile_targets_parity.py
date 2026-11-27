#!/usr/bin/env python3
"""Record vanilla 26.3 projectile witnesses into .local and run the Rust witness suites.

Compiles and runs the three real-server 26.3 oracles under
mods/sunrise/src/test/java/dev/sunriselab/parity/:
  - VanillaProjectileTargetsOracle  -> witness.json (#119 non-mob pickable targets)
  - VanillaFishingTargetsOracle     -> fishing_witness.json (fishing bobber pickable + items)
  - VanillaProjectilesParityOracle  -> projectiles_120_124_witness.json (#120-#124)

Outputs and a meta.json manifest are written to .local/witness/<version>/projectile_targets/
(gitignored). Unless --record-only is given, builds the sunrise-client lib test binary once
and runs the registered witness tests against that directory via SUNRISE_PROJECTILE_WITNESS_DIR.

  test_projectile_targets_parity.py [--prepare] [--record-only] [--filter SUBSTRING] [--resources DIR]
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

PARITY = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity'
ORACLES = [
    ('VanillaProjectileTargetsOracle', 'witness.json'),
    ('VanillaFishingTargetsOracle', 'fishing_witness.json'),
    ('VanillaProjectilesParityOracle', 'projectiles_120_124_witness.json'),
]
TESTS = [
    'game::creatures::projectile_targets_tests::witness_armor_stand_targets',
    'game::creatures::projectile_targets_tests::witness_painting_and_cushion_targets',
    'game::creatures::projectile_targets_tests::witness_item_frame_targets',
    'game::creatures::projectile_targets_tests::witness_leash_knot_targets',
    'game::creatures::projectile_targets_tests::witness_falling_block_targets',
    'game::server_side::fishing::fishing_targets_tests::witness_fishing_bobber_targets',
    'game::creatures::projectiles_120_124_tests::witness_spectator_player_120',
    'game::creatures::projectiles_120_124_tests::witness_wind_charge_end_crystal_121',
    'game::creatures::projectiles_120_124_tests::witness_trident_enderman_dodge_122',
    'game::creatures::projectiles_120_124_tests::witness_piercing_arrow_multihit_123',
    'game::creatures::projectiles_120_124_tests::witness_firework_rocket_targets_124',
    'game::creatures::projectiles_120_124_tests::witness_review_round1_regressions',
]


def java_version_string():
    result = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument('--record-only', action='store_true')
    parser.add_argument('--filter', default='')
    parser.add_argument('--resources', type=Path, default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    args = parser.parse_args()

    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    version = lock['minecraft']['version']
    java_major = str(lock['minecraft']['java_major'])

    build = lab.LOCAL / 'projectile-targets-parity'
    build.mkdir(parents=True, exist_ok=True)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    witness_dir = lab.LOCAL / 'witness' / version / 'projectile_targets'
    witness_dir.mkdir(parents=True, exist_ok=True)
    deps = os.pathsep.join(map(str, libraries))
    oracle_sources = [PARITY / f'{cls}.java' for cls, _ in ORACLES]

    def run(command, name, cwd=lab.ROOT, env=None):
        path = build / (name + '.log')
        with path.open('w', encoding='utf-8') as log:
            result = subprocess.run(list(map(str, command)), cwd=cwd, env=env, timeout=3600, stdout=log,
                                    stderr=subprocess.STDOUT)
        if result.returncode:
            sys.exit(f'{name} failed: see {path}')
        return path

    compile_cmd = ['javac', '--release', java_major, '-proc:none', '-nowarn',
                   '-cp', str(jars['server']) + os.pathsep + deps, '-d', classes, *oracle_sources]
    run(compile_cmd, 'compile-java')

    commands = {}
    outputs = {}
    classpath = os.pathsep.join([str(classes), str(jars['server']), deps])
    for cls, filename in ORACLES:
        output = witness_dir / filename
        # Passing "-" makes the oracle create and remove its own temp world so parity_shim tracks only the JSON output.
        cmd = ['java', '-Xmx2G', '-cp', classpath, f'dev.sunriselab.parity.{cls}', '-', output]
        run(cmd, f'oracle-{cls}')
        if not output.is_file():
            sys.exit(f'{cls} did not write {output}')
        commands[filename] = [str(part) for part in cmd]
        outputs[filename] = {'sha256': lab.digest(output, 'sha256'), 'bytes': output.stat().st_size}

    input_hashes = {
        'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
        'server.jar': lab.digest(jars['server'], 'sha256'),
    }
    for src in oracle_sources:
        input_hashes[src.relative_to(lab.ROOT).as_posix()] = lab.digest(src, 'sha256')
    lab.write_json(witness_dir / 'meta.json', {
        'game_version': version,
        'java_major': lock['minecraft']['java_major'],
        'java_version': java_version_string(),
        'harness': 'tools/test_projectile_targets_parity.py',
        'witness_env': 'SUNRISE_PROJECTILE_WITNESS_DIR',
        'commands': commands,
        'input_hashes': input_hashes,
        'outputs': outputs,
    })

    if args.record_only:
        print(f'Recorded {len(ORACLES)} projectile witnesses into {witness_dir}')
        return

    chosen = [name for name in TESTS if args.filter in name]
    if not chosen:
        sys.exit(f'no projectile witness tests match --filter {args.filter!r}')

    with tempfile.TemporaryDirectory(prefix='sunrise-projectile-targets-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        binaries = [lab.parity_fixtures()]
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env['SUNRISE_PROJECTILE_WITNESS_DIR'] = str(witness_dir.resolve())
        for idx, test in enumerate(chosen):
            # Run each witness test in a fresh process so process-global registries stay isolated.
            log = run([binaries[0], test, '--exact', '--ignored', '--nocapture'], f'rust-{idx}', cwd=scratch, env=env)
            if '1 passed; 0 failed' not in log.read_text(encoding='utf-8'):
                sys.exit(f'expected 1 passed; 0 failed for {test} in {log}')

    print(f'Projectile targets parity passed ({len(chosen)} tests against {witness_dir})')


if __name__ == '__main__':
    main()
