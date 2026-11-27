#![cfg(test)]
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
#[cfg(test)]
pub(crate) mod pumpkin_equippable_use_tests {
    use super::*;
    use std::sync::{atomic::AtomicI32, mpsc};
    use sunrise_core::position::ChunkPos;
    use sunrise_protocol::packets::{
        configuration::ClientInformation,
        play::{Serverbound, UseItem},
    };
    use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};

    #[test]
    #[ignore = "daily"]
    fn h207_connected_use_item_adopts_actual_pumpkin_equippable_and_preserves_controls() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
            assert!(game.world.loaded(8, 8));
            game.lan_host(
                sunrise_gameplay::game::lan::LanSettings {
                    port: 0,
                    guest_commands: true,
                    force_game_mode: false,
                },
                NetConfig {
                    announce: false,
                    ..NetConfig::loopback(0, "pumpkin-use-test".into(), 2)
                },
                HubConfig {
                    default_game_mode: 0,
                    guest_commands: true,
                    force_game_mode: false,
                    hardcore: false,
                    view_distance: 2,
                    simulation_distance: 2,
                    dimension: "minecraft:overworld".into(),
                    allow_flight: false,
                    seed: 0,
                    max_players: 2,
                    integrated: false,
                    levels: Default::default(),
                    level_seeds: Default::default(),
                },
            )
            .unwrap();
            let (tx, _rx) = mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 2,
                    uuid: 207,
                    name: "PumpkinGuest".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(2),
                    brand: "test".into(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:25565".parse().unwrap(),
                    expected: false,
                },
                2,
                [8.5, 200.0, 8.5],
                2,
            );
            guest.dimension = "minecraft:overworld".into();
            guest.placed = true;
            guest.loaded = true;
            guest.selected = 0;
            guest.permission = 2;
            game.server.lan.as_mut().unwrap().hub.guests.push(guest);
            let clock = game.world.game_time;
            let eq = "minecraft:equippable={slot:\"head\"}";
            let marker = "minecraft:custom_data={h207:207}";
            let helmet = "minecraft:diamond_helmet[minecraft:custom_data={old:1}]";
            let mut failures = Vec::new();
            for (i, (label, mode, component, count, old, changed)) in [
            ("empty", "survival", Some(eq), 3, None, true),
            ("swap", "survival", Some(eq), 1, Some(helmet), true),
            ("creative", "creative", Some(eq), 3, None, true),
            ("prototype", "survival", None, 3, None, false),
            ("false", "survival", Some("minecraft:equippable={slot:\"head\",swappable:0b}"), 3, None, false),
            ("binding", "survival", Some(eq), 3, Some("minecraft:diamond_helmet[minecraft:enchantments={\"minecraft:binding_curse\":1},minecraft:custom_data={old:1}]"), false),
            ("same", "survival", Some(eq), 3, Some("minecraft:carved_pumpkin[minecraft:equippable={slot:\"head\"},minecraft:custom_data={h207:207}]"), false),
        ].into_iter().enumerate() {
            let held = format!("minecraft:carved_pumpkin[{}{}{}]", component.unwrap_or(""), if component.is_some() { "," } else { "" }, marker);
            let before = game.as_guest(2, |g| {
                assert_eq!(g.ui.registries.permission, 2);
                assert!(g.run_command("clear @s") || g.inventory.iter().all(Option::is_none));
                g.run_command("item replace entity @s armor.head with air");
                g.run_command(&format!("gamemode {mode}"));
                assert!(g.run_command(&format!("item replace entity @s weapon.mainhand with {held} {count}")));
                if let Some(old) = old { assert!(g.run_command(&format!("item replace entity @s armor.head with {old}"))); }
                assert_eq!(g.player.as_ref().unwrap().position, Vec3::new(8.5, 200.0, 8.5));
                let hand = g.held().unwrap().clone();
                assert_eq!(hand.item.as_ref(), "minecraft:carved_pumpkin");
                assert_eq!(hand.count, count);
                assert!(hand.data().unwrap().other("minecraft:custom_data").is_some());
                let actual = hand.data().unwrap().other("minecraft:equippable");
                assert_eq!(actual.is_some(), component.is_some());
                if let Some(actual) = actual {
                    assert_eq!(actual.get("slot"), Some(&sunrise_core::nbt::Tag::String("head".into())));
                    assert_eq!(actual.get("swappable").and_then(sunrise_items::item_data::component_bool), if label == "false" { Some(false) } else { None });
                }
                if label == "binding" { assert_eq!(g.armor[0].as_ref().unwrap().enchantment_level("minecraft:binding_curse"), 1); }
                if label == "same" { assert!(g.armor[0].as_ref().unwrap().same_item_same_components(&hand)); }
                (g.inventory.clone(), g.armor.clone(), hand)
            }).unwrap();
            let sequence = 207 + i as i32;
            game.lan_guest_packet(2, Serverbound::UseItem(UseItem { hand: 0, yaw: 0.0, pitch: 0.0, sequence }));
            assert!(game.server.lan.as_ref().unwrap().hub.guest(2).unwrap().ack_up_to.get() >= sequence);
            game.as_guest(2, |g| {
                let mut expected_inventory = before.0.clone();
                let mut expected_armor = before.1.clone();
                if changed {
                    let mut worn = before.2.clone();
                    worn.count = 1;
                    expected_armor[0] = Some(worn);
                    if label == "swap" { expected_inventory[0] = before.1[0].clone(); }
                    else if label == "empty" { expected_inventory[0].as_mut().unwrap().count = 2; }
                }
                if g.inventory != expected_inventory || g.armor != expected_armor {
                    failures.push((label, g.inventory.clone(), g.armor.clone()));
                }
                assert_eq!(g.world.game_time, clock);
            }).unwrap();
        }
            assert!(
                failures.is_empty(),
                "actual connected UseItem Equippable adoption failures: {failures:?}"
            );
        });
    }
}

