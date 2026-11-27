use crate::game::native_hooks::block_entity_neighbors_tests::Listener;
use mod_api::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use sunrise_core::position::BlockPos;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_level::chunk::Chunk;

#[test]
#[ignore = "daily"]
fn installed_block_entity_neighbor_parts_dispatch_through_game() {
    let notices = Arc::new(Mutex::new(Vec::new()));
    let weak = Arc::new(AtomicBool::new(true));
    let queries = Arc::new(AtomicUsize::new(0));
    let mods: Mods = vec![ModEntry::new(
        "neighbor_counter",
        Box::new(Listener(notices.clone(), weak.clone(), queries.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        with_game(|game| {
            let directory = tempfile::tempdir().unwrap();
            game.server.save.0 = Arc::new(sunrise_sim::save::SaveDir::new(
                directory.path().to_path_buf(),
                sunrise_sim::save::LevelShape::OVERWORLD,
                Some(Default::default()),
            ));
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            let glass = registry.default_state("minecraft:glass").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let source = BlockPos::new(4, 64, 4);
            let adjacent = BlockPos::new(3, 64, 4);
            let far = BlockPos::new(6, 64, 4);
            let bare = BlockPos::new(4, 64, 3);
            for (pos, state) in [
                (source, chest),
                (adjacent, glass),
                (BlockPos::new(5, 64, 4), stone),
                (far, glass),
                (bare, stone),
            ] {
                game.world.set(pos.x, pos.y, pos.z, state).unwrap();
            }
            for enabled in [true, false] {
                weak.store(enabled, Ordering::SeqCst);
                queries.store(0, Ordering::SeqCst);
                notices.lock().unwrap().clear();
                game.container_changed((source.x, source.y, source.z));
                let heard = notices.lock().unwrap();
                assert_eq!(
                    heard
                        .iter()
                        .filter(|&&(p, n)| p == adjacent && n == source)
                        .count(),
                    1
                );
                assert_eq!(
                    heard
                        .iter()
                        .filter(|&&(p, n)| p == far && n == source)
                        .count(),
                    usize::from(enabled)
                );
                assert_eq!(heard.iter().filter(|&&(p, _)| p == bare).count(), 0);
                assert_eq!(heard.len(), if enabled { 2 } else { 1 });
                assert_eq!(queries.load(Ordering::SeqCst), 1);
            }
        });
    });
}
