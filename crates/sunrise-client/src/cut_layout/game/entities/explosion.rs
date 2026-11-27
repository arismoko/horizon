#![cfg(test)]
use mod_api::ExplosionStart;
use sunrise_core::geometry::Vec3;
use sunrise_core::position::BlockPos;
use sunrise_game_data::text::Text;
use sunrise_gameplay::game::server_side::explosion::{
    BlockInteraction, ExplicitBlastSource, ExplosionCause, ExplosionSource,
};
use sunrise_items::items::ItemStack;
use sunrise_world::registry::BlockStateId;
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_core::random::RandomSource;
    use sunrise_gameplay::game::server_side::explosion::tests::exploded_positions_stepwise;

    use super::*;

    #[test]
    #[ignore = "daily"]
    fn rays_reusing_their_last_block_explode_what_each_step_did() {
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        let registry = &game.pack.assets.registry;
        let states: Vec<BlockStateId> = [
            "minecraft:stone",
            "minecraft:air",
            "minecraft:water",
            "minecraft:dirt",
            "minecraft:obsidian",
            "minecraft:air",
            "minecraft:oak_planks",
            "minecraft:glass",
        ]
        .iter()
        .map(|n| registry.default_state(n).unwrap())
        .collect();
        // A jumble of blocks of every resistance around the blast, in the
        // loaded chunk at the origin.
        for x in 0..16 {
            for z in 0..16 {
                for y in 56..74 {
                    let i = ((x * 7 + y * 3 + z * 5) as usize) % states.len();
                    game.world.set(x, y, z, states[i]).unwrap();
                }
            }
        }
        let center = [8.3, 64.6, 7.9];
        for (radius, resists) in [(4.0, None), (6.0, None), (4.0, Some(([8, 64, 8], 1200.0)))] {
            game.world.random.set_seed(7);
            let expected = exploded_positions_stepwise(&mut game, center, radius, resists);
            game.world.random.set_seed(7);
            let start = ExplosionStart::from(mod_api::__private::built::ExplosionStart {
                dimension: mod_api::DimensionKey::OVERWORLD,
                center,
                radius,
                fire: false,
                interaction: mod_api::BlockInteraction::Destroy,
                damage_type: "minecraft:explosion",
                source: None,
                owner: None,
            });
            let got = game.exploded_positions(center, radius, resists, false, &start);
            assert!(expected.len() > 20, "{radius}: {}", expected.len());
            assert_eq!(got, expected, "radius {radius}, {resists:?}");
        }
    }
    #[test]
    #[ignore = "requires prepared resources; tools/test_explosion_items_parity.py"]
    fn explosion_items_parity_fixture() {
        run_explosion_items_parity_fixture();
    }
    pub(crate) fn run_explosion_items_parity_fixture() {
        let directory = std::env::var_os("SUNRISE_EXPLOSION_ITEMS_FIXTURE")
            .expect("set SUNRISE_EXPLOSION_ITEMS_FIXTURE to the fixture directory");
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.run_explosion_items_fixture(std::path::Path::new(&directory))
                .unwrap();
        });
    }
}

#[cfg(test)]
#[path = "explosion/explosion_part_tests.rs"]
mod explosion_part_tests;

#[cfg(test)]
mod shulker_lock_drop_tests {
    use super::super::game_tests::tests::server_game;
    use super::*;
    use sunrise_core::nbt::Tag;
    use sunrise_level::containers::BlockEntity;
    use sunrise_sim::save;

