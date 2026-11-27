#!/usr/bin/env python3
"""Compare authoritative movement, use, interact and spectator packet handling with 26.3.

Vanilla calls the real play listener on a flat GameTestServer with no socket.
Sunrise feeds equivalent packets through the normal Hub inbox and Game dispatch.
The packet clock advances deterministically; poses are compared as IEEE bits.
Recordings stay under .local; the harness prints only case names and verdicts.
"""
import json
import os
from pathlib import Path
import subprocess
import tempfile

import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness, test_rust_parity as base


NAME = 'server-player-packets-parity'
ORACLE = 'VanillaServerPlayerPacketsOracle'
TEST = 'game::server_side::lan::player_packet_tests::server_player_packet_witness'
CASES = {
    '141': ['credits_pose', 'credits_move_count', 'credits_idle_credit', 'ordinary_idle_credit',
            'repeated_position', 'invalid_rotation'],
    '142': ['empty', 'disabled', 'survival', 'spectator'],
    '143': ['missing_true', 'missing_false', 'boat_true', 'boat_false', 'next_input'],
    '144': ['stand_overworld', 'level_overworld', 'stand_the_nether', 'level_the_nether', 'player', 'missing',
            'arrow', 'upward_frame', 'same_level_false', 'same_level_true', 'mixed_return', 'mixed_stay'],
    '145': [f'{route}_{tick}' for route in ('vehicle', 'player', 'loading_vehicle', 'loading_player')
            for tick in (19, 20, 21, 22)] + ['vehicle_then_accept', 'status', 'position'],
    '146': ['loaded_crystal', 'reloaded_crystal'],
}


def main():
    build = lab.LOCAL / NAME
    classes = build / 'classes'
    classes.mkdir(parents=True, exist_ok=True)
    vanilla, rust = build / 'vanilla.json', build / 'rust.json'
    for output in (vanilla, rust, build / 'report.json'):
        output.unlink(missing_ok=True)
    resources = Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)).resolve(strict=True)
    lock, jars, libraries = base.verify_artifacts()
    source = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity' / (ORACLE + '.java')
    sources = [Path(__file__), source, lab.ROOT / 'Cargo.lock',
               lab.ROOT / 'crates/sunrise-client/src/cut_layout/game/network/lan/player_packet_tests.rs']
    hashes = base.source_hashes(sources)
    cp = os.pathsep.join(map(str, (jars['server'], *libraries)))
    oracle_harness.run_logged(['javac', '--release', lock['minecraft']['java_major'], '-proc:none',
                               '-cp', cp, '-d', classes, source], build, 'compile-java')
    with tempfile.TemporaryDirectory(dir=build) as scratch:
        env = lab.game_env(SUNRISE_TEST_RESOURCES=resources, SUNRISE_PLAYER_PACKET_REPORT=rust)
        with (build / 'vanilla.log').open('w', encoding='utf-8') as log:
            subprocess.run(['java', '-Xmx2g', '-Djava.awt.headless=true', '-Duser.home=' + scratch,
                            '-cp', str(classes) + os.pathsep + cp, 'dev.sunriselab.parity.' + ORACLE,
                            scratch, vanilla], cwd=scratch, env=env, stdout=log,
                           stderr=subprocess.STDOUT, timeout=180, check=True)
        oracle_harness.run_logged([lab.parity_fixtures(),
                                   TEST, '--exact', '--ignored', '--nocapture'],
                                  build, 'rust', env=env)
    expected, actual = (json.loads(path.read_text(encoding='utf-8')) for path in (vanilla, rust))
    names = {f'{group}/{name}' for group, cases in CASES.items() for name in cases}
    if names != expected.keys() or names != actual.keys():
        raise AssertionError('Incomplete server-player packet observations')
    failed = sorted(name for name in names if actual[name] != expected[name])
    base.ensure_sources_unchanged(sources, hashes)
    lab.write_json(build / 'report.json', dict(
        minecraft=lab.VERSION, server_jar_sha256=lab.digest(jars['server'], 'sha256'),
        sources_sha256=hashes, failures=failed, observations=len(names),
        head=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=lab.ROOT, text=True).strip()))
    if failed:
        raise AssertionError(f'{NAME}: FAIL ({", ".join(failed)})')
    print(f'{NAME}: PASS against Minecraft {lab.VERSION}')


if __name__ == '__main__':
    main()
