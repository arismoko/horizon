#!/usr/bin/env python3
"""Record vanilla 26.3 cloud/falling_block loaded Rotation witness into .local and run its Rust test (#2680).

Compiles and runs VanillaCommandExecOracle on the cloud and falling-block loaded Rotation corpus group
against the pinned server jar, writing cloud_falling_loaded_rotation.json and meta.json into
.local/witness/<version>/cloud_falling_loaded_rotation/ (gitignored). Unless --record / --record-only
is given, runs the sunrise-client parity fixture via lab.parity_fixtures() with
SUNRISE_CLOUD_FALLING_ROTATION_WITNESS pointing at the recorded witness file.

  test_cloud_falling_loaded_rotation_parity.py [--prepare] [--record] [--resources DIR]
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import command_exec_corpus as corpus, lab, test_rust_parity as base

SEED = 1
TEST = 'game::server_side::commands::data::rotation_tests::a_cloud_and_a_falling_block_save_the_rotation_they_were_loaded_with'


def java_version_string():
    result = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument('--record', '--record-only', dest='record_only', action='store_true',
                        help='record the vanilla witness into .local/witness/<version>/cloud_falling_loaded_rotation/ and exit')
    parser.add_argument('--resources', type=Path, default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    args = parser.parse_args()

    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    version = lock['minecraft']['version']
    java_major = str(lock['minecraft']['java_major'])

    build = lab.LOCAL / 'cloud-falling-loaded-rotation-parity'
    build.mkdir(parents=True, exist_ok=True)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    witness_dir = lab.LOCAL / 'witness' / version / 'cloud_falling_loaded_rotation'
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness_path = witness_dir / 'cloud_falling_loaded_rotation.json'

    oracle = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCommandExecOracle.java'
    dependency_path = os.pathsep.join(map(str, libraries))

    def run(command, name, cwd=lab.ROOT, env=None):
        path = build / (name + '.log')
        with path.open('w', encoding='utf-8') as log:
            result = subprocess.run(list(map(str, command)), cwd=cwd, env=env, timeout=3600,
                                    stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            tail = path.read_text(encoding='utf-8', errors='replace')[-3000:]
            sys.exit(f'{name} failed ({result.returncode}): see {path}\n{tail}')
        return path

    script = [*corpus.group_setup(), *corpus.cloud_and_falling_block_loaded_rotation()]
    script_text = ''.join(
        json.dumps({k: v for k, v in op.items() if k != 'expect_result'}, ensure_ascii=False) + '\n'
        for op in script
    )
    script_path = build / 'script.jsonl'
    if not script_path.is_file() or script_path.read_text(encoding='utf-8') != script_text:
        script_path.write_text(script_text, encoding='utf-8')

    compile_cmd = [
        'javac', '--release', java_major, '-proc:none',
        '-cp', str(jars['server']) + os.pathsep + dependency_path,
        '-d', str(classes), str(oracle),
    ]
    run(compile_cmd, 'compile-java')

    server_sha = lab.digest(jars['server'], 'sha256')
    oracle_sha = lab.digest(oracle, 'sha256')
    lock_sha = lab.digest(lab.LOCK, 'sha256')
    script_sha = lab.digest(script_path, 'sha256')
    java_ver = java_version_string()
    input_stamp = hashlib.sha256(json.dumps({
        'lock': lock_sha,
        'server': server_sha,
        'oracle': oracle_sha,
        'script': script_sha,
        'java_major': java_major,
        'java_version': java_ver,
        'seed': SEED,
    }, sort_keys=True).encode()).hexdigest()

    vanilla_dir = build / 'vanilla'
    vanilla_out = build / 'vanilla.jsonl'
    stamp_path = build / 'oracle-inputs.sha256'
    shim_active = bool(os.environ.get('PARITY_SHIM_REAL_PATH')) and os.environ.get('PARITY_JAVA_CACHE') != '0'
    can_reuse = (
        shim_active
        and vanilla_out.is_file()
        and (vanilla_dir / 'universe').is_dir()
        and stamp_path.is_file()
        and stamp_path.read_text(encoding='utf-8').strip() == input_stamp
    )
    if not can_reuse:
        shutil.rmtree(vanilla_dir, ignore_errors=True)
        vanilla_dir.mkdir(parents=True, exist_ok=True)
        vanilla_out.unlink(missing_ok=True)

    java_cmd = [
        'java', '-Xmx3G', '-Duser.timezone=UTC',
        '-cp', os.pathsep.join([str(classes), str(jars['server']), dependency_path]),
        'dev.sunriselab.parity.VanillaCommandExecOracle',
        str(vanilla_dir / 'universe'), str(script_path), str(vanilla_out), str(SEED),
    ]
    run(java_cmd, 'java', cwd=vanilla_dir)
    stamp_path.write_text(input_stamp + '\n', encoding='utf-8')

    reference = [json.loads(line) for line in vanilla_out.read_text(encoding='utf-8').splitlines()]
    recorded = [
        op for op in script
        if (op['op'] == 'cmd' and op.get('record', True)) or (op['op'] == 'tick' and op.get('record'))
    ]
    if len(recorded) != len(reference):
        sys.exit(f'Java reference record count differs: expected {len(recorded)}, got {len(reference)}')
    for op, row in zip(recorded, reference):
        if 'expect_result' in op and row.get('results') != [[True, op['expect_result']]]:
            sys.exit(f'Java fixture guard failed: {op}: {row.get("results")}')

    witness = corpus.extract_cloud_falling_rotation_witness(
        reference, version, server_sha, java_ver, lock['minecraft']['java_major'],
    )
    lab.write_json(witness_path, witness)

    lab.write_json(witness_dir / 'meta.json', {
        'game_version': version,
        'java_major': lock['minecraft']['java_major'],
        'java_version': java_ver,
        'harness': 'tools/test_cloud_falling_loaded_rotation_parity.py',
        'witness_env': 'SUNRISE_CLOUD_FALLING_ROTATION_WITNESS',
        'oracle': oracle.relative_to(lab.ROOT).as_posix(),
        'command': java_cmd,
        'input_hashes': {
            'research-lock.json': lock_sha,
            'server.jar': server_sha,
            oracle.relative_to(lab.ROOT).as_posix(): oracle_sha,
            'tools/command_exec_corpus.py': lab.digest(lab.ROOT / 'tools/command_exec_corpus.py', 'sha256'),
            'script.jsonl': script_sha,
        },
        'outputs': {
            witness_path.name: {
                'sha256': lab.digest(witness_path, 'sha256'),
                'bytes': witness_path.stat().st_size,
            },
        },
    })

    if args.record_only:
        print(f'Recorded {version} cloud/falling_block loaded Rotation witness into {witness_dir}')
        return

    with tempfile.TemporaryDirectory(prefix='sunrise-loaded-rotation-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env['SUNRISE_CLOUD_FALLING_ROTATION_WITNESS'] = str(witness_path.resolve())
        binary = lab.parity_fixtures()
        log = run([binary, TEST, '--exact', '--ignored', '--nocapture'], 'rust', cwd=scratch, env=env)
        if '1 passed; 0 failed' not in log.read_text(encoding='utf-8'):
            sys.exit(f'expected 1 passed; 0 failed for {TEST} in {log}')

    print(f'Cloud and falling-block loaded Rotation parity passed ({TEST} against {witness_path})')


if __name__ == '__main__':
    main()