    #[test]
    #[ignore = "daily"]
    fn destruction_captures_the_external_shulker_lock_before_removal() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.build_registries();
        game.set_allow_commands(true);
        game.block_ticks
            .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
            .unwrap();
        assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
        assert!(
            game.ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::BLOCK_DROPS),
            "actual drop rule must be enabled"
        );
        game.creative = false;
        game.selected = 0;
        game.inventory[0] = Some(ItemStack::new("minecraft:diamond_pickaxe", 1));
        let clock = game.world.game_time;
        let mut missing = Vec::new();
        for (x, z, kind, locked, route) in [
            (3, 3, "shulker_box", true, "command"),
            (11, 3, "shulker_box", true, "player"),
            (3, 11, "shulker_box", true, "explosion"),
            (11, 11, "red_shulker_box", false, "command"),
            (7, 3, "chest", true, "command"),
            (7, 11, "crafter", true, "command"),
        ] {
            let shulker = kind.ends_with("shulker_box");
            let mut fields = Vec::new();
            if shulker {
                fields.push("CustomName:\"Box\"");
                fields.push("Items:[{Slot:0b,id:\"minecraft:diamond\",count:3,components:{\"minecraft:custom_data\":{h211:1}}},{Slot:26b,id:\"minecraft:apple\",count:5}]");
            }
            if locked {
                fields.push("lock:{items:\"minecraft:diamond\"}");
            }
            assert!(game.run_command(&format!(
                "setblock {x} 200 {z} minecraft:{kind}{{{}}}",
                fields.join(",")
            )));
            let pos = [x, 200, z];
            let key = (x, 200, z);
            assert_eq!(
                &*game.res.facts().get(game.world.block(x, 199, z)).name,
                "minecraft:stone",
                "positive floor"
            );
            let actual_lock = game.world.block_lock(key).cloned();
            assert_eq!(actual_lock.is_some(), locked, "actual external source lock");
            if locked {
                assert!(matches!(&actual_lock,Some(Tag::Compound(c)) if !c.is_empty()));
            }
            let entity = game
                .world
                .block_entities
                .get(&key)
                .expect("actual command-created source");
            assert!(
                save::block_entity_encode::encode(
                    key,
                    entity,
                    &format!("minecraft:{kind}"),
                    &Default::default()
                )
                .get("lock")
                .is_none(),
                "external lock must not be invented in modeled serializer"
            );
            if shulker {
                let BlockEntity::Chest(slots) = entity else {
                    panic!("modeled shulker inventory missing");
                };
                assert_eq!(slots[0].as_ref().unwrap().count, 3);
                assert_eq!(slots[26].as_ref().unwrap().count, 5);
                assert_eq!(
                    game.world.block_names.get(&key),
                    Some(&Tag::String("Box".into()))
                );
            }
            let start = game.dropped.len();
            match route {
                "command" => {
                    assert!(game.run_command(&format!("setblock {x} 200 {z} air destroy")))
                }
                "player" => game.break_block(pos).unwrap(),
                "explosion" => game
                    .server_explode(
                        pos.map(|v| f64::from(v) + 0.5),
                        2.0,
                        BlockInteraction::Destroy,
                        false,
                    )
                    .unwrap(),
                _ => unreachable!(),
            }
            game.flush_spill();
            assert!(
                game.res.facts().is_air(game.world.block(x, 200, z)),
                "actual destruction occurred: {route}"
            );
            assert!(!game.world.block_entities.contains_key(&key));
            assert!(
                game.world.block_lock(key).is_none(),
                "actual external source was removed"
            );
            let item_id = format!("minecraft:{kind}");
            let drops: Vec<_> = game.dropped[start..]
                .iter()
                .filter(|d| *d.stack.item == item_id)
                .collect();
            assert_eq!(drops.len(), 1, "actual one-item drop: {kind}/{route}");
            let stack = &drops[0].stack;
            assert_eq!(stack.count, 1);
            let data = stack.data();
            let dropped_lock = data.and_then(|d| d.other("minecraft:lock"));
            if shulker {
                let data = data.expect("actual shulker components");
                assert_eq!(data.custom_name, Some(Tag::String("Box".into())));
                let slots = &data.container;
                assert_eq!(slots.len(), 2);
                let diamond = &slots.iter().find(|(slot, _)| *slot == 0).unwrap().1;
                assert_eq!((&*diamond.item, diamond.count), ("minecraft:diamond", 3));
                assert_eq!(
                    diamond
                        .data()
                        .and_then(|d| d.other("minecraft:custom_data"))
                        .and_then(|t| t.get("h211")),
                    Some(&Tag::Int(1))
                );
                assert_eq!(
                    slots.iter().find(|(slot, _)| *slot == 26).unwrap().1.count,
                    5
                );
            }
            if shulker && locked {
                if dropped_lock != actual_lock.as_ref() {
                    missing.push((route, dropped_lock.cloned(), actual_lock));
                }
            } else {
                assert!(
                    dropped_lock.is_none(),
                    "non-shulker/unlocked policy unchanged"
                );
            }
            assert_eq!(game.world.game_time, clock, "zero observation ticks");
        }
        assert!(
            missing.is_empty(),
            "actual external lock lost at destruction capture sites: {missing:?}"
        );
    }
}

#[cfg(test)]
mod explicit_source_tests {
    use super::*;
    use sunrise_sim::{mobs::Mob, motion::PlayerMotion};