#[cfg(test)]
pub(crate) mod stand_lodestone_inventory_tick_tests {
    use sunrise_core::{geometry::Vec3, position::BlockPos};
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_sim::motion::PlayerMotion;

    #[test]
    #[ignore = "daily"]
    fn actual_stand_equipment_compasses_tick_after_loaded_lodestone_removal() {
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        game.build_registries();
        game.set_allow_commands(true);
        game.level.health = 20.0;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(14.5, 200.0, 14.5)).unwrap());
        game.block_ticks
            .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
        assert!(game.run_command("setblock 12 200 12 lodestone"));
        assert!(game.world.loaded(12, 12));
        let mut ids = Vec::new();
        for (n, dim, tracked, target) in [
            (1, "minecraft:overworld", true, true),
            (2, "minecraft:overworld", false, true),
            (3, "minecraft:the_nether", true, true),
            (4, "minecraft:overworld", true, false),
        ] {
            let who = format!("0000a551-0000-0000-0000-{n:012x}");
            let x = f64::from(n * 2) + 0.5;
            assert!(game.run_command(&format!("summon armor_stand {x} 200.0 4.5 {{UUID:[I;42321,0,0,{n}],Health:20f,NoGravity:1b}}")));
            let id = game
                .armor_stands
                .iter()
                .find(|a| game.persist.uuid_of(a.id) == Some([42321, 0, 0, n]))
                .unwrap()
                .id;
            let tracker = format!(
                "{}tracked:{tracked}",
                if target {
                    format!("target:{{dimension:\"{dim}\",pos:[I;12,200,12]}},")
                } else {
                    String::new()
                }
            );
            for hand in ["mainhand", "offhand"] {
                assert!(game.run_command(&format!("item replace entity {who} weapon.{hand} with compass[minecraft:lodestone_tracker={{{tracker}}},minecraft:custom_data={{marker:{n}}}] 1")));
            }
            let a = game.armor_stands.iter().find(|a| a.id == id).unwrap();
            assert_eq!(a.position, Vec3::new(x, 200.0, 4.5));
            assert!(a.no_gravity);
            assert!(a.first_tick, "fresh actual stand has not ticked yet");
            assert_eq!(a.health, 20.0);
            for stack in a.equipment.iter().take(2).flatten() {
                assert_eq!(&*stack.item, "minecraft:compass");
                assert_eq!(stack.count, 1);
                assert_eq!(
                    stack
                        .data()
                        .unwrap()
                        .other("minecraft:lodestone_tracker")
                        .unwrap()
                        .get("target")
                        .is_some(),
                    target
                );
            }
            assert!(game.persist.raw.remove(&id).is_some());
            ids.push((id, n, target));
        }
        for hand in ["mainhand", "offhand"] {
            assert!(game.run_command(&format!("item replace entity @s weapon.{hand} with compass[minecraft:lodestone_tracker={{target:{{dimension:\"minecraft:overworld\",pos:[I;12,200,12]}},tracked:true}}] 1")));
        }
        assert!(game.run_command("setblock 12 200 12 air"));
        assert_eq!(
            &*game.res.facts().get(game.world.block(12, 200, 12)).name,
            "minecraft:air"
        );
        game.world.game_time += 1;
        game.tick_lodestone_compasses();
        for slot in [0, sunrise_gameplay::game::shield::OFFHAND_SLOT] {
            assert!(
                game.stack_in(slot)
                    .unwrap()
                    .data()
                    .unwrap()
                    .other("minecraft:lodestone_tracker")
                    .unwrap()
                    .get("target")
                    .is_none(),
                "actual player tick positive control"
            );
        }
        assert!(
            game.entity_ticking_area()(2, 4),
            "actual entity-ticking chunk admission"
        );
        game.tick_armor_stands().unwrap();
        let mut mismatches = Vec::new();
        for (id, n, target) in ids {
            let a = game.armor_stands.iter().find(|a| a.id == id).unwrap();
            assert_eq!(a.position, Vec3::new(f64::from(n * 2) + 0.5, 200.0, 4.5));
            assert_eq!(a.health, 20.0);
            assert!(a.no_gravity);
            for (slot, stack) in a.equipment.iter().take(2).enumerate() {
                let stack = stack.as_ref().unwrap();
                assert_eq!(stack.count, 1);
                let data = stack.data().unwrap();
                assert_eq!(
                    data.other("minecraft:custom_data").unwrap().get("marker"),
                    Some(&sunrise_core::nbt::Tag::Int(n))
                );
                let present = data
                    .other("minecraft:lodestone_tracker")
                    .unwrap()
                    .get("target")
                    .is_some();
                if present != (target && n != 1) {
                    mismatches.push((n, slot, present));
                }
            }
            let saved = sunrise_sim::entity_nbt::armor_stand_to_nbt(
                a,
                [42321, 0, 0, n],
                None,
                &Default::default(),
            );
            assert!(
                saved.get("equipment").is_some(),
                "actual no-raw model writer"
            );
            for slot in 0..2 {
                let key = sunrise_sim::entities::ArmorStand::SLOTS[slot];
                let saved_target = saved
                    .get("equipment")
                    .unwrap()
                    .get(key)
                    .unwrap()
                    .get("components")
                    .unwrap()
                    .get("minecraft:lodestone_tracker")
                    .unwrap()
                    .get("target")
                    .is_some();
                let modeled_target = a.equipment[slot]
                    .as_ref()
                    .unwrap()
                    .data()
                    .unwrap()
                    .other("minecraft:lodestone_tracker")
                    .unwrap()
                    .get("target")
                    .is_some();
                assert_eq!(
                    saved_target, modeled_target,
                    "no-raw writer preserves actual tracker state"
                );
            }
        }
        assert!(
            mismatches.is_empty(),
            "actual stand equipment inventory tick mismatches: {mismatches:?}"
        );
    }
}

