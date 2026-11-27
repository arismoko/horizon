#!/usr/bin/env python3
"""Compare production gateway state counters with actual vanilla methods.

No server/world bootstrap; full gameplay ticks, portal entry and arrival/destination
resolution are not covered.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, test_rust_parity as base

TEST = 'game::server_side::end::cooldown_tests::gateway_counters_match_actual_vanilla_methods'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--work-dir', type=Path)
    parser.add_argument('--resources', type=Path, default=lab.RESOURCES)
    parser.add_argument('--reference-only', action='store_true')
    args = parser.parse_args()
    build = (args.work_dir or lab.LOCAL / 'rust-gateway-cooldown-parity').resolve()
    build.mkdir(parents=True, exist_ok=True)
    lock, jars, libraries = base.verify_artifacts()
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    source = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaGatewayCooldownOracle.java'
    reference = build / 'reference.json'
    dependency_path = os.pathsep.join(map(str, [jars['server'], *libraries]))
    def run(command, name, env=None):
        with (build / (name + '.log')).open('w', encoding='utf-8') as log:
            subprocess.run(list(map(str, command)), cwd=lab.ROOT if name == 'build-native' else build, env=env,
                           stdout=log, stderr=subprocess.STDOUT, check=True)
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none',
         '-cp', dependency_path, '-d', classes, source], 'compile-java')
    run(['java', '-Xmx512M', '-cp', str(classes) + os.pathsep + dependency_path,
         'dev.sunriselab.parity.VanillaGatewayCooldownOracle', reference], 'java')
    observed = json.loads(reference.read_text())
    if len(observed['contact']) != 5 or any(len(row) != 4 for row in observed['contact']):
        raise SystemExit('actual vanilla reference shape failed')
    print('Actual vanilla methods: 4 classes, 20 guarded observations', flush=True)
    if args.reference_only:
        return
    inherited = dict(os.environ)
    for key in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME'):
        inherited.pop(key, None)
    env = lab.game_env(inherited, SUNRISE_TELEMETRY_URL="off", SUNRISE_PORTAL_REFERENCE=reference.resolve(),
                       SUNRISE_PORTAL_REPORT=(build / 'native.json').resolve(),
                       SUNRISE_TEST_RESOURCES=args.resources.resolve(strict=True))
    executables = [lab.parity_fixtures()]
    with tempfile.TemporaryDirectory(prefix='sunrise-portal-cooldown-') as scratch:
        with (build / 'native.log').open('w', encoding='utf-8') as log:
            subprocess.run([executables[0], TEST, '--ignored', '--exact', '--nocapture'],
                           cwd=scratch, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    output = (build / 'native.log').read_text(encoding='utf-8', errors='replace')
    if not re.search(r'test result: ok\. 1 passed; 0 failed;', output):
        raise SystemExit('native test did not execute exactly one passing production regression')
    native = json.loads((build / 'native.json').read_text())
    for field in ('contact',):
        if native[field] != observed['contact']:
            raise SystemExit(f'{field} differs from actual vanilla methods')
    if native['phases'] != [[4, 5], [4, 4], [3, 4], [3, 3]]:
        raise SystemExit('production Game before/after-move phase counters differ')
    print('Production gateway counters and Game phases match', flush=True)


if __name__ == '__main__':
    main()
