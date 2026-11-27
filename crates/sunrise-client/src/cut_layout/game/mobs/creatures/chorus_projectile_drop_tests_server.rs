use crate::game::creatures::chorus_projectile_drop_tests::source;
use crate::game::creatures::chorus_projectile_drop_tests::with_game;
use sunrise_core::position::BlockPos;
use sunrise_core::random::RandomSource;
use sunrise_gameplay_common::game::GameGameExt1;

#[test]
#[ignore = "daily"]
fn default_drop_rng_continues_identically_with_table_sequence_or_level_fallback() {
    for named in [false, true] {
        with_game(|game| {
            let root =
                std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
            let mut table: serde_json::Value = serde_json::from_slice(
                &std::fs::read(root.join("data/minecraft/loot_table/blocks/gravel.json")).unwrap(),
            )
            .unwrap();
            if !named {
                table.as_object_mut().unwrap().remove("random_sequence");
            }
            let files = vec![(
                "loot_table/minecraft:blocks/gravel".to_owned(),
                vec![serde_json::to_vec(&table).unwrap()],
            )];
            let registry = game.pack.assets.registry;
            // The level callback requires its catalog for the world lifetime.
            // These two fixture catalogs live only for this test process.
            let items: &'static sunrise_items::items::Items = Box::leak(Box::new(
                sunrise_items::items::Items::load(
                    &root,
                    registry,
                    game.res.facts(),
                    &sunrise_world::loot::PackOverlay {
                        files: &files,
                        conditions: &sunrise_world::conditions::LoadConditions::default(),
                        ..sunrise_world::loot::PackOverlay::default()
                    },
                    game.res.item_defaults(),
                )
                .unwrap(),
            ));
            let state = registry.default_state("minecraft:gravel").unwrap();
            assert_eq!(
                items
                    .block_loot_table(registry, state)
                    .and_then(|id| items.loot_sequence(&id))
                    .is_some(),
                named
            );
            source(game, 4, 4, "gravel");
            game.level.seed = 116;
            game.world.random.set_seed(116);
            game.server.rand_sequences = Default::default();
            game.with_level(|b, level, ticks| {
                level.items = items;
                assert!(b.destroy_block(level, ticks, BlockPos::new(4, 200, 4), true));
            });
            game.server_process_level_events().unwrap();
            assert_eq!(game.dropped.len(), 1);
            let pool = game.server.rand_sequences.to_nbt();
            let sequence_words = match pool
                .get("sequences")
                .and_then(|v| v.get("minecraft:blocks/gravel"))
                .and_then(|v| v.get("source"))
            {
                Some(sunrise_core::nbt::Tag::LongArray(v)) => v.clone(),
                _ => vec![],
            };
            assert_eq!(
                !sequence_words.is_empty(),
                named,
                "actual selected random source"
            );
            let mut random = game.world.random.clone();
            let continuation: Vec<i64> = (0..4).map(|_| random.next_long()).collect();
            let drops: Vec<_> = game
                .dropped
                .iter()
                .map(|d| (d.stack.item.to_string(), d.stack.count))
                .collect();
            println!(
                "h116-continuation {}",
                serde_json::json!({"named":named,"level_continuation":continuation,"sequence_words":sequence_words,"drops":drops})
            );
        });
    }
}
