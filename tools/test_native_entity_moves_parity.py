#!/usr/bin/env python3
"""Check native entity movement bounds/facing against both pinned vanilla jars."""
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness as harness, test_rust_parity as base, integration_env


def rows(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'native-entity-moves'
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    fixture = lab.ROOT / 'tests/fixtures/native_entity_moves.jsonl'
    golden = fixture.with_name('native_entity_moves_expected.jsonl')
    sources = [fixture, golden, Path(__file__).resolve(), lab.ROOT / 'Cargo.toml',
               lab.ROOT / 'Cargo.lock', lab.ROOT / 'tools/oracle_harness.py',
               lab.ROOT / 'tools/test_rust_parity.py', lab.ROOT / 'tools/lab.py',
               lab.ROOT / 'tools/integration_env.py',
               lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaNativeEntityMovesOracle.java',
               lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/ParityData.java']
    sources += sorted((lab.ROOT / 'crates').rglob('*.rs'))
    hashes = base.source_hashes(sources)
    expected = rows(golden)
    assert len(expected) == len(rows(fixture))
    classes, _, _ = harness.compile_oracle(build, 'VanillaNativeEntityMovesOracle',
                                         jars, libraries, lock['minecraft']['java_major'])
    for side in ('server', 'client'):
        output = build / (side + '-expected.jsonl')
        dependencies = os.pathsep.join(map(str, [jars[side], *libraries]))
        harness.java_run(build, classes, dependencies, 'VanillaNativeEntityMovesOracle',
                         [fixture, output], side)
        assert rows(output) == expected, f'{side}: committed outcomes differ from vanilla'
    executable = lab.parity_fixtures()
    with tempfile.TemporaryDirectory(prefix='sunrise-move-properties-') as scratch:
        env = integration_env.game_environment(os.environ, scratch)
        for key in ('XDG_CACHE_HOME', 'TMPDIR', 'SUNRISE_LAUNCHER_TRASH',
                    'SUNRISE_LAUNCHER_CLIPBOARD', 'SUNRISE_LAUNCHER_DESKTOP'):
            directory = Path(scratch) / key
            directory.mkdir()
            env[key] = str(directory)
        with (build / 'rust.log').open('w') as output:
            subprocess.run([executable,
                            'game::server_side::native_entity_moves::tests::move_boundaries_match_vanilla',
                            '--exact', '--test-threads=1', '--nocapture'], cwd=scratch, env=env,
                           stdout=output, stderr=subprocess.STDOUT, check=True, timeout=900)
        text = (build / 'rust.log').read_text()
        assert re.search(r'^test result: ok\. 1 passed;', text, re.MULTILINE), text
    base.ensure_sources_unchanged(sources, hashes)
    lab.write_json(report, dict(schema=1, minecraft=lock['minecraft']['version'],
        sources_sha256=hashes, cases=len(expected),
        input_sha256=lab.digest(fixture, 'sha256'),
        expected_sha256=lab.digest(golden, 'sha256'),
        jars_sha256={side: lab.digest(jars[side], 'sha256') for side in ('client', 'server')},
        rust_test_executable_sha256=lab.digest(Path(executable), 'sha256'),
        scope='Native entity spawn-bound and facing calculations, compared with Level and Direction. '
              'Does not exercise teleport networking, rider state or a game tick loop.'))
    print(f'{len(expected)} native entity movement cases match both vanilla jars: {report}', flush=True)


if __name__ == '__main__':
    main()
