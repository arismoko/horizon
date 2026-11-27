use crate::game::native_server::command_failure_tests::with_failure_sources;
use sunrise_game_data::text;
use sunrise_gameplay_common::game::in_own_process;
use sunrise_gameplay_common::world_biomes;

#[test]
#[ignore = "daily"]
fn tracked_executor_failure_survives_saved_depth_boundary() {
    use sunrise_core::position::BlockPos;
    use sunrise_level::chunk::block_entity::BlockEntity;
    use sunrise_sim::save::{LevelShape, SaveDir};
    use {
        sunrise_gameplay::game::native_entity_refs::tests::run_with_game_stack,
        sunrise_gameplay_common::game::minecarts::CartKind,
        sunrise_gameplay_common::game::minecarts::Minecart,
    };

    in_own_process(
        "cut_layout::game::server_side::native_server_server::command_failure_tests::tracked_executor_failure_survives_saved_depth_boundary",
        || {
            run_with_game_stack(|| {
                with_failure_sources(|game| {
                    let pos = (1, 64, 1);
                    let fallback = text::Text::tr("command.failed", &[]).plain();
                    for (command, expected) in [
                        ("rejectsavededge", "tracked failure"),
                        ("rejectsavedover", fallback.as_str()),
                    ] {
                        let Some(BlockEntity::CommandBlock(block)) =
                            game.world.block_entities.get_mut(&pos)
                        else {
                            panic!("expected a command block");
                        };
                        block.set_command(command);
                        game.carts
                            .update(0, |cart| cart.command.set_command(command));
                        game.world.game_time += 1;
                        game.command_block_tick(BlockPos::new(pos.0, pos.1, pos.2));
                        assert!(game.perform_cart_command(99));

                        let Some(BlockEntity::CommandBlock(block)) =
                            game.world.block_entities.get(&pos)
                        else {
                            panic!("expected a command block");
                        };
                        for command_block in [&**block, &game.carts[0].command] {
                            assert_eq!(&command_block.last_output_string()[11..], expected);
                        }
                        game.world.modified.insert((0, 0));
                        game.save_level(true).unwrap();
                        let reopened = SaveDir::new(
                            game.server.save.0.root.clone(),
                            LevelShape::OVERWORLD,
                            Some(Default::default()),
                        );
                        let reloaded = reopened
                            .load(0, 0, game.pack.assets.registry, world_biomes!(game))
                            .unwrap();
                        let Some((_, BlockEntity::CommandBlock(block))) =
                            reloaded.block_entities.iter().find(|(key, _)| *key == pos)
                        else {
                            panic!("saved block missing");
                        };
                        let entities = reopened.load_entities(0, 0).unwrap();
                        let cart = entities
                            .iter()
                            .find_map(|tag| Minecart::from_nbt(tag, 99, CartKind::CommandBlock))
                            .unwrap();
                        for command_block in [&**block, &cart.command] {
                            assert_eq!(command_block.command, command);
                            assert_eq!(command_block.last_execution, game.world.game_time);
                            assert_eq!(command_block.success_count, 0);
                            assert_eq!(&command_block.last_output_string()[11..], expected);
                        }
                    }
                })
            })
        },
    );
}
