#!/usr/bin/env python3
"""Compare a NoAI axolotl's and nautilus's air across load, tick, save and reload with pinned vanilla.

Java loads each case from NBT with a signed-short `Air`, controls the eye's water and the active
effect queries, places water, bubble columns (at the eye, at the feet, or both) or a suffocating
block at the eye and feet, forces a lethal in-wall hurt where the case asks for it, then runs the
real `LivingEntity.baseTick` until `tickEffects`, saves, reloads that save and ticks again.
Records the vanilla witness and `meta.json` under `.local/witness/<version>/noai_breather_air/`
(gitignored), then runs the `sunrise-client-lib` Rust witness tests against it through
`SUNRISE_NOAI_BREATHER_AIR_WITNESS` and compares every output row byte-for-byte.
"""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness as harness, test_rust_parity as base, integration_env

CASES = 2 * (5 * 8 + 1) * 6
TESTS = [
    'mobs::ai::noai_breather_air_tests::noai_breathers_match_the_pinned_oracle',
    'mobs::ai::noai_breather_air_tests::noai_breathers_keep_air_above_their_maximum',
]


def java_version():
    out = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (out.stderr or out.stdout).splitlines()[0].strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--resources', type=Path,
                        default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    parser.add_argument('--record', '--record-only', dest='record', action='store_true',
                        help='record the vanilla oracle witness and meta.json under .local/witness/<version>/noai_breather_air/')
    args = parser.parse_args()
    resources = args.resources.resolve()
    if not args.record and not (resources / 'entity-types.json').is_file():
        raise SystemExit('prepared resources required; pass --resources PATH')
    lock, jars, libraries = base.verify_artifacts()
    version = lock['minecraft']['version']
    build = lab.LOCAL / 'rust-noai-breather-air-parity'
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    # Keep build / 'server.txt' intact so tools/parity_shim.py can reuse unchanged oracle runs.
    (build / 'rust.txt').unlink(missing_ok=True)
    java_root = lab.ROOT / 'mods/sunrise/src/test/java'
    oracle = java_root / 'dev/sunriselab/parity/VanillaNoAiBreatherAirOracle.java'
    parity_data = java_root / 'dev/sunriselab/parity/ParityData.java'
    block_update_oracle = java_root / 'dev/sunriselab/parity/VanillaBlockUpdateOracle.java'
    sources = [Path(__file__), oracle, lab.ROOT / 'tools/lab.py',
               lab.ROOT / 'tools/oracle_harness.py', lab.ROOT / 'tools/test_rust_parity.py',
               lab.ROOT / 'tools/integration_env.py', lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock',
               lab.ROOT / 'crates/sunrise-sim/src/mobs.rs',
               lab.ROOT / 'crates/sunrise-sim/src/breathing.rs',
               lab.ROOT / 'crates/sunrise-sim/src/entity_nbt.rs',
               lab.ROOT / 'crates/sunrise-sim/src/entity_nbt/common.rs',
               lab.ROOT / 'crates/sunrise-sim/src/entity_nbt/mob_load.rs',
               lab.ROOT / 'crates/sunrise-sim/src/entity_nbt/mob_read.rs',
               lab.ROOT / 'crates/sunrise-sim/src/entity_nbt/mob_write.rs',
               lab.ROOT / 'crates/sunrise-sim/src/mobs/ai.rs',
               lab.ROOT / 'crates/sunrise-sim/src/mobs/living.rs',
               lab.ROOT / 'crates/sunrise-sim/src/mobs/world.rs',
               parity_data, block_update_oracle]
    hashes = None if args.record else base.source_hashes(sources)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    harness.run_logged(['javac', '--release', lock['minecraft']['java_major'], '-proc:none',
                        '-sourcepath', java_root, '-cp', dependencies, '-d', classes, oracle],
                       build, 'compile-java')
    oracle_cmd = ['java', '-Xmx512M', '-Djava.awt.headless=true', '-cp',
                  str(classes) + os.pathsep + dependencies,
                  'dev.sunriselab.parity.VanillaNoAiBreatherAirOracle', build / 'server.txt']
    harness.run_logged(oracle_cmd, build, 'server')
    rows = (build / 'server.txt').read_text(encoding='utf-8').splitlines()
    if len(rows) != CASES:
        raise AssertionError(f'expected {CASES} oracle rows, got {len(rows)}')
    witness_dir = lab.LOCAL / 'witness' / version / 'noai_breather_air'
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness = witness_dir / 'noai_breather_air.txt'
    server_jar_sha = lab.digest(jars['server'], 'sha256')
    java_ver = java_version()
    header = [
        f"# minecraft {version} VanillaNoAiBreatherAirOracle witness",
        "# recorded by: python3 tools/test_noai_breather_air_parity.py",
        f"# server_jar_sha256: {server_jar_sha}",
        f"# java: {java_ver}",
        "# columns: species scene mask air y_bits loaded ticked saved reloaded again health_bits",
    ]
    witness.write_text('\n'.join(header + rows) + '\n', encoding='utf-8')
    lab.write_json(witness_dir / 'meta.json', {
        'minecraft': version,
        'game_version': version,
        'java_major': lock['minecraft']['java_major'],
        'java_version': java_ver,
        'harness': 'tools/test_noai_breather_air_parity.py',
        'witness_env': 'SUNRISE_NOAI_BREATHER_AIR_WITNESS',
        'command': [str(part) for part in oracle_cmd],
        'input_hashes': {
            'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
            'server.jar': server_jar_sha,
            'tools/test_noai_breather_air_parity.py': lab.digest(Path(__file__), 'sha256'),
            'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaNoAiBreatherAirOracle.java': lab.digest(oracle, 'sha256'),
            'mods/sunrise/src/test/java/dev/sunriselab/parity/ParityData.java': lab.digest(parity_data, 'sha256'),
            'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaBlockUpdateOracle.java': lab.digest(block_update_oracle, 'sha256'),
        },
        'output': {
            'file': witness.name,
            'rows': len(rows),
            'sha256': lab.digest(witness, 'sha256'),
        },
    })
    if args.record:
        print(f'Recorded {len(rows)} rows to {witness}', flush=True)
        return
    with tempfile.TemporaryDirectory(prefix='sunrise-noai-breather-air-') as scratch:
        env = integration_env.game_environment(os.environ, scratch)
        env.update(SUNRISE_TEST_RESOURCES=str(resources),
                   SUNRISE_NOAI_BREATHER_AIR_WITNESS=str(witness.resolve()),
                   SUNRISE_NOAI_BREATHER_AIR_OUTPUT=str(build / 'rust.txt'))
        env.setdefault('CARGO_BUILD_JOBS', '1')
        env.setdefault('RUST_TEST_THREADS', '2')
        harness.run_logged(['cargo', 'test', '--locked', '-p', 'sunrise-client-lib', '--lib',
                            '--', '--ignored', '--exact', *TESTS], build, 'rust', env=env)
    rust_log = (build / 'rust.log').read_text(encoding='utf-8')
    expected_summary = f'{len(TESTS)} passed; 0 failed'
    if expected_summary not in rust_log:
        raise AssertionError(f'expected "{expected_summary}" in {build / "rust.log"}')
    matched = harness.compare(build / 'server.txt', build / 'rust.txt', CASES)
    base.ensure_sources_unchanged(sources, hashes)
    lab.write_json(report, dict(schema=1, minecraft=version,
        sources_sha256=hashes, server_jar_sha256=server_jar_sha,
        matched_output_lines=matched, output_sha256=lab.digest(build / 'rust.txt', 'sha256'),
        witness_path=str(witness.relative_to(lab.ROOT)),
        witness_tests=TESTS,
        scope=__doc__))
    print(f'{matched} NoAI breather air cases match the pinned server ({len(TESTS)} tests against {witness}): {report}', flush=True)


if __name__ == '__main__':
    main()
