#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use mod_api::BlockEdit;
    use mod_api::BlockRun;

    use sunrise_level::chunk::Chunk;

    #[test]
    #[ignore = "daily"]
    fn typed_bulk_late_refusals_preserve_the_world_and_emit_no_changes() {
        use mod_api::{BlockEditError, StatePredicate};
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let air = registry.default_state("minecraft:air").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let api_air = air.try_into_api().unwrap();
            let api_stone = stone.try_into_api().unwrap();
            let air_filter = StatePredicate::block(api_air.block(), &[]).unwrap();
            let stone_filter = StatePredicate::block(api_stone.block(), &[]).unwrap();
            let mut edit = BlockEdit {
                palette: vec![api_stone],
                // Filter 65,536 must stay out of bounds even when its index could truncate to zero.
                filters: vec![stone_filter; 65_536],
                runs: vec![BlockRun {
                    x: 0,
                    z: 0,
                    y: 64,
                    height: 1,
                    surface: None,
                    block: 0,
                    only: Some(65_536),
                }],
            };
            game.world.block_log = Some(Vec::new());
            assert_eq!(game.world.block(0, 64, 0), air);
            let refused = game.apply_block_edit(&edit);
            assert_eq!(
                game.world.block(0, 64, 0),
                air,
                "an out-of-bounds filter index must preserve the world"
            );
            assert!(game.world.block_log.as_ref().unwrap().is_empty());
            assert_eq!(refused, Err(BlockEditError::FilterOutOfRange));

            edit.filters.push(air_filter);
            edit.palette
                .push(mod_api::__private::block_state_from_ordinal(
                    api_stone.block(),
                    u16::MAX,
                ));
            let refused = game.apply_block_edit(&edit);
            assert_eq!(
                game.world.block(0, 64, 0),
                air,
                "late palette refusal must preserve the world"
            );
            assert!(game.world.block_log.as_ref().unwrap().is_empty());
            assert!(matches!(refused, Err(BlockEditError::InvalidState(_))));

            edit.palette.pop();
            assert_eq!(game.apply_block_edit(&edit), Ok(1));
            assert_eq!(
                game.world.block(0, 64, 0),
                stone,
                "the run must match filter 65,536, not filter 0"
            );
            game.world.block_log = None;
        });
    }
}
