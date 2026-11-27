use super::*;

#[test]
#[ignore = "daily"]
fn inventory_repair_does_not_replace_an_open_menus_predicted_cursor() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use sunrise_core::nbt::Tag;
    use sunrise_protocol::{
        item::Stack,
        packets::play::{self, Clientbound as C},
    };

    with_client_game(|game| {
        let (mut reader, _writer) = connect_menu_test(game);
        spawn_menu_test(game);
        let table = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:menu")
            .unwrap()
            .id("minecraft:crafting")
            .unwrap();
        let stone = game.item_id("minecraft:stone");
        let stick = game.item_id("minecraft:stick");
        game.handle_packet(C::OpenScreen(play::OpenScreen {
            container: 7,
            menu: table,
            title: Tag::String("Crafting".into()),
        }))
        .unwrap();
        let mut items = vec![None; 46];
        items[2] = Some(Stack::new(stone, 4));
        game.handle_packet(C::ContainerSetContent(play::ContainerSetContent {
            container: 7,
            state: 12,
            items,
            carried: None,
        }))
        .unwrap();
        let menu = game
            .player_container_menu(game.local_player().unwrap())
            .unwrap();
        let pickup = mod_api::MenuClick::Pickup {
            slot: Some(2),
            button: mod_api::MouseButton::Left,
        };
        assert!(menu.click(game, std::slice::from_ref(&pickup)));
        assert_eq!(menu.slot(game, 2), Some(None));

        // An in-flight inventory repair arrives before the server can acknowledge the pickup.
        let mut items = vec![None; 46];
        items[2] = Some(Stack::new(stick, 2));
        game.handle_packet(C::ContainerSetContent(play::ContainerSetContent {
            container: 0,
            state: 45,
            items,
            carried: Some(Stack::new(stick, 1)),
        }))
        .unwrap();
        assert_eq!(menu.carried(game).unwrap().unwrap().count(), 4);
        assert_eq!(menu.slot(game, 2), Some(None));
        assert!(menu.click(game, &[pickup]));
        assert_eq!(menu.slot(game, 2).unwrap().unwrap().count(), 4);
        assert_eq!(menu.carried(game), Some(None));
        for count in [Some(4), None] {
            let click = (0..32)
                .find_map(|_| {
                    let (id, body) = reader.recv().unwrap();
                    match play::Serverbound::decode(id, &body).unwrap() {
                        play::Serverbound::ContainerClick(click) => Some(click),
                        _ => None,
                    }
                })
                .unwrap();
            assert_eq!((click.container, click.state, click.slot), (7, 12, 2));
            assert_eq!(click.carried.map(|hash| hash.count), count);
        }
        game.handle_packet(C::ContainerClose(play::ContainerClose { container: 7 }))
            .unwrap();
        assert_eq!(
            game.carried.as_ref().unwrap().item.as_ref(),
            "minecraft:stick"
        );
        assert_eq!(game.client.remote.as_ref().unwrap().state_id, 45);
        let inventory = game
            .player_container_menu(game.local_player().unwrap())
            .unwrap();
        assert_eq!(inventory.slot(game, 2).unwrap().unwrap().count(), 2);
    });
}

