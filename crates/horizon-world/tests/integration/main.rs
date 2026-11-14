//! The crate's integration tests in one binary (tools/refactors/merge_integration_tests.py):
//! one link instead of one per file. A new file beside this one needs its `mod` line.

mod biome_data;
mod biome_direct_lists;
mod biome_points_selection;
mod biome_predicate_bounds;
mod biome_region;
mod block_collision_scan;
mod block_entities;
mod block_predicate_bounds;
mod carver_generation;
mod carving_stage;
mod collision_data;
mod decoration_bounds;
mod edge_backoff;
mod feature_catalog;
mod feature_graph_bounds;
mod feature_placement;
mod feature_region;
mod feature_sort_bounds;
mod jigsaw_bounds;
mod material_rule_specialization;
mod material_stage;
mod movement_properties;
mod placement_bounds;
mod placement_queries;
mod processor_bounds;
mod registered_biome_source;
mod retrogen_biomes;
mod rule_test_bounds;
mod snbt_unicode_order;
mod state_provider_bounds;
mod template_loading;
mod tick_data;
mod weighted_provider;
mod world_io;
mod world_movement;
mod world_ray_limits;

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
