#!/usr/bin/env python3
"""Compare a sulfur cube's server load/save/tick/sync and client MAX_FUSE sync/tick with pinned vanilla 26.3.

Server mode: Java loads each case on a detached ServerLevel through Entity.load (`fuse` absent, at
the signed boundaries, as every numeric tag type and as a string; `Invulnerable` absent, false and
true), saves it, reads the MAX_FUSE a late tracker's snapshot carries, runs tickFuse three times and
reads what packDirty sends to trackers already following it, then what a later load of `fuse:-1`
sends them. Sunrise runs the production load, save, `tick_fuse` and `mob_meta`, with `send_dirty`'s
rule for what goes to existing trackers.

Client mode: Java creates a sulfur cube on a detached ClientLevel, drives `SynchedEntityData.assignValues`
(invoking `SulfurCube.onSyncedDataUpdated` for `MAX_FUSE` and `FROM_BUCKET`) and client `tickFuse`
across signed boundaries (-2147483648, -5, -1, 0, 1, 40, 2147483647), re-syncing `MAX_FUSE` after
ticking, and syncing `FROM_BUCKET`. Sunrise runs the production `client_defaults`,
`remote_mob_data::apply` and `sulfur_cube::client_tick`.

Records the vanilla witnesses and `meta.json` into `.local/witness/<version>/sulfur_cube_fuse/`
(gitignored) through `java`/`javac` (cached by `tools/parity_shim.py` under `tools/parity.py`), then
runs the Rust tests against that folder via `SUNRISE_SULFUR_CUBE_FUSE_WITNESS_DIR`. Pass `--record`
to run only the pinned vanilla Java oracle and write the local witness files.
"""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import (
    lab,
    oracle_harness as harness,
    test_model_baking_parity as baking,
    test_rust_parity as base,
    integration_env,
)

SERVER_LINES = 1 + 14 * 3
CLIENT_LINES = 1 + 24
SERVER_WITNESS_NAME = 'sulfur_cube_fuse_server.txt'
CLIENT_WITNESS_NAME = 'sulfur_cube_fuse_client.txt'
WITNESS_ENV = 'SUNRISE_SULFUR_CUBE_FUSE_WITNESS_DIR'
CLIENT_TESTS = [
    'game::server_side::lan::sources_tests::a_loaded_sulfur_cube_fuse_matches_the_pinned_oracle',
    'game::client_side::remote_mob_data::tests::sulfur_cube_max_fuse_sets_the_client_fuse',
]
LIB_TEST = 'mobs::sulfur_cube::tnt_rule_tests::a_loaded_fuse_is_kept_as_it_is_and_leaves_invulnerable_alone'


def java_version_string():
    result = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def run_oracle(build, lock, jars, libraries, server_out, client_out):
    java_root = lab.ROOT / 'mods/sunrise/src/test/java'
    oracle = java_root / 'dev/sunriselab/parity/VanillaSulfurCubeFuseOracle.java'
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))
    harness.run_logged(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                        '-sourcepath', java_root, '-cp', dependencies, '-d', classes, oracle],
                       build, 'compile-java')
    java_cmd = ['java', '-Xmx1G', '-Djava.awt.headless=true', '-cp',
                str(classes) + os.pathsep + dependencies,
                'dev.sunriselab.parity.VanillaSulfurCubeFuseOracle', server_out, client_out]
    harness.run_logged(java_cmd, build, 'vanilla')
    return oracle, java_cmd


