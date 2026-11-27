#!/usr/bin/env python3
"""Compare source-silent explosions and potion impacts without starting a game/server.

The Java world records real ServerLevel.explode packets at radius zero and real
AbstractThrownPotion.onHit level events. Rust drives TNT fuse expiry and projectile
impacts in an isolated Game with silent audio. Only particle presence and sound
emission are compared, plus same-seed bottle XP/server-RNG silence invariance
on each implementation. Absolute bottle RNG ownership, terrain, damage,
potion durations and pitch are not compared.
"""
import os
from pathlib import Path
import tempfile

import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness as h, test_model_baking_parity as baking, integration_env


def main():
    lock, jars, _ = h.base.verify_artifacts()
    build = lab.LOCAL / 'source-silence-parity'
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    resources = Path(os.environ.get('SUNRISE_TEST_RESOURCES',
        lab.LOCAL / 'sunrise-client' / lock['minecraft']['version'])).resolve()
    if not (resources / 'movement-properties.nbt').is_file():
        raise RuntimeError('Prepared resources required: set SUNRISE_TEST_RESOURCES')
    sources = [Path(__file__).resolve(),
        lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaSourceSilenceOracle.java',
        lab.ROOT / 'crates/sunrise-gameplay-common/src/game.rs',
        lab.ROOT / 'crates/sunrise-sim/src/mobs.rs',
        lab.ROOT / 'crates/sunrise-sim/src/mobs/hostile.rs',
        lab.ROOT / 'crates/sunrise-sim/src/mobs/wither.rs',
        lab.ROOT / 'crates/sunrise-sim/src/mobs/sulfur_cube.rs',
        lab.ROOT / 'crates/sunrise-gameplay/src/game/entities/explosion.rs',
        lab.ROOT / 'crates/sunrise-gameplay/src/game/entities/tnt.rs',
        lab.ROOT / 'crates/sunrise-client-gameplay/src/game/items/potions.rs',
        lab.ROOT / 'crates/sunrise-gameplay/src/game/world/persist.rs',
        lab.ROOT / 'crates/sunrise-gameplay-common/src/game/mobs/creatures/mod.rs',
        lab.ROOT / 'crates/sunrise-gameplay/src/game/mobs/wither.rs',
        lab.ROOT / 'crates/sunrise-gameplay/src/game/mobs/dragon.rs',
        lab.ROOT / 'crates/sunrise-gameplay-common/src/game/entities/minecarts.rs',
    ]
    hashes = h.base.source_hashes(sources)
    classes, dependencies, _ = h.compile_oracle(build, 'VanillaSourceSilenceOracle',
        jars, baking.client_libraries(), lock['minecraft']['java_major'])
    with tempfile.TemporaryDirectory(prefix='source-silence-', dir=build) as scratch:
        env = integration_env.game_environment(os.environ, scratch)
        env.update(SUNRISE_TEST_RESOURCES=str(resources),
            SUNRISE_SOURCE_SILENCE_OUTPUT=scratch,
            CARGO_BUILD_JOBS=os.environ.get('CARGO_BUILD_JOBS', '2'),
            RUST_TEST_THREADS='2')
        h.run_logged([lab.parity_fixtures(),
            'game::server_side::tnt::tests::silent_', '--include-ignored', '--nocapture'], build, 'rust', env=env)
        rust = build / 'rust.txt'
        rust.write_text(''.join((Path(scratch) / f'{name}.txt').read_text()
            for name in ('explosion', 'potions', 'bottles')), encoding='utf-8')
        h.run_logged(['java', '-Xmx1G', '-Djava.awt.headless=true', '-cp',
            str(classes) + os.pathsep + dependencies, 'dev.sunriselab.parity.VanillaSourceSilenceOracle',
            build / 'client.txt'], build, 'client', env=env)
    matched = h.compare(build / 'client.txt', rust, 21)
    h.base.ensure_sources_unchanged(sources, hashes)
    lab.write_json(report, dict(schema=1, minecraft=lock['minecraft']['version'],
        matched_output_lines=matched, sources_sha256=hashes,
        client_jar_sha256=lab.digest(jars['client'], 'sha256'),
        scope=__doc__, output_sha256=lab.digest(rust, 'sha256')))
    print(f'{matched} source-silence cases match vanilla: {report}')


if __name__ == '__main__':
    main()
