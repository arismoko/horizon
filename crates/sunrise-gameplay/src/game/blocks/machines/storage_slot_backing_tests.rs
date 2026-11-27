//! `ItemsBackend::container_slot_backing` and `ResourceHandler::backing` on the game's own
//! containers: `WorldlyContainer.getSlotsForFace` picks the slot a side reaches,
//! `ChestBlock.getContainer` / `CompoundContainer` join a double chest, and
//! `ComposterBlock.getContainer` builds a container per access, so a composter has no backing.
//! The double chest's open menu (`MenuRef::slot_backing`) and its `ItemPlace::MenuSlot` places
//! report the same slots as its sides.
use super::*;
use crate::game::{ServerGame, game_tests::tests::server_game};
use mod_api::{ITEMS, ServerHost, StorageSlot};
use std::sync::Arc;
use sunrise_core::{
    geometry::{Direction, Vec3},
    position::BlockPos,
};
use sunrise_gameplay_common::game::{
    GameGameExt1 as _, Resources, chat_common::ChatCommonGameExt1 as _,
    native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _,
    native_menu_refs_common::NativeMenuRefsCommonGameExt1 as _,
    native_server::NativeServerGameExt1 as _,
};
use sunrise_level::chunk::Chunk;
use sunrise_sim::motion::PlayerMotion;

const LEFT: [i32; 3] = [0, 64, 0];
const RIGHT: [i32; 3] = [1, 64, 0];
const FURNACE: [i32; 3] = [4, 64, 4];
const COMPOSTER: [i32; 3] = [8, 64, 8];
/// The guest who opens the double chest (`native_item_places::tests::guest`), and their UUID.
const GUEST: u32 = 1;
const GUEST_UUID: u128 = 123;

fn game<'a>(resources: &'a mut Option<Arc<Resources>>) -> ServerGame<'a> {
    let mut game = server_game(resources);
    game.build_registries();
    game.set_allow_commands(true);
    for x in -1..=1 {
        for z in -1..=1 {
            let mut chunk = Chunk::empty(x, z);
            chunk.lit = true;
            game.world.chunks.insert((x, z), Arc::new(chunk));
            game.block_ticks
                .add_empty_chunk(BlockPos::new(x * 16, 0, z * 16).chunk())
                .unwrap();
        }
    }
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 64.0, 8.5)).unwrap());
    for command in [
        "forceload add 0 0",
        "fill 0 63 0 15 63 15 stone",
        "setblock 0 64 0 minecraft:chest[type=left,facing=north]",
        "setblock 1 64 0 minecraft:chest[type=right,facing=north]",
        "setblock 4 64 4 minecraft:furnace",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    game.update_chunk_tickets();
    game
}

/// `game` opened to the LAN, with `GUEST` online to open the double chest's menu.
fn host_game<'a>(resources: &'a mut Option<Arc<Resources>>) -> ServerGame<'a> {
    use crate::game::native_item_places::tests::open_to_lan;
    use sunrise_gameplay_common::game::server_side::native_item_places::tests::guest;

    let mut game = game(resources);
    open_to_lan(&mut game);
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .push(guest(GUEST));
    game
}

fn side(game: &ServerGame<'_>, pos: [i32; 3], side: Option<Direction>) -> BlockSide {
    BlockSide {
        level: game.level("minecraft:overworld").unwrap(),
        pos,
        side,
    }
}

/// What the `ITEMS` handler at `pos` reached from `face` reports for each of its indices.
fn backing(
    game: &mut ServerGame<'_>,
    pos: [i32; 3],
    face: Option<Direction>,
) -> Option<Vec<Option<StorageSlot>>> {
    let level = game.level("minecraft:overworld").unwrap();
    let handler = level.capability(game, &ITEMS, pos, face)?;
    let host: &mut dyn ServerHost = game;
    let size = handler.size(host);
    Some(
        (0..size)
            .map(|index| handler.backing(host, index))
            .collect(),
    )
}

