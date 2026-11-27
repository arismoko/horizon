#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_level::containers::CreakingHeart;
use sunrise_sim::mobs::Mob;
use sunrise_sim::mobs::creaking_heart::HeartEvent;
#[cfg(test)]
#[path = "tests/creaking_heart_events_tests.rs"]
pub(crate) mod creaking_heart_events_tests;

#[cfg(test)]
pub(crate) mod nested_spawn_tests {
    use super::*;
    use crate::game::server_side::entity_server::taken_pass_tests;

    #[test]
    #[ignore = "daily"]
    fn lifted_heart_callback_keeps_nested_spawns_until_the_next_pass() {
        taken_pass_tests::in_both_dimensions(|game, balance| {
            let mut heart = CreakingHeart::default();
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs
                .push(Mob::new(70, kind, Vec3::new(0.5, 80.0, 0.5), 0.0));
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            for others in [false, true] {
                let at = [4 + i32::from(others), 80, 4];
                taken_pass_tests::assert_mob_spawn_survives(game, balance, |game| {
                    game.with_heart((0, 80, 0), &mut heart, others, |_, world, _, _, events| {
                        assert_eq!(world.is_some(), others);
                        assert_eq!(
                            world.map_or_else(Vec::new, |world| world
                                .others
                                .iter()
                                .map(|mob| mob.id)
                                .collect::<Vec<_>>()),
                            if others { vec![70] } else { vec![] }
                        );
                        events.push(HeartEvent::SetBlock(at, stone));
                    })
                    .unwrap();
                });
                assert_eq!(
                    game.world.block(at[0], at[1], at[2]),
                    stone,
                    "heart event was not applied after restoring the pass"
                );
            }
        });
    }
}