    #[test]
    #[ignore = "daily"]
    fn explicit_sources_keep_names_without_inventing_kill_credit() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.options.dedicated = None;
            game.creative = false;
            game.ui.difficulty = 2;
            game.build_registries();
            for mode in 0..5 {
                let nested = mode == 4;
                game.server.combat.0 = Default::default();
                game.mobs.clear();
                game.orbs.clear();
                game.dropped.clear();
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(10.5, 64.0, 8.5)).unwrap());
                let kind = game.res.mob_types().get("minecraft:husk").unwrap().clone();
                let species = kind.species;
                if mode == 2 || nested {
                    game.mobs
                        .push(Mob::new(43, kind, Vec3::new(40.0, 64.0, 40.0), 0.0));
                }
                // An enclosing blast must not leak its owner into an explicit source.
                game.server.combat.0.explosion_owner = Some(99);
                game.server
                    .combat
                    .0
                    .exploded
                    .push((99, Text::literal("Outer")));
                let cause = (mode != 0).then(|| ExplosionCause {
                    entity: (43, Text::literal("Igniter")),
                    player: false,
                    guest: None,
                    species: (mode != 1).then_some(species),
                });
                if nested {
                    game.options.dedicated = Some(Vec::new());
                    game.server.combat.0.explicit_explosion_cause = Some(cause);
                    game.server.combat.0.explicit_explosion_direct = Some(Text::literal("Outer"));
                    game.server.combat.0.blast_owner = Some((99, Text::literal("Outer")));
                    game.server.combat.0.blast_guest = Some(17);
                    let center = [8.5, 64.0, 8.5];
                    let level = game.level("minecraft:overworld").unwrap();
                    assert_eq!(
                        game.level_explode(
                            level,
                            None,
                            center,
                            2.0,
                            false,
                            mod_api::ExplosionInteraction::None,
                            mod_api::ExplosionSettings::default()
                        ),
                        Ok(())
                    );
                    assert!(matches!(
                        game.server.combat.0.explicit_explosion_cause,
                        Some(Some(_))
                    ));
                    assert!(game.server.combat.0.explicit_explosion_direct.is_some());
                    assert_eq!(
                        game.server
                            .combat
                            .0
                            .blast_owner
                            .as_ref()
                            .map(|owner| owner.0),
                        Some(99)
                    );
                    assert_eq!(game.server.combat.0.blast_guest, Some(17));
                } else {
                    game.server_explode_from_cause(
                        [8.5, 64.0, 8.5],
                        2.0,
                        BlockInteraction::Keep,
                        false,
                        ExplosionSource::default()
                            .with_entity(
                                7,
                                "minecraft:end_crystal",
                                sunrise_sim::health::DirectEntity::Other,
                            )
                            .with_silence(true),
                        Some(ExplicitBlastSource {
                            direct: (
                                7,
                                "minecraft:end_crystal",
                                sunrise_sim::health::DirectEntity::Other,
                            ),
                            cause,
                        }),
                    )
                    .unwrap();
                }
                let events = std::mem::take(&mut game.player.as_mut().unwrap().events);
                game.record_combat(&events);
                assert_eq!(
                    game.server
                        .combat
                        .0
                        .last_blow()
                        .map(|(kind, id)| (kind.to_owned(), id)),
                    Some((
                        if mode == 0 || nested {
                            "minecraft:explosion"
                        } else {
                            "minecraft:player_explosion"
                        }
                        .into(),
                        if mode == 0 || nested { 0 } else { 43 }
                    ))
                );
                assert_eq!(
                    game.server.combat.0.kill_credit(game.world.game_time),
                    (mode == 2).then_some(43)
                );
                if mode == 1 || mode == 3 || nested {
                    let death = game
                        .combat_death_message_for(Text::literal("Victim"))
                        .component();
                    if nested {
                        assert_eq!(death["with"].as_array().unwrap().len(), 1);
                    } else {
                        assert_eq!(death["with"][1]["text"].as_str(), Some("Igniter"));
                    }
                }
                assert_eq!(game.server.combat.0.explosion_owner, Some(99));
                assert_eq!(
                    game.server.combat.0.explicit_explosion_cause.is_some(),
                    nested
                );
            }
        });
    }
}

#[cfg(test)]
#[path = "tests/explosion_mob_source_tests.rs"]
mod explosion_mob_source_tests;
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
