#[cfg(any(test, feature = "test-support"))]
pub mod sweep_tests {
    /// The game's player sweeps its move through two stone blocks, over the game's
    /// own world and assets, through the production `BLOCK_CONTACT_HOOKS` fire.
    /// Returns the player's velocity x before and after, and whether it died.
    pub fn sweep_game_player(game: &mut crate::game::ServerGame<'_>) -> (f64, f64, bool) {
        game.options.dedicated = Some(Vec::new());
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
        );
        for x in [1, 2] {
            game.world.set(x, 100, 0, stone).unwrap();
        }
        if game.player.is_none() {
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::new(0.5, 100.0, 0.5),
                )
                .unwrap(),
            );
        }
        game.player
            .as_mut()
            .unwrap()
            .history
            .add(sunrise_world::entity_move::MovementRecord {
                from: sunrise_core::geometry::Vec3::new(0.5, 100.0, 0.5),
                to: sunrise_core::geometry::Vec3::new(2.5, 100.0, 0.5),
                original: None,
            });
        sweep_game_player_live(
            game,
            sunrise_core::geometry::Vec3::new(0.5, 100.0, 0.5),
            false,
        )
    }

    /// Runs the server-turn player inside-block sweep: collect (optionally after
    /// `server_stuck_in_blocks`), flush live against `&mut ServerGame`, and finish freezing.
    pub fn sweep_game_player_live(
        game: &mut crate::game::ServerGame<'_>,
        from: sunrise_core::geometry::Vec3,
        with_stuck_pass: bool,
    ) -> (f64, f64, bool) {
        use sunrise_sim::motion::MotionWorld;
        let before = game.player.as_ref().unwrap().velocity.x;
        let effects = {
            let sunrise_gameplay_common::game::CommonGame {
                world,
                pack,
                motion,
                health_resources,
                damage_hooks,
                loop_context,
                collision,
                player,
                ..
            } = &mut game.common;
            let entities: [sunrise_core::geometry::Aabb; 0] = [];
            sunrise_sim::block_contact::in_level(
                sunrise_sim::parallel::TickThread::mint(),
                Some("minecraft:overworld"),
                || {
                    let mut w = MotionWorld {
                        side: sunrise_sim::motion::dry_side(),
                        tag_items: None,
                        block_registries: Some(&pack.block_registries),
                        loop_context,
                        damage_hooks,
                        entities: &entities,
                        player: player.as_mut().unwrap(),
                        scene: world,
                        assets: &pack.assets,
                        collision_binding: collision,
                        resources: motion,
                        health: health_resources,
                    };
                    if with_stuck_pass {
                        w.server_stuck_in_blocks(from).unwrap();
                    }
                    w.collect_server_block_damage(from).unwrap()
                },
            )
        };
        game.flush_player_inside(effects);
        {
            let sunrise_gameplay_common::game::CommonGame {
                world,
                pack,
                motion,
                health_resources,
                damage_hooks,
                loop_context,
                collision,
                player,
                ..
            } = &mut game.common;
            let entities: [sunrise_core::geometry::Aabb; 0] = [];
            let mut w = MotionWorld {
                side: sunrise_sim::motion::dry_side(),
                tag_items: None,
                block_registries: Some(&pack.block_registries),
                loop_context,
                damage_hooks,
                entities: &entities,
                player: player.as_mut().unwrap(),
                scene: world,
                assets: &pack.assets,
                collision_binding: collision,
                resources: motion,
                health: health_resources,
            };
            let _ = w.finish_server_block_damage_freezing();
        }
        let player = game.player.as_ref().unwrap();
        (before, player.velocity.x, player.health.dead_or_dying())
    }
    /// The item's batch sweep tests: the same part, the same assertions as the
    /// player's, over `Game::sweep_item_inside`.
    pub fn item_at_blocks(
        game: &mut crate::game::ServerGame<'_>,
    ) -> sunrise_sim::entities::ItemEntity {
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
        );
        for x in [1, 2] {
            game.world.set(x, 100, 0, stone).unwrap();
        }
        let mut item = sunrise_sim::entities::ItemEntity::new(
            7,
            sunrise_items::items::ItemStack::new("minecraft:stone", 1),
            sunrise_core::geometry::Vec3::new(2.5, 100.0, 0.5),
            sunrise_core::geometry::Vec3::ZERO,
            0.0,
        );
        item.previous = sunrise_core::geometry::Vec3::new(0.5, 100.0, 0.5);
        item
    }
}
