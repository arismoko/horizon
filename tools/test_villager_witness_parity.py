#!/usr/bin/env python3
"""Compare cached villager witnesses, hurt reputation and offers without starting a server."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, test_rust_parity as base


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game', type=Path, default=lab.GAME)
    parser.add_argument('--resources', type=Path, default=lab.RESOURCES)
    parser.add_argument('--java-only', action='store_true')
    args = parser.parse_args(argv)
    lab.GAME = args.game
    lab.config = lambda: (json.loads(lab.LOCK.read_text(encoding='utf-8')), args.game)
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'villager-witness-parity'
    classes = build / 'classes'
    classes.mkdir(parents=True, exist_ok=True)
    oracle = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaVillagerWitnessOracle.java'
    sources = [Path(__file__), oracle,
               lab.ROOT / 'crates/sunrise-sim/src/mobs/villager.rs',
               lab.ROOT / 'crates/sunrise-sim/src/mobs/villager_brain.rs',
               lab.ROOT / 'crates/sunrise-sim/src/mobs/villager_brain/memories.rs',
               lab.ROOT / 'crates/sunrise-sim/src/mobs/villager_brain/schedules.rs',
               lab.ROOT / 'crates/sunrise-sim/src/mobs/villager_brain/tasks.rs',
               lab.ROOT / 'crates/sunrise-sim/src/mobs/villager_brain/reputation.rs',
               lab.ROOT / 'crates/sunrise-sim/src/mobs/villager_brain/reputation_witness_tests.rs',
               lab.ROOT / 'crates/sunrise-gameplay/src/game/mobs/villagers.rs',
               lab.ROOT / 'crates/sunrise-client/src/cut_layout/game/mobs/tests/villager_witness_tests.rs',
               lab.ROOT / 'crates/sunrise-gameplay/src/game/network/lan/mod.rs',
               lab.ROOT / 'crates/sunrise-client/src/cut_layout/game/entities/kill_die_vibration_tests.rs']
    hashes = base.source_hashes(sources)
    clean = dict(os.environ)
    for key in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME'):
        clean.pop(key, None)
    env = lab.game_env(clean, SUNRISE_TEST_RESOURCES=args.resources.resolve(), PYTHONNOUSERSITE=1)
    scratch = Path(lab._scratch_dir('villager-reputation-cwd-'))
    cp = os.pathsep.join(map(str, [jars['server'], *libraries]))
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                    '-cp', cp, '-d', str(classes), str(oracle)], cwd=scratch, env=env, check=True)
    expected = build / 'vanilla.txt'
    subprocess.run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + cp,
                    'dev.sunriselab.parity.VanillaVillagerWitnessOracle', str(expected)],
                   cwd=scratch, env=env, check=True)
    rows = expected.read_text(encoding='utf-8').splitlines()
    if [line.split()[0] for line in rows] != ['witnesses', 'cached', 'fresh', 'hurt', 'hurt_order', 'hurt_order_buyer']:
        raise AssertionError('Missing/extra actual Java records')
    if args.java_only:
        base.ensure_sources_unchanged(sources, hashes)
        print('\n'.join(rows))
        return
    env['SUNRISE_REPUTATION_ORACLE'] = str(expected.resolve())
    artifacts = {}
    executed = {}
    for package, group, count in [
        ('sunrise-client-lib', 'mobs::villager_brain::reputation_witness_tests', 1),
        ('sunrise-client', 'game::server_side::villagers::witness_tests', 4),
        ('sunrise-client', 'game::server_side::lan::merchant_packet_tests', 1),
    ]:
        if package not in artifacts:
            source = (lab.ROOT / 'crates/sunrise-client/src' / ('client_lib.rs' if package == 'sunrise-client-lib' else 'lib.rs')).resolve()
            built = subprocess.run(['cargo', 'test', '--locked', '-p', package, '--lib', '--no-run', '--message-format=json'],
                                   cwd=lab.ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, encoding='utf-8')
            (build / (package + '-build.jsonl')).write_text(built.stdout, encoding='utf-8')
            found = []
            for line in built.stdout.splitlines():
                try: value = json.loads(line)
                except json.JSONDecodeError: continue
                if value.get('reason') == 'compiler-artifact' and value.get('executable') and value.get('profile', {}).get('test') and value.get('target', {}).get('kind') == ['lib'] and Path(value['target']['src_path']).resolve() == source:
                    found.append(value['executable'])
            if built.returncode or len(set(found)) != 1:
                raise RuntimeError(f'{package} --lib artifact failure; see build log')
            artifacts[package] = found[0]
        artifact = artifacts[package]
        listed = subprocess.run([artifact, group, '--list'], cwd=scratch, env=env, capture_output=True, text=True, check=True).stdout
        if len([line for line in listed.splitlines() if line.startswith(group + '::') and line.endswith(': test')]) != count:
            raise AssertionError(f'{group}: wrong listed test count')
        ran = subprocess.run([artifact, group, '--include-ignored', '--nocapture'], cwd=scratch,
                             env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, encoding='utf-8')
        (build / (group.replace('::', '-') + '.log')).write_text(ran.stdout, encoding='utf-8')
        print(ran.stdout)
        # Fresh-process fixtures print their own summaries before the outer suite does.
        summaries = re.findall(r'^test result: ok\. (\d+) passed; 0 failed;', ran.stdout, re.MULTILINE)
        if ran.returncode or not summaries or int(summaries[-1]) != count:
            raise AssertionError(f'{group}: execution did not pass all {count} tests')
        executed[group] = count
    base.ensure_sources_unchanged(sources, hashes)
    lab.write_json(build / 'report.json', {'minecraft': lock['minecraft']['version'],
        'sources_sha256': hashes, 'executed_tests': executed, 'java_records': len(rows), 'scope': 'Lazy cached witness visibility and same-tick hurt reputation, offer and entity-event order; no server/world loop.'})
    print('Villager witnesses, hurt reputation and offer packet order match vanilla')


if __name__ == '__main__':
    main()
