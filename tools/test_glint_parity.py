#!/usr/bin/env python3
"""Compare item enchantment glint transforms, atlases, decal UV3, and world/GUI UVs with vanilla 26.3.

Compiles and runs VanillaItemGlintOracle against the pinned 26.3 client jar on JDK 25,
writing the witness and meta.json into .local/witness/<version>/glint/, then runs the
8 sunrise-client-lib tests and 1 sunrise-client test against it via SUNRISE_ITEM_GLINT_WITNESS."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness as harness, test_rust_parity as base, test_model_baking_parity as baking

ORACLE = 'VanillaItemGlintOracle'

LIB_TESTS = [
    'item_render::glint_tests::glint_uv_matches_vanilla_26_3_texture_transform_bits',
    'item_render::glint_tests::end_crystal_world_sink_uses_atlas_uv_and_item_glint_scale_26_3',
    'item_render::glint_tests::block_item_glint_uses_blocks_atlas_uv_26_3',
    'item_render::glint_tests::compass_special_foil_uses_decal_uv_26_3',
    'item_render::glint_tests::shield_banner_pattern_blends_low_alpha_without_depth_write_26_3',
    'item_render::glint_tests::special_model_trident_and_shield_glint_uses_entity_scale_26_3',
    'item_render::glint_tests::translucent_foil_variants_composite_with_overlapping_translucent_surface_in_oit_26_3',
    'item_render::glint_tests::glint_atlases_match_vanilla_26_3_mip_and_aniso',
]

CLIENT_TESTS = [
    'game::client_side::item_draw::tests::item_display_end_crystal_world_submit_uses_vanilla_glint_scale_and_atlas_uv_26_3',
]


def record(lock, jars, libraries, build):
    version = lock['minecraft']['version']
    witness_dir = lab.LOCAL / 'witness' / version / 'glint'
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness = witness_dir / 'witness.json'
    classes, dependencies, oracle_path = harness.compile_oracle(
        build, ORACLE, jars, libraries, lock['minecraft']['java_major']
    )
    harness.java_run(build, classes, dependencies, ORACLE, [witness], 'client')
    java_ver = subprocess.check_output(['java', '--version'], text=True).splitlines()[0].strip()
    lab.write_json(witness_dir / 'meta.json', dict(
        minecraft=version,
        client_jar_sha256=lab.digest(jars['client'], 'sha256'),
        java_version=java_ver,
        command='python3 tools/test_glint_parity.py',
        oracle=f'mods/sunrise/src/test/java/dev/sunriselab/parity/{ORACLE}.java',
        inputs_sha256={
            'oracle': lab.digest(oracle_path, 'sha256'),
        },
    ))
    return witness


def run_logged(command, build, name, cwd=lab.ROOT, env=None):
    path = build / f'{name}.log'
    with path.open('w', encoding='utf-8') as log:
        result = subprocess.run([str(c) for c in command], cwd=cwd, env=env, timeout=3600,
                                stdout=log, stderr=subprocess.STDOUT)
    if result.returncode:
        tail = path.read_text(encoding='utf-8', errors='replace').splitlines()[-40:]
        print('\n'.join(tail), file=sys.stderr, flush=True)
        sys.exit(f'{name} failed: see {path}')
    return path


def build_test_binary(build, package, target_name, env):
    compiled = run_logged(
        ['cargo', 'test', '--locked', '-p', package, '--lib', '--no-run', '--message-format=json'],
        build, f'compile-{package}', env=env,
    )
    binaries = []
    for line in compiled.read_text(encoding='utf-8').splitlines():
        if not line.startswith('{'):
            continue
        row = json.loads(line)
        if (row.get('reason') == 'compiler-artifact' and row.get('executable')
                and row.get('target', {}).get('name') == target_name and row.get('profile', {}).get('test')):
            binaries.append(Path(row['executable']))
    assert len(binaries) == 1, f'expected one {target_name} test binary, found {binaries}'
    return binaries[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--resources', type=Path, default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    parser.add_argument('--record-only', action='store_true')
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'glint-parity'
    build.mkdir(parents=True, exist_ok=True)
    witness = record(lock, jars, libraries, build)
    if args.record_only:
        print(f'wrote {witness} ({witness.stat().st_size} bytes)')
        return
    with tempfile.TemporaryDirectory(prefix='sunrise-glint-parity-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env['SUNRISE_ITEM_GLINT_WITNESS'] = str(witness.resolve())

        lib_bin = build_test_binary(build, 'sunrise-client-lib', 'sunrise_client_lib', env)
        lib_log = run_logged([lib_bin, '--ignored', '--exact', *LIB_TESTS], build, 'client-lib-tests', cwd=scratch, env=env)
        expected_lib = f'{len(LIB_TESTS)} passed; 0 failed'
        assert expected_lib in lib_log.read_text(encoding='utf-8'), f'expected "{expected_lib}" in {lib_log}'

        client_bin = lab.parity_fixtures()
        client_log = run_logged([client_bin, '--ignored', '--exact', *CLIENT_TESTS], build, 'client-tests', cwd=scratch, env=env)
        expected_client = f'{len(CLIENT_TESTS)} passed; 0 failed'
        assert expected_client in client_log.read_text(encoding='utf-8'), f'expected "{expected_client}" in {client_log}'

    total = len(LIB_TESTS) + len(CLIENT_TESTS)
    print(f'Item glint parity passed ({total} tests against {witness})')


if __name__ == '__main__':
    main()
