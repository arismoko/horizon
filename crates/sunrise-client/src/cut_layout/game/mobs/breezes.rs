#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    /// `Entity.getOnPos(1.0E-5F)` keeps a fence's, a wall's or a fence gate's
    /// own cell while the feet rest on its 1.5-block collision; on a full block
    /// it is the block below the feet. Reading 0.2 below the feet, as the dust
    /// used to, lands in air on all three.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn the_ground_of_a_breeze_is_the_fence_wall_or_gate_it_stands_on() {
        let mut resources = None;
        let mut game = game_with_breeze(&mut resources, Vec3::new(8.5, 65.5, 8.5));
        use sunrise_gameplay_common::game::{Side, SideGame};
        let (common, side) = game.common_and_state_mut();
        let items = || match (&common.server_items, Side::world_tables(side)) {
            (Some(items), _) => &**items,
            (None, Some(tables)) => &tables.items,
            (None, None) => common.res.items(),
        };
        let registry = common.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        common.world.set(8, 63, 8, stone).unwrap();
        for name in [
            "minecraft:oak_fence",
            "minecraft:cobblestone_wall",
            "minecraft:oak_fence_gate",
        ] {
            let state = registry.default_state(name).unwrap();
            common.world.set(8, 64, 8, state).unwrap();
            let _ = common.mobs[0].update_retained_ground(
                &common.world,
                common.collision,
                &items().entity_type_tags,
                true,
                None,
            );
            assert_eq!(
                common.breeze_ground(&common.mobs[0]),
                ([8, 64, 8], state),
                "{name}"
            );
        }
        // Replacing the shape between moves doesn't select a new support.
        common.world.set(8, 64, 8, stone).unwrap();
        assert_eq!(common.breeze_ground(&common.mobs[0]), ([8, 64, 8], stone));
        // The same breeze on the full block's top reads that block.
        common.mobs[0].position = Vec3::new(8.5, 65.0, 8.5);
        let _ = common.mobs[0].update_retained_ground(
            &common.world,
            common.collision,
            &items().entity_type_tags,
            true,
            None,
        );
        assert_eq!(common.breeze_ground(&common.mobs[0]), ([8, 64, 8], stone));
    }

    #[cfg(feature = "client")]
    use sunrise_core::geometry::Vec3;
    #[cfg(feature = "client")]
    use sunrise_core::random::LegacyRandom;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::entity::Entity;

    #[cfg(feature = "client")]
    #[allow(unused_imports)]
    pub(crate) use {
        crate::game::client_side::breezes_client::tests::game_with_breeze,
        sunrise_client_gameplay::game::client_side::breezes_client::tests::made_block_particles,
    };

    /// A breeze standing on a fence rests its feet at 64 + 1.5, where its own
    /// cell is air: `Breeze.emitGroundParticles` asks `getBlockStateOn` for the
    /// supporting block, so the crumbs are the fence's. Every one of them is
    /// made by the particle's own generator (`Particle.random`), so the
    /// client level's random is where it was before the tick.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn the_dust_of_a_breeze_on_a_fence_keeps_the_client_level_random() {
        let mut resources = None;
        let mut game = game_with_breeze(&mut resources, Vec3::new(8.5, 65.5, 8.5));
        let registry = &game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let fence = registry.default_state("minecraft:oak_fence").unwrap();
        game.world.set(8, 63, 8, stone).unwrap();
        game.world.set(8, 64, 8, fence).unwrap();
        let (common, side) =
            sunrise_gameplay_common::game::SideGame::common_and_state_mut(&mut game);
        let items = match (
            &common.server_items,
            sunrise_gameplay_common::game::Side::world_tables(side),
        ) {
            (Some(items), _) => &**items,
            (None, Some(tables)) => &tables.items,
            (None, None) => common.res.items(),
        };
        let _ = common.mobs[0].update_retained_ground(
            &common.world,
            common.collision,
            &items.entity_type_tags,
            true,
            None,
        );
        assert_eq!(
            game.breeze_ground(&game.mobs[0]),
            ([8, 64, 8], fence),
            "the feet's own cell is air; the fence is what holds them up"
        );
        game.world.random = sunrise_core::random::LevelRandom::for_level(LegacyRandom::new(1234));
        let level = game.world.random.state();

        game.tick_breeze_effects();

        assert_eq!(
            made_block_particles(&game),
            1,
            "vanilla emits one crumb of the fence the breeze stands on"
        );
        assert_eq!(
            game.world.random.state(),
            level,
            "the crumb's constructor draws belong to the particle, not the level"
        );
    }

    /// The control: a breeze standing on a full block's top also crumbles it,
    /// and its constructor's draws still leave the client level's random alone.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn the_dust_of_a_breeze_on_a_full_block_keeps_the_client_level_random() {
        let mut resources = None;
        let mut game = game_with_breeze(&mut resources, Vec3::new(8.5, 64.0, 8.5));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(8, 63, 8, stone).unwrap();
        let (common, side) =
            sunrise_gameplay_common::game::SideGame::common_and_state_mut(&mut game);
        let items = match (
            &common.server_items,
            sunrise_gameplay_common::game::Side::world_tables(side),
        ) {
            (Some(items), _) => &**items,
            (None, Some(tables)) => &tables.items,
            (None, None) => common.res.items(),
        };
        let _ = common.mobs[0].update_retained_ground(
            &common.world,
            common.collision,
            &items.entity_type_tags,
            true,
            None,
        );
        game.world.random = sunrise_core::random::LevelRandom::for_level(LegacyRandom::new(1234));
        let level = game.world.random.state();

        game.tick_breeze_effects();

        assert_eq!(
            game.world.random.state(),
            level,
            "the crumb's constructor draws belong to the particle, not the level"
        );
        assert_eq!(made_block_particles(&game), 1);
    }
}
