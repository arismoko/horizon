//! The crate's integration tests in one binary (tools/refactors/merge_integration_tests.py):
//! one link instead of one per file. A new file beside this one needs its `mod` line.

mod array_declarators;
mod array_initializer_lists;
mod atomic_member_reads;
mod callable_macro_boundary;
mod global_constant_division;
mod legacy_texture_size;
mod length_array_sizes;
mod local_constants;
mod naga_glsl_atomics;
mod naga_glsl_overloads;
mod pack_compat;
mod real_packs;
mod reversed_culling_feature;
mod signed_builtin_indices;
mod stage_local_reachability;
mod terrain_fade;
mod uniform_initial_values;

/// Every module file beside this one is declared above by an active `mod` line, so none is silently left unrun.
#[test]
fn every_integration_file_is_declared() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/integration");
    let declared: Vec<&str> = include_str!("main.rs").lines().map(str::trim).collect();
    for entry in std::fs::read_dir(root).unwrap() {
        let name = entry
            .unwrap()
            .path()
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if name != "main" {
            let line = format!("mod {name};");
            assert!(
                declared.contains(&line.as_str()),
                "tests/integration/{name}.rs is not declared"
            );
        }
    }
}
