#![cfg(test)]
#[cfg(test)]
pub(crate) mod player_status_tests {
    #[cfg(feature = "client")]
    use crate::game::client_side::shader_packs::player_status_tests::{
        rain_on_the_player, stand_player,
    };
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game_side;
    #[cfg(feature = "client")]
    use sunrise_level::effects::EffectInstance;
    #[cfg(feature = "client")]
    use sunrise_sim::motion::MotionWorld;

    /// The player booleans only custom uniforms read follow the window's player (spec 3.4):
    /// `LivingEntity.isAlive`, `Entity.isCurrentlyGlowing` (the Glowing effect fallback),
    /// `Entity.isInLava` and `Entity.isInWaterOrRain`.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn pack_status_follows_the_player() {
        sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
            stand_player(game);

            let status = game.pack_player_status();
            assert!(status.alive);
            assert!(!status.glowing && !status.in_lava && !status.ridden && !status.wet);

            // Rain over the player's open plains column, and none once it stops.
            rain_on_the_player(game, true);
            assert!(game.pack_player_status().wet, "rain");
            rain_on_the_player(game, false);
            assert!(!game.pack_player_status().wet);
            game.player.as_mut().unwrap().fluids.was_touching_water = true;
            assert!(game.pack_player_status().wet, "water");
            game.player.as_mut().unwrap().fluids.was_touching_water = false;

            game.potions
                .effects
                .add(EffectInstance::new("glowing", 200, 0));
            assert!(game.pack_player_status().glowing);

            let lava = game
                .pack
                .assets
                .registry
                .default_state("minecraft:lava")
                .unwrap();
            assert!(game.world.set_stored(0, 64, 0, lava));
            let meta = sunrise_gameplay_common::game::side_ops::world::WorldSideGameExt::side_game_side_meta(&*game);
            let side = game_side!(
                game.common,
                meta,
                sunrise_gameplay_common::game::side_ops::tables::SideOps::side_level_features(
                    &game.client,
                    game.common.level.enabled_features.as_arc(),
                ),
                sunrise_gameplay_common::game::side_ops::tables::SideOps::side_game_tag_items(
                    &game.client,
                    &game.common.res,
                )
            );
            let mut world = MotionWorld {
                side: &side,
                entities: &[],
                player: game.common.player.as_mut().unwrap(),
                scene: &mut game.common.world,
                assets: &game.common.pack.assets,
                collision_binding: game.common.collision,
                resources: game.common.motion,
                health: &game.common.health_resources,
                tag_items: None,
                loop_context: &game.common.loop_context,
                damage_hooks: &game.common.damage_hooks,
                block_registries: None,
            };
            world.fluid_base_tick().unwrap();
            assert!(game.pack_player_status().in_lava);

            game.player
                .as_mut()
                .unwrap()
                .set_health(0.0, false)
                .unwrap();
            assert!(!game.pack_player_status().alive);
        });
    }
}
