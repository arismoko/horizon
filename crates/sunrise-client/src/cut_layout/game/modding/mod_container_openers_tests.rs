//! ContainerOpenersCounter timing, including lost openers.
#![cfg(test)]

#[cfg(test)]
pub(crate) mod tests {
    use super::super::native_entity_refs::tests::run_fresh_cases;
    use mod_api::{
        BlockState, BlockUpdateFlags, Flow, Hooks, HostBlockStates, MenuReplace,
        ModContainerViewersEvent, ModEntry, PlayerContainerOpenEvent, ServerHost, Text,
        VanillaGameEventEvent,
    };
    use std::sync::{Arc, Mutex};
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay_common::game::lan_actions::MenuOpen;
    use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt1;
    use sunrise_level::containers::{
        BlockEntity, ModBlockEntity, ModBlockEntityFields, ModContainer,
    };
    use sunrise_sim::lan::menu::Slots;

    pub(crate) const POS: (i32, i32, i32) = (3, 64, 4);
    pub(crate) const AT: [i32; 3] = [POS.0, POS.1, POS.2];

    pub(crate) fn with_container(
        block: &str,
        test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
    ) {
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let state = game.pack.assets.registry.default_state(block).unwrap();
            let level = game.level("minecraft:overworld").unwrap();
            assert_eq!(
                game.level_set_block(level, AT, state, BlockUpdateFlags::NEIGHBORS),
                Ok(true)
            );
            test(game);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_replacement_reusing_the_block_menu_window_is_not_announced_twice() {
        if run_fresh_cases(
            "cut_layout::game::server_side::mod_container_openers_tests::tests::a_replacement_reusing_the_block_menu_window_is_not_announced_twice",
            "SUNRISE_GUEST_MENU_REENTRY_CASE",
            &["window_wrap"],
        ) {
            return;
        }
        struct Replace(Arc<Mutex<Vec<String>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Replace {
            fn vanilla_game_event(
                &self,
                host: &mut dyn ServerHost,
                event: &mut VanillaGameEventEvent<'_>,
            ) -> Flow {
                if event.event == "minecraft:container_open" {
                    let player = host.players().acting_player().unwrap();
                    // ServerPlayer.nextContainerCounter wraps 100 back to 1. Reuse the
                    // outer opening's window ID without reusing its menu identity.
                    for window in (2..=100).chain([1]) {
                        assert_eq!(
                            player
                                .open_chest_menu(
                                    host,
                                    1,
                                    Text::literal("replacement"),
                                    Vec::new(),
                                    MenuReplace::InPlace
                                )
                                .unwrap()
                                .container_id(host),
                            Some(window)
                        );
                    }
                }
                Flow::Continue
            }

            fn player_container_open(
                &self,
                _: &mut dyn ServerHost,
                event: &mut PlayerContainerOpenEvent<'_>,
            ) {
                self.0.lock().unwrap().push(event.menu_type.into());
            }
        }
        let heard = Arc::new(Mutex::new(Vec::new()));
        with_container("minecraft:chest", |game| {
            let result = mod_platform::testing::with_mods(
                vec![ModEntry::new(
                    "replace_menu",
                    Box::new(Replace(heard.clone())),
                )]
                .into(),
                || game.open_block_menu(12, POS),
            );
            assert_eq!(result, MenuOpen::Opened);
            let guest = game.server.lan.as_ref().unwrap().hub.guest(12).unwrap();
            let menu = guest.menu.as_ref().unwrap();
            assert_eq!(menu.window, 1);
            assert_eq!(&*menu.registry_id, "minecraft:generic_9x1");
            let heard = heard.lock().unwrap();
            assert_eq!(heard.len(), 100);
            assert!(heard.iter().all(|kind| kind == "minecraft:generic_9x1"));
            assert!(!game.server.lid_openers.contains_key(&POS));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mod_openers_change_synchronously_and_recheck_five_ticks_later() {
        type Notices = Arc<Mutex<Vec<(u32, u32)>>>;
        type Events = Arc<Mutex<Vec<(String, Option<u32>)>>>;
        struct Listener(Notices, Events);
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn mod_container_viewers(
                &self,
                _: &mut dyn ServerHost,
                e: &mut ModContainerViewersEvent,
            ) {
                self.0.lock().unwrap().push((e.previous, e.viewers));
            }
            fn vanilla_game_event(
                &self,
                _: &mut dyn ServerHost,
                e: &mut VanillaGameEventEvent,
            ) -> mod_api::Flow {
                if e.event == "minecraft:container_open" || e.event == "minecraft:container_close" {
                    self.1.lock().unwrap().push((e.event.to_owned(), e.cause));
                }
                mod_api::Flow::Continue
            }
        }
        let notices = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "openers_test",
            Box::new(Listener(notices.clone(), events.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_guest_turn_tests::with_guests(|game| {
                let pos = (3, 64, 5);
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                let state = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                game.world.block_entities.insert(
                    pos,
                    BlockEntity::Mod(Box::new(ModBlockEntity::new(
                        "openers_test:box".into(),
                        ModBlockEntityFields {
                            items: vec![None; 9],
                            ..Default::default()
                        },
                    ))),
                );
                game.world.block_entities.set_mod_containers(
                    [(
                        state.0,
                        Arc::new(ModContainer {
                            slots: 9,
                            menu: Some("minecraft:generic_3x3".into()),
                            ..Default::default()
                        }),
                    )]
                    .into_iter()
                    .collect(),
                );
                let start = game.world.game_time;
                for conn in [12, 13] {
                    game.server.lan.as_mut().unwrap().hub.open_container(
                        conn,
                        "minecraft:generic_3x3",
                        Tag::String("box".into()),
                        pos,
                        Slots::Block(pos),
                        vec![None; 9],
                        None,
                    );
                    game.player_menu_opening(conn);
                }
                assert_eq!(*notices.lock().unwrap(), [(0, 1), (1, 2)]);
                assert_eq!(
                    *events.lock().unwrap(),
                    [("minecraft:container_open".into(), Some(12))]
                );
                game.guest_disconnected(12);
                assert_eq!(notices.lock().unwrap().last(), Some(&(2, 1)));
                // A lost menu is repaired by the scheduled recheck, not each tick.
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update_all(|g| {
                        if g.conn == 13 {
                            g.menu = None;
                        }
                    });
                game.world.game_time = start + 4;
                game.tick_mod_container_viewers();
                assert_eq!(notices.lock().unwrap().len(), 3);
                game.world.game_time = start + 5;
                game.tick_mod_container_viewers();
                assert_eq!(notices.lock().unwrap().last(), Some(&(1, 0)));
                assert_eq!(
                    events.lock().unwrap().last(),
                    Some(&("minecraft:container_close".into(), None))
                );
                assert!(game.server.mod_container_viewers.is_empty());

                // Native menus can send a position as extra data without opening its container.
                let before = notices.lock().unwrap().len();
                for (conn, slots) in [(13, Slots::Block(pos)), (12, Slots::Native)] {
                    game.server.lan.as_mut().unwrap().hub.open_container(
                        conn,
                        "minecraft:generic_3x3",
                        Tag::String("box".into()),
                        pos,
                        slots,
                        vec![None; 9],
                        None,
                    );
                    game.player_menu_opening(conn);
                }
                game.world.game_time = start + 10;
                game.tick_mod_container_viewers();
                game.guest_close_all(12);
                assert_eq!(&notices.lock().unwrap()[before..], [(0, 1), (1, 1)]);

                // A recheck for the old cabinet must not report on its same-type replacement.
                let before_notices = notices.lock().unwrap().len();
                let before_events = events.lock().unwrap().len();
                let entity = game.world.block_entities.get(&pos).unwrap().clone();
                game.world.block_entities.replace(pos, entity).unwrap();
                game.world.game_time = start + 15;
                game.tick_mod_container_viewers();
                game.guest_close_all(13);
                assert_eq!(notices.lock().unwrap().len(), before_notices);
                assert_eq!(events.lock().unwrap().len(), before_events);
                assert!(game.server.mod_container_viewers.is_empty());

                // If a second player opens the replacement, the first player's stale menu
                // cannot close it, count on a recheck, or appear in its viewer list.
                let open = |game: &mut sunrise_gameplay::game::ServerGame<'_>, conn| {
                    game.server.lan.as_mut().unwrap().hub.open_container(
                        conn,
                        "minecraft:generic_3x3",
                        Tag::String("box".into()),
                        pos,
                        Slots::Block(pos),
                        vec![None; 9],
                        None,
                    );
                    game.player_menu_opening(conn);
                };
                let level = game.level("minecraft:overworld").unwrap();
                let replacement_viewer = game.player(12).unwrap();
                for recheck_first in [false, true] {
                    open(game, 13);
                    let entity = game.world.block_entities.get(&pos).unwrap().clone();
                    game.world.block_entities.replace(pos, entity).unwrap();
                    open(game, 12);
                    let replacement = game.block_entity(level, [pos.0, pos.1, pos.2]).unwrap();
                    if recheck_first {
                        game.world.game_time += 5;
                        game.tick_mod_container_viewers();
                        assert_eq!(notices.lock().unwrap().last(), Some(&(1, 1)));
                        assert_eq!(replacement.viewers(game), Some(vec![replacement_viewer]));
                    }
                    let before_notices = notices.lock().unwrap().len();
                    let before_events = events.lock().unwrap().len();
                    game.guest_close_all(13);
                    assert_eq!(
                        notices.lock().unwrap().len(),
                        before_notices,
                        "stale close notified"
                    );
                    assert_eq!(
                        events.lock().unwrap().len(),
                        before_events,
                        "stale close emitted an event"
                    );
                    assert_eq!(replacement.viewers(game), Some(vec![replacement_viewer]));
                    game.guest_close_all(12);
                    assert_eq!(notices.lock().unwrap().last(), Some(&(1, 0)));
                    assert_eq!(
                        events.lock().unwrap().last(),
                        Some(&("minecraft:container_close".into(), Some(12)))
                    );
                }

                // Combined menus count each block entity; an ordinary close retains its source.
                let other = (4, 64, 5);
                game.world.set(other.0, other.1, other.2, state).unwrap();
                let entity = game.world.block_entities.get(&pos).unwrap().clone();
                game.world.block_entities.insert(other, entity);
                let before = notices.lock().unwrap().len();
                game.server.lan.as_mut().unwrap().hub.open_container(
                    13,
                    "minecraft:generic_3x3",
                    Tag::String("box".into()),
                    pos,
                    Slots::Double(pos, other),
                    vec![None; 18],
                    None,
                );
                game.player_menu_opening(13);
                assert_eq!(&notices.lock().unwrap()[before..], [(0, 1), (0, 1)]);
                game.guest_close_all(13);
                assert_eq!(&notices.lock().unwrap()[before + 2..], [(1, 0), (1, 0)]);
                assert_eq!(notices.lock().unwrap().last(), Some(&(1, 0)));
                assert_eq!(
                    events.lock().unwrap().last(),
                    Some(&("minecraft:container_close".into(), Some(13)))
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_shulker_close_sounds_at_its_centre_when_the_listener_replaces_it() {
        if run_fresh_cases(
            "cut_layout::game::server_side::mod_container_openers_tests::tests::a_shulker_close_sounds_at_its_centre_when_the_listener_replaces_it",
            "SUNRISE_GUEST_MENU_REENTRY_CASE",
            &["shulker_close"],
        ) {
            return;
        }
        struct Replace;
        #[mod_api::__subscriptions]
        impl Hooks for Replace {
            fn vanilla_game_event(
                &self,
                host: &mut dyn ServerHost,
                event: &mut VanillaGameEventEvent<'_>,
            ) -> Flow {
                if event.event == "minecraft:container_close" {
                    let level = host.level(event.dimension).unwrap();
                    let barrel = BlockState::of(
                        &host.block_states(),
                        mod_api::__private::block_key("minecraft:barrel").unwrap(),
                        &[],
                    )
                    .unwrap();
                    assert_eq!(
                        level.set_block(host, AT, barrel, BlockUpdateFlags::NEIGHBORS),
                        Ok(true)
                    );
                }
                Flow::Continue
            }
        }
        with_container("minecraft:shulker_box", |game| {
            game.open_block_menu(12, POS);
            assert!(
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(12)
                    .unwrap()
                    .menu
                    .is_some()
            );
            let barrel = game
                .pack
                .assets
                .registry
                .default_state("minecraft:barrel")
                .unwrap();
            game.server.audio_capture_active = true;
            game.server.pending_sounds.clear();
            mod_platform::testing::with_mods(
                vec![ModEntry::new("replace_shulker", Box::new(Replace))].into(),
                || game.guest_close_all(12),
            );
            assert_eq!(game.world.block(POS.0, POS.1, POS.2), barrel);
            let sounds = game
                .server
                .pending_sounds
                .iter()
                .map(|s| (s.event.as_str(), s.position))
                .collect::<Vec<_>>();
            assert_eq!(
                sounds,
                [("minecraft:block.shulker_box.close", [3.5, 64.5, 4.5])]
            );
        });
    }
}
