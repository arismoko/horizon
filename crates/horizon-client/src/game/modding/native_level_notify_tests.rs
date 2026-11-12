//! #968: a level's break progress and block updates wait for the outermost commit.
use super::native_entity_refs::tests::with_game;
use horizon_client::chunk::Chunk;
use mod_api::HostTransaction;
use std::sync::Arc;

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn break_progress_and_block_updates_wait_for_the_commit() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.world.block_log = Some(Vec::new());
        let level = game.level(game.dim.id()).unwrap();
        let host: &mut dyn mod_api::ServerHost = game;
        let aborted: Result<(), ()> = host.transaction(|host, _| {
            assert!(level.set_break_progress(host, -7, [1, 64, 1], 3));
            assert!(level.send_block_updated(host, [2, 64, 2]));
            Err(())
        });
        assert!(aborted.is_err());
        assert!(!game.destroying.contains_key(&-7), "no crack after abort");
        assert!(!game.world.block_log.as_ref().unwrap().contains(&(2, 64, 2)));

        let host: &mut dyn mod_api::ServerHost = game;
        host.transaction(|host, _| {
            assert!(level.set_break_progress(host, -7, [1, 64, 1], 3));
            assert!(level.send_block_updated(host, [2, 64, 2]));
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(
            game.destroying.get(&-7).map(|c| (c.0, c.1)),
            Some(([1, 64, 1], 3))
        );
        assert_eq!(game.world.block_log.as_ref().unwrap(), &vec![(2, 64, 2)]);

        // Outside a transaction the crack is cleared at once (any stage outside 0..10).
        assert!(game.level_set_break_progress(level, -7, [1, 64, 1], -1));
        assert!(!game.destroying.contains_key(&-7));
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn bone_meal_on_an_invalid_block_changes_nothing() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(5, 64, 5, stone).unwrap();
        game.world.modified.clear();
        let level = game.level(game.dim.id()).unwrap();
        assert!(!game.level_bone_meal(level, [5, 64, 5]));
        assert!(!game.level_bone_meal(level, [100_000, 64, 0]));
        assert_eq!(game.world.block(5, 64, 5), stone);
        assert!(game.world.modified.is_empty());
    });
}
