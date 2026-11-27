//! Processor and pool admission uses the gameplay block-property bindings.
#![cfg(test)]

use std::{ops::ControlFlow, path::Path, sync::Arc};
use sunrise_gameplay::game::native_entity_refs::tests::with_game;

fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
#[ignore = "daily"]
fn generic_processor_and_referencing_pool_accept_block_properties() {
    let _publication = sunrise_sim::datapack_net::isolate_server_publication();
    let scratch = tempfile::tempdir().unwrap();
    let pack = scratch.path().join("datapacks/processors");
    write(
        &pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"fixture","min_format":94,"max_format":94}}"#,
    );
    write(
        &scratch.path().join("datapacks.json"),
        r#"{"enabled":["vanilla","file/processors"],"disabled":[]}"#,
    );
    // A vanilla rule processor matching a block state with properties.
    write(
        &pack.join("data/registry_processor_test/worldgen/processor_list/rule.json"),
        r#"{"processors":[{"processor_type":"minecraft:rule","rules":[{"input_predicate":{"predicate_type":"minecraft:blockstate_match","block_state":{"id":"minecraft:oak_stairs","properties":{"facing":"east","half":"top","shape":"straight","waterlogged":"false"}}},"location_predicate":{"predicate_type":"minecraft:always_true"},"output_state":{"id":"minecraft:cobblestone"}}]}]}"#,
    );
    write(
        &pack.join("data/registry_processor_test/worldgen/template_pool/rule.json"),
        r#"{"fallback":"minecraft:empty","elements":[{"weight":1,"element":{"element_type":"minecraft:single_pool_element","location":"minecraft:village/plains/houses/plains_small_house_1","processors":"registry_processor_test:rule","projection":"rigid"}}]}"#,
    );
    // Unsupported processors must still fail admission, including their pools.
    write(
        &pack.join("data/registry_processor_test/worldgen/processor_list/rejected.json"),
        r#"{"processors":[{"processor_type":"registry_processor_test:unknown"}]}"#,
    );
    write(
        &pack.join("data/registry_processor_test/worldgen/template_pool/rejected.json"),
        r#"{"fallback":"minecraft:empty","elements":[{"weight":1,"element":{"element_type":"minecraft:single_pool_element","location":"minecraft:village/plains/houses/plains_small_house_1","processors":"registry_processor_test:rejected","projection":"rigid"}}]}"#,
    );
    with_game(|game| {
        let mut request = sunrise_gameplay::game::world_loading::WorldRequest::new(
            "processor registries",
            Some("processor registries"),
            Some(1),
            true,
            None,
            Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
        )
        .unwrap();
        request.save_root = scratch.path().to_owned();
        game.load_world(request).unwrap();
        let host: &dyn mod_api::Host = game;
        for registry in [
            "minecraft:worldgen/processor_list",
            "minecraft:worldgen/template_pool",
        ] {
            assert!(
                host.registry_entry(registry, "registry_processor_test:rule")
                    .is_some(),
                "{registry}"
            );
            let mut accepted = false;
            assert_eq!(
                host.visit_registry_entries(registry, &mut |id| {
                    accepted |= id == "registry_processor_test:rule";
                    assert_ne!(id, "registry_processor_test:rejected", "{registry}");
                    ControlFlow::Continue(())
                }),
                Ok(ControlFlow::Continue(()))
            );
            assert!(accepted, "{registry}");
            assert_eq!(
                host.registry_entry(registry, "registry_processor_test:rejected"),
                None
            );
        }
    });
}
