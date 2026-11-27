//! Generator construction cannot turn shared source resources into a level registry owner.
use std::sync::Arc;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_world::worldgen::generator::{GeneratorData, OverworldGenerator};

fn assert_preset_unavailable(game: &sunrise_gameplay::game::ServerGame<'_>, data: &GeneratorData) {
    let host: &dyn mod_api::Host = game;
    assert_eq!(
        host.visit_registry_entries("minecraft:worldgen/world_preset", &mut |_| panic!(
            "unavailable registry visited"
        )),
        Err(mod_api::RegistryReadError::UnknownRegistry)
    );
    assert_eq!(
        host.registry_entry("minecraft:worldgen/world_preset", "minecraft:normal"),
        None
    );
    assert_eq!(
        data.visit_registry_ids("minecraft:worldgen/world_preset", &mut |_| panic!(
            "unavailable registry visited"
        )),
        None
    );
    assert_eq!(
        data.registry_contains("minecraft:worldgen/world_preset", "minecraft:normal"),
        None
    );
}

#[test]
#[ignore = "daily"]
fn world_preset_availability_does_not_depend_on_shared_generator_history() {
    let _publication = sunrise_sim::datapack_net::isolate_server_publication();
    let scratch = tempfile::tempdir().unwrap();
    with_game(|game| {
        let mut request = sunrise_gameplay::game::world_loading::WorldRequest::new(
            "registry ownership",
            Some("registry ownership"),
            Some(1),
            true,
            None,
            Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
        )
        .unwrap();
        request.save_root = scratch.path().to_owned();
        game.load_world(request).unwrap();
        let data = game.end.locator().unwrap().generator_data().clone();
        let other_world_data = data.clone();
        assert_preset_unavailable(game, &data);
        let generated = OverworldGenerator::for_world(
            other_world_data.clone(),
            7,
            "minecraft:normal",
            "minecraft:overworld",
            false,
        )
        .unwrap();
        assert!(Arc::ptr_eq(generated.data(), &data));
        assert_preset_unavailable(game, &other_world_data);
        let saved_stem = serde_json::json!({
            "type": "minecraft:overworld",
            "generator": {
                "type": "minecraft:noise",
                "settings": "minecraft:overworld",
                "biome_source": { "type": "minecraft:fixed", "biome": "minecraft:plains" }
            }
        });
        let saved = OverworldGenerator::for_world_with_settings(
            data.clone(),
            9,
            "minecraft:normal",
            "minecraft:overworld",
            false,
            Some(&saved_stem),
            None,
        )
        .unwrap();
        assert!(Arc::ptr_eq(saved.data(), &other_world_data));
        assert_preset_unavailable(game, &data);
    });
}