#[test]
#[ignore = "daily"]
fn a_double_chests_halves_report_the_same_slots_each_with_its_owning_half() {
    let mut resources = None;
    let mut game = game(&mut resources);
    let level = game.level("minecraft:overworld").unwrap();
    let from_left = backing(&mut game, LEFT, None).unwrap();
    let from_right = backing(&mut game, RIGHT, None).unwrap();
    assert_eq!(from_left.len(), 54);
    assert_eq!(from_left, from_right);
    let first = from_left[0].unwrap().pos;
    let second = from_left[27].unwrap().pos;
    assert_ne!(first, second);
    let mut halves = [first, second];
    halves.sort();
    assert_eq!(halves, [LEFT, RIGHT]);
    for (index, slot) in from_left.iter().enumerate() {
        let pos = [first, second][index / 27];
        assert_eq!(
            *slot,
            Some(StorageSlot {
                level,
                pos,
                slot: index % 27
            })
        );
        assert_eq!(
            game.container_slot_backing(side(&game, LEFT, None), index),
            *slot
        );
    }
    assert_eq!(
        game.container_slot_backing(side(&game, LEFT, None), 54),
        None
    );
}

/// The open double chest menu (`ChestMenu` over `ChestBlock.getContainer`'s `CompoundContainer`):
/// `MenuRef::slot_backing` and `ItemPlace::MenuSlot::backing` report each container slot as the
/// owning half's own slot, the one the sides report. `ChestBlock.getBlockType` makes the right
/// half `FIRST`, so the compound container lists its 27 slots before the left's, from whichever
/// half the menu was opened.
#[test]
#[ignore = "daily"]
fn an_open_double_chest_menu_backs_each_slot_as_its_sides_do_until_it_closes() {
    use mod_api::{ItemPlace, MenuRef, MenuSlotBacking};
    use sunrise_gameplay_common::game::lan_actions::MenuOpen;

    let mut resources = None;
    let mut game = host_game(&mut resources);
    let level = game.level("minecraft:overworld").unwrap();
    let player = game.player(GUEST_UUID).unwrap();
    let sides = backing(&mut game, LEFT, None).unwrap();
    assert_eq!(sides.len(), 54);
    let mut opened: Option<MenuRef> = None;
    for [x, y, z] in [LEFT, RIGHT] {
        assert_eq!(game.open_block_menu(GUEST, (x, y, z)), MenuOpen::Opened);
        let menu = game.player_container_menu(player).unwrap();
        // The 54 container slots, then the player's 36.
        assert_eq!(menu.slot_count(&game), Some(90));
        for index in 0..54 {
            let storage = StorageSlot {
                level,
                pos: [RIGHT, LEFT][usize::from(index >= 27)],
                slot: index % 27,
            };
            assert_eq!(sides[index], Some(storage));
            assert_eq!(
                game.container_slot_backing(side(&game, LEFT, None), index),
                Some(storage)
            );
            assert_eq!(
                menu.slot_backing(&game, index),
                Some(MenuSlotBacking::BlockEntity {
                    level,
                    pos: storage.pos,
                    slot: storage.slot,
                }),
                "menu slot {index}"
            );
            let place = ItemPlace::MenuSlot { menu, slot: index };
            assert_eq!(place.backing(&mut game), Some(storage), "menu slot {index}");
            let strict = ItemPlace::Strict(Box::new(place));
            assert_eq!(
                strict.backing(&mut game),
                Some(storage),
                "menu slot {index}"
            );
            let one_by_one = ItemPlace::OneByOne(Box::new(strict));
            assert_eq!(
                one_by_one.backing(&mut game),
                Some(storage),
                "menu slot {index}"
            );
        }
        // The player's inventory in the same menu backs no block slot: menu slot 54 is the
        // inventory's slot 9. Nor does a slot past the menu.
        assert_eq!(
            menu.slot_backing(&game, 54),
            Some(MenuSlotBacking::Player(9))
        );
        for slot in 54..90 {
            assert!(
                matches!(
                    menu.slot_backing(&game, slot),
                    Some(MenuSlotBacking::Player(_))
                ),
                "menu slot {slot}"
            );
            assert_eq!(
                ItemPlace::MenuSlot { menu, slot }.backing(&mut game),
                None,
                "menu slot {slot}"
            );
        }
        assert_eq!(menu.slot_backing(&game, 90), None);
        assert_eq!(
            ItemPlace::MenuSlot { menu, slot: 90 }.backing(&mut game),
            None
        );
        // `ServerPlayer.openMenu` closed the menu that was open: its handle reaches nothing.
        if let Some(replaced) = opened {
            assert_eq!(
                ItemPlace::MenuSlot {
                    menu: replaced,
                    slot: 0
                }
                .backing(&mut game),
                None
            );
        }
        opened = Some(menu);
    }
    let menu = opened.unwrap();
    // Closed: the handle outlives the menu but reaches nothing it showed.
    game.guest_close_all(GUEST);
    assert!(
        game.server.lan.as_ref().unwrap().hub.guests[0]
            .menu
            .is_none()
    );
    for slot in [0, 54] {
        assert_eq!(menu.slot_backing(&game, slot), None, "menu slot {slot}");
        assert_eq!(
            ItemPlace::MenuSlot { menu, slot }.backing(&mut game),
            None,
            "menu slot {slot}"
        );
    }
    // What remains is the player's own inventory menu, which no block entity backs.
    let inventory = game.player_container_menu(player).unwrap();
    assert_eq!(inventory.container_id(&game), Some(0));
    for slot in 0..46 {
        assert_eq!(
            ItemPlace::MenuSlot {
                menu: inventory,
                slot
            }
            .backing(&mut game),
            None,
            "inventory menu slot {slot}"
        );
    }
}

