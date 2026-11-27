#!/usr/bin/env python3
"""Compare Creaking hurt order for an arrow and remembered-player wall damage."""
import argparse
import json
import os
import tempfile
from pathlib import Path

import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness, test_rust_parity as base
from tools.integration_env import game_environment

LIVE_TEST = ('game::server_side::commands::world_cmds::registered_damage_tests::'
             'live_damage_and_blast_creaking_reactions_and_timed_player_credit')
BORROWED_TEST = ('game::server_side::commands::world_cmds::registered_damage_tests::creaking_borrowed::'
                 'borrowed_creaking_hurt_finishes_before_the_tick_continues')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reference-only', action='store_true')
    args = parser.parse_args()
    parent = lab.LOCAL / 'creaking-hit-order-parity'
    parent.mkdir(parents=True, exist_ok=True)
    run = Path(tempfile.mkdtemp(prefix='run-', dir=parent))
    classes = run / 'classes'
    classes.mkdir()
    lock, jars, libraries = base.verify_artifacts()
    cp = os.pathsep.join(map(str, [jars['server'], *libraries]))
    src = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity'
    oracle_harness.run_logged(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-cp', cp,
                               '-d', classes, src / 'VanillaCommandExecOracle.java',
                               src / 'VanillaCreakingHitOrderOracle.java'], run, 'javac')
    commands = [
        '/tick freeze', '/gamerule spawn_mobs false', '/forceload add 0 0',
        '/tp @s 14.5 64 14.5', '/time set night',
        '/setblock 8 63 8 pale_oak_log[axis=y]',
        '/setblock 8 64 8 creaking_heart[axis=y,creaking_heart_state=awake]{creaking:[I;85,0,0,1]}',
        '/setblock 8 65 8 pale_oak_log[axis=y]',
        '/summon creaking 4.5 64 4.5 {UUID:[I;85,0,0,1],home_pos:[I;8,64,8],NoAI:1b,NoGravity:1b}',
        '/summon arrow 5.5 64.8 4.5 {UUID:[I;85,0,0,3],NoGravity:1b,item:{id:"minecraft:arrow",count:1}}',
    ]
    ops = [{'op': 'cmd', 'cmd': cmd, 'record': False} for cmd in commands[:4]]
    ops.append({'op': 'settle', 'required_chunks': [[0, 0]]})
    ops.extend({'op': 'cmd', 'cmd': cmd, 'record': False} for cmd in commands[4:])
    ops.append({'op': 'creaking_hit_order', 'creaking': '00000055-0000-0000-0000-000000000001',
                'arrow': '00000055-0000-0000-0000-000000000003'})
    borrowed = [
        '/setblock 8 63 10 pale_oak_log[axis=y]',
        '/setblock 8 64 10 creaking_heart[axis=y,creaking_heart_state=awake]{creaking:[I;86,0,0,1]}',
        '/setblock 8 65 10 pale_oak_log[axis=y]',
        '/summon creaking 4.5 64 5.5 {UUID:[I;86,0,0,1],home_pos:[I;8,64,10],NoAI:1b,NoGravity:1b}',
    ]
    ops.extend({'op': 'cmd', 'cmd': cmd, 'record': False} for cmd in borrowed)
    ops.append({'op': 'creaking_hit_order', 'creaking': '00000056-0000-0000-0000-000000000001'})
    script = run / 'scene.jsonl'
    script.write_text(''.join(json.dumps(op) + '\n' for op in ops))
    java, rust = run / 'java.json', run / 'rust.json'
    oracle_harness.java_run(run, classes, cp, 'VanillaCommandExecOracle',
                            [run / 'world', script, java, '1'], 'java',
                            jvm=['-Dsunrise.parity.flat=true', '-Dsunrise.loadedFixture=true'])
    arrow_expected, borrowed_expected = map(json.loads, java.read_text().splitlines())
    if [row['animation'] for row in arrow_expected] != [8, 7]:
        raise AssertionError('vanilla did not hit the Creaking and run its next tick')
    if ([(row['remembered'], row['clock'], row['animation'], row['flashes'], row['emitter'])
         for row in borrowed_expected] != [(True, 0, 0, 0, 0), (False, 0, 7, 1, 100), (False, 0, 0, 1, 100)]):
        raise AssertionError('vanilla did not react to the wall on the reference-expiry tick')
    if args.reference_only:
        print('creaking_hit_order: vanilla reference captured')
        return
    env = game_environment(os.environ, run)
    env.update(SUNRISE_TEST_RESOURCES=str(lab.RESOURCES.resolve()), SUNRISE_CREAKING_HIT_ORDER_OUTPUT=str(rust))
    oracle_harness.run_logged([lab.parity_fixtures(), LIVE_TEST,
                               '--ignored', '--exact'], run, 'rust', env=env)
    if json.loads(rust.read_text()) != arrow_expected:
        raise AssertionError('Creaking arrow hit and next tick differ from vanilla')
    borrowed_result = run / 'rust-borrowed.json'
    env.update(SUNRISE_BORROWED_CREAKING_CASE='expiry', SUNRISE_CREAKING_BORROWED_OUTPUT=str(borrowed_result))
    oracle_harness.run_logged([lab.parity_fixtures(), BORROWED_TEST,
                               '--ignored', '--exact'], run, 'rust-borrowed', env=env)
    if json.loads(borrowed_result.read_text()) != borrowed_expected:
        raise AssertionError('Creaking remembered-player wall hurt and resumed tick differ from vanilla')
    print('creaking_hit_order: PASS')


if __name__ == '__main__':
    main()
