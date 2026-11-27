use super::{connect_menu_test, spawn_menu_test};
use crate::game::remote::tests::menu_packets_tests::packets_through_pong;
use std::time::{Duration, Instant};
use sunrise_client::hud::HudBatch;
use sunrise_core::{geometry::Vec3, nbt::Tag};
use sunrise_items::items::ItemStack;
use sunrise_protocol::{
    item::Stack,
    packets::play::{
        Clientbound as C, ContainerSetSlot, MountScreenOpen, OpenScreen, PlayerPosition,
        Serverbound as S,
    },
};
use sunrise_sim::{menu::Input, mobs::Mob};
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay_common::game::Screen, sunrise_gameplay_common::game::in_own_process,
    sunrise_gameplay_common::mount_rules::CHEST,
};

#[test]
#[ignore = "daily"]
fn a_mount_chest_beyond_the_slot_range_ends_the_connection() {
    in_own_process(
        "cut_layout::game::client_side::remote_client::tests::menu_packets_tests::a_mount_chest_beyond_the_slot_range_ends_the_connection",
        || {
            // `new SimpleContainer(getInventorySize(inventoryColumns))` is built
            // before the entity is looked at: a chest that cannot be built ends the
            // connection for a horse, a nautilus and a missing entity alike.
            for (species, columns) in [
                (Some("minecraft:horse"), i32::MAX),
                (Some("minecraft:horse"), -1),
                (Some("minecraft:nautilus"), -1),
                (None, i32::MAX),
            ] {
                with_client_game(|game| {
                    let (_reader, _writer) = connect_menu_test(game);
                    spawn_menu_test(game);
                    if let Some(species) = species {
                        let kind = game.res.mob_types().get(species).unwrap().clone();
                        game.mobs.push(Mob::new(41, kind, Vec3::ZERO, 0.0));
                    }
                    game.handle_packet(C::MountScreenOpen(MountScreenOpen {
                        container: 8,
                        columns,
                        entity: 41,
                    }))
                    .unwrap();
                    assert!(game.client.mount_menu.opened_mob().is_none());
                    let deadline = Instant::now() + Duration::from_secs(5);
                    while game.client.remote.is_some() && Instant::now() < deadline {
                        game.poll_remote().unwrap();
                        std::thread::yield_now();
                    }
                    assert!(
                        game.client.remote.is_none(),
                        "a mount chest of {columns} columns for {species:?} must disconnect"
                    );
                });
            }
        },
    );
}