#[test]
#[ignore = "daily"]
fn a_furnaces_sides_share_a_slot_and_its_other_faces_stay_distinct() {
    let mut resources = None;
    let mut game = game(&mut resources);
    let level = game.level("minecraft:overworld").unwrap();
    let slot = |slot| {
        Some(StorageSlot {
            level,
            pos: FURNACE,
            slot,
        })
    };
    let up = backing(&mut game, FURNACE, Some(Direction::Up)).unwrap();
    let down = backing(&mut game, FURNACE, Some(Direction::Down)).unwrap();
    assert_eq!(up, [slot(0)]);
    assert_eq!(down, [slot(2), slot(1)]);
    for face in [
        Direction::North,
        Direction::East,
        Direction::South,
        Direction::West,
    ] {
        let horizontal = backing(&mut game, FURNACE, Some(face)).unwrap();
        assert_eq!(horizontal, [slot(1)]);
        assert_eq!(horizontal[0], down[1]);
    }
    assert_ne!(up[0], down[0]);
}

#[test]
#[ignore = "daily"]
fn a_composter_has_no_backing_on_any_face_at_any_level() {
    let mut resources = None;
    let mut game = game(&mut resources);
    let [x, y, z] = COMPOSTER;
    for level in [0, 3, 7, 8] {
        assert!(
            game.run_command(&format!(
                "setblock {x} {y} {z} minecraft:composter[level={level}]"
            )),
            "level {level}"
        );
        for face in [
            None,
            Some(Direction::Up),
            Some(Direction::Down),
            Some(Direction::North),
            Some(Direction::East),
            Some(Direction::South),
            Some(Direction::West),
        ] {
            assert_eq!(
                game.container_slot_backing(side(&game, COMPOSTER, face), 0),
                None,
                "level {level} face {face:?}"
            );
            if let Some(reported) = backing(&mut game, COMPOSTER, face) {
                assert!(reported.iter().all(Option::is_none), "level {level}");
            }
        }
    }
}
