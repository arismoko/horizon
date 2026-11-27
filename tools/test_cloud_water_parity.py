#!/usr/bin/env python3
"""Record the vanilla 26.3 area_effect_cloud fluid witness into .local and run the Rust witness test.

Runs VanillaCommandExecOracle through tools/test_command_exec_parity.py --reference-only
(cached by tools/parity_shim.py), writes cloud_water.json and meta.json into
.local/witness/<version>/cloud_water/, and unless --record/--record-only is given runs
game::client_side::potions::cloud_water_tests::a_cloud_in_water_resets_its_fall_distance_and_a_dry_one_keeps_it
through lab.parity_fixtures() with SUNRISE_CLOUD_WATER_WITNESS.

  test_cloud_water_parity.py [--prepare] [--record|--record-only] [--resources DIR] [--work-dir DIR]
"""
import argparse
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import command_exec_corpus as corpus_module, lab, test_rust_parity as base

TEST = 'game::client_side::potions::cloud_water_tests::a_cloud_in_water_resets_its_fall_distance_and_a_dry_one_keeps_it'
GROUP_NAME = 'area effect cloud water fall distance'
ORACLE = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCommandExecOracle.java'
RECORDED_OPS = (
    'falling', 'destroy_progress', 'weather_state', 'chunk_ready', 'server_distances',
    'map_state', 'frame_map_state', 'light', 'player_state', 'player_loop', 'mob_anger',
    'projectile_saved_type', 'raid_state', 'menu_state', 'craft_menu_state', 'map_files',
    'merchant_state', 'merchant_saved_stats',
)


def java_version_string():
    result = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def recorded_count(lines):
    return sum(
        1 for op in lines
        if op['op'] in RECORDED_OPS
        or (op['op'] == 'cmd' and op.get('record', True))
        or (op['op'] == 'tick' and op.get('record'))
    )