#[test]
#[ignore = "daily"]
fn rejected_menu_packets_retire_the_previous_client_menu() {
    in_own_process(
        "cut_layout::game::client_side::remote_client::tests::menu_packets_tests::rejected_menu_packets_retire_the_previous_client_menu",
        || {
            with_client_game(|game| {
                let (mut reader, _writer) = connect_menu_test(game);
                let remote = game.client.remote.as_mut().unwrap();
                remote.menu.set(Some((7, "minecraft:generic_9x3".into())));
                let generation = remote.menu.generation;
                game.set_screen(Screen::Inventory);
                game.carried = Some(ItemStack::new("minecraft:stone", 1));
                game.handle_packet(C::OpenScreen(OpenScreen {
                    container: 7,
                    menu: -1,
                    title: Tag::String("unsupported".into()),
                }))
                .unwrap();
                let remote = game.client.remote.as_ref().unwrap();
                assert!(remote.menu.as_ref().is_none());
                assert_ne!(remote.menu.generation, generation);
                assert!(game.carried.is_none());
                assert_eq!(game.client.screen, Screen::Playing);
                assert!(
                    packets_through_pong(game, &mut reader)
                        .into_iter()
                        .any(|p| { matches!(p, S::ContainerClose(close) if close.container == 7) })
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn rejected_mounts_preserve_the_previous_client_menu_without_sending_close() {
    in_own_process(
        "cut_layout::game::client_side::remote_client::tests::menu_packets_tests::rejected_mounts_preserve_the_previous_client_menu_without_sending_close",
        || {
            with_client_game(|game| {
                let (mut reader, _writer) = connect_menu_test(game);
                spawn_menu_test(game);
                let zombie = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                game.mobs.push(Mob::new(41, zombie, Vec3::ZERO, 0.0));
                let previous = (7, "minecraft:generic_9x3".to_owned());
                let remote = game.client.remote.as_mut().unwrap();
                remote.menu.set(Some(previous.clone()));
                let generation = remote.menu.generation;
                game.set_screen(Screen::Inventory);
                let carried = ItemStack::new("minecraft:stone", 3);
                game.carried = Some(carried.clone());
                for entity in [i32::MAX, 41] {
                    game.handle_packet(C::MountScreenOpen(MountScreenOpen {
                        container: 8,
                        columns: 6,
                        entity,
                    }))
                    .unwrap();
                    let remote = game.client.remote.as_ref().unwrap();
                    assert_eq!(remote.menu.as_ref(), Some(&previous));
                    assert_eq!(remote.menu.generation, generation);
                    assert_eq!(game.carried.as_ref(), Some(&carried));
                    assert_eq!(game.client.screen, Screen::Inventory);
                    assert!(
                        !packets_through_pong(game, &mut reader)
                            .into_iter()
                            .any(|p| matches!(p, S::ContainerClose(_)))
                    );
                }
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn remote_mount_is_installed_before_screen_init_and_keeps_all_columns() {
    use mod_api::{ClientHost, Hooks, ModEntry, ScreenInitPostEvent};
    use std::sync::{Arc, Mutex};

    // Listening only to Init.Post leaves screen_opening disabled. The menu
    // must still be installed when Gui.setScreen publishes the new view.
    struct ObserveMenu(Arc<Mutex<Vec<Option<String>>>>);
    #[mod_api::__subscriptions]
    impl Hooks for ObserveMenu {
        fn screen_init_post(&self, _: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            if event.screen.id == "inventory" {
                self.0
                    .lock()
                    .unwrap()
                    .push(event.screen.menu.map(str::to_owned));
            }
        }
    }

    in_own_process(
        "cut_layout::game::client_side::remote_client::tests::menu_packets_tests::remote_mount_is_installed_before_screen_init_and_keeps_all_columns",
        || {
            let observed = Arc::new(Mutex::new(Vec::new()));
            let mods = vec![ModEntry::new(
                "mount_observer",
                Box::new(ObserveMenu(observed.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_client_game(|game| {
                    let (mut reader, _writer) = connect_menu_test(game);
                    spawn_menu_test(game);
                    game.handle_packet(C::PlayerPosition(PlayerPosition {
                        teleport: 1,
                        position: [0.5, 65.0, 0.5],
                        delta: [0.0; 3],
                        yaw: 0.0,
                        pitch: 0.0,
                        relatives: 0,
                    }))
                    .unwrap();
                    game.client.remote.as_mut().unwrap().level_load.finish();
                    let open = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                                container,
                                entity,
                                species,
                                columns| {
                        let ty = game.res.mob_types().get(species).unwrap().clone();
                        game.mobs.push(Mob::new(entity as u32, ty, Vec3::ZERO, 0.0));
                        game.handle_packet(C::MountScreenOpen(MountScreenOpen {
                            container,
                            columns,
                            entity,
                        }))
                        .unwrap();
                        assert_eq!(game.client.screen, Screen::Inventory);
                        assert_eq!(game.client.mount_menu.opened_mob(), Some(entity as u32));
                    };
                    open(game, 8, 41, "minecraft:horse", 6);
                    assert_eq!(
                        *observed.lock().unwrap(),
                        [Some("sunrise:mount6".to_owned())]
                    );
                    assert_eq!(game.mount_menu_columns(), Some(6));
                    assert_eq!(game.slot_count(), 56);
                    let last = CHEST + 17;
                    assert_eq!(game.mount_slot_position(last), Some((170.0, 54.0)));
                    let batch = HudBatch::scratch([854, 480]);
                    let (ox, oy) = game.inventory_origin(&batch);
                    let hovered = game.slot_at(&batch, ox + 171.0, oy + 55.0);
                    assert_eq!(hovered, Some(last));
                    game.handle_packet(C::ContainerSetSlot(ContainerSetSlot {
                        container: 8,
                        slot: 19,
                        state: 5,
                        item: Some(Stack::new(game.item_id("minecraft:stone"), 3)),
                    }))
                    .unwrap();
                    assert_eq!(game.slot_view(last).unwrap().count, 3);
                    assert!(game.remote_menu_ops(&[(hovered, 0, Input::Pickup)]));
                    assert!(game.slot_view(last).is_none());
                    assert_eq!(game.carried.as_ref().unwrap().count, 3);
                    let packets = packets_through_pong(game, &mut reader);
                    assert!(packets.into_iter().any(|p| matches!(p,
                    S::ContainerClick(click) if (click.container, click.state, click.slot) == (8, 5, 19))));

                    game.set_screen(Screen::Playing);
                    // `HorseInventoryMenu` adds its chest slots only `if (inventoryColumns > 0)`:
                    // a count whose size wraps positive (`-1431655764 * 3` is 4 as Java's `int`) lays out none.
                    open(game, 10, 43, "minecraft:horse", -1431655764);
                    assert_eq!(game.mount_menu_columns(), Some(0));
                    assert_eq!(game.slot_count(), 38);
                    assert_eq!(game.mount_slot_position(CHEST), None);
                    game.set_screen(Screen::Playing);
                    open(game, 9, 42, "minecraft:zombie_nautilus", 6);
                    assert_eq!(game.mount_menu_columns(), Some(0));
                    assert_eq!(game.slot_count(), 38);
                    assert_eq!(game.mount_slot_position(CHEST), None);
                    game.handle_packet(C::ContainerSetSlot(ContainerSetSlot {
                        container: 9,
                        slot: 2,
                        state: 1,
                        item: Some(Stack::new(game.item_id("minecraft:stone"), 1)),
                    }))
                    .unwrap();
                    assert_eq!(game.slot_view(9).unwrap().count, 1);
                });
            });
        },
    );
}
