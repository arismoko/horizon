//! A piglin converting on the tick its held item's admiration runs out, ticked for real by the
//! game (#4824). `Piglin.customServerAiStep` runs `Brain.tick` first, where
//! `StopHoldingItemIfNoLongerAdmiring` puts away the item (`PiglinAi.stopHoldingOffHandItem`),
//! then `AbstractPiglin.customServerAiStep` converts: `Piglin.finishConversion` drops the whole
//! inventory and `Mob.convertTo` takes the piglin's place, all before `travel` moves it.
use super::native_entity_refs::tests::{prepare_entity_ticking_neighborhood, with_game};
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_core::position::ChunkPos;
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::{Species, finalize};

#[test]
#[ignore = "daily"]
fn a_piglin_converting_as_its_admiring_ends_drops_its_stash_and_is_replaced_before_travel() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.build_registries();
            let loot = sunrise_world::loot::LootData::load(
                &game.options.resources,
                &Default::default(),
                game.res.item_defaults(),
            )
            .unwrap();
            game.server.server_loot.0.set(Some(Arc::new(loot)));
            game.set_allow_commands(true);
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
            for command in [
                "tick freeze",
                "forceload add -32 -32 31 31",
                "fill -31 99 -31 31 99 31 stone",
                "summon minecraft:piglin 0.5 100 0.5 {PersistenceRequired:1b}",
            ] {
                assert!(game.run_command(command), "{command}");
            }
            game.creative = false;
            let piglin = game
                .mobs
                .iter()
                .position(|m| m.species() == Species::Piglin);
            let m = &mut game.mobs[piglin.unwrap()];
            let id = m.id;
            // A golden sword it won't swap for a gold block, three nuggets put away, and a gold
            // block admired in the off hand whose `ADMIRING_ITEM` runs out this tick.
            m.extra.set_main_hand(Some("minecraft:golden_sword".into()));
            m.extra.inventory = vec![ItemStack::new("minecraft:gold_nugget", 3)];
            finalize::set_item_stack(
                m,
                finalize::OFFHAND,
                ItemStack::new("minecraft:gold_block", 1),
            );
            m.extra.using_item = 1;
            // One tick from converting, and walking.
            m.extra.counter2 = 300;
            m.ground.on_ground = true;
            m.velocity = Vec3::new(0.3, -0.08, 0.0);
            let before = m.position;
            game.dropped.clear();

            assert!(game.run_command("tick step 1"));
            game.tick_server(false).unwrap();

            assert!(
                !game.mobs.iter().any(|m| m.id == id && !m.removed),
                "the piglin converted"
            );
            let zombified = game
                .mobs
                .iter()
                .find(|m| m.species() == Species::ZombifiedPiglin)
                .expect("a zombified piglin took its place");
            assert_eq!(
                zombified.position, before,
                "the zombified piglin stands where the piglin converted, before travel"
            );
            assert_eq!(
                finalize::item_stack(zombified, finalize::OFFHAND),
                None,
                "the zombified piglin doesn't carry the admired block"
            );
            assert_eq!(
                zombified.extra.main_hand_id(),
                Some("minecraft:golden_sword")
            );
            let dropped: Vec<(String, u32)> = game
                .dropped
                .iter()
                .map(|d| (d.stack.item.to_string(), d.stack.count))
                .collect();
            for expected in [("minecraft:gold_block", 1), ("minecraft:gold_nugget", 3)] {
                assert!(
                    dropped
                        .iter()
                        .any(|(item, count)| item == expected.0 && *count == expected.1),
                    "{expected:?} dropped: {dropped:?}"
                );
            }
        })
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
