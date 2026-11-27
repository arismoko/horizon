#!/usr/bin/env python3
"""Compare pig/strider Interact admission and all direct passenger identities with 26.3."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness, test_rust_parity as base

NAME = 'steerable-boarding'
ORACLE = 'VanillaSteerableBoardingOracle'
TEST = 'game::server_side::mounts::steerable_boarding_tests::interact_with_a_pig_or_strider'
CASES = {f'{kind}/{case}' for kind in ('pig', 'strider')
         for case in ('empty', 'occupied', 'secondary', 'occupied_feed', 'occupied_saddle',
                      'player_occupied', 'player_occupied_saddle')}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--work-dir', type=Path, default=lab.LOCAL / NAME)
    build = parser.parse_args().work_dir.resolve()
    classes = build / 'classes'
    classes.mkdir(parents=True, exist_ok=True)
    vanilla, rust, report = (build / name for name in ('vanilla.json', 'rust.json', 'report.json'))
    for output in (vanilla, rust, report):
        output.unlink(missing_ok=True)

    lock, jars, libraries = base.verify_artifacts()
    source = lab.ROOT / f'mods/sunrise/src/test/java/dev/sunriselab/parity/{ORACLE}.java'
    sources = [Path(__file__).resolve(), source, lab.ROOT / 'Cargo.lock',
               lab.ROOT / 'crates/sunrise-client/src/cut_layout/game/mobs/tests/steerable_boarding_tests.rs']
    hashes = base.source_hashes(sources)
    cp = os.pathsep.join(map(str, (jars['server'], *libraries)))
    oracle_harness.run_logged(['javac', '--release', lock['minecraft']['java_major'], '-proc:none',
                               '-cp', cp, '-d', classes, source], build, 'compile-java')
    with tempfile.TemporaryDirectory(dir=build) as scratch:
        with (build / 'vanilla.log').open('w', encoding='utf-8') as log:
            subprocess.run(['java', '-Xmx2g', '-Djava.awt.headless=true', '-Duser.home=' + scratch,
                            '-cp', str(classes) + os.pathsep + cp, 'dev.sunriselab.parity.' + ORACLE,
                            scratch, vanilla], cwd=scratch, stdout=log, stderr=subprocess.STDOUT,
                           timeout=180, check=True)
    resources = Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)).resolve(strict=True)
    env = lab.game_env(SUNRISE_TEST_RESOURCES=resources, SUNRISE_STEERABLE_BOARDING_REPORT=rust)
    oracle_harness.run_logged([lab.parity_fixtures(),
                               TEST, '--exact', '--ignored', '--nocapture'], build, 'rust', env=env)
    expected, actual = (json.loads(path.read_text(encoding='utf-8')) for path in (vanilla, rust))
    if expected.keys() != CASES or actual.keys() != CASES:
        raise AssertionError(f'{NAME}: incomplete observations')
    failed = sorted(name for name in CASES if actual[name] != expected[name])
    base.ensure_sources_unchanged(sources, hashes)
    lab.write_json(report, dict(minecraft=lab.VERSION, server_jar_sha256=lab.digest(jars['server'], 'sha256'),
                                sources_sha256=hashes, failures=failed, observations=len(CASES),
                                head=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=lab.ROOT,
                                                             text=True).strip()))
    if failed:
        raise SystemExit(f'{NAME}: FAIL ({", ".join(failed)})')
    print(f'{NAME}: PASS against Minecraft {lab.VERSION}')


if __name__ == '__main__':
    main()
