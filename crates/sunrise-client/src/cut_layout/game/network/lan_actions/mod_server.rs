#[cfg(test)]
pub(crate) mod tests {
    use sunrise_gameplay::game::lan_actions::stop_menu::StopMenu;
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;

    #[test]
    #[ignore = "daily"]
    fn block_menu_click_keeps_spent_action_on_cancellation() {
        use mod_api::BlockUpdateFlags;
        use std::sync::Arc;
        use sunrise_core::nbt::Tag;
        use sunrise_gameplay_common::game::lan_actions::MenuOpen;
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::lan::guest::GuestKind;
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let chest = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            assert_eq!(
                game.level_set_block(level, [0, 64, 0], chest, BlockUpdateFlags::NEIGHBORS),
                Ok(true)
            );
            let pos = (0, 64, 0);
            assert_eq!(
                game.lan_open_container(12, (2, 64, 0)),
                MenuOpen::NoProvider
            );
            assert_eq!(game.lan_open_container(999, pos), MenuOpen::Refused);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| {
                    guest.kind = GuestKind::Fake;
                });
            assert_eq!(game.lan_open_container(12, pos), MenuOpen::Refused);
            assert!(
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .menu
                    .is_none()
            );
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| {
                    guest.kind = GuestKind::Network;
                });
            mod_platform::testing::with_mods(
                vec![mod_api::ModEntry::new(
                    "stop_menu",
                    Box::new(StopMenu(None)),
                )]
                .into(),
                || {
                    assert_eq!(game.lan_open_container(12, pos), MenuOpen::Cancelled);
                    assert!(
                        game.server.lan.as_ref().unwrap().hub.guests[0]
                            .menu
                            .is_none()
                    );
                },
            );
            let lock = Tag::compound([(
                "lock",
                Tag::compound([("items", Tag::String("minecraft:gold_ingot".into()))]),
            )]);
            assert_eq!(game.lan_open_container(13, pos), MenuOpen::Opened);
            game.world.set_block_lock(pos, &lock);
            assert_eq!(game.lan_open_container(12, pos), MenuOpen::Refused);
            assert!(
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .menu
                    .is_none()
            );
            assert_eq!(game.server.lid_openers.get(&pos), Some(&1));
            game.guest_close_all(13);
            assert!(!game.server.lid_openers.contains_key(&pos));
            game.world
                .set_block_lock(pos, &Tag::Compound(Default::default()));
            assert_eq!(game.lan_open_container(12, pos), MenuOpen::Opened);
            assert!(
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .menu
                    .is_some()
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_parts_pick_and_use_on_the_server_and_for_a_guest() {
        use mod_api::InteractionResult as Answer;
        use mod_api::{
            self, BlockPart, CloneItemStackEvent, Hooks, Host, HostSide, ItemPart, ItemStack,
            Registrar, UseItemOnEvent, UseOnEvent, UseWithoutItemEvent,
        };
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicI32, AtomicUsize},
        };
        use sunrise_level::chunk::Chunk;
        use sunrise_protocol::packets::{configuration::ClientInformation, play};
        use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
        use {
            crate::game::server_side::native_entity_refs::tests::run_fresh_cases,
            crate::game::server_side::native_entity_refs::tests::with_game,
            sunrise_gameplay::game::server_side::lan::LanSettings,
        };

        if run_fresh_cases(
            "cut_layout::game::server_side::lan_actions_server::tests::block_parts_pick_and_use_on_the_server_and_for_a_guest",
            "SUNRISE_PICK_PARTS_CHILD",
            &["players"],
        ) {
            return;
        }
        type Picks = Arc<Mutex<Vec<(u32, u128, bool)>>>;
        struct Picker(Picks);
        #[mod_api::sunrise_part]
        impl BlockPart for Picker {
            fn clone_item_stack(
                &self,
                host: &dyn Host,
                event: &mut CloneItemStackEvent<'_>,
            ) -> Option<ItemStack> {
                assert!(event.player.food(host).is_some());
                self.0.lock().unwrap().push((
                    event.player.entity().id(),
                    event.player.uuid(),
                    event.include_data,
                ));
                event.include_data.then(|| {
                    mod_api::ItemStack::new(
                        mod_api::__private::item_key("minecraft:diamond").unwrap(),
                        1,
                    )
                    .unwrap()
                })
            }
        }
        #[derive(Clone)]
        struct User {
            answer: Arc<Mutex<Option<Answer>>>,
            replace_with: Arc<Mutex<Option<sunrise_world::registry::BlockStateId>>>,
            empty_answer: Arc<Mutex<Option<Answer>>>,
            empty_food: Arc<Mutex<Option<i32>>>,
            empty_clicks: Arc<Mutex<Vec<(sunrise_world::registry::BlockStateId, u128)>>>,
            clicks: Arc<Mutex<Vec<(u128, mod_api::client::first_person::Hand)>>>,
            item_uses: Arc<AtomicUsize>,
        }
        #[mod_api::sunrise_part]
        impl BlockPart for User {
            fn use_item_on(
                &self,
                host: HostSide<'_>,
                event: &mut UseItemOnEvent<'_>,
            ) -> Option<Answer> {
                let HostSide::Server(host) = host else {
                    return None;
                };
                let replacement = self.replace_with.lock().unwrap().take();
                if let Some(state) = replacement {
                    let level = host.level("minecraft:overworld").unwrap();
                    assert_eq!(
                        level.set_block(
                            host,
                            event.click.pos,
                            state.try_into_api().unwrap(),
                            mod_api::BlockUpdateFlags::KNOWN_SHAPE,
                        ),
                        Ok(true)
                    );
                }
                self.clicks
                    .lock()
                    .unwrap()
                    .push((event.click.player, event.click.hand));
                *self.answer.lock().unwrap()
            }
            fn use_without_item(
                &self,
                host: HostSide<'_>,
                event: &mut UseWithoutItemEvent<'_>,
            ) -> Option<Answer> {
                let HostSide::Server(host) = host else {
                    return None;
                };
                assert_eq!(event.click.hand, mod_api::client::first_person::Hand::Main);
                if let Some(level) = self.empty_food.lock().unwrap().take() {
                    let player = host.player(event.click.player).unwrap();
                    assert!(player.set_food_level(host, level));
                }
                self.empty_clicks.lock().unwrap().push((
                    sunrise_gameplay_common::game::api_values::TryIntoEngine::try_into_engine(
                        &event.state,
                        &(),
                    )
                    .unwrap(),
                    event.click.player,
                ));
                *self.empty_answer.lock().unwrap()
            }
        }
        // The held items' own `Item.useOn`, counted, taking the click.
        #[mod_api::sunrise_part]
        impl ItemPart for User {
            fn use_on(&self, _: mod_api::HostSide<'_>, _: &mut UseOnEvent<'_>) -> Option<Answer> {
                self.item_uses
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Some(Answer::SuccessServer)
            }
        }
        struct Mod {
            picks: Picks,
            user: User,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let block_key = r.block_key("minecraft:stone");
                r.block_part(block_key, Picker(self.picks.clone()));
                let block_key = r.block_key("minecraft:beehive");
                r.block_part(block_key, Picker(self.picks.clone()));
                let block_key = r.block_key("minecraft:chest");
                r.block_part(block_key, Picker(self.picks.clone()));
                let block_key = r.block_key("minecraft:lever");
                r.block_part(block_key, self.user.clone());
                let block_key = r.block_key("minecraft:cauldron");
                r.block_part(block_key, self.user.clone());
                let block_key = r.block_key("minecraft:jukebox");
                r.block_part(block_key, self.user.clone());
                let block_key = r.block_key("minecraft:decorated_pot");
                r.block_part(block_key, self.user.clone());
                let block_key = r.block_key("minecraft:redstone_ore");
                r.block_part(block_key, self.user.clone());
                let block_key = r.block_key("minecraft:chiseled_bookshelf");
                r.block_part(block_key, self.user.clone());
                let block_key = r.block_key("minecraft:cake");
                r.block_part(block_key, self.user.clone());
                let item = r.item_key("minecraft:stick");
                r.item_part(item, self.user.clone());
                let item = r.item_key("minecraft:water_bucket");
                r.item_part(item, self.user.clone());
                let item = r.item_key("minecraft:stone");
                r.item_part(item, self.user.clone());
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let user = User {
            answer: Arc::new(Mutex::new(Some(Answer::Success))),
            replace_with: Arc::new(Mutex::new(None)),
            empty_answer: Arc::new(Mutex::new(None)),
            empty_food: Arc::new(Mutex::new(None)),
            empty_clicks: Arc::new(Mutex::new(Vec::new())),
            clicks: Arc::new(Mutex::new(Vec::new())),
            item_uses: Arc::new(AtomicUsize::new(0)),
        };
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "picker",
            Box::new(Mod {
                picks: seen.clone(),
                user: user.clone(),
            }),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let state = |name| game.pack.assets.registry.default_state(name).unwrap();
                let stone = state("minecraft:stone");
                let crop = state("minecraft:wheat");
                let chest = state("minecraft:chest");
                game.world.set(0, 64, 0, stone).unwrap();
                game.world.set(1, 64, 0, crop).unwrap();
                let settings = LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                };
                let net = NetConfig {
                    listen: false,
                    announce: false,
                    lod_distance: 0,
                    ..NetConfig::lan(0, String::new(), 2)
                };
                let config = HubConfig {
                    default_game_mode: 1,
                    guest_commands: false,
                    force_game_mode: false,
                    hardcore: false,
                    view_distance: 2,
                    simulation_distance: 2,
                    dimension: game.dim.id().to_owned(),
                    allow_flight: true,
                    seed: 0,
                    max_players: 2,
                    integrated: true,
                    levels: game.server.level_shapes.clone(),
                    level_seeds: game.server.levels.0.runtime.seeds(),
                };
                game.lan_host(settings, net, config).unwrap();
                let (tx, _rx) = std::sync::mpsc::channel();
                let make_guest = |conn, uuid, entity_id| {
                    let mut guest = Guest::new(
                        Joined {
                            conn,
                            uuid,
                            name: format!("guest{conn}"),
                            properties: Vec::new(),
                            info: ClientInformation::new(8),
                            brand: String::new(),
                            sender: PlayerSender::new(tx.clone(), Arc::new(AtomicUsize::new(0))),
                            latency: Arc::new(AtomicI32::new(0)),
                            address: "127.0.0.1:1".parse().unwrap(),
                            expected: false,
                        },
                        entity_id,
                        [0.5, 64.0, 2.0],
                        10,
                    );
                    guest.placed = true;
                    guest.game_mode = 1;
                    guest
                };
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .push(make_guest(12, 12, 12));

                // BlockState.useItemOn replaces the template. Only TryWithEmptyHand
                // reaches useWithoutItem, which toggles this lever on the main hand.
                use mod_api::client::first_person::Hand;
                use std::sync::atomic::Ordering::SeqCst;
                let lever = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:lever")
                    .unwrap();
                let hit = sunrise_gameplay_common::game::Hit {
                    pos: [0, 64, 4],
                    face: [0, 1, 0],
                    point: [0.5, 65.0, 4.5],
                    inside: false,
                };
                game.world.set(0, 64, 4, lever).unwrap();
                game.common.inventory[game.common.selected] =
                    Some(sunrise_items::items::ItemStack::new("minecraft:stick", 1));
                for (answer, item_uses, powered, swings) in [
                    (Answer::Success, 0, false, 1),
                    (Answer::SuccessServer, 0, false, 1),
                    (Answer::Consume, 0, false, 0),
                    (Answer::Pass, 1, false, 1),
                    (Answer::Fail, 2, false, 1),
                    (Answer::TryWithEmptyHand, 2, true, 1),
                ] {
                    *user.answer.lock().unwrap() = Some(answer);
                    game.swing = Default::default();
                    let before = game.swings_started;
                    let empty_before = user.empty_clicks.lock().unwrap().len();
                    assert!(game.use_hand(Some(&hit)).unwrap());
                    assert_eq!(
                        user.empty_clicks.lock().unwrap().len() - empty_before,
                        usize::from(answer == Answer::TryWithEmptyHand),
                    );
                    assert_eq!(
                        game.common.inventory[game.common.selected]
                            .as_ref()
                            .unwrap()
                            .count,
                        1
                    );
                    assert_eq!(game.swings_started - before, swings);
                    assert_eq!(user.item_uses.load(SeqCst), item_uses);
                    assert_eq!(game.world.block(0, 64, 4) != lever, powered);
                }
                let calls = user.clicks.lock().unwrap().len();
                game.world.set(0, 64, 4, stone).unwrap();
                assert!(game.use_hand(Some(&hit)).unwrap());
                assert_eq!(user.item_uses.load(SeqCst), 3);
                assert_eq!(user.clicks.lock().unwrap().len(), calls);

                // A block-specific useItemOn is replaced too, before its bucket exchange.
                let cauldron = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:cauldron")
                    .unwrap();
                let full = game
                    .with_props(
                        game.pack
                            .assets
                            .registry
                            .default_state("minecraft:water_cauldron")
                            .unwrap(),
                        &[("level", "3")],
                    )
                    .unwrap();
                let cauldron_hit = sunrise_gameplay_common::game::Hit {
                    pos: [5, 64, 4],
                    point: [5.5, 65.0, 4.5],
                    ..hit
                };
                game.creative = false;
                for (answer, expected, bucket, item_calls) in [
                    (Some(Answer::Success), cauldron, "minecraft:water_bucket", 3),
                    (None, full, "minecraft:bucket", 3),
                    (Some(Answer::Pass), cauldron, "minecraft:water_bucket", 4),
                ] {
                    game.world.set(5, 64, 4, cauldron).unwrap();
                    game.common.inventory[game.common.selected] = Some(
                        sunrise_items::items::ItemStack::new("minecraft:water_bucket", 1),
                    );
                    *user.answer.lock().unwrap() = answer;
                    assert!(game.use_hand(Some(&cauldron_hit)).unwrap());
                    assert_eq!(game.world.block(5, 64, 4), expected);
                    let held = game.common.inventory[game.common.selected]
                        .as_ref()
                        .unwrap();
                    assert_eq!((&*held.item, held.count), (bucket, 1));
                    assert_eq!(user.item_uses.load(SeqCst), item_calls);
                }

                // TRY_WITH_EMPTY_HAND still ejects a jukebox's disc, not just its
                // level-only success result: the game owns that side effect.
                let jukebox = game
                    .with_props(
                        game.pack
                            .assets
                            .registry
                            .default_state("minecraft:jukebox")
                            .unwrap(),
                        &[("has_record", "true")],
                    )
                    .unwrap();
                game.world.set(6, 64, 4, jukebox).unwrap();
                game.world.block_entities.insert(
                    (6, 64, 4),
                    sunrise_level::containers::BlockEntity::Jukebox(
                        sunrise_level::containers::Jukebox {
                            items: [Some(sunrise_items::items::ItemStack::new(
                                "minecraft:music_disc_13",
                                1,
                            ))],
                            ticks: 0,
                            playing: false,
                            song_clock_present: false,
                            loaded_clock: false,
                        },
                    ),
                );
                let jukebox_hit = sunrise_gameplay_common::game::Hit {
                    pos: [6, 64, 4],
                    point: [6.5, 65.0, 4.5],
                    ..hit
                };
                *user.answer.lock().unwrap() = Some(Answer::TryWithEmptyHand);
                *user.empty_answer.lock().unwrap() = Some(Answer::Success);
                assert!(game.use_hand(Some(&jukebox_hit)).unwrap());
                assert!(matches!(
                    game.world.block_entities.get(&(6, 64, 4)),
                    Some(sunrise_level::containers::BlockEntity::Jukebox(j)) if j.record().is_some()
                ));
                *user.empty_answer.lock().unwrap() = None;
                let calls = user.clicks.lock().unwrap().len();
                assert!(game.use_hand(Some(&jukebox_hit)).unwrap());
                assert_eq!(user.clicks.lock().unwrap().len(), calls + 1);
                assert!(matches!(
                    game.world.block_entities.get(&(6, 64, 4)),
                    Some(sunrise_level::containers::BlockEntity::Jukebox(j)) if j.record().is_none()
                ));
                assert!(
                    game.dropped
                        .iter()
                        .any(|item| &*item.stack.item == "minecraft:music_disc_13")
                );
                assert_eq!(user.item_uses.load(SeqCst), 4);

                // ServerPlayerGameMode uses the state it read before useItemOn,
                // even when that method replaces the block before asking for empty use.
                let floor_lever = game.with_props(lever, &[("face", "floor")]).unwrap();
                let powered = game
                    .with_props(floor_lever, &[("powered", "true")])
                    .unwrap();
                let cobblestone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:cobblestone")
                    .unwrap();
                let replaced_hit = sunrise_gameplay_common::game::Hit {
                    pos: [8, 64, 4],
                    point: [8.5, 65.0, 4.5],
                    ..hit
                };
                game.world.set(8, 63, 4, stone).unwrap();
                game.world.set(8, 64, 4, floor_lever).unwrap();
                *user.replace_with.lock().unwrap() = Some(cobblestone);
                *user.answer.lock().unwrap() = Some(Answer::TryWithEmptyHand);
                assert!(game.use_hand(Some(&replaced_hit)).unwrap());
                assert_eq!(game.world.block(8, 64, 4), powered);
                assert_eq!(user.item_uses.load(SeqCst), 4);

                assert_eq!(
                    user.empty_clicks
                        .lock()
                        .unwrap()
                        .last()
                        .map(|(state, _)| *state),
                    Some(floor_lever)
                );

                // Every supplied empty result replaces the lever; TRY is not retried.
                let mut item_uses = 4;
                for answer in [
                    Answer::Success,
                    Answer::SuccessServer,
                    Answer::Consume,
                    Answer::Pass,
                    Answer::Fail,
                    Answer::TryWithEmptyHand,
                ] {
                    game.world.set(8, 64, 4, floor_lever).unwrap();
                    *user.empty_answer.lock().unwrap() = Some(answer);
                    let calls = user.empty_clicks.lock().unwrap().len();
                    let before = game.swings_started;
                    assert!(game.use_hand(Some(&replaced_hit)).unwrap());
                    assert_eq!(
                        game.swings_started - before,
                        u32::from(answer != Answer::Consume)
                    );
                    assert_eq!(game.world.block(8, 64, 4), floor_lever);
                    assert_eq!(user.empty_clicks.lock().unwrap().len(), calls + 1);
                    if matches!(
                        answer,
                        Answer::Pass | Answer::Fail | Answer::TryWithEmptyHand
                    ) {
                        item_uses += 1;
                    }
                    assert_eq!(user.item_uses.load(SeqCst), item_uses);
                }

                // The template's TRY reaches the part too; the off hand never does.
                *user.answer.lock().unwrap() = None;
                *user.empty_answer.lock().unwrap() = Some(Answer::Success);
                assert!(game.use_hand(Some(&replaced_hit)).unwrap());
                assert_eq!(game.world.block(8, 64, 4), floor_lever);
                *user.answer.lock().unwrap() = Some(Answer::TryWithEmptyHand);
                game.hand = Hand::Off;
                game.offhand = Some(sunrise_items::items::ItemStack::new("minecraft:stick", 1));
                let calls = user.empty_clicks.lock().unwrap().len();
                assert!(game.use_hand(Some(&replaced_hit)).unwrap());
                assert_eq!(user.empty_clicks.lock().unwrap().len(), calls);
                item_uses += 1;
                assert_eq!(user.item_uses.load(SeqCst), item_uses);
                game.hand = Hand::Main;
                game.offhand = None;

                // Ore's item-method PASS must not be mistaken for TRY.
                let ore = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:redstone_ore")
                    .unwrap();
                game.world.set(11, 64, 4, ore).unwrap();
                game.common.inventory[game.common.selected] =
                    Some(sunrise_items::items::ItemStack::new("minecraft:stone", 1));
                let ore_hit = sunrise_gameplay_common::game::Hit {
                    pos: [11, 64, 4],
                    point: [11.5, 65.0, 4.5],
                    ..hit
                };
                *user.answer.lock().unwrap() = None;
                let calls = user.empty_clicks.lock().unwrap().len();
                assert!(game.use_hand(Some(&ore_hit)).unwrap());
                assert_eq!(user.empty_clicks.lock().unwrap().len(), calls);
                item_uses += 1;
                assert_eq!(user.item_uses.load(SeqCst), item_uses);
                assert_eq!(
                    game.world.block(11, 64, 4),
                    game.with_props(ore, &[("lit", "true")]).unwrap()
                );
                *user.answer.lock().unwrap() = Some(Answer::TryWithEmptyHand);
                game.common.inventory[game.common.selected] =
                    Some(sunrise_items::items::ItemStack::new("minecraft:stick", 1));

                // The hook precedes the pot's wobble and the bookshelf's removal.
                let pot = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:decorated_pot")
                    .unwrap();
                game.world.set(10, 64, 4, pot).unwrap();
                let pot_hit = sunrise_gameplay_common::game::Hit {
                    pos: [10, 64, 4],
                    point: [10.5, 65.0, 4.5],
                    ..hit
                };
                game.start_audio_capture();
                game.server.lan_block_events.clear();
                assert!(game.use_hand(Some(&pot_hit)).unwrap());
                assert!(!game.server.lan_block_events.iter().any(|&(p, b0, b1, _)| {
                    p == pot_hit.pos && b0 == 1 && b1 == crate::game::decorated_pot::WOBBLE_NEGATIVE
                }));
                *user.empty_answer.lock().unwrap() = None;
                assert!(game.use_hand(Some(&pot_hit)).unwrap());
                assert!(game.server.lan_block_events.iter().any(|&(p, b0, b1, _)| {
                    p == pot_hit.pos && b0 == 1 && b1 == crate::game::decorated_pot::WOBBLE_NEGATIVE
                }));
                game.stop_audio_capture();
                let shelf = game
                    .with_props(
                        game.pack
                            .assets
                            .registry
                            .default_state("minecraft:chiseled_bookshelf")
                            .unwrap(),
                        &[("facing", "north"), ("slot_0_occupied", "true")],
                    )
                    .unwrap();
                game.world.set(9, 64, 4, shelf).unwrap();
                let mut books = Box::<sunrise_level::containers::ChiseledBookshelf>::default();
                books.items[0] = Some(sunrise_items::items::ItemStack::new("minecraft:book", 1));
                game.world.block_entities.insert(
                    (9, 64, 4),
                    sunrise_level::containers::BlockEntity::ChiseledBookshelf(books),
                );
                let shelf_hit = sunrise_gameplay_common::game::Hit {
                    pos: [9, 64, 4],
                    face: [0, 0, -1],
                    point: [9.75, 64.75, 4.0],
                    inside: false,
                };
                *user.empty_answer.lock().unwrap() = Some(Answer::Success);
                assert!(game.use_hand(Some(&shelf_hit)).unwrap());
                assert!(matches!(
                    game.world.block_entities.get(&(9, 64, 4)),
                    Some(sunrise_level::containers::BlockEntity::ChiseledBookshelf(b)) if b.items[0].is_some()
                ));
                *user.empty_answer.lock().unwrap() = None;
                assert!(game.use_hand(Some(&shelf_hit)).unwrap());
                assert!(matches!(
                    game.world.block_entities.get(&(9, 64, 4)),
                    Some(sunrise_level::containers::BlockEntity::ChiseledBookshelf(b)) if b.items[0].is_none()
                ));

                // The real guest packet reaches empty use with the guest's UUID.
                game.world.set(0, 64, 4, lever).unwrap();
                {
                    let value = Some(sunrise_items::items::ItemStack::new("minecraft:stick", 1));
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |g| g.inventory[0] = value);
                }
                *user.answer.lock().unwrap() = Some(Answer::TryWithEmptyHand);
                *user.empty_answer.lock().unwrap() = Some(Answer::Success);
                game.lan_guest_packet(
                    12,
                    play::Serverbound::UseItemOn(play::UseItemOn {
                        hand: 0,
                        pos: hit.pos,
                        face: 1,
                        cursor: [0.5, 1.0, 0.5],
                        inside: false,
                        world_border: false,
                        sequence: 1,
                    }),
                );
                assert_eq!(user.clicks.lock().unwrap().last(), Some(&(12, Hand::Main)));
                assert_eq!(user.empty_clicks.lock().unwrap().last(), Some(&(lever, 12)));
                assert_eq!(user.item_uses.load(SeqCst), item_uses);
                assert_eq!(game.world.block(0, 64, 4), lever);

                // A delegating Part's food write precedes vanilla's canEat check.
                let cake = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:cake")
                    .unwrap();
                let bitten = game.with_props(cake, &[("bites", "1")]).unwrap();
                *user.empty_answer.lock().unwrap() = None;
                for (food_write, expected, food, item_calls) in [
                    (None, bitten, 12, item_uses),
                    (Some(20), cake, 20, item_uses + 1),
                ] {
                    game.world.set(0, 64, 4, cake).unwrap();
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| {
                            guest.game_mode = 0;
                            guest.food = 10;
                        });
                    *user.empty_food.lock().unwrap() = food_write;
                    game.lan_guest_packet(
                        12,
                        play::Serverbound::UseItemOn(play::UseItemOn {
                            hand: 0,
                            pos: hit.pos,
                            face: 1,
                            cursor: [0.5, 1.0, 0.5],
                            inside: false,
                            world_border: false,
                            sequence: 2,
                        }),
                    );
                    assert_eq!(game.world.block(0, 64, 4), expected);
                    assert_eq!(game.server.lan.as_ref().unwrap().hub.guests[0].food, food);
                    assert_eq!(user.item_uses.load(SeqCst), item_calls);
                }
                {
                    let value = 1;
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| guest.game_mode = value);
                }
                game.world.set(0, 64, 4, stone).unwrap();
                game.lan_guest_packet(
                    12,
                    play::Serverbound::PickItemFromBlock(play::PickItemFromBlock {
                        pos: [0, 64, 0],
                        include_data: true,
                    }),
                );
                let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
                assert_eq!(
                    &*guest.inventory[guest.selected].as_ref().unwrap().item,
                    "minecraft:diamond"
                );
                game.lan_guest_packet(
                    12,
                    play::Serverbound::PickItemFromBlock(play::PickItemFromBlock {
                        pos: [0, 64, 0],
                        include_data: false,
                    }),
                );
                let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
                assert_eq!(
                    &*guest.inventory[guest.selected].as_ref().unwrap().item,
                    "minecraft:stone"
                );
                game.lan_guest_packet(
                    12,
                    play::Serverbound::PickItemFromBlock(play::PickItemFromBlock {
                        pos: [1, 64, 0],
                        include_data: true,
                    }),
                );
                let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
                assert_eq!(
                    &*guest.inventory[guest.selected].as_ref().unwrap().item,
                    "minecraft:wheat_seeds"
                );
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .push(make_guest(13, 1300, 130));
                game.lan_guest_packet(
                    13,
                    play::Serverbound::PickItemFromBlock(play::PickItemFromBlock {
                        pos: [0, 64, 0],
                        include_data: false,
                    }),
                );
                let second = &game.server.lan.as_ref().unwrap().hub.guests[1];
                assert_eq!(
                    &*second.inventory[second.selected].as_ref().unwrap().item,
                    "minecraft:stone"
                );
                assert_eq!(
                    *seen.lock().unwrap(),
                    [(12, 12, true), (12, 12, false), (130, 1300, false)]
                );

                // addBlockDataToItem runs on the part's item: a chest picked with
                // its data gives the part's diamond carrying the chest's contents.
                game.world.set(3, 64, 0, chest).unwrap();
                let mut slots = vec![None; 27];
                slots[0] = Some(sunrise_items::items::ItemStack::new("minecraft:apple", 2));
                game.world.block_entities.insert(
                    (3, 64, 0),
                    sunrise_level::containers::BlockEntity::Chest(slots),
                );
                game.lan_guest_packet(
                    12,
                    play::Serverbound::PickItemFromBlock(play::PickItemFromBlock {
                        pos: [3, 64, 0],
                        include_data: true,
                    }),
                );
                let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
                let picked = guest.inventory[guest.selected].as_ref().unwrap();
                assert_eq!(&*picked.item, "minecraft:diamond");
                let Some(sunrise_core::nbt::Tag::Compound(components)) =
                    sunrise_items::item_data::component_snapshot(picked)
                else {
                    panic!("the chest's data is on the diamond");
                };
                assert!(components.contains_key(&"minecraft:container".into()));
                let expected = [
                    (12, 12, true),
                    (12, 12, false),
                    (130, 1300, false),
                    (12, 12, true),
                ];
                assert_eq!(*seen.lock().unwrap(), expected);

                let hive = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:beehive")
                    .unwrap();
                let hive = game.with_props(hive, &[("honey_level", "1")]).unwrap();
                game.world.set(2, 64, 0, hive).unwrap();
                game.world.block_entities.insert(
                    (2, 64, 0),
                    sunrise_level::containers::BlockEntity::Beehive(Box::default()),
                );
                game.creative = true;
                game.break_block([2, 64, 0]).unwrap();
                assert!(
                    game.dropped
                        .iter()
                        .any(|item| &*item.stack.item == "minecraft:beehive")
                );
                assert_eq!(*seen.lock().unwrap(), expected);
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn fake_body_does_not_count_toward_sleep() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.server.lan_acting_fake = false;
            let real = game.sleep_counts();
            assert_eq!(real.0, 1);
            game.server.lan_acting_fake = true;
            assert_eq!(game.sleep_counts(), (0, 0, 0));
            game.server.lan_acting_fake = false;
            assert_eq!(game.sleep_counts(), real);
        });
    }
}

