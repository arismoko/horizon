#!/usr/bin/env python3
"""Compare native comparator/hopper saves and resumed transfers with a headless Game."""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile

import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, test_rust_parity as base


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--local', type=Path, default=lab.LOCAL)
    parser.add_argument('--resources', type=Path)
    parser.add_argument('--java-only', action='store_true')
    args = parser.parse_args()
    lab.LOCAL = args.local.resolve()
    lock, jars, libraries = base.verify_artifacts()
    build = lab.ROOT / '.local/redstone-persistence-parity'
    build.mkdir(parents=True, exist_ok=True)
    oracle = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaRedstonePersistenceOracle.java'
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                    '-cp', dependencies, '-d', str(build), str(oracle),
                    str(oracle.with_name('ParityData.java'))], check=True, timeout=120)
    reference = build / 'vanilla.txt'
    subprocess.run(['java', '-Xmx512M', '-cp', str(build) + os.pathsep + dependencies,
                    'dev.sunriselab.parity.VanillaRedstonePersistenceOracle', str(reference)],
                   cwd=build, check=True, timeout=120)
    if args.java_only:
        print(reference.read_text())
        return
    resources = (args.resources or lab.LOCAL / 'sunrise-client' / lock['minecraft']['version']).resolve()
    env = lab.game_env(SUNRISE_TEST_RESOURCES=resources, SUNRISE_PERSISTENCE_ORACLE=reference)
    executables = [lab.parity_fixtures()]
    with tempfile.TemporaryDirectory(prefix='sunrise-redstone-persistence-') as scratch:
        result = subprocess.run([executables[0], 'game::server_side::persist::tests::comparator_and_hopper_live_state_survives_chunk_reload',
                                 '--ignored', '--exact', '--nocapture'], cwd=scratch, env=env,
                                capture_output=True, text=True, timeout=180)
        print(result.stdout + result.stderr)
        result.check_returncode()
        assert 'test result: ok. 1 passed; 0 failed;' in result.stdout, 'regression test did not run'
    print('Comparator output, live hopper cooldown and six resumed transfer ticks match vanilla.')


if __name__ == '__main__':
    main()
