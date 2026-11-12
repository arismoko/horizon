//! Production gameplay triggers, with an isolated game process per fixture.
use super::*;
use crate::game::minecarts::{CartKind, Minecart};
use horizon_client::save::PendingContainerLoot;
use horizon_core::{geometry::Vec3, nbt::Tag, position::BlockPos};
use horizon_world::loot::{LootData, PackOverlay};
use std::sync::{Arc, Mutex};

const POS: (i32, i32, i32) = (8, 64, 8);
const TABLE: &str = "test:first_access";

fn fresh(name: &str) -> bool {
    if std::env::var("HORIZON_CONTAINER_LOOT_TEST").as_deref() == Ok(name) {
        std::fs::write(
            std::env::var_os("HORIZON_CONTAINER_LOOT_MARKER").unwrap(),
            name,
        )
        .unwrap();
        return false;
    }
    // Game fixtures use process-global native registration. Also bound fixture
    // memory while retaining the ordinary test runner's default parallelism.
    static FIXTURES: Mutex<()> = Mutex::new(());
    let _lock = FIXTURES.lock().unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("container-loot-{}-{name}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("game::container_loot::tests::{name}"),
            "--include-ignored",
            "--nocapture",
        ])
        .env("HORIZON_CONTAINER_LOOT_TEST", name)
        .env("HORIZON_CONTAINER_LOOT_MARKER", root.join("executed"))
        .env("HORIZON_HOME", root.join("home"))
        .env("HORIZON_LAUNCHER_HOME", root.join("launcher"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("HORIZON_NO_AUDIO", "1")
        .current_dir(&root)
        .output()
        .unwrap();
    let executed = root.join("executed").exists();
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        result.status.success() && executed,
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    true
}

fn with_game(test: impl FnOnce(&mut Game<'_>)) {
    crate::game::native_guest_turn_tests::with_guests(|game| {
        game.build_registries();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
        let table = |item: &str, count: i32, bonus: i32| serde_json::json!({"pools":[{"rolls":1,"bonus_rolls":bonus,"entries":[{"type":"minecraft:item","name":item,"modifier":{"type":"minecraft:set_count","count":count}}]}]});
        let files = [
            ("loot_table/test:first_access".into(), vec![serde_json::to_vec(&table("minecraft:diamond",1,1)).unwrap()]),
            ("loot_table/test:left".into(), vec![serde_json::to_vec(&table("minecraft:diamond",1,0)).unwrap()]),
            ("loot_table/test:right".into(), vec![serde_json::to_vec(&table("minecraft:emerald",2,0)).unwrap()]),
            ("loot_table/test:entity".into(), vec![serde_json::to_vec(&serde_json::json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:emerald","condition":{"type":"minecraft:entity_properties","entity":"this","predicate":{"minecraft:entity_type":"minecraft:player"}}}]}]})).unwrap()]),
        ];
        let loot = LootData::load(
            &game.options.resources,
            &PackOverlay {
                files: &files,
                ..PackOverlay::NONE
            },
            game.res.item_defaults(),
        )
        .unwrap();
        assert!(loot.has_table(TABLE));
        game.server_loot.set(Some(Arc::new(loot)));
        test(game);
    });
}

fn place(game: &mut Game<'_>, pos: (i32, i32, i32), table: &str, seed: i64) {
    assert!(game.run_command(&format!(
        "setblock {} {} {} minecraft:chest{{LootTable:\"{table}\",LootTableSeed:{seed}L}}",
        pos.0, pos.1, pos.2
    )));
    pending(game, pos);
}
fn pending(game: &Game<'_>, pos: (i32, i32, i32)) {
    let pending = game.world.chunks[&(pos.0 >> 4, pos.2 >> 4)]
        .saved
        .as_ref()
        .unwrap()
        .pending_loot
        .get(&pos)
        .unwrap();
    assert!(matches!(pending.seed, Some(Tag::Long(37 | 91))));
    assert!(
        game.world
            .block_entities
            .get(&pos)
            .unwrap()
            .slots()
            .iter()
            .all(Option::is_none)
    );
}
fn consumed(game: &Game<'_>, pos: (i32, i32, i32)) {
    assert!(
        !game.world.chunks[&(pos.0 >> 4, pos.2 >> 4)]
            .saved
            .as_ref()
            .unwrap()
            .pending_loot
            .contains_key(&pos)
    );
}
fn count(game: &Game<'_>, pos: (i32, i32, i32), item: &str) -> u32 {
    game.world.block_entities.get(&pos).map_or(0, |e| {
        e.slots()
            .iter()
            .flatten()
            .filter(|s| &*s.item == item)
            .map(|s| s.count)
            .sum()
    })
}
fn dropped(game: &Game<'_>, item: &str) -> u32 {
    game.dropped
        .iter()
        .filter(|d| &*d.stack.item == item)
        .map(|d| d.stack.count)
        .sum()
}
fn open(game: &mut Game<'_>, pos: (i32, i32, i32), luck: f64) {
    game.as_guest(12, |g| {
        let p = g.player.as_mut().unwrap();
        let key = p.attributes.key("minecraft:luck").unwrap();
        let h = p.attributes.get_instance(Some(key)).unwrap().unwrap();
        p.attributes
            .edit(&h, |a, o| a.set_base_value(luck, o))
            .unwrap();
        assert!(
            g.native_acting_player().is_some(),
            "the opener must be admitted"
        );
        g.open_block_effect_menu([pos.0, pos.1, pos.2]);
    })
    .unwrap();
}

#[test]
#[ignore = "requires prepared resources"]
fn player_open_uses_admitted_guest_luck_and_entity() {
    if fresh("player_open_uses_admitted_guest_luck_and_entity") {
        return;
    }
    with_game(|g| {
        for (pos, luck, want) in [(POS, 2.0, 3), ((10, 64, 8), 0.0, 1)] {
            place(g, pos, TABLE, 37);
            open(g, pos, luck);
            consumed(g, pos);
            assert_eq!(count(g, pos, "minecraft:diamond"), want);
            open(g, pos, luck);
            assert_eq!(count(g, pos, "minecraft:diamond"), want);
        }
        let pos = (12, 64, 8);
        place(g, pos, "test:entity", 37);
        open(g, pos, 0.0);
        consumed(g, pos);
        assert_eq!(count(g, pos, "minecraft:emerald"), 1);
    });
}

#[test]
#[ignore = "requires prepared resources"]
fn hopper_pull_unpacks_source_once() {
    if fresh("hopper_pull_unpacks_source_once") {
        return;
    }
    with_game(|g| {
        assert!(g.run_command("setblock 8 63 8 minecraft:hopper"));
        place(g, POS, TABLE, 37);
        g.hopper_try_move_items((8, 63, 8));
        consumed(g, POS);
        assert_eq!(count(g, (8, 63, 8), "minecraft:diamond"), 1);
        g.hoppers.insert(
            (8, 63, 8),
            crate::game::machines::HopperState::with_cooldown(0),
        );
        g.hopper_try_move_items((8, 63, 8));
        assert_eq!(
            count(g, (8, 63, 8), "minecraft:diamond") + count(g, POS, "minecraft:diamond"),
            1
        );
    });
}

#[test]
#[ignore = "requires prepared resources"]
fn hopper_push_unpacks_destination_once() {
    if fresh("hopper_push_unpacks_destination_once") {
        return;
    }
    with_game(|g| {
        assert!(g.run_command("setblock 7 64 8 minecraft:hopper[facing=east]{Items:[{Slot:0b,id:\"minecraft:cobblestone\",count:1}]}"));
        place(g, POS, TABLE, 37);
        g.hopper_try_move_items((7, 64, 8));
        consumed(g, POS);
        assert_eq!(count(g, POS, "minecraft:diamond"), 1);
        assert_eq!(count(g, POS, "minecraft:cobblestone"), 1);
        assert!(g.run_command("item replace block 7 64 8 container.0 with minecraft:cobblestone"));
        g.hoppers.insert(
            (7, 64, 8),
            crate::game::machines::HopperState::with_cooldown(0),
        );
        g.hopper_try_move_items((7, 64, 8));
        assert_eq!(count(g, POS, "minecraft:diamond"), 1);
    });
}

#[test]
#[ignore = "requires prepared resources"]
fn comparator_first_read_unpacks_and_measures() {
    if fresh("comparator_first_read_unpacks_and_measures") {
        return;
    }
    with_game(|g| {
        place(g, POS, TABLE, 37);
        assert!(g.run_command("setblock 9 64 8 minecraft:comparator[facing=west]"));
        let tick = |g: &mut Game<'_>| {
            g.with_level(|b, l, t| b.scheduled_block_tick(l, t, BlockPos::new(9, 64, 8)));
        };
        tick(g);
        consumed(g, POS);
        assert_eq!(count(g, POS, "minecraft:diamond"), 1);
        assert!(
            g.block_ticks
                .comparator_outputs
                .get(&BlockPos::new(9, 64, 8))
                .copied()
                .unwrap_or(0)
                > 0
        );
        tick(g);
        assert_eq!(count(g, POS, "minecraft:diamond"), 1);
    });
}

#[test]
#[ignore = "requires prepared resources"]
fn block_break_unpacks_before_contents_drop() {
    if fresh("block_break_unpacks_before_contents_drop") {
        return;
    }
    with_game(|g| {
        place(g, POS, TABLE, 37);
        g.as_guest(12, |g| {
            g.creative = false;
            g.break_block([8, 64, 8]).unwrap();
        })
        .unwrap();
        consumed(g, POS);
        assert_eq!(dropped(g, "minecraft:diamond"), 1);
        g.as_guest(12, |g| g.break_block([8, 64, 8]).unwrap())
            .unwrap();
        assert_eq!(dropped(g, "minecraft:diamond"), 1);
    });
}

#[test]
#[ignore = "requires prepared resources"]
fn explosion_unpacks_before_contents_drop() {
    if fresh("explosion_unpacks_before_contents_drop") {
        return;
    }
    with_game(|g| {
        place(g, POS, TABLE, 37);
        let explode = |g: &mut Game<'_>| {
            g.server_explode(
                [8.5, 64.5, 8.5],
                4.0,
                crate::game::explosion::BlockInteraction::Destroy,
                false,
            )
            .unwrap()
        };
        explode(g);
        consumed(g, POS);
        assert_eq!(dropped(g, "minecraft:diamond"), 1);
        g.dropped.clear();
        explode(g);
        assert_eq!(dropped(g, "minecraft:diamond"), 0);
    });
}

#[test]
#[ignore = "requires prepared resources"]
fn item_replace_unpacks_before_writing() {
    if fresh("item_replace_unpacks_before_writing") {
        return;
    }
    with_game(|g| {
        place(g, POS, TABLE, 37);
        assert!(g.run_command("item replace block 8 64 8 container.0 with minecraft:stone"));
        consumed(g, POS);
        assert_eq!(count(g, POS, "minecraft:diamond"), 1);
        assert_eq!(
            g.world.block_entities.get(&POS).unwrap().slots()[0]
                .as_ref()
                .unwrap()
                .item
                .as_ref(),
            "minecraft:stone"
        );
        assert!(g.run_command("item replace block 8 64 8 container.0 with minecraft:stone"));
        assert_eq!(count(g, POS, "minecraft:diamond"), 1);
    });
}

#[test]
#[ignore = "requires prepared resources"]
fn double_chest_rolls_each_half_once() {
    if fresh("double_chest_rolls_each_half_once") {
        return;
    }
    with_game(|g| {
        assert!(g.run_command("setblock 8 64 8 minecraft:chest[facing=north,type=left]{LootTable:\"test:left\",LootTableSeed:37L}"));
        assert!(g.run_command("setblock 9 64 8 minecraft:chest[facing=north,type=right]{LootTable:\"test:right\",LootTableSeed:91L}"));
        pending(g, POS);
        pending(g, (9, 64, 8));
        open(g, POS, 2.0);
        consumed(g, POS);
        consumed(g, (9, 64, 8));
        assert_eq!(count(g, POS, "minecraft:diamond"), 1);
        assert_eq!(count(g, (9, 64, 8), "minecraft:emerald"), 2);
        open(g, POS, 2.0);
        assert_eq!(count(g, POS, "minecraft:diamond"), 1);
        assert_eq!(count(g, (9, 64, 8), "minecraft:emerald"), 2);
    });
}

fn cart(g: &mut Game<'_>) -> u32 {
    g.next_entity += 1;
    let id = g.next_entity;
    let mut c = Minecart::new(id, CartKind::Chest, Vec3::new(8.5, 64.0, 8.5));
    c.loot = Some(PendingContainerLoot {
        table: Tag::String(TABLE.into()),
        seed: Some(Tag::Long(37)),
    });
    assert!(c.items.iter().all(Option::is_none));
    g.carts.push(c);
    id
}
#[test]
#[ignore = "requires prepared resources"]
fn chest_minecart_open_uses_opener_once() {
    if fresh("chest_minecart_open_uses_opener_once") {
        return;
    }
    with_game(|g| {
        let id = cart(g);
        g.as_guest(12, |g| {
            let p = g.player.as_mut().unwrap();
            let k = p.attributes.key("minecraft:luck").unwrap();
            let h = p.attributes.get_instance(Some(k)).unwrap().unwrap();
            p.attributes
                .edit(&h, |a, o| a.set_base_value(2.0, o))
                .unwrap();
            let i = g.carts.iter().position(|c| c.id == id).unwrap();
            g.open_cart_container(i);
            g.open_cart_container(i);
        })
        .unwrap();
        let c = g.carts.iter().find(|c| c.id == id).unwrap();
        assert!(c.loot.is_none());
        assert_eq!(
            c.items
                .iter()
                .flatten()
                .filter(|s| &*s.item == "minecraft:diamond")
                .map(|s| s.count)
                .sum::<u32>(),
            3
        );
    });
}
#[test]
#[ignore = "requires prepared resources"]
fn chest_minecart_break_unpacks_before_drop() {
    if fresh("chest_minecart_break_unpacks_before_drop") {
        return;
    }
    with_game(|g| {
        let id = cart(g);
        for _ in 0..2 {
            if let Some(i) = g.carts.iter().position(|c| c.id == id) {
                g.hurt_cart(i, 100.0, false, false);
            }
        }
        assert!(!g.carts.iter().any(|c| c.id == id));
        assert_eq!(dropped(g, "minecraft:diamond"), 1);
    });
}

#[test]
#[ignore = "requires prepared resources"]
fn hopper_minecart_pull_unpacks_source_once() {
    if fresh("hopper_minecart_pull_unpacks_source_once") {
        return;
    }
    with_game(|g| {
        place(g, POS, TABLE, 37);
        let mut hopper = Minecart::new(900, CartKind::Hopper, Vec3::new(8.5, 62.5, 8.5));
        g.hopper_cart_suck(&mut hopper);
        consumed(g, POS);
        assert_eq!(
            hopper
                .items
                .iter()
                .flatten()
                .filter(|s| &*s.item == "minecraft:diamond")
                .map(|s| s.count)
                .sum::<u32>(),
            1
        );
        g.hopper_cart_suck(&mut hopper);
        assert_eq!(
            hopper
                .items
                .iter()
                .flatten()
                .filter(|s| &*s.item == "minecraft:diamond")
                .map(|s| s.count)
                .sum::<u32>(),
            1
        );
    });
}

#[test]
#[ignore = "requires prepared resources"]
fn dropper_push_unpacks_destination_once() {
    if fresh("dropper_push_unpacks_destination_once") {
        return;
    }
    with_game(|g| {
        assert!(g.run_command("setblock 7 64 8 minecraft:dropper[facing=east]{Items:[{Slot:0b,id:\"minecraft:cobblestone\",count:1}]}"));
        place(g, POS, TABLE, 37);
        let state = g.world.block(7, 64, 8);
        g.dispense_from(BlockPos::new(7, 64, 8), state).unwrap();
        consumed(g, POS);
        assert_eq!(count(g, POS, "minecraft:diamond"), 1);
        assert_eq!(count(g, POS, "minecraft:cobblestone"), 1);
        assert!(g.run_command("item replace block 7 64 8 container.0 with minecraft:cobblestone"));
        g.dispense_from(BlockPos::new(7, 64, 8), state).unwrap();
        assert_eq!(count(g, POS, "minecraft:diamond"), 1);
    });
}

#[test]
#[ignore = "requires prepared resources"]
fn dispenser_selection_unpacks_source_once() {
    if fresh("dispenser_selection_unpacks_source_once") {
        return;
    }
    with_game(|g| {
        assert!(g.run_command(
            "setblock 8 64 8 minecraft:dispenser{LootTable:\"test:first_access\",LootTableSeed:37L}"
        ));
        pending(g, POS);
        let state = g.world.block(8, 64, 8);
        g.dispense_from(BlockPos::new(8, 64, 8), state).unwrap();
        consumed(g, POS);
        assert_eq!(dropped(g, "minecraft:diamond"), 1);
        g.dispense_from(BlockPos::new(8, 64, 8), state).unwrap();
        assert_eq!(dropped(g, "minecraft:diamond"), 1);
    });
}

#[test]
#[ignore = "requires prepared resources"]
fn execute_if_items_unpacks_before_reading() {
    if fresh("execute_if_items_unpacks_before_reading") {
        return;
    }
    with_game(|g| {
        place(g, POS, TABLE, 37);
        for _ in 0..2 {
            assert!(g.run_command(
                "execute if items block 8 64 8 container.* minecraft:diamond run say found"
            ));
            consumed(g, POS);
            assert_eq!(count(g, POS, "minecraft:diamond"), 1);
        }
    });
}

#[test]
#[ignore = "requires prepared resources"]
fn loot_insert_unpacks_before_inserting() {
    if fresh("loot_insert_unpacks_before_inserting") {
        return;
    }
    with_game(|g| {
        place(g, POS, TABLE, 37);
        for _ in 0..2 {
            assert!(g.run_command("loot insert 8 64 8 loot {pools:[{rolls:1,entries:[{type:\"minecraft:item\",name:\"minecraft:stone\"}]}]}"));
            consumed(g, POS);
            assert_eq!(count(g, POS, "minecraft:diamond"), 1);
        }
        assert_eq!(count(g, POS, "minecraft:stone"), 2);
    });
}

#[test]
#[ignore = "requires prepared resources"]
fn detector_rail_comparator_unpacks_cart_once() {
    if fresh("detector_rail_comparator_unpacks_cart_once") {
        return;
    }
    with_game(|g| {
        assert!(g.run_command("fill 7 63 7 10 63 9 minecraft:stone"));
        let id = cart(g);
        assert!(g.run_command("setblock 8 64 8 minecraft:detector_rail[powered=true]"));
        assert!(g.carts.iter().find(|c| c.id == id).unwrap().loot.is_some());
        assert!(g.run_command("setblock 9 64 8 minecraft:comparator[facing=west]"));
        for _ in 0..2 {
            g.with_level(|b, l, t| b.scheduled_block_tick(l, t, BlockPos::new(9, 64, 8)));
            let c = g.carts.iter().find(|c| c.id == id).unwrap();
            assert!(c.loot.is_none());
            assert_eq!(
                c.items
                    .iter()
                    .flatten()
                    .filter(|s| &*s.item == "minecraft:diamond")
                    .map(|s| s.count)
                    .sum::<u32>(),
                1
            );
            assert!(
                g.block_ticks
                    .comparator_outputs
                    .get(&BlockPos::new(9, 64, 8))
                    .copied()
                    .unwrap_or(0)
                    > 0
            );
        }
    });
}

fn double(game: &mut Game<'_>) {
    assert!(game.run_command("setblock 8 64 8 minecraft:chest[facing=north,type=left]{LootTable:\"test:left\",LootTableSeed:37L}"));
    assert!(game.run_command("setblock 9 64 8 minecraft:chest[facing=north,type=right]{LootTable:\"test:right\",LootTableSeed:91L}"));
    pending(game, POS);
    pending(game, (9, 64, 8));
}
#[test]
#[ignore = "requires prepared resources"]
fn double_chest_hopper_pull_reads_only_needed_half() {
    if fresh("double_chest_hopper_pull_reads_only_needed_half") {
        return;
    }
    with_game(|g| {
        assert!(g.run_command("setblock 8 63 8 minecraft:hopper"));
        double(g);
        // Vanilla's compound visits RIGHT (x=9) before LEFT (x=8), even
        // when the hopper is under the left block. RIGHT holds two emeralds.
        g.hopper_try_move_items((8, 63, 8));
        consumed(g, (9, 64, 8));
        pending(g, POS);
        assert_eq!(count(g, (8, 63, 8), "minecraft:emerald"), 1);
        g.hoppers.insert(
            (8, 63, 8),
            crate::game::machines::HopperState::with_cooldown(0),
        );
        g.hopper_try_move_items((8, 63, 8));
        pending(g, POS);
        assert_eq!(count(g, (8, 63, 8), "minecraft:emerald"), 2);
        for _ in 0..2 {
            g.hoppers.insert(
                (8, 63, 8),
                crate::game::machines::HopperState::with_cooldown(0),
            );
            g.hopper_try_move_items((8, 63, 8));
        }
        consumed(g, POS);
        assert_eq!(count(g, (8, 63, 8), "minecraft:diamond"), 1);
        assert_eq!(count(g, (8, 63, 8), "minecraft:emerald"), 2);
    });
}

#[test]
#[ignore = "requires prepared resources"]
fn double_chest_hopper_push_keeps_other_half_pending() {
    if fresh("double_chest_hopper_push_keeps_other_half_pending") {
        return;
    }
    with_game(|g| {
        assert!(g.run_command("setblock 7 64 8 minecraft:hopper[facing=east]{Items:[{Slot:0b,id:\"minecraft:cobblestone\",count:1}]}"));
        double(g);
        g.hopper_try_move_items((7, 64, 8));
        consumed(g, (9, 64, 8));
        pending(g, POS);
        assert_eq!(count(g, (9, 64, 8), "minecraft:emerald"), 2);
        assert_eq!(count(g, (9, 64, 8), "minecraft:cobblestone"), 1);
        assert!(g.run_command("item replace block 7 64 8 container.0 with minecraft:cobblestone"));
        g.hoppers.insert(
            (7, 64, 8),
            crate::game::machines::HopperState::with_cooldown(0),
        );
        g.hopper_try_move_items((7, 64, 8));
        pending(g, POS);
        assert_eq!(count(g, (9, 64, 8), "minecraft:emerald"), 2);
    });
}

fn pending_bytes(game: &Game<'_>, pos: (i32, i32, i32)) -> Vec<u8> {
    pending(game, pos);
    let pending = game.world.block_entity_extras(pos).pending_loot.unwrap();
    let mut fields = horizon_core::nbt::Compound::new();
    pending.write_tags(&mut fields);
    horizon_core::nbt::write_named(
        &horizon_core::nbt::NamedTag {
            name: "".into(),
            tag: Tag::Compound(fields),
        },
        horizon_core::nbt::Limits::default(),
    )
    .unwrap()
}
#[test]
#[ignore = "requires prepared resources"]
fn clone_copy_preserves_pending_loot_independently() {
    if fresh("clone_copy_preserves_pending_loot_independently") {
        return;
    }
    with_game(|g| {
        place(g, POS, TABLE, 37);
        let original = pending_bytes(g, POS);
        // An overwritten destination's different table must not survive.
        place(g, (12, 64, 8), "test:right", 91);
        assert!(g.run_command("clone 8 64 8 8 64 8 12 64 8"));
        assert_eq!(pending_bytes(g, POS), original);
        assert_eq!(pending_bytes(g, (12, 64, 8)), original);
        open(g, (12, 64, 8), 0.0);
        consumed(g, (12, 64, 8));
        assert_eq!(count(g, (12, 64, 8), "minecraft:diamond"), 1);
        assert_eq!(count(g, (12, 64, 8), "minecraft:emerald"), 0);
        assert_eq!(pending_bytes(g, POS), original);
        open(g, (12, 64, 8), 0.0);
        assert_eq!(count(g, (12, 64, 8), "minecraft:diamond"), 1);
    });
}
#[test]
#[ignore = "requires prepared resources"]
fn clone_move_transfers_pending_loot() {
    if fresh("clone_move_transfers_pending_loot") {
        return;
    }
    with_game(|g| {
        place(g, POS, TABLE, 37);
        let original = pending_bytes(g, POS);
        place(g, (12, 64, 8), "test:right", 91);
        assert!(g.run_command("clone 8 64 8 8 64 8 12 64 8 replace move"));
        consumed(g, POS);
        assert!(g.world.block_entities.get(&POS).is_none());
        assert_eq!(pending_bytes(g, (12, 64, 8)), original);
        assert_eq!(dropped(g, "minecraft:diamond"), 0);
        assert_eq!(dropped(g, "minecraft:emerald"), 0);
        open(g, (12, 64, 8), 0.0);
        consumed(g, (12, 64, 8));
        assert_eq!(count(g, (12, 64, 8), "minecraft:diamond"), 1);
    });
}
#[test]
#[ignore = "requires prepared resources"]
fn structure_capture_disk_round_trip_keeps_pending_loot() {
    if fresh("structure_capture_disk_round_trip_keeps_pending_loot") {
        return;
    }
    with_game(|g| {
        crate::game::growth::install_live_features(
            &g.res,
            &g.pack,
            &g.options,
            &Default::default(),
        );
        place(g, POS, TABLE, 37);
        let original = pending_bytes(g, POS);
        // fresh() supplies a unique scratch working directory for this world.
        g.world_root = std::env::current_dir().unwrap().join("world");
        assert!(g.save_structure_area(
            "test:pending",
            [8, 64, 8],
            [1, 1, 1],
            true,
            "fixture",
            true,
            false
        ));
        let captured = g.template_entry("test:pending").unwrap();
        let Tag::List(blocks) = captured.tag.get("blocks").unwrap() else {
            panic!("captured blocks")
        };
        let nbt = blocks[0].get("nbt").expect("captured block entity");
        assert_eq!(nbt.get("LootTable"), Some(&Tag::String(TABLE.into())));
        assert_eq!(nbt.get("LootTableSeed"), Some(&Tag::Long(37)));
        g.structure_templates.clear();
        let entry = g
            .template_entry("test:pending")
            .expect("read saved structure from disk");
        assert_eq!(entry.tag, captured.tag);
        g.world
            .chunks
            .insert((1, 0), Arc::new(horizon_client::chunk::Chunk::empty(1, 0)));
        assert!(g.place_template_entry(
            &entry,
            BlockPos::new(20, 64, 8),
            crate::game::structure_blocks::load_settings(
                horizon_world::worldgen::structure::Mirror::None,
                horizon_world::worldgen::structure::Rotation::None,
                1.0,
                37,
                false
            ),
            horizon_world::worldgen::structure::Mirror::None,
            horizon_world::worldgen::structure::Rotation::None
        ));
        let placed = g
            .world
            .block_entity_extras((20, 64, 8))
            .pending_loot
            .unwrap();
        assert_eq!(placed.table, Tag::String(TABLE.into()));
        assert!(matches!(placed.seed, Some(Tag::Long(_))));
        assert!(
            g.world
                .block_entities
                .get(&(20, 64, 8))
                .unwrap()
                .slots()
                .iter()
                .all(Option::is_none)
        );
        assert_eq!(pending_bytes(g, POS), original);
        open(g, (20, 64, 8), 0.0);
        consumed(g, (20, 64, 8));
        assert_eq!(count(g, (20, 64, 8), "minecraft:diamond"), 1);
        assert_eq!(pending_bytes(g, POS), original);
        open(g, (20, 64, 8), 0.0);
        assert_eq!(count(g, (20, 64, 8), "minecraft:diamond"), 1);
    });
}

#[test]
#[ignore = "requires prepared resources"]
fn clone_selection_modes_match_pending_payloads() {
    if fresh("clone_selection_modes_match_pending_payloads") {
        return;
    }
    with_game(|g| {
        place(g, POS, TABLE, 37);
        let original = pending_bytes(g, POS);
        place(g, (13, 64, 8), "test:right", 91);
        let skipped = pending_bytes(g, (13, 64, 8));
        for mode in ["masked", "filtered minecraft:chest", "replace"] {
            place(g, (12, 64, 8), "test:right", 91);
            assert!(g.run_command(&format!("clone 8 64 8 9 64 8 12 64 8 {mode}")));
            assert_eq!(pending_bytes(g, (12, 64, 8)), original);
            assert_eq!(pending_bytes(g, POS), original);
            if mode == "replace" {
                consumed(g, (13, 64, 8));
                assert!(g.world.block_entities.get(&(13, 64, 8)).is_none());
            } else {
                assert_eq!(pending_bytes(g, (13, 64, 8)), skipped);
            }
            assert_eq!(dropped(g, "minecraft:emerald"), 0);
        }
        let before = pending_bytes(g, (12, 64, 8));
        assert!(!g.run_command("clone 8 64 8 8 64 8 12 64 8 filtered minecraft:stone"));
        assert_eq!(pending_bytes(g, (12, 64, 8)), before);
    });
}