#[cfg(test)]
pub(crate) mod respawn_world_point_tests {
    use std::sync::{Arc, atomic::AtomicI32, mpsc};
    use sunrise_core::geometry::Vec3;
    use sunrise_protocol::packets::{configuration::ClientInformation, play::Clientbound};
    use sunrise_sim::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig};
    use {
        crate::game::Settings, crate::game::load_resources, sunrise_gameplay_common::game::Options,
        sunrise_gameplay_common::game::Progress,
    };

    /// PlayerList.respawn sends world respawn data, independently of the personal landing.
    #[test]
    #[ignore = "daily"]
    fn guest_respawn_keeps_world_information_distinct_from_personal_landing() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        let scratch = std::env::temp_dir().join(format!(
            "sunrise-respawn-point-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&scratch).unwrap();
        game.server.world_root = scratch.clone();
        let world = [0, 100, 0];
        let personal = [5.5, 101.0, 5.5];
        game.level.world_spawn = Some(world);
        game.spawn = Some(Vec3::new(0.5, 100.0, 0.5));
        game.lan_host(
            sunrise_gameplay::game::server_side::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "respawn-test".into(), 2)
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
        let (tx, rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn: 2,
                uuid: 330,
                name: "RespawnGuest".into(),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            2,
            [0.5, 100.0, 0.5],
            2,
        );
        guest.dimension = "minecraft:overworld".into();
        guest.placed = true;
        guest.loaded = true;
        guest.spawn = Some(personal);
        guest.health = 0.0;
        guest.dead = true;
        game.server.lan.as_mut().unwrap().hub.guests.push(guest);
        game.guest_respawn(2);
        let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
        assert_eq!(guest.uuid, 330);
        assert_eq!(guest.position, personal);
        assert_eq!(guest.health, 20.0);
        assert!(!guest.dead);
        let mut packets = Vec::new();
        for out in rx.try_iter() {
            match out {
                Out::Packet(body) => packets.push(body),
                Out::Packets(bodies) => packets.extend(bodies),
                Out::Disconnect(_) => panic!("unexpected disconnect"),
            }
        }
        let mut sequence = Vec::new();
        let mut points = Vec::new();
        for body in packets {
            let mut r = sunrise_protocol::buf::Reader::new(&body);
            let id = r.varint().unwrap();
            match Clientbound::decode(id, r.rest()).unwrap() {
                Clientbound::Respawn(_) => sequence.push(84),
                Clientbound::SetDefaultSpawnPosition(p) => {
                    sequence.push(99);
                    points.push((
                        p.pos.dimension,
                        p.pos.pos,
                        p.yaw.to_bits(),
                        p.pitch.to_bits(),
                    ));
                }
                _ => {}
            }
        }
        assert_eq!(sequence, [84, 99, 99]);
        assert_eq!(points, vec![("minecraft:overworld".into(), world, 0, 0); 2]);
        drop(game);
        std::fs::remove_dir_all(scratch).unwrap();
    }
}

