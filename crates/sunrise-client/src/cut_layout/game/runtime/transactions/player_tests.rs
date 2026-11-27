use mod_api::{AttachmentAccess, AttachmentHolder, Host, PlayerRef, ServerHost};
use std::sync::{Arc, Mutex};
use sunrise_core::nbt::{Compound, Tag};
use sunrise_items::items::ItemStack;
use sunrise_mod_runtime::attachments::{
    AttachmentCodec, AttachmentError, AttachmentType, Holder, index::AttachmentIndex,
};
use {
    sunrise_gameplay::game::native_guest_turn_tests::with_guests,
    sunrise_gameplay_common::game::Dim,
};

const VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "player_test",
    name: "value",
    default: || 0,
    codec: Some(AttachmentCodec {
        encode: |value| Ok(Compound::from([("n".into(), Tag::Int(*value))])),
        decode: |data| match data.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: true,
    sync_codec: None,
};
const OTHER: AttachmentType<i32> = AttachmentType {
    name: "other",
    ..VALUE
};

fn rows(index: &Arc<Mutex<AttachmentIndex<AttachmentHolder>>>, name: &str) -> Vec<(u128, i32)> {
    let snapshot = index
        .lock()
        .unwrap()
        .snapshot(&("player_test".into(), name.into()));
    let mut values: Vec<_> = snapshot
        .iter()
        .map(|row| {
            let AttachmentHolder::Player(player) = row.holder else {
                panic!("not a player")
            };
            let ty = if name == "value" { &VALUE } else { &OTHER };
            (player.uuid(), *row.entry.read(ty).unwrap())
        })
        .collect();
    values.sort_unstable();
    values
}

fn write_player_storage(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    player: PlayerRef,
    count: u32,
) {
    let host: &dyn Host = game;
    let stack = host
        .item_from_snbt(&format!(r#"{{id:"minecraft:stone",count:{count}}}"#))
        .unwrap();
    player
        .set_inventory_slot(game, 0, Some(stack.clone()))
        .unwrap();
    player.set_ender_chest_slot(game, 0, Some(stack)).unwrap();
    player
        .set_persistent_data_key(game, &"value".into(), Some(&Tag::Int(count as i32)))
        .unwrap();
}

fn assert_player_storage(
    saved: Option<&Tag>,
    inventory: &[Option<ItemStack>],
    ender: &[Option<ItemStack>],
    count: u32,
) {
    let value = saved
        .and_then(|saved| saved.get("NeoForgeData"))
        .and_then(|data| data.get("value"));
    assert_eq!(value, Some(&Tag::Int(count as i32)));
    for slots in [inventory, ender] {
        let stack = slots[0].as_ref().unwrap();
        assert_eq!(stack.item.as_ref(), "minecraft:stone");
        assert_eq!(stack.count, count);
    }
}

#[test]
#[ignore = "daily"]
fn abort_restores_fake_players_with_the_same_uuid_in_different_levels() {
    with_guests(|game| {
        let overworld = game.level("minecraft:overworld").unwrap();
        let outside = game.fake_player(overworld, 99, "test").unwrap();
        assert!(game.enter_command_level(Dim::Nether).is_some());
        let nether = game.level("minecraft:the_nether").unwrap();
        let inside = game.fake_player(nether, 99, "test").unwrap();
        assert_ne!(outside, inside);
        write_player_storage(game, outside, 3);
        write_player_storage(game, inside, 5);

        let result: Result<(), ()> = game.storage_transaction(|game, _| {
            write_player_storage(game, outside, 7);
            write_player_storage(game, inside, 11);
            Err(())
        });
        assert_eq!(result, Err(()));
        let guests = &game.server.lan.as_ref().unwrap().hub.guests;
        for (player, count) in [(outside, 3), (inside, 5)] {
            let guest = &guests[guests.identity_index(99, player.entity().id()).unwrap()];
            assert_player_storage(
                guest.saved.as_ref(),
                &guest.inventory,
                &guest.ender_items,
                count,
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn abort_restores_player_inventory_and_data_after_the_acting_player_dies() {
    with_guests(|game| {
        let player = game.player(12).unwrap();
        write_player_storage(game, player, 13);
        game.as_guest(12, |game| {
            let result: Result<(), ()> = game.storage_transaction(|game, _| {
                write_player_storage(game, player, 17);
                // Health itself is not a journaled write in this case.
                game.player
                    .as_mut()
                    .unwrap()
                    .set_health(0.0, false)
                    .unwrap();
                Err(())
            });
            assert_eq!(result, Err(()));
            assert!(game.native_live_acting_body().is_none());
            assert_player_storage(
                game.level.player_extra.as_ref(),
                &game.inventory,
                game.test_ender_items(),
                13,
            );
        })
        .unwrap();
    });
}

#[test]
#[ignore = "daily"]
fn abort_restores_the_suspended_guest_without_touching_the_inner_guest() {
    with_guests(|game| {
        let outer = game.player(12).unwrap();
        let inner = game.player(13).unwrap();
        write_player_storage(game, outer, 19);
        write_player_storage(game, inner, 23);
        game.as_guest(12, |game| {
            game.open_storage_transaction();
            write_player_storage(game, outer, 29);
            game.as_guest(13, |game| {
                assert_eq!(game.test_lan_guest_swaps(), [12, 13]);
                game.close_storage_transaction(false);
                assert_player_storage(
                    game.level.player_extra.as_ref(),
                    &game.inventory,
                    game.test_ender_items(),
                    23,
                );
            })
            .unwrap();
            assert_player_storage(
                game.level.player_extra.as_ref(),
                &game.inventory,
                game.test_ender_items(),
                19,
            );
        })
        .unwrap();
    });
}

#[test]
#[ignore = "daily"]
fn nested_player_turns_preserve_binding_and_rollback_whole_membership() {
    with_guests(|game| {
        game.options.dedicated = Some(Vec::new());
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_all(|g| g.uuid += 100);
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let mut bindings = Vec::new();
        for (conn, uuid, value) in [(12, 112, 7), (13, 113, 8)] {
            let player = game.player(uuid).unwrap();
            let map = game
                .attachment_map_quiet(Holder::Player(uuid), AttachmentAccess::Existing)
                .unwrap();
            VALUE.set(map, Holder::Level, value).unwrap();
            if uuid == 112 {
                OTHER.set(map, Holder::Level, 4).unwrap();
            }
            let storage = game
                .server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .attachments_mut(conn)
                .unwrap();
            bindings.push(storage.bind_index(&index, AttachmentHolder::Player(player)));
        }
        let mut saved = game
            .test_attachment_slot(Holder::Player(112), AttachmentAccess::Existing)
            .unwrap()
            .clone();
        let saved_nbt = saved.try_to_nbt().unwrap();
        let before = index
            .lock()
            .unwrap()
            .snapshot(&("player_test".into(), "value".into()));
        game.open_storage_transaction();
        game.as_guest(12, |game| {
            let host: &mut dyn ServerHost = game;
            VALUE.set(host, Holder::Player(112), 9).unwrap();
            OTHER.remove(host, Holder::Player(112)).unwrap();
            game.as_guest(13, |game| {
                assert!(
                    game.test_attachment_slot(Holder::Player(112), AttachmentAccess::Existing)
                        .is_err()
                );
                game.open_storage_transaction();
                let host: &mut dyn ServerHost = game;
                VALUE.set(host, Holder::Player(113), 88).unwrap();
                OTHER.set(host, Holder::Player(113), 6).unwrap();
                game.close_storage_transaction(false);
                assert_eq!(rows(&index, "value"), [(112, 9), (113, 8)]);
                assert!(rows(&index, "other").is_empty());
            })
            .unwrap();
            assert_eq!(rows(&index, "value"), [(112, 9), (113, 8)]);
        })
        .unwrap();
        game.close_storage_transaction(false);
        assert_eq!(rows(&index, "value"), [(112, 7), (113, 8)]);
        assert_eq!(rows(&index, "other"), [(112, 4)]);
        assert_eq!(VALUE.get_existing(&mut saved, Holder::Level), Ok(Some(7)));
        assert_eq!(saved.try_to_nbt().unwrap(), saved_nbt);
        assert_eq!(before[0].entry.read(&VALUE), Ok(&7));
        for binding in bindings {
            assert!(
                binding.unregister(),
                "guest turns and rollback never rebind"
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn retained_player_records_and_save_snapshots_do_not_keep_residency() {
    with_guests(|game| {
        game.options.dedicated = Some(Vec::new());
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_all(|g| g.uuid += 100);
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        for (conn, uuid) in [(12, 112), (13, 113)] {
            let player = game.player(uuid).unwrap();
            let map = game
                .test_attachment_slot(Holder::Player(uuid), AttachmentAccess::Existing)
                .unwrap();
            VALUE.set(map, Holder::Level, uuid as i32).unwrap();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .attachments_mut(conn)
                .unwrap()
                .bind_index(&index, AttachmentHolder::Player(player));
        }
        let before = index
            .lock()
            .unwrap()
            .snapshot(&("player_test".into(), "value".into()));
        let mut saved = game
            .test_attachment_slot(Holder::Player(112), AttachmentAccess::Existing)
            .unwrap()
            .clone();
        let mut removed = game.server.lan.as_mut().unwrap().hub.guests.remove(0);
        assert_eq!(rows(&index, "value"), [(113, 113)]);
        VALUE
            .set(&mut removed.attachments, Holder::Level, 99)
            .unwrap();
        assert_eq!(rows(&index, "value"), [(113, 113)]);
        assert_eq!(VALUE.get_existing(&mut saved, Holder::Level), Ok(Some(112)));
        game.server.lan.as_mut().unwrap().hub.guests.clear();
        assert!(rows(&index, "value").is_empty());
        assert_eq!(before[0].entry.read(&VALUE), Ok(&112));
        assert_eq!(before[1].entry.read(&VALUE), Ok(&113));
    });
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
