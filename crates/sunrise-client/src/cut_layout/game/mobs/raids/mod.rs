#![cfg(test)]
#[cfg(test)]
#[path = "tests/bed_loss.rs"]
pub(crate) mod bed_loss_tests;

#[cfg(test)]
#[path = "tests/bar_players.rs"]
pub(crate) mod bar_players_tests;

#[cfg(test)]
#[path = "tests/victory.rs"]
pub(crate) mod victory_tests;

#[cfg(test)]
#[path = "tests/spawn_placements.rs"]
pub(crate) mod spawn_placement_tests;

#[cfg(test)]
pub(crate) mod entity_ticking_tests {
    use std::{collections::HashSet, sync::Arc};
    use sunrise_client_lib::raid::RaidLevel;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay::game::server_side::raids::Lvl;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::ticket_levels::Sources;

    #[test]
    #[ignore = "daily"]
    fn a_wave_spawns_only_where_entities_tick() {
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.world
                .chunks
                .insert((6, 0), Arc::new(Chunk::empty(6, 0)));
            // A player in chunk (0, 0) with a simulation distance of 5: chunk
            // (6, 0) is loaded but its entities do not tick.
            game.server.tickets.0.update(Some(Sources {
                players: vec![(0, 0)],
                view_distance: 10,
                simulation_distance: 5,
                ..Sources::default()
            }));
            let centers = HashSet::new();
            let level = Lvl {
                game,
                difficulty: 2,
                centers: &centers,
            };
            assert!(level.is_position_entity_ticking([8, 64, 8]));
            assert!(!level.is_position_entity_ticking([6 * 16 + 8, 64, 8]));
            assert!(
                !level.is_position_entity_ticking([-40, 64, 8]),
                "not loaded"
            );
        });
    }
}
