#!/usr/bin/env python3
"""Compare the rotations four turning paths write with pinned vanilla 26.3, as raw float bits.

Java (VanillaTurnOracle) runs the real Fox$FoxPounceGoal.tick and DolphinJumpGoal.tick from a set
of motions, pitches and (for the fox) its jumping flag; DragonSittingScanningPhase.doServerTick
with its scan's player at a set of places, from a set of yaws, yRotA values and head positions;
and Entity.lookAt(FEET) to a set of offsets, including straight up and down and the eye itself.
Recorded witnesses and meta.json are written into .local/witness/<version>/mob_turn/ (gitignored).
Sunrise runs the production fox Pounce and dolphin Jump goal ticks (sunrise-client-lib), and the
dragon's scanning turn and the client's look-at rotation (sunrise-parity-fixtures) against that
directory through SUNRISE_MOB_TURN_WITNESS_DIR. Every case is a fresh observation on both sides,
and each of the four outputs must be complete.
"""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness as harness, test_rust_parity as base, integration_env

OUTPUTS = {'fox.txt': 13 * 3 * 2, 'dolphin.txt': 13 * 3, 'dragon.txt': 7 * 5 * 3 * 2, 'look_at.txt': 36}
WITNESS_ENV = 'SUNRISE_MOB_TURN_WITNESS_DIR'
LIB_TESTS = [
    'mobs::fox::pounce_turn_tests::the_pounce_turn_matches_the_pinned_oracle',
    'mobs::dolphin::jump_turn_tests::the_jump_turn_matches_the_pinned_oracle',
]
CLIENT_TESTS = [
    'game::server_side::dragon::tests::the_scanning_turn_matches_the_pinned_oracle',
    'game::remote::tests::client_read_tests::look_at_matches_the_pinned_oracle',
]


