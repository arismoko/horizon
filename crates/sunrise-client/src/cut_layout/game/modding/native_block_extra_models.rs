#![cfg(test)]
#[cfg(all(test, feature = "client"))]
mod tests {
    use mod_api::BlockExtraModelsError;
    use mod_api::prelude::ClientRenderClientHost;

    /// The live Host refuses an unloaded position and only accepts registered standalone models.
    #[test]
    #[ignore = "daily"]
    fn host_sets_only_loaded_cells_with_registered_standalone_models() {
        use {
            crate::game::server_side::native_entity_refs::tests::with_client_game,
            sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
        };

        if run_fresh_cases(
            "cut_layout::game::native_block_extra_models::tests::host_sets_only_loaded_cells_with_registered_standalone_models",
            "SUNRISE_TEST_BLOCK_EXTRA_MODELS_CASE",
            &["host"],
        ) {
            return;
        }
        let _standalone =
            sunrise_client::models::standalone_fixture(vec!["minecraft:block/gold_block".into()]);
        with_client_game(|game| {
            assert_eq!(
                ClientRenderClientHost::set_block_extra_models(game, [0, 9999, 0], &[]),
                Err(BlockExtraModelsError::NotLoaded),
            );

            let render = &game.client.client_pack.render;
            let generation = render.generation;
            let state = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let slot = render.model_catalog.state_slots(state).unwrap()[0];
            let model = mod_api::__private::built::model_ref(generation, slot);
            assert!(
                render.resolve_model(model).is_ok(),
                "the state slot is baked"
            );
            assert!(
                !render
                    .model_catalog
                    .standalone_slots()
                    .any(|registered| registered == slot)
            );
            let held = render
                .model(&mod_api::Identifier::parse("minecraft:block/gold_block").unwrap())
                .unwrap();
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            ClientRenderClientHost::set_block_extra_models(game, [0, 64, 0], &[held]).unwrap();
            let stale = mod_api::__private::built::model_ref(generation + 1, slot);
            for (bad, reason) in [
                (model, mod_api::ModelError::UnknownModel),
                (stale, mod_api::ModelError::StaleModel),
            ] {
                assert_eq!(
                    ClientRenderClientHost::set_block_extra_models(game, [0, 64, 0], &[bad]),
                    Err(BlockExtraModelsError::Model(reason)),
                );
                assert_eq!(game.world.extra_models(0, 64, 0), [held]);
            }
        });
    }
}