def extract_cloud_water_witness(groups, records, lock):
    """Build the 26.3 cloud-water witness directly from VanillaCommandExecOracle's /data get output."""
    index = 0
    slice_records = None
    for name, lines in groups:
        count = recorded_count(lines)
        if name == GROUP_NAME:
            slice_records = records[index:index + count]
            break
        index += count
    if slice_records is None:
        sys.exit(f'missing {GROUP_NAME} group')
    fall_reads = {}
    motion_reads = {}
    marker = ' has the following entity data: '
    motion_re = re.compile(r'^\[([^,\]]+)d,\s*([^,\]]+)d,\s*([^,\]]+)d\]$')
    for row in slice_records:
        match = re.fullmatch(
            r'/data get entity (00000937-0000-0000-0000-[0-9a-f]{12}) (fall_distance|Motion)',
            row.get('cmd', ''),
        )
        if not match:
            continue
        text = ''.join(span[0] for span in row['messages'][0])
        if marker not in text:
            sys.exit(f'unexpected vanilla /data get feedback: {row}')
        raw = text.split(marker, 1)[1]
        uid, field = match.group(1), match.group(2)
        if field == 'fall_distance':
            if not raw.endswith('d'):
                sys.exit(f'expected double SNBT for fall_distance, got {raw!r}')
            fall_reads.setdefault(uid, []).append(raw)
        else:
            m = motion_re.match(raw)
            if not m:
                sys.exit(f'expected 3-double SNBT list for Motion, got {raw!r}')
            motion_reads.setdefault(uid, []).append((raw, list(m.groups())))
    ticks = list(corpus_module.CLOUD_WATER_TICKS)
    cases = []
    for n, (case_name, dx, dy, dz, radius, fall, duration, extra) in enumerate(corpus_module.CLOUD_WATER_CASES, 1):
        uid = f'00000937-0000-0000-0000-{n:04x}00000001'
        falls = fall_reads.get(uid)
        motions = motion_reads.get(uid)
        if not falls or len(falls) != len(ticks):
            sys.exit(f'expected {len(ticks)} fall_distance reads for {case_name} ({uid}), got {falls!r}')
        if not motions or len(motions) != len(ticks):
            sys.exit(f'expected {len(ticks)} Motion reads for {case_name} ({uid}), got {motions!r}')
        observations = []
        for tick, fall_raw, (motion_raw, comps) in zip(ticks, falls, motions):
            observations.append({
                'tick': tick,
                'fall_distance': fall_raw,
                'fall_bits': struct.pack('>d', float(fall_raw[:-1])).hex(),
                'motion': motion_raw,
                'motion_bits': [struct.pack('>d', float(c)).hex() for c in comps],
            })
        cases.append({
            'name': case_name,
            'offset': [dx, dy, dz],
            'radius': radius,
            'initial_fall_distance': fall,
            'duration': duration,
            'extra_nbt': extra,
            'observations': observations,
        })
    return {
        'minecraft': lock['minecraft']['version'],
        'oracle': 'dev.sunriselab.parity.VanillaCommandExecOracle',
        'group': GROUP_NAME,
        'record_cmd': 'python3 tools/test_cloud_water_parity.py --record',
        'server_jar_sha1': lock['minecraft']['server_sha1'],
        'water_offset': list(corpus_module.CLOUD_WATER_BLOCK),
        'blocks': [list(b) for b in corpus_module.CLOUD_WATER_BLOCKS],
        'cases': cases,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument('--record', '--record-only', dest='record', action='store_true',
                        help='record the vanilla witness into .local and skip the Rust test')
    parser.add_argument('--resources', type=Path,
                        default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    parser.add_argument('--work-dir', type=Path)
    args = parser.parse_args()

    if args.prepare:
        lab.prepare()
    lock, jars, _ = base.verify_artifacts()
    version = lock['minecraft']['version']

    work = (args.work_dir or lab.LOCAL / 'cloud-water-parity').resolve()
    work.mkdir(parents=True, exist_ok=True)
    witness_dir = lab.LOCAL / 'witness' / version / 'cloud_water'
    witness_dir.mkdir(parents=True, exist_ok=True)

    group_ops = corpus_module.area_effect_cloud_water_fall()
    groups_path = work / 'groups.json'
    groups_path.write_text(json.dumps([[GROUP_NAME, group_ops]]), encoding='utf-8')
    oracle_cmd = [
        sys.executable, str(lab.ROOT / 'tools/test_command_exec_parity.py'),
        '--groups-file', str(groups_path),
        '--work-dir', str(work),
        '--reference-only',
    ]
    result = subprocess.run(oracle_cmd, cwd=lab.ROOT)
    if result.returncode:
        return result.returncode

    reference = [json.loads(line) for line in (work / 'vanilla.jsonl').read_text(encoding='utf-8').splitlines()]
    groups = [('setup', corpus_module.group_setup()), (GROUP_NAME, group_ops)]
    expected_total = sum(recorded_count(lines) for _, lines in groups)
    if len(reference) != expected_total:
        sys.exit(f'expected {expected_total} vanilla records, got {len(reference)}')

    witness = extract_cloud_water_witness(groups, reference, lock)
    witness_path = witness_dir / 'cloud_water.json'
    lab.write_json(witness_path, witness)

    corpus_path = lab.ROOT / 'tools/command_exec_corpus.py'
    harness_path = Path(__file__).resolve()
    lab.write_json(witness_dir / 'meta.json', {
        'game_version': version,
        'java_major': lock['minecraft']['java_major'],
        'java_version': java_version_string(),
        'harness': 'tools/test_cloud_water_parity.py',
        'oracle': 'dev.sunriselab.parity.VanillaCommandExecOracle',
        'witness_env': 'SUNRISE_CLOUD_WATER_WITNESS',
        'command': oracle_cmd,
        'input_hashes': {
            'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
            'server.jar': lab.digest(jars['server'], 'sha256'),
            ORACLE.relative_to(lab.ROOT).as_posix(): lab.digest(ORACLE, 'sha256'),
            corpus_path.relative_to(lab.ROOT).as_posix(): lab.digest(corpus_path, 'sha256'),
            harness_path.relative_to(lab.ROOT).as_posix(): lab.digest(harness_path, 'sha256'),
        },
        'outputs': {
            witness_path.name: {
                'sha256': lab.digest(witness_path, 'sha256'),
                'bytes': witness_path.stat().st_size,
            },
        },
    })

    if args.record:
        print(f'Recorded {len(witness["cases"])} cloud-water cases to {witness_path}')
        return 0

    with tempfile.TemporaryDirectory(prefix='sunrise-cloud-water-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env['SUNRISE_CLOUD_WATER_WITNESS'] = str(witness_path.resolve())
        binary = lab.parity_fixtures()
        log_path = work / 'rust-fixture.log'
        with log_path.open('w', encoding='utf-8') as log:
            ran = subprocess.run(
                [str(binary), TEST, '--exact', '--ignored', '--nocapture'],
                cwd=scratch, env=env, timeout=3600, stdout=log, stderr=subprocess.STDOUT,
            )
        if ran.returncode or '1 passed; 0 failed' not in log_path.read_text(encoding='utf-8'):
            tail = log_path.read_text(encoding='utf-8', errors='replace')[-3000:]
            sys.exit(f'{TEST} failed against {witness_path}; see {log_path}\n{tail}')

    print(f'Cloud water parity passed ({len(witness["cases"])} cases against {witness_path})')
    return 0


if __name__ == '__main__':
    sys.exit(main())
