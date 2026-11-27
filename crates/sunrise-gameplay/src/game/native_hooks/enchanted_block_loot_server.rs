#[cfg(test)]
use super::*;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::ModEntry;
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};
    use sunrise_core::random::{LegacyRandom, RandomSource};
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    use sunrise_gameplay_common::game::native_hooks::enchanted_block_loot::tests::Fortune;
    use sunrise_gameplay_common::game::native_hooks::enchanted_block_loot::tests::POS;
    use sunrise_gameplay_common::game::native_hooks::enchanted_block_loot::tests::ROLLS;
    use sunrise_gameplay_common::game::native_hooks::enchanted_block_loot::tests::Read;

    /// gravel.json's `table_bonus` is `[0.1, 0.14285715, 0.25, 1.0]`: at
    /// level 3 every break gives flint, at level 0 few do. The answer reaches
    /// the table on a player's break, a level-side break and a blast, each
    /// naming its breaker; a listener that answers nothing changes neither
    /// the drops nor either random.
    #[test]
    #[ignore = "daily"]
    fn a_listener_sets_the_level_a_block_table_reads() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let gravel = game
                .pack
                .assets
                .registry
                .default_state("minecraft:gravel")
                .unwrap();
            game.server.lan_acting_entity = Some(42);
            game.server.lan_acting = Some(1);
            let mut pick = ItemStack::new("minecraft:iron_pickaxe", 1);
            pick.enchant("minecraft:fortune", 3);
            let reads = Arc::new(Mutex::new(Vec::new()));
            let mods = |answer| {
                vec![ModEntry::new(
                    "fortune",
                    Box::new(Fortune {
                        answer,
                        reads: reads.clone(),
                    }),
                )]
                .into()
            };

            let unheard = break_gravel(game, gravel, None);
            assert!(unheard.0 < ROLLS / 2, "{unheard:?}");
            with_mods(mods(None), || {
                assert_eq!(break_gravel(game, gravel, None), unheard);
            });
            assert_eq!(reads.lock().unwrap().len(), ROLLS);
            assert_eq!(
                reads.lock().unwrap()[0],
                Read {
                    pos: POS,
                    state: gravel.try_into_api().unwrap(),
                    tool: None,
                    enchantment: "minecraft:fortune".into(),
                    level: 0,
                    entity: Some(42),
                }
            );
            reads.lock().unwrap().clear();

            game.server.lan_acting_entity = None;
            game.server.lan_acting = None;
            let origin = sunrise_core::geometry::Vec3::ZERO;
            let mut host = sunrise_sim::motion::PlayerMotion::new(game.motion, origin).unwrap();
            host.entity_id = 7;
            game.player = Some(host);
            with_mods(mods(None), || {
                game.block_loot_at(POS, gravel, None);
            });
            game.player = None;
            assert_eq!(reads.lock().unwrap()[0].entity, Some(7));
            reads.lock().unwrap().clear();

            assert_eq!(break_gravel(game, gravel, Some(&pick)).0, ROLLS);
            with_mods(mods(Some(0)), || {
                assert!(break_gravel(game, gravel, Some(&pick)).0 < ROLLS / 2);
            });
            let first = reads.lock().unwrap().swap_remove(0);
            assert_eq!(first.level, 3);
            assert_eq!(first.tool.as_deref(), Some("minecraft:iron_pickaxe"));
            reads.lock().unwrap().clear();

            with_mods(mods(Some(3)), || {
                assert_eq!(break_gravel(game, gravel, None).0, ROLLS);
                reads.lock().unwrap().clear();

                game.dropped.clear();
                let at = BlockPos::new(POS[0], POS[1], POS[2]);
                game.with_level(|_, level, _| level.drop_resources_now(at, gravel));
                assert_eq!(&*game.dropped[0].stack.item, "minecraft:flint");
                let level_side = reads.lock().unwrap().swap_remove(0);
                assert_eq!(level_side.entity, None);
                reads.lock().unwrap().clear();

                game.server.combat.0.explosion_direct = Some((9, "minecraft:tnt"));
                game.world.set(POS[0], POS[1], POS[2], gravel).unwrap();
                let center = POS.map(|c| f64::from(c) + 0.5);
                let blast = crate::game::explosion::BlockInteraction::Destroy;
                game.server_explode(center, 4.0, blast, false).unwrap();
                game.server.combat.0.explosion_direct = None;
                let blast = reads.lock().unwrap().swap_remove(0);
                assert_eq!((blast.tool, blast.entity), (None, Some(9)));
            });
        });
    }

    /// `ROLLS` player breaks of gravel: how many gave flint, and where the
    /// level random and the table's sequence stand after them.
    pub(crate) fn break_gravel(
        game: &mut crate::game::ServerGame<'_>,
        gravel: BlockStateId,
        tool: Option<&ItemStack>,
    ) -> (usize, i32, i32) {
        *game.world.random = LegacyRandom::new(7);
        game.server.rand_sequences = Default::default();
        let flint = (0..ROLLS)
            .flat_map(|_| game.block_loot_at(POS, gravel, tool))
            .filter(|stack| &*stack.item == "minecraft:flint")
            .count();
        let seed = game.level.seed;
        let sequence = game
            .server
            .rand_sequences
            .get("minecraft:blocks/gravel", seed);
        (
            flint,
            game.common.world.random.next_int(),
            sequence.next_int(),
        )
    }
}