#[cfg(test)]
pub(crate) mod postbaseline_lodestone_controls {
    //! Added after baseline: source-backed slots and preservation, not new runtime breadth.

    use std::sync::Arc;
    use sunrise_core::position::BlockPos;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_items::items::ItemStack;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::entities::ArmorStand;

    fn stack<'g>(
        game: &'g sunrise_gameplay::game::ServerGame<'_>,
        id: u32,
        slot: usize,
    ) -> &'g ItemStack {
        game.armor_stands
            .iter()
            .find(|a| a.id == id)
            .unwrap()
            .equipment[slot]
            .as_ref()
            .unwrap()
    }
    fn has_target(stack: &ItemStack) -> bool {
        stack
            .data()
            .unwrap()
            .other("minecraft:lodestone_tracker")
            .unwrap()
            .get("target")
            .is_some()
    }
    fn item(id: &str, tracker: &str, marker: usize, count: usize) -> String {
        format!(
            "{{id:\"{id}\",count:{count},components:{{\"minecraft:lodestone_tracker\":{{{tracker}}},\"minecraft:custom_data\":{{marker:{marker}}}}}}}"
        )
    }

    #[test]
    #[ignore = "daily"]
    fn postbaseline_six_slots_noncompass_guard_and_dimension_preservation() {
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        game.build_registries();
        game.set_allow_commands(true);
        game.block_ticks
            .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
        assert!(game.run_command("setblock 12 200 12 lodestone"));
        let tracked = "target:{dimension:\"minecraft:overworld\",pos:[I;12,200,12]},tracked:true";
        let slots = ArmorStand::SLOTS
            .iter()
            .enumerate()
            .map(|(slot, key)| format!("{key}:{}", item("minecraft:compass", tracked, slot, 1)))
            .collect::<Vec<_>>()
            .join(",");
        assert!(game.run_command(&format!("summon armor_stand 4.5 200.0 4.5 {{UUID:[I;42321,0,0,100],Health:20f,NoGravity:1b,equipment:{{{slots}}}}}")));
        let mixed = format!(
            "mainhand:{},offhand:{},head:{},feet:{}",
            item("minecraft:stone", tracked, 66, 3),
            item(
                "minecraft:compass",
                "target:{dimension:\"minecraft:the_nether\",pos:[I;12,200,12]},tracked:true",
                77,
                1
            ),
            item(
                "minecraft:compass",
                "target:{dimension:\"minecraft:overworld\",pos:[I;12,200,12]},tracked:false",
                88,
                1
            ),
            item("minecraft:compass", "tracked:true", 99, 1)
        );
        assert!(game.run_command(&format!("summon armor_stand 8.5 200.0 8.5 {{UUID:[I;42321,0,0,101],Health:20f,NoGravity:1b,equipment:{{{mixed}}}}}")));
        game.world
            .chunks
            .insert((4, 4), Arc::new(Chunk::empty(4, 4)));
        game.block_ticks
            .add_empty_chunk(BlockPos::new(64, 0, 64).chunk())
            .unwrap();
        game.load_ticking_neighbourhood(0, 0);
        assert!(game.run_command("fill 64 199 64 79 199 79 stone"));
        assert!(game.run_command(&format!("summon armor_stand 65.5 200.0 65.5 {{UUID:[I;42321,0,0,102],Health:20f,NoGravity:1b,equipment:{{mainhand:{}}}}}",item("minecraft:compass",tracked,100,1))));
        let id = |n| {
            game.armor_stands
                .iter()
                .find(|a| game.persist.uuid_of(a.id) == Some([42321, 0, 0, n]))
                .unwrap()
                .id
        };
        let (all, mixed, inactive) = (id(100), id(101), id(102));
        // Direct class ticks do not refresh the headless fixture's tickets.
        // Use the existing ticket model: only chunk zero is forced to tick.
        game.server
            .tickets
            .0
            .update(Some(sunrise_sim::ticket_levels::Sources {
                forced: vec![(0, 0)],
                ..Default::default()
            }));
        assert!(game.entity_ticking_area()(4, 4));
        assert!(!game.entity_ticking_area()(65, 65));
        assert!(game.world.loaded(65, 65));
        let before: Vec<_> = (0..6)
            .map(|slot| {
                game.armor_stands
                    .iter()
                    .find(|a| a.id == mixed)
                    .unwrap()
                    .equipment[slot]
                    .as_ref()
                    .map(sunrise_core::nbt::Tag::from)
            })
            .collect();
        assert!(has_target(stack(&game, mixed, 0)));
        assert!(has_target(stack(&game, mixed, 1)));
        assert!(has_target(stack(&game, mixed, 5)));
        assert!(!has_target(stack(&game, mixed, 2)));
        assert!(has_target(stack(&game, inactive, 0)));
        for slot in 0..6 {
            assert!(has_target(stack(&game, all, slot)));
        }
        let dimension = game.dim;
        let chunks: std::collections::HashSet<_> = game.world.chunks.keys().copied().collect();
        assert!(game.run_command("setblock 12 200 12 air"));
        game.world.game_time += 1;
        game.tick_armor_stands().unwrap();
        assert!(
            !game
                .armor_stands
                .iter()
                .find(|a| a.id == all)
                .unwrap()
                .first_tick,
            "active stand really ticked"
        );
        assert!(
            game.armor_stands
                .iter()
                .find(|a| a.id == inactive)
                .unwrap()
                .first_tick,
            "non-ticking guard unchanged"
        );
        for slot in 0..6 {
            let s = stack(&game, all, slot);
            assert!(!has_target(s), "source-backed equipment slot {slot}");
            assert_eq!(s.count, 1);
            assert_eq!(
                s.data()
                    .unwrap()
                    .other("minecraft:custom_data")
                    .unwrap()
                    .get("marker"),
                Some(&sunrise_core::nbt::Tag::Int(slot as i32))
            );
        }
        let after: Vec<_> = (0..6)
            .map(|slot| {
                game.armor_stands
                    .iter()
                    .find(|a| a.id == mixed)
                    .unwrap()
                    .equipment[slot]
                    .as_ref()
                    .map(sunrise_core::nbt::Tag::from)
            })
            .collect();
        assert_eq!(
            before, after,
            "actual non-compass/untracked/other-dimension/targetless stacks preserved"
        );
        assert!(has_target(stack(&game, inactive, 0)));
        assert_eq!(game.dim, dimension);
        assert_eq!(
            game.world
                .chunks
                .keys()
                .copied()
                .collect::<std::collections::HashSet<_>>(),
            chunks
        );
    }
}
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
