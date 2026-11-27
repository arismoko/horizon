#![cfg(test)]

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::tables::GameItems as _;
    use sunrise_sim::motion::PlayerMotion;

    /// `suffocatesAt` counts what `isSuffocating` counts, not only cubes that render solid: a
    /// body in ice or a closed shulker box, walled in on three sides, is pushed out 0.1 a tick to
    /// the free east; one in glass (`isSuffocating` never) is not.
    #[test]
    #[ignore = "daily"]
    fn a_player_is_pushed_out_of_ice_and_a_closed_shulker_box_but_not_glass() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let mut chunk = sunrise_level::chunk::Chunk::empty(0, 0);
            chunk.lit = true;
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            for (block, want) in [
                ("minecraft:ice", [Some(0.1), None]),
                ("minecraft:shulker_box", [Some(0.1), None]),
                ("minecraft:glass", [None, None]),
            ] {
                let state = registry.default_state(block).unwrap();
                for y in 64..=65 {
                    game.world.set(3, y, 5, state).unwrap();
                    for (x, z) in [(2, 5), (3, 4), (3, 6)] {
                        game.world.set(x, y, z, stone).unwrap();
                    }
                }
                let body = PlayerMotion::new(game.motion, Vec3::new(3.5, 64.0, 5.5)).unwrap();
                assert_eq!(
                    sunrise_gameplay_common::game::closest_space::closest_space_nudge(
                        &body,
                        &game.world,
                        game.collision,
                        game.level_suffocation(),
                    ),
                    want,
                    "{block}"
                );
            }
        });
    }
}