def java_version_string():
    result = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument('--resources', type=Path,
                        default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    parser.add_argument('--record', '--record-only', dest='record', action='store_true',
                        help='record the witness files and meta.json into .local without running Rust')
    args = parser.parse_args()
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    version = lock['minecraft']['version']
    build = lab.LOCAL / 'rust-mob-turn-parity'
    build.mkdir(parents=True, exist_ok=True)
    server, rust = build / 'server', build / 'rust'
    server.mkdir(exist_ok=True)
    rust.mkdir(exist_ok=True)
    for name in OUTPUTS:
        (rust / name).unlink(missing_ok=True)
    java_root = lab.ROOT / 'mods/sunrise/src/test/java'
    oracle = java_root / 'dev/sunriselab/parity/VanillaTurnOracle.java'
    parity_data = java_root / 'dev/sunriselab/parity/ParityData.java'
    block_update_oracle = java_root / 'dev/sunriselab/parity/VanillaBlockUpdateOracle.java'
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    harness.run_logged(['javac', '--release', lock['minecraft']['java_major'], '-proc:none',
                        '-sourcepath', java_root, '-cp', dependencies, '-d', classes, oracle],
                       build, 'compile-java')
    oracle_cmd = ['java', '-Xmx1G', '-Djava.awt.headless=true', '-cp',
                  str(classes) + os.pathsep + dependencies,
                  'dev.sunriselab.parity.VanillaTurnOracle', server]
    harness.run_logged(oracle_cmd, build, 'server')

    witness_dir = lab.LOCAL / 'witness' / version / 'mob_turn'
    witness_dir.mkdir(parents=True, exist_ok=True)
    outputs_meta = {}
    for name, lines in OUTPUTS.items():
        actual = (server / name).read_text(encoding='utf-8').splitlines()
        if len(actual) != lines:
            raise SystemExit(f'{name}: expected {lines} lines, got {len(actual)}')
        dest = witness_dir / name
        shutil.copyfile(server / name, dest)
        outputs_meta[name] = {
            'sha256': lab.digest(dest, 'sha256'),
            'lines': len(actual),
            'bytes': dest.stat().st_size,
        }
    input_hashes = {
        'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
        'server.jar': lab.digest(jars['server'], 'sha256'),
        oracle.relative_to(lab.ROOT).as_posix(): lab.digest(oracle, 'sha256'),
        parity_data.relative_to(lab.ROOT).as_posix(): lab.digest(parity_data, 'sha256'),
        block_update_oracle.relative_to(lab.ROOT).as_posix(): lab.digest(block_update_oracle, 'sha256'),
    }
    lab.write_json(witness_dir / 'meta.json', {
        'game_version': version,
        'java_major': lock['minecraft']['java_major'],
        'java_version': java_version_string(),
        'harness': 'tools/test_mob_turn_parity.py',
        'witness_env': WITNESS_ENV,
        'command': [str(part) for part in oracle_cmd],
        'input_hashes': input_hashes,
        'outputs': outputs_meta,
    })
    if args.record:
        print(f'recorded {sum(OUTPUTS.values())} {version} turn witness lines in {witness_dir}', flush=True)
        return

    resources = args.resources.resolve()
    if not (resources / 'entity-types.json').is_file():
        raise SystemExit('prepared resources required; pass --resources PATH')
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    sources = [Path(__file__), oracle, lab.ROOT / 'tools/lab.py',
               lab.ROOT / 'tools/oracle_harness.py', lab.ROOT / 'tools/test_rust_parity.py',
               lab.ROOT / 'tools/integration_env.py', lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock',
               lab.ROOT / 'crates/sunrise-core/src/geometry/trig.rs',
               lab.ROOT / 'crates/sunrise-core/src/jmath.rs',
               lab.ROOT / 'crates/sunrise-sim/src/mobs/fox.rs',
               lab.ROOT / 'crates/sunrise-sim/src/mobs/dolphin.rs',
               lab.ROOT / 'crates/sunrise-gameplay/src/game/mobs/dragon.rs',
               lab.ROOT / 'crates/sunrise-gameplay-common/src/game/network/remote/mod.rs',
               lab.ROOT / 'crates/sunrise-client/src/cut_layout/game/network/remote/client_read_tests.rs',
               parity_data, block_update_oracle]
    hashes = base.source_hashes(sources)
    with tempfile.TemporaryDirectory(prefix='sunrise-mob-turn-') as scratch:
        env = integration_env.game_environment(os.environ, scratch)
        env.update(
            SUNRISE_TEST_RESOURCES=str(resources),
            SUNRISE_TURN_OUTPUT_DIR=str(rust),
            **{WITNESS_ENV: str(witness_dir.resolve())},
        )
        env.setdefault('CARGO_BUILD_JOBS', '1')
        env.setdefault('RUST_TEST_THREADS', '2')
        harness.run_logged(['cargo', 'test', '--locked', '-p', 'sunrise-client-lib', '--lib',
                            '--', '--ignored', '--exact', *LIB_TESTS], build, 'rust-lib', env=env)
        lib_log = (build / 'rust-lib.log').read_text(encoding='utf-8')
        if f'{len(LIB_TESTS)} passed; 0 failed' not in lib_log:
            raise SystemExit(f'expected {len(LIB_TESTS)} passed; 0 failed in {build / "rust-lib.log"}')
        harness.run_logged([lab.parity_fixtures(), '--ignored', '--exact', *CLIENT_TESTS],
                           build, 'rust-game', env=env)
        game_log = (build / 'rust-game.log').read_text(encoding='utf-8')
        if f'{len(CLIENT_TESTS)} passed; 0 failed' not in game_log:
            raise SystemExit(f'expected {len(CLIENT_TESTS)} passed; 0 failed in {build / "rust-game.log"}')
    matched = {}
    for name, lines in OUTPUTS.items():
        matched[name] = harness.compare(witness_dir / name, rust / name, lines)
    base.ensure_sources_unchanged(sources, hashes)
    lab.write_json(report, dict(schema=1, minecraft=version,
        sources_sha256=hashes, server_jar_sha256=lab.digest(jars['server'], 'sha256'),
        matched_output_lines=matched, scope=__doc__))
    print(f'{sum(matched.values())} turn cases match the pinned server: {report}', flush=True)


if __name__ == '__main__':
    main()
