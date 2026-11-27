#!/usr/bin/env python3
"""Compare display entity lifecycle, billboards, sorting, culling, missing glyphs, and atlas padding with vanilla 26.3.

Compiles and runs VanillaDisplayParityOracle against the pinned 26.3 client jar on JDK 25,
writing witness.json and meta.json into .local/witness/<version>/displays/, then runs the
17 Rust parity tests across sunrise-client-render, sunrise-client-lib, and sunrise-client against it
via SUNRISE_DISPLAY_PARITY_WITNESS."""
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

ORACLE = 'VanillaDisplayParityOracle'

RENDER_TESTS = [
    'gfx::entity_batch::tests::classic_polygon_offset_text_quads_sort_far_to_near_like_vanilla_26_3',
    'gfx::entity_batch::tests::classic_translucent_item_quads_sort_far_to_near_like_vanilla_26_3',
    'gfx::oit_features::tests::oit_text_polygon_offset_depth_bias_matches_vanilla_26_3',
    'hud::tests::atlas::packed_sprites_have_clamp_to_edge_padding_matching_vanilla_26_3',
    'hud::tests::text::missing_glyph_quads_match_vanilla_26_3_witness',
]

LIB_TESTS = [
    'text_display::tests::non_unit_quaternion_blend_and_matrix_match_vanilla_26_3_bits',
    'text_display::tests::negative_width_or_height_culling_box_matches_vanilla_26_3_bits',
    'text_display::tests::billboard_matrix_matches_vanilla_26_3_bits',
]

CLIENT_TESTS = [
    'game::client_side::text_displays::display_parity_tests::initial_start_min_value_and_duration_edge_cases_match_vanilla_26_3',
    'game::client_side::text_displays::display_parity_tests::teleport_duration_unclamped_negative_same_target_and_rotation_match_vanilla_26_3',
    'game::client_side::text_displays::display_parity_tests::display_shadow_respects_pre_tick_and_invisibility_flag_26_3',
    'game::client_side::text_displays::display_parity_tests::display_camera_angles_and_billboard_use_active_camera_not_player_26_3',
    'game::client_side::text_displays::display_parity_tests::display_lifecycle_pre_tick_deferred_swap_live_culling_and_culled_progress_26_3',
    'game::client_side::text_displays::display_parity_tests::text_display_render_order_solid_split_and_see_through_sort_26_3',
    'game::client_side::text_displays::display_parity_tests::text_display_resolves_inline_player_head_profiles_26_3',
    'game::client_side::text_displays::display_parity_tests::bold_missing_glyph_displays_and_block_display_83224_match_vanilla_26_3',
    'game::client_side::text_displays::display_parity_tests::review_parity_witness_matches_vanilla_26_3',
]


def record(lock, jars, libraries, build):
    version = lock['minecraft']['version']
    witness_dir = lab.LOCAL / 'witness' / version / 'displays'
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness = witness_dir / 'witness.json'
    classes, dependencies, oracle_path = harness.compile_oracle(
        build, ORACLE, jars, libraries, lock['minecraft']['java_major']
    )
    harness.java_run(build, classes, dependencies, ORACLE, [witness], 'client')
    data = json.loads(witness.read_text(encoding='utf-8'))
    assert data['meta']['game_version'] == version, data['meta']
    java_ver = subprocess.check_output(['java', '--version'], text=True).splitlines()[0].strip()
    lab.write_json(witness_dir / 'meta.json', dict(
        minecraft=version,
        client_jar_sha256=lab.digest(jars['client'], 'sha256'),
        java_version=java_ver,
        command='python3 tools/test_displays_parity.py',
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
    build = lab.LOCAL / 'displays-parity'
    build.mkdir(parents=True, exist_ok=True)
    witness = record(lock, jars, libraries, build)
    if args.record_only:
        print(f'wrote {witness} ({witness.stat().st_size} bytes)')
        return
    with tempfile.TemporaryDirectory(prefix='sunrise-displays-parity-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env['SUNRISE_DISPLAY_PARITY_WITNESS'] = str(witness.resolve())

        for package, target_name, name, tests in (
            ('sunrise-client-render', 'sunrise_client_render', 'render-tests', RENDER_TESTS),
            ('sunrise-client-lib', 'sunrise_client_lib', 'client-lib-tests', LIB_TESTS),
            ('sunrise-client', 'sunrise_client', 'client-tests', CLIENT_TESTS),
        ):
            binary = build_test_binary(build, package, target_name, env)
            log = run_logged([binary, '--ignored', '--exact', *tests], build, name, cwd=scratch, env=env)
            expected = f'{len(tests)} passed; 0 failed'
            assert expected in log.read_text(encoding='utf-8'), f'expected "{expected}" in {log}'

    total = len(RENDER_TESTS) + len(LIB_TESTS) + len(CLIENT_TESTS)
    print(f'Display parity passed ({total} tests against {witness})')


if __name__ == '__main__':
    main()
