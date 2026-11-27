//! `Entity.checkBelowWorld`: a dropped item is discarded below its level's
//! floor minus 64, so the Nether and the End lose one at y = -64 while the
//! Overworld keeps it down to y = -128.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::game_tests::tests::server_game;
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_sim::motion::PlayerMotion;

#[test]
#[ignore = "daily"]
fn items_fall_to_each_dimensions_own_void_floor() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 70.0, 8.5)).unwrap());
    for x in -2..=2 {
        for z in -2..=2 {
            game.world
                .chunks
                .insert((x, z), Arc::new(Chunk::empty(x, z)));
        }
    }
    game.update_chunk_tickets();
    assert!(game.entity_ticking_area()(8, 8));
    let keys: HashSet<(i32, i32)> = [(0, 0)].into();
    for (dim, floor) in [
        (Dim::Overworld, -128.0),
        (Dim::Nether, -64.0),
        (Dim::End, -64.0),
    ] {
        game.dim = dim;
        game.world.shape = dim.shape();
        for (y, survives) in [(floor + 0.5, true), (floor - 0.5, false)] {
            game.dropped.clear();
            game.spawn_item(
                ItemStack::new("minecraft:stone", 1),
                [8.5, y, 8.5],
                [0.0, 0.0, 0.0],
                0,
            );
            game.server.ticked_entities_through = u32::MAX;
            game.tick_items();
            assert_eq!(
                !game.dropped.is_empty(),
                survives,
                "{dim:?}: an item at y {y}, against the floor {floor}"
            );
            let mut saved = HashMap::new();
            let mut failed = HashMap::new();
            game.save_every_entity(&keys, &mut saved, &mut failed, false);
            assert!(failed.is_empty());
            assert_eq!(
                saved.values().map(Vec::len).sum::<usize>() > 0,
                survives,
                "{dim:?}: an item at y {y} in the save"
            );
        }
    }
}