#[test]
#[ignore = "daily"]
fn retained_effects_rebind_on_reconfiguration_and_refusals_do_not_block_clicks() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use std::sync::Arc;
    use sunrise_core::{nbt::Tag, vanilla_bytes::Writer};
    use sunrise_protocol::{
        buf::{Codec, Reader},
        item::{Component, Hashed, Slot, Stack, component, consume_effects as stream},
        packets::{
            configuration::{RegistryData, RegistryEntry},
            play,
        },
        registry::{Dynamic, SyncedNames},
    };
    use sunrise_world::component_value::consume_effects::{self, Type};

    consume_effects::with_test_registry(|| {
        consume_effects::install(
            ["reconfigure:first", "reconfigure:second"]
                .into_iter()
                .map(|name| {
                    (
                        Arc::from(name),
                        Type {
                            schema: 17,
                            load: Box::new(|_| {
                                unreachable!("a retained network value must not reload NBT")
                            }),
                            read: Box::new(|bytes, project| {
                                let byte = *bytes.first().ok_or("missing charge")?;
                                // Emulate the mod save adapter refusing a value but consuming its Wire byte.
                                Ok((
                                    1,
                                    (project && byte != 0xff).then(|| {
                                        [("charge".into(), Tag::Int(i32::from(byte)))].into()
                                    }),
                                ))
                            }),
                        },
                    )
                })
                .collect(),
        )
        .unwrap();
        let configure = |swap: bool| {
            let mut entries: Vec<_> = stream::configuration()
                .map(|(id, schema)| RegistryEntry {
                    id: id.into(),
                    data: Some(Tag::Long(schema as i64)),
                })
                .collect();
            if swap {
                entries.swap(5, 6);
            }
            let mut registry = Dynamic::default();
            registry
                .add_consume_effects(RegistryData {
                    registry: stream::REGISTRY.into(),
                    entries,
                })
                .unwrap();
            Arc::new(registry)
        };
        with_client_game(|game| {
            let (mut reader, _writer) = connect_menu_test(game);
            spawn_menu_test(game);
            let item = game.item_id("minecraft:stone");
            let protection = |count, bytes| {
                let mut stack = Stack::new(item, count);
                stack
                    .added
                    .push((component::DEATH_PROTECTION, Component::Raw(bytes)));
                stack
            };
            let receive = |stack, mapping: &Dynamic| {
                let mut wire = Writer::new();
                Slot::write(&Some(stack), &mut wire);
                Slot::read(&mut Reader::new(&wire.data).with_names(mapping)).unwrap()
            };
            let old = configure(false);
            game.set_registries(old.clone()).unwrap();
            let retained = game.to_stack(&receive(protection(2, vec![1, 5, 7]), &old));
            assert!(retained.as_ref().unwrap().wire_patch().is_some());
            let new = configure(true);
            game.set_registries(new.clone()).unwrap();
            let sent = game.to_network(&retained).unwrap();
            assert_eq!(
                sent.component(component::DEATH_PROTECTION),
                Some(&Component::Raw(vec![1, 6, 7]))
            );
            let expected_hash = |stack, game: &sunrise_client_gameplay::game::ClientGame<'_>| {
                let names = SyncedNames {
                    builtin: &game.client.remote.as_ref().unwrap().builtin,
                    dynamic: &new,
                };
                Hashed::create(&stack, &names).unwrap()
            };
            assert_eq!(
                game.hashed_stack(&retained).unwrap(),
                Some(expected_hash(protection(2, vec![1, 6, 7]), game))
            );
            assert_ne!(
                game.hashed_stack(&retained).unwrap(),
                Some(expected_hash(protection(2, vec![1, 5, 7]), game))
            );

            // The refused custom effect precedes a valid built-in. It must not poison
            // structural framing or either side's hash of the ordinary survivor.
            let received = receive(protection(2, vec![2, 6, 0xff, 2]), &new);
            let projected = expected_hash(protection(2, vec![1, 2]), game);
            assert_eq!(expected_hash(received.clone().unwrap(), game), projected);
            game.carried = game.to_stack(&received);
            assert_eq!(game.hashed_stack(&game.carried).unwrap(), Some(projected));
            let menu = game
                .local_player()
                .and_then(|player| game.player_container_menu(player))
                .unwrap();
            assert!(menu.click(
                game,
                &[mod_api::MenuClick::Pickup {
                    slot: Some(36),
                    button: mod_api::MouseButton::Left
                }]
            ));
            let (id, body) = reader.recv().unwrap();
            let play::Serverbound::ContainerClick(click) =
                play::Serverbound::decode(id, &body).unwrap()
            else {
                panic!("expected an ordinary click with the admitted projection");
            };
            assert_eq!(click.slot, 36);
            assert!(click.carried.is_none());
            assert_eq!(
                click.changed,
                vec![(36, Some(expected_hash(protection(2, vec![1, 2]), game)))]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn client_menu_kind_names_the_server_menu_type() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::tests::client_menu_tests::client_menu_kind_names_the_server_menu_type",
        || {
            use crate::game::server_side::native_entity_refs::tests::with_client_game;
            use sunrise_core::nbt::Tag;
            use sunrise_protocol::packets::play::{self, Clientbound as C};

            with_client_game(|game| {
                let (_reader, _writer) = connect_menu_test(game);
                spawn_menu_test(game);
                let live = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
                    game.local_player()
                        .and_then(|player| game.player_container_menu(player))
                        .unwrap()
                };
                // InventoryMenu has no MenuType.
                let mut old = live(game);
                assert_eq!(old.kind(game), Some(None));
                for (window, id) in [
                    (1, "minecraft:furnace"),
                    (2, "minecraft:blast_furnace"),
                    (3, "minecraft:smoker"),
                    (4, "minecraft:brewing_stand"),
                    (5, "minecraft:stonecutter"),
                    (6, "minecraft:smithing"),
                    (7, "minecraft:generic_9x3"),
                ] {
                    let menu = game
                        .client
                        .remote
                        .as_ref()
                        .unwrap()
                        .builtin
                        .get("minecraft:menu")
                        .unwrap()
                        .id(id)
                        .unwrap();
                    game.handle_packet(C::OpenScreen(play::OpenScreen {
                        container: window,
                        menu,
                        title: Tag::String(id.into()),
                    }))
                    .unwrap();
                    let open = live(game);
                    assert_eq!(open.kind(game), Some(Some(id.to_owned())), "{id}");
                    // ScreenView.menu reads the same open menu name.
                    assert_eq!(game.remote_menu_type(), Some(id));
                    assert_eq!(old.kind(game), None, "the replaced menu's handle is stale");
                    old = open;
                }
                game.handle_packet(C::ContainerClose(play::ContainerClose { container: 7 }))
                    .unwrap();
                assert_eq!(old.kind(game), None, "a closed menu's handle is stale");
                // HorseInventoryMenu is opened by MountScreenOpen, which names no MenuType.
                game.remote_set_menu(
                    Some((
                        8,
                        format!("{}3", sunrise_gameplay_common::game::remote::MOUNT_MENU),
                    )),
                    None,
                );
                assert_eq!(live(game).kind(game), Some(None));
            });
        },
    );
}

/// `AbstractContainerMenu.getStateId` on a client (#5323): the open menu's handle reads the state
/// of the last content or slot packet for its window, not the inventory's, and the inventory's
/// handle reads its own once the menu closes.
#[test]
#[ignore = "daily"]
fn client_menu_state_id_follows_its_windows_packets() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use sunrise_core::nbt::Tag;
    use sunrise_protocol::packets::play::{self, Clientbound as C};

    with_client_game(|game| {
        let (_reader, _writer) = connect_menu_test(game);
        spawn_menu_test(game);
        let table = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:menu")
            .unwrap()
            .id("minecraft:crafting")
            .unwrap();
        game.handle_packet(C::OpenScreen(play::OpenScreen {
            container: 7,
            menu: table,
            title: Tag::String("Crafting".into()),
        }))
        .unwrap();
        let content = |container, state| {
            C::ContainerSetContent(play::ContainerSetContent {
                container,
                state,
                items: vec![None; 46],
                carried: None,
            })
        };
        game.handle_packet(content(7, 12)).unwrap();
        let menu = game
            .player_container_menu(game.local_player().unwrap())
            .unwrap();
        assert_eq!(menu.state_id(game), Some(12));
        game.handle_packet(C::ContainerSetSlot(play::ContainerSetSlot {
            container: 7,
            state: 13,
            slot: 1,
            item: None,
        }))
        .unwrap();
        assert_eq!(menu.state_id(game), Some(13));
        // The inventory menu's sync while the table is open is not the table's.
        game.handle_packet(content(0, 45)).unwrap();
        assert_eq!(menu.state_id(game), Some(13));
        game.handle_packet(C::ContainerClose(play::ContainerClose { container: 7 }))
            .unwrap();
        assert_eq!(menu.state_id(game), None);
        let inventory = game
            .player_container_menu(game.local_player().unwrap())
            .unwrap();
        assert_eq!(inventory.state_id(game), Some(45));
    });
}

use sunrise_gameplay_common::game::native_menu_refs_common::NativeMenuRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