def record_witness(build, lock, jars, libraries):
    version = lock['minecraft']['version']
    witness_dir = lab.LOCAL / 'witness' / version / 'sulfur_cube_fuse'
    witness_dir.mkdir(parents=True, exist_ok=True)
    server_out = witness_dir / SERVER_WITNESS_NAME
    client_out = witness_dir / CLIENT_WITNESS_NAME
    oracle, java_cmd = run_oracle(build, lock, jars, libraries, server_out, client_out)
    server_lines = len(server_out.read_text(encoding='utf-8').splitlines())
    client_lines = len(client_out.read_text(encoding='utf-8').splitlines())
    if server_lines != SERVER_LINES or client_lines != CLIENT_LINES:
        raise AssertionError(
            f'Unexpected witness line counts: server={server_lines} (expected {SERVER_LINES}), '
            f'client={client_lines} (expected {CLIENT_LINES})'
        )
    lab.write_json(witness_dir / 'meta.json', {
        'game_version': version,
        'java_major': lock['minecraft']['java_major'],
        'java_version': java_version_string(),
        'harness': 'tools/test_sulfur_cube_fuse_parity.py',
        'witness_env': WITNESS_ENV,
        'command': [str(part) for part in java_cmd],
        'input_hashes': {
            'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
            'client.jar': lab.digest(jars['client'], 'sha256'),
            oracle.relative_to(lab.ROOT).as_posix(): lab.digest(oracle, 'sha256'),
        },
        'outputs': {
            SERVER_WITNESS_NAME: {'sha256': lab.digest(server_out, 'sha256'), 'bytes': server_out.stat().st_size},
            CLIENT_WITNESS_NAME: {'sha256': lab.digest(client_out, 'sha256'), 'bytes': client_out.stat().st_size},
        },
    })
    return witness_dir, server_out, client_out, server_lines, client_lines


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--resources', type=Path,
                        default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    parser.add_argument('--record', '--record-only', action='store_true',
                        help='Run only the pinned vanilla Java oracle and write local witnesses under .local/witness/')
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'rust-sulfur-cube-fuse-parity'
    build.mkdir(parents=True, exist_ok=True)
    if args.record:
        witness_dir, _, _, server_lines, client_lines = record_witness(build, lock, jars, libraries)
        print(f'Recorded {server_lines} server and {client_lines} client witness lines for {lock["minecraft"]["version"]} into {witness_dir}', flush=True)
        return

    resources = args.resources.resolve()
    if not (resources / 'entity-types.json').is_file():
        raise SystemExit('prepared resources required; pass --resources PATH')
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    for name in ['rust_server.txt', 'rust_client.txt']:
        (build / name).unlink(missing_ok=True)
    oracle = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaSulfurCubeFuseOracle.java'
    sources = [Path(__file__), oracle, lab.ROOT / 'tools/lab.py',
               lab.ROOT / 'tools/oracle_harness.py', lab.ROOT / 'tools/test_rust_parity.py',
               lab.ROOT / 'tools/test_model_baking_parity.py',
               lab.ROOT / 'tools/integration_env.py', lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock',
               lab.ROOT / 'crates/sunrise-sim/src/mobs.rs',
               lab.ROOT / 'crates/sunrise-sim/src/entity_nbt.rs',
               lab.ROOT / 'crates/sunrise-sim/src/entity_nbt/common.rs',
               lab.ROOT / 'crates/sunrise-sim/src/entity_nbt/mob_read.rs',
               lab.ROOT / 'crates/sunrise-sim/src/entity_nbt/mob_write.rs',
               lab.ROOT / 'crates/sunrise-sim/src/mobs/sulfur_cube.rs',
               lab.ROOT / 'crates/sunrise-gameplay/src/game/network/lan/mod.rs',
               lab.ROOT / 'crates/sunrise-gameplay/src/game/network/lan/sulfur_cube_meta_tests.rs',
               lab.ROOT / 'crates/sunrise-client-gameplay/src/game/network/remote_mob_data.rs',
               lab.ROOT / 'crates/sunrise-world/src/nbt_path.rs']
    hashes = base.source_hashes(sources)
    witness_dir, server_out, client_out, _, _ = record_witness(build, lock, jars, libraries)
    with tempfile.TemporaryDirectory(prefix='sunrise-sulfur-cube-fuse-') as scratch:
        env = integration_env.game_environment(os.environ, scratch)
        env.update(SUNRISE_TEST_RESOURCES=str(resources),
                   SUNRISE_SULFUR_CUBE_FUSE_WITNESS_DIR=str(witness_dir.resolve()),
                   SUNRISE_SULFUR_CUBE_FUSE_OUTPUT=str(build / 'rust_server.txt'),
                   SUNRISE_SULFUR_CUBE_CLIENT_FUSE_OUTPUT=str(build / 'rust_client.txt'))
        env.setdefault('CARGO_BUILD_JOBS', '1')
        env.setdefault('RUST_TEST_THREADS', '2')
        harness.run_logged([lab.parity_fixtures(), *CLIENT_TESTS, '--exact', '--ignored'],
                           build, 'rust', env=env)
        client_log = (build / 'rust.log').read_text(encoding='utf-8')
        expected_client = f'{len(CLIENT_TESTS)} passed; 0 failed'
        if expected_client not in client_log:
            raise AssertionError(f'Expected "{expected_client}" in {build / "rust.log"}')
        harness.run_logged(['cargo', 'test', '--locked', '-p', 'sunrise-client-lib', '--lib', '--',
                            '--ignored', '--exact', LIB_TEST],
                           build, 'rust-lib', env=env)
        lib_log = (build / 'rust-lib.log').read_text(encoding='utf-8')
        if '1 passed; 0 failed' not in lib_log:
            raise AssertionError(f'Expected "1 passed; 0 failed" for {LIB_TEST} in {build / "rust-lib.log"}')
    matched_server = harness.compare(server_out, build / 'rust_server.txt', SERVER_LINES)
    matched_client = harness.compare(client_out, build / 'rust_client.txt', CLIENT_LINES)
    base.ensure_sources_unchanged(sources, hashes)
    lab.write_json(report, dict(schema=1, minecraft=lock['minecraft']['version'],
        sources_sha256=hashes, client_jar_sha256=lab.digest(jars['client'], 'sha256'),
        witness_dir=witness_dir.relative_to(lab.ROOT).as_posix(),
        matched_server_lines=matched_server, matched_client_lines=matched_client,
        matched_output_lines=matched_server + matched_client,
        server_output_sha256=lab.digest(build / 'rust_server.txt', 'sha256'),
        client_output_sha256=lab.digest(build / 'rust_client.txt', 'sha256'),
        scope=__doc__))
    print(f'{matched_server} server and {matched_client} client sulfur cube fuse lines match pinned {lock["minecraft"]["version"]}: {report}', flush=True)


if __name__ == '__main__':
    main()
