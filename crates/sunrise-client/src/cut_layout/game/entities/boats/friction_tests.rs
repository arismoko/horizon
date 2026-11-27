use std::sync::Arc;
use std::sync::atomic::Ordering;
use sunrise_core::geometry::Aabb;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::boats_common::Boat;
use sunrise_gameplay_common::game::boats_common::BoatStatusGameExt;
use sunrise_gameplay_common::game::boats_common::Status;

#[test]
#[ignore = "daily"]
fn ground_friction_averages_the_blocks_under_the_hull_and_asks_subscribed_parts() {
    let calls = mod_platform::__private::part_tests::friction_game_thread();
    crate::entry::install_client_hooks();
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        game.world
            .chunks
            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        game.world.set(5, 64, 5, stone).unwrap();
        // This box rests on two full blocks; the stone beside it is outside the slab.
        game.world.set(7, 64, 5, stone).unwrap();
        let bounds = Aabb::new(Vec3::new(5.0, 65.0, 5.0), Vec3::new(7.0, 65.5, 6.0));
        let boat = Boat::new(12, "oak".into(), Vec3::new(6.0, 65.0, 5.5), 0.0);
        for (block, expected, reads) in [
            ("minecraft:stone", 0.6, 0),
            ("minecraft:white_wool", 0.79, 1),
        ] {
            let state = registry.default_state(block).unwrap();
            game.world.set(6, 64, 5, state).unwrap();
            calls.store(0, Ordering::SeqCst);
            assert!(
                (game.boat_ground_friction(&boat, bounds) - expected).abs() < 1.0e-7,
                "{block}"
            );
            assert_eq!(calls.load(Ordering::SeqCst), reads);
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_status_refresh_asks_friction_as_the_chest_boat_it_is() {
    // Yellow wool's part answers 0.5 for a chest boat and 0 (no ground) for any other entity.
    let calls = mod_platform::__private::part_tests::friction_game_thread();
    crate::entry::install_client_hooks();
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        game.world
            .chunks
            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
        let wool = game
            .pack
            .assets
            .registry
            .default_state("minecraft:yellow_wool")
            .unwrap();
        for x in 5..8 {
            game.world.set(x, 64, 5, wool).unwrap();
        }
        for (chest, expected) in [(true, Status::OnLand), (false, Status::InAir)] {
            let mut boat = Boat::new(12, "oak".into(), Vec3::new(6.0, 65.0, 5.5), 0.0);
            boat.chest = chest;
            game.boats.clear();
            game.boats.push(boat);
            calls.store(0, Ordering::SeqCst);
            game.refresh_boat_status(0);
            assert_eq!(game.boats[0].status, expected, "chest boat: {chest}");
            assert!(calls.load(Ordering::SeqCst) > 0);
        }
    });
}