#[cfg(test)]
#[cfg(any())]
pub(crate) mod guest_close_listener_tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicI32, mpsc};
    use sunrise_core::position::ChunkPos;
    use sunrise_gameplay_common::game::lan_actions::guest_close_listener_tests::close;
    use sunrise_gameplay_common::game::lan_actions::guest_close_listener_tests::open;
    use sunrise_gameplay_common::game::lan_actions::guest_close_listener_tests::reset_listener;
    use sunrise_gameplay_common::game::lan_actions::guest_close_listener_tests::selected;
    use sunrise_level::chunk::Chunk;
    use sunrise_protocol::packets::configuration::ClientInformation;
    use sunrise_protocol::packets::play;
    use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
    use {
        crate::game::Settings, crate::game::load_resources, sunrise_gameplay_common::game::Options,
        sunrise_gameplay_common::game::Progress,
    };

    #[test]
    #[ignore = "daily"]
    fn ordinary_guest_final_close_posts_to_actual_listener() {
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources required"),
        );
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0 15 15"));
        assert!(game.run_command("fill 0 99 0 15 99 15 stone"));
        assert!(game.run_command("setblock 6 100 4 chest"));
        assert!(game.run_command("setblock 9 100 4 sculk_sensor strict"));
        game.sculk_prepare();
        game.sculk_sync_index();
        assert!(selected(&game).is_none());
        game.lan_host(
            sunrise_gameplay::game::server_side::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "guest-close-test".into(), 4)
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
                max_players: 4,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();

        let mut receivers = Vec::new();
        for conn in 1..=3 {
            let (tx, rx) = mpsc::channel();
            receivers.push(rx);
            let mut guest = Guest::new(
                Joined {
                    conn,
                    uuid: u128::from(conn) + 11200,
                    name: format!("CloseGuest{conn}"),
                    properties: Vec::new(),
                    info: ClientInformation::new(2),
                    brand: "test".into(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:25565".parse().unwrap(),
                    expected: false,
                },
                conn as i32 + 100,
                [4.5, 100.0, 4.5],
                2,
            );
            guest.dimension = "minecraft:overworld".into();
            guest.placed = true;
            guest.loaded = true;
            guest.game_mode = if conn == 3 { 3 } else { 0 };
            guest.on_ground = true;
            assert_eq!(guest.health, 20.0);
            game.server.lan.as_mut().unwrap().hub.guests.push(guest);
        }
        let clock = game.world.game_time;
        let mut missing = Vec::new();
        let window = open(&mut game, 1, 1);
        assert_eq!(game.server.lid_openers.get(&(6, 100, 4)), Some(&1));
        assert_eq!(selected(&game).unwrap().event, "minecraft:container_open");
        reset_listener(&mut game);
        close(&mut game, 1, window + 20);
        assert_eq!(
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guest(1)
                .unwrap()
                .menu
                .as_ref()
                .unwrap()
                .window,
            window
        );
        assert_eq!(game.server.lid_openers.get(&(6, 100, 4)), Some(&1));
        assert!(selected(&game).is_none());
        close(&mut game, 1, window);
        assert!(
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guest(1)
                .unwrap()
                .menu
                .is_none()
        );
        assert!(!game.server.lid_openers.contains_key(&(6, 100, 4)));
        match selected(&game) {
            Some(info) => {
                assert_eq!(info.event, "minecraft:container_close");
                assert_eq!(info.pos, [6.5, 100.5, 4.5]);
            }
            None => missing.push("single final close"),
        }
        reset_listener(&mut game);
        let first = open(&mut game, 1, 2);
        assert_eq!(selected(&game).unwrap().event, "minecraft:container_open");
        reset_listener(&mut game);
        let last = open(&mut game, 2, 1);
        assert_eq!(game.server.lid_openers.get(&(6, 100, 4)), Some(&2));
        assert!(selected(&game).is_none());
        close(&mut game, 1, first);
        assert!(
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guest(2)
                .unwrap()
                .menu
                .is_some()
        );
        assert_eq!(game.server.lid_openers.get(&(6, 100, 4)), Some(&1));
        assert!(selected(&game).is_none());
        close(&mut game, 2, last);
        assert!(!game.server.lid_openers.contains_key(&(6, 100, 4)));
        match selected(&game) {
            Some(info) => {
                assert_eq!(info.event, "minecraft:container_close");
                assert_eq!(info.pos, [6.5, 100.5, 4.5]);
            }
            None => missing.push("shared final close"),
        }
        reset_listener(&mut game);
        let spectator = open(&mut game, 3, 1);
        assert!(!game.server.lid_openers.contains_key(&(6, 100, 4)));
        assert!(selected(&game).is_none());
        close(&mut game, 3, spectator);
        assert!(
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guest(3)
                .unwrap()
                .menu
                .is_none()
        );
        assert!(selected(&game).is_none());
        assert!(game.run_command("setblock 6 100 4 air strict"));
        assert!(
            game.run_command("summon oak_chest_boat 6.5 100 4.5 {UUID:[I;112,0,0,9],NoGravity:1b}")
        );
        let boat = game.boats.last().unwrap().id;
        // Chest boats open for a sneaking guest; a plain empty-hand click boards.
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(1, |g| g.sneaking = true);
        game.lan_guest_packet(
            1,
            S::Interact(play::Interact {
                id: boat as i32,
                hand: 0,
                location: [0.0; 3],
                secondary: true,
            }),
        );
        let vehicle_window = game
            .server
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guest(1)
            .unwrap()
            .menu
            .as_ref()
            .expect("actual vehicle interaction must open menu")
            .window;
        reset_listener(&mut game);
        close(&mut game, 1, vehicle_window);
        assert!(
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guest(1)
                .unwrap()
                .menu
                .is_none()
        );
        assert_eq!(selected(&game).unwrap().event, "minecraft:container_close");
        assert_eq!(selected(&game).unwrap().pos, [6.5, 100.0, 4.5]);
        for guest in &game.server.lan.as_mut().unwrap().hub.guests {
            assert_eq!(guest.uuid, u128::from(guest.conn) + 11200);
            assert_eq!(guest.health, 20.0);
            assert_eq!(guest.position, [4.5, 100.0, 4.5]);
        }
        assert_eq!(game.world.game_time, clock);
        assert!(
            missing.is_empty(),
            "actual ordinary guest final close must be heard: {missing:?}"
        );
        drop(receivers);
    }
}

#[cfg(test)]
#[path = "tests/boat_holder_movement_tests_server.rs"]
pub(crate) mod boat_holder_movement_tests;

#[cfg(test)]
#[path = "tests/crack_feedback_tests_server.rs"]
pub(crate) mod crack_feedback_tests;

#[path = "tick_server.rs"]
pub(crate) mod tick;

#[path = "pickups_server.rs"]
pub(crate) mod pickups;

#[path = "guest_body_server.rs"]
pub(crate) mod guest_body;

#[path = "attack_checks_server.rs"]
pub(crate) mod attack_checks;
