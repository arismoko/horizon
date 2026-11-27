#!/usr/bin/env python3
"""Compare the exact dismount trace of a dying pig with nested passengers."""
import argparse
import os
from pathlib import Path
from tempfile import TemporaryDirectory

import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, test_rust_parity as parity


SOURCES = [
    'crates/sunrise-client/src/cut_layout/game/entities/vehicle_removal_tests.rs',
    'crates/sunrise-gameplay/src/game/modding/native_entity_remove.rs',
    'crates/sunrise-gameplay/src/game/modding/native_entity_riding.rs',
    'crates/sunrise-gameplay-common/src/game/mobs/creatures/borrowed_tick.rs',
    'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaVehicleRemovalOracle.java',
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--resources', type=Path, default=lab.RESOURCES)
    parser.add_argument('--java-only', action='store_true')
    args = parser.parse_args()
    lock, jars, libraries = parity.verify_artifacts()
    work = lab.cargo_target() / 'mob-removal-order'
    work.mkdir(parents=True, exist_ok=True)
    sources = lab.ROOT / 'mods/sunrise/src/test/java'
    oracle = sources / 'dev/sunriselab/parity/VanillaVehicleRemovalOracle.java'
    classpath = os.pathsep.join(map(str, [jars['server'], *libraries]))
    with TemporaryDirectory(dir=work) as temporary:
        folder = Path(temporary)
        vanilla = folder / 'vanilla.txt'
        sunrise = folder / 'sunrise.txt'
        parity.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                    '-sourcepath', str(sources), '-cp', classpath, '-d', str(folder), str(oracle)])
        parity.run(['java', '-cp', str(folder) + os.pathsep + classpath,
                    'dev.sunriselab.parity.VanillaVehicleRemovalOracle', str(vanilla)])
        if args.java_only:
            print('Vanilla removal trace recorded')
            return
        parity.run([lab.parity_fixtures(),
                    'game::server_side::vehicle_removal_tests::mob_removal_order_witness', '--exact', '--ignored'],
                   env=lab.game_env(SUNRISE_TEST_RESOURCES=str(args.resources.resolve()),
                                    SUNRISE_VEHICLE_REMOVAL_TRACE=str(sunrise)))
        count = parity.compare(vanilla, sunrise)
        print(f'Mob removal: all {count} observations match vanilla')


if __name__ == '__main__':
    main()
