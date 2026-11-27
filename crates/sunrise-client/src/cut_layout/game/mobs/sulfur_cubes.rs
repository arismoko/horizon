#![cfg(test)]
#[cfg(test)]
pub(crate) mod off_level_tests {

    use sunrise_core::geometry::Vec3;
    use sunrise_sim::mobs::sulfur_cube;

    #[test]
    #[ignore = "daily"]
    fn power_does_not_prime_a_frozen_sulfur_cube() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let power = game
                .pack
                .assets
                .registry
                .default_state("minecraft:redstone_block")
                .unwrap();
            let kind = game
                .res
                .mob_types()
                .get("minecraft:sulfur_cube")
                .unwrap()
                .clone();
            for (id, x, mount) in [(73, 1, Some(72)), (74, 4, None)] {
                game.world.set(x, 79, 1, power).unwrap();
                let mut m = sunrise_sim::mobs::Mob::new(
                    id,
                    kind.clone(),
                    Vec3::new(f64::from(x), 80.0, 1.0),
                    0.0,
                );
                m.extra.set_body(Some("minecraft:tnt".into()));
                m.extra.generic_attributes =
                    game.motion.attributes.create(&m.kind.id).map(Box::new);
                sulfur_cube::adopt_body(&mut m);
                m.extra.mount = mount;
                game.mobs.push(m);
            }
            let ghost = sunrise_sim::mobs::Mob::new(72, kind, Vec3::ZERO, 0.0);
            game.persist.off_level.insert(
                72,
                sunrise_gameplay_common::game::entity_memory::OffLevel::Mob(ghost),
            );
            let primable = game.sulfur_cubes_adopted_explosive();
            game.sulfur_cube_power_priming(&primable);
            assert!(!sulfur_cube::primed(game.mob_object(73).unwrap()));
            assert!(sulfur_cube::primed(game.mob_object(74).unwrap()));
        });
    }
}
