#![cfg(test)]
use mod_api::__private::game::player_ref;
use mod_api::BlockEntityError;
use sunrise_core::nbt::Tag;
use sunrise_level::containers::BlockEntity;
#[cfg(test)]
pub(crate) mod tests {

    use mod_api::HostTransaction;
    use sunrise_gameplay_common::game::server_side::native_block_entity_refs::tests::compound;
    use sunrise_gameplay_common::game::{
        native_block_entity_read::NativeBlockEntityReadGameExt1 as _,
        native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _,
        native_server::NativeServerGameExt1 as _,
    };

    use super::*;

    use mod_api::LevelRef;

    use mod_api::ServerHost;

    use mod_api::TransactionError;

    use std::sync::Arc;

    use sunrise_level::chunk::Chunk;

    use sunrise_sim::lan::menu::Slots;

    #[test]
    #[ignore = "daily"]
    fn player_opens_provider_and_refuses_non_provider() {
        use mod_api::*;
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::native_block_entity_refs::tests::player_opens_provider_and_refuses_non_provider",
            "SUNRISE_BE_MENU_CASE",
            &[
                "provider",
                "replacement",
                "existing",
                "close",
                "close_reentry",
            ],
        ) {
            return;
        }
        struct ReplaceDuringOpen;
        impl ReplaceDuringOpen {
            fn replace(host: &mut dyn ServerHost) {
                let level = host.level("minecraft:overworld").unwrap();
                for block in ["minecraft:stone", "minecraft:chest"] {
                    let state = host.block_state_id(block).unwrap();
                    assert_eq!(
                        level.set_block(host, [0, 64, 0], state, BlockUpdateFlags::NEIGHBORS),
                        Ok(true)
                    );
                }
            }
        }
        #[mod_api::__subscriptions]
        impl Hooks for ReplaceDuringOpen {
            fn inventory_block_start(
                &self,
                host: &mut dyn ServerHost,
                event: &mut InventoryBlockStartEvent<'_>,
            ) -> Flow {
                if event.pos == [0, 64, 0]
                    && !std::env::var("SUNRISE_BE_MENU_CASE")
                        .unwrap()
                        .starts_with("close")
                {
                    Self::replace(host);
                }
                Flow::Continue
            }
            fn player_container_close(
                &self,
                host: &mut dyn ServerHost,
                _event: &mut PlayerContainerCloseEvent<'_>,
            ) {
                let case = std::env::var("SUNRISE_BE_MENU_CASE").unwrap();
                if case.starts_with("close") {
                    Self::replace(host);
                    if case == "close_reentry" {
                        let level = host.level("minecraft:overworld").unwrap();
                        let other = host.block_entity(level, [5, 64, 0]).unwrap();
                        let player = host.player(12).unwrap();
                        assert!(player.open_menu(host, other).unwrap());
                    }
                }
            }
        }
        let case = std::env::var("SUNRISE_BE_MENU_CASE").unwrap();
        let replacing = case != "provider";
        if replacing {
            let mods: Mods = vec![ModEntry::new("replace", Box::new(ReplaceDuringOpen))].into();
            mod_platform::__private::install(mods.clone());
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        }
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            for (x, name) in [
                (0, "minecraft:chest"),
                (2, "minecraft:furnace"),
                (4, "minecraft:oak_sign"),
            ] {
                let state = game.pack.assets.registry.default_state(name).unwrap();
                assert_eq!(
                    game.level_set_block(level, [x, 64, 0], state, BlockUpdateFlags::NEIGHBORS),
                    Ok(true)
                );
            }
            let player = game.player(12).unwrap();
            let chest = game.block_entity(level, [0, 64, 0]).unwrap();
            if replacing {
                use sunrise_protocol::{buf::Reader, ids::play::clientbound};
                use sunrise_sim::lan::{Out, PlayerSender};
                let (tx, rx) = std::sync::mpsc::channel();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.sender = PlayerSender::new(tx, Arc::default());
                    });
                let previous = if case != "replacement" {
                    let state = game.world.block(0, 64, 0);
                    assert_eq!(
                        game.level_set_block(level, [5, 64, 0], state, BlockUpdateFlags::NEIGHBORS),
                        Ok(true)
                    );
                    let other = game.block_entity(level, [5, 64, 0]).unwrap();
                    assert!(player.open_menu(game, other).unwrap());
                    let host = game.server.lan.as_ref().unwrap();
                    Some(host.hub.guest(12).unwrap().menu.as_ref().unwrap().window)
                } else {
                    None
                };
                while rx.try_recv().is_ok() {}
                assert_eq!(player.open_menu(game, chest), Err(BlockEntityError::Stale));
                let replacement = game.block_entity(level, [0, 64, 0]).unwrap();
                assert_ne!(chest, replacement);
                assert_eq!(chest.position(game), None);
                let menu = game
                    .server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(12)
                    .unwrap()
                    .menu
                    .as_ref();
                assert_eq!(menu.is_some(), case == "close_reentry");
                if let Some(menu) = menu {
                    assert!(matches!(menu.slots, Slots::Block((5, 64, 0))));
                }
                let mut packets = Vec::new();
                for out in rx.try_iter() {
                    if let Out::Packet(packet) = out {
                        let mut reader = Reader::new(&packet);
                        let kind = reader.varint().unwrap();
                        if matches!(
                            kind,
                            clientbound::OPEN_SCREEN | clientbound::CONTAINER_CLOSE
                        ) {
                            packets.push((kind, reader.varint().unwrap()));
                        }
                    }
                }
                let expected: Vec<_> = previous
                    .map(|window| (clientbound::CONTAINER_CLOSE, window))
                    .into_iter()
                    .chain(menu.map(|menu| (clientbound::OPEN_SCREEN, menu.window)))
                    .collect();
                assert_eq!(packets, expected);
                return;
            }
            let furnace = game.block_entity(level, [2, 64, 0]).unwrap();
            let sign = game.block_entity(level, [4, 64, 0]).unwrap();
            use sunrise_gameplay_common::game::lan_actions::MenuOpen;
            assert!(!player.open_menu(game, sign).unwrap());
            assert_eq!(game.open_block_menu(999, (0, 64, 0)), MenuOpen::Refused);
            game.world
                .block_entities
                .insert((0, 64, 0), BlockEntity::Chest(vec![None; 2]));
            assert!(!player.open_menu(game, chest).unwrap());
            game.world
                .block_entities
                .insert((0, 64, 0), BlockEntity::Chest(vec![None; 27]));
            let dimension = game
                .server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| {
                    std::mem::replace(&mut guest.dimension, "minecraft:the_nether".to_owned())
                });
            assert!(!player.open_menu(game, chest).unwrap());
            {
                let value = dimension;
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.dimension = value);
            }
            let host = game.server.lan.take();
            assert!(!player.open_menu(game, chest).unwrap());
            assert_eq!(game.open_block_menu(12, (0, 64, 0)), MenuOpen::Refused);
            game.server.lan = host;
            let mut guest = game.server.lan.as_mut().unwrap().hub.guests.remove(0);
            guest.kind = sunrise_sim::lan::guest::GuestKind::Fake;
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .insert(0, guest);
            // Fake players have no client menu. This is a refusal, not a hook cancellation.
            assert_eq!(game.open_block_menu(12, (0, 64, 0)), MenuOpen::Refused);
            assert!(!player.open_menu(game, chest).unwrap());
            let mut guest = game.server.lan.as_mut().unwrap().hub.guests.remove(0);
            guest.kind = sunrise_sim::lan::guest::GuestKind::Network;
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .insert(0, guest);
            assert!(player.open_menu(game, chest).unwrap());
            assert_eq!(chest.lock(game), None);
            assert!(chest.set_lock(game, Some(Compound::new())).unwrap());
            assert_eq!(chest.lock(game), Some(&Compound::new()));
            let stick_lock = compound("{items:\"minecraft:stick\"}");
            assert!(chest.set_lock(game, Some(stick_lock.clone())).unwrap());
            let lock = chest.lock(game).unwrap();
            assert_eq!(lock, &stick_lock);
            let Some(Tag::Compound(resident)) = game.world.block_lock((0, 64, 0)) else {
                panic!("the lock was not stored with the block entity");
            };
            assert!(std::ptr::eq(lock, resident));
            assert!(!player.open_menu(game, chest).unwrap());
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
            assert!(player.open_menu(game, chest).unwrap());
            {
                let value = None;
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |g| g.inventory[0] = value);
            }
            assert!(chest.set_lock(game, None).unwrap());
            assert_eq!(chest.lock(game), None);
            assert!(player.open_menu(game, chest).unwrap());
            assert!(player.open_menu(game, furnace).unwrap());

            // Both halves of a double chest participate, whichever half is opened.
            use sunrise_world::chunk::BlockState;
            for (x, kind) in [(6, "left"), (7, "right")] {
                let state = game
                    .pack
                    .assets
                    .registry
                    .resolve(&BlockState {
                        name: "minecraft:chest".into(),
                        properties: [
                            ("facing".into(), "north".into()),
                            ("type".into(), kind.into()),
                            ("waterlogged".into(), "false".into()),
                        ]
                        .into_iter()
                        .collect(),
                    })
                    .unwrap();
                assert_eq!(
                    game.level_set_block(level, [x, 64, 0], state, BlockUpdateFlags::NEIGHBORS),
                    Ok(true)
                );
            }
            let left = game.block_entity(level, [6, 64, 0]).unwrap();
            let right = game.block_entity(level, [7, 64, 0]).unwrap();
            assert!(game.chest_partner((6, 64, 0)).is_some());
            for half in [left, right] {
                assert!(
                    half.set_lock(game, Some(compound("{items:\"minecraft:stick\"}")))
                        .unwrap()
                );
                assert!(!player.open_menu(game, left).unwrap());
                assert!(!player.open_menu(game, right).unwrap());
                assert!(half.set_lock(game, None).unwrap());
                assert!(player.open_menu(game, left).unwrap());
            }
            game.world.block_entities.remove(&(2, 64, 0));
            assert_eq!(
                player.open_menu(game, furnace),
                Err(BlockEntityError::Stale)
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn viewers_follow_open_menus_per_level_and_leave_out_fake_players() {
        use mod_api::{BlockUpdateFlags, EntityRef, Host};
        use sunrise_gameplay_common::game::Dim;
        use sunrise_world::chunk::BlockState;
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let chest =
                |game: &mut sunrise_gameplay::game::ServerGame<'_>, level, x: i32, kind: &str| {
                    let state = game
                        .pack
                        .assets
                        .registry
                        .resolve(&BlockState {
                            name: "minecraft:chest".into(),
                            properties: [
                                ("facing".into(), "north".into()),
                                ("type".into(), kind.into()),
                                ("waterlogged".into(), "false".into()),
                            ]
                            .into_iter()
                            .collect(),
                        })
                        .unwrap();
                    assert_eq!(
                        game.level_set_block(level, [x, 64, 0], state, BlockUpdateFlags::NEIGHBORS),
                        Ok(true)
                    );
                    game.block_entity(level, [x, 64, 0]).unwrap()
                };
            let single = chest(game, level, 0, "single");
            let left = chest(game, level, 6, "left");
            let right = chest(game, level, 7, "right");
            let epoch = game.native_lifecycle.epoch().unwrap();
            // with_guests: uuids and entity ids 12 and 13.
            let guest12 = player_ref(EntityRef::new(epoch, 12), 12);
            let guest13 = player_ref(EntityRef::new(epoch, 13), 13);
            let host: &dyn Host = game;
            assert_eq!(single.viewers(host), Some(vec![]));

            // Two players open the chest, then close it one by one.
            assert!(guest12.open_menu(game, single).unwrap());
            assert!(guest13.open_menu(game, single).unwrap());
            assert_eq!(single.viewers(game), Some(vec![guest12, guest13]));
            game.guest_close_all(12);
            assert_eq!(single.viewers(game), Some(vec![guest13]));
            game.guest_close_all(13);
            assert_eq!(single.viewers(game), Some(vec![]));

            // A menu the acting player's action opened counts before the hub records it,
            // and only once when the hub already has it.
            game.as_guest(13, |game| {
                game.container = Some((0, 64, 0));
                assert_eq!(single.viewers(game), Some(vec![guest13]));
                game.container = None;
            })
            .unwrap();
            assert!(guest12.open_menu(game, single).unwrap());
            game.as_guest(12, |game| {
                game.container = Some((0, 64, 0));
                assert_eq!(single.viewers(game), Some(vec![guest12]));
                game.container = None;
            })
            .unwrap();
            game.guest_close_all(12);

            // A guest that turns spectator during its own turn stops being a viewer then,
            // though the hub still has the old mode until the turn ends.
            assert!(guest12.open_menu(game, single).unwrap());
            game.as_guest(12, |game| {
                assert!(game.set_game_mode(sunrise_command::args::GameMode::Spectator));
                assert_eq!(single.viewers(game), Some(vec![]));
            })
            .unwrap();
            game.guest_close_all(12);
            game.as_guest(12, |game| {
                assert!(game.set_game_mode(sunrise_command::args::GameMode::Survival));
            })
            .unwrap();

            // A double chest's menu holds both halves, whichever was opened.
            assert!(guest12.open_menu(game, right).unwrap());
            assert_eq!(left.viewers(game), Some(vec![guest12]));
            assert_eq!(right.viewers(game), Some(vec![guest12]));
            assert_eq!(single.viewers(game), Some(vec![]));

            // A fake player holding the same menu is not a viewer.
            assert!(guest13.open_menu(game, left).unwrap());
            let mut fake = game.server.lan.as_mut().unwrap().hub.guests.remove(1);
            fake.kind = sunrise_sim::lan::guest::GuestKind::Fake;
            game.server.lan.as_mut().unwrap().hub.guests.insert(1, fake);
            assert_eq!(left.viewers(game), Some(vec![guest12]));

            // The same position in another level has none of these viewers.
            let nether = LevelRef::new(epoch, mod_api::DimensionKey::NETHER);
            game.enter_level(Dim::Nether).unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let nether_right = chest(game, nether, 7, "single");
            assert_eq!(nether_right.viewers(game), Some(vec![]));
            assert_eq!(right.viewers(game), Some(vec![guest12]), "parked level");
            game.enter_level(Dim::Overworld).unwrap();

            // A stale handle reads None.
            game.world.block_entities.remove(&(7, 64, 0));
            assert_eq!(right.viewers(game), None);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn writes_merge_data_name_and_roll_back_with_a_transaction() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_game_at};
        use mod_api::BlockUpdateFlags;
        if run_fresh_cases(
            "cut_layout::game::server_side::native_block_entity_refs::tests::writes_merge_data_name_and_roll_back_with_a_transaction",
            "SUNRISE_VANILLA_BE_REGISTRY_CHILD",
            &["vanilla"],
        ) {
            return;
        }
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root = sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
            .unwrap()
            .prepared()
            .to_path_buf();
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let flags = BlockUpdateFlags::NEIGHBORS;
            let states = ["minecraft:chest", "minecraft:daylight_detector"];
            for (i, name) in states.into_iter().enumerate() {
                let state = game.pack.assets.registry.default_state(name).unwrap();
                assert_eq!(
                    game.level_set_block(level, [i as i32 * 2, 64, 0], state, flags),
                    Ok(true)
                );
            }
            let chest = game.block_entity(level, [0, 64, 0]).unwrap();
            let sensor = game.block_entity(level, [2, 64, 0]).unwrap();
            let stick = "{Items:[{Slot:3b,id:\"minecraft:stick\",count:3}]}";
            let slot3 = |game: &sunrise_gameplay::game::ServerGame<'_>| match game
                .world
                .block_entities
                .get(&(0, 64, 0))
            {
                Some(BlockEntity::Chest(items)) => items[3].as_ref().map(|s| s.count),
                _ => panic!("chest block entity missing"),
            };

            // A merge loads the data, ignores the identity keys and keeps the rest.
            assert!(
                chest
                    .merge_data(
                        game,
                        compound(&format!("{{id:\"minecraft:furnace\",x:9,{}", &stick[1..]))
                    )
                    .unwrap()
            );
            assert_eq!(slot3(game), Some(3));
            assert_eq!(
                chest.kind(game).and_then(|kind| kind.id(game)),
                Some("minecraft:chest")
            );
            assert_eq!(chest.position(game), Some([0, 64, 0]));
            assert!(
                !chest.merge_data(game, compound(stick)).unwrap(),
                "no change"
            );
            assert!(chest.merge_data(game, compound("{Items:[]}")).unwrap());
            assert_eq!(slot3(game), None);

            // Names: a chest has one, a daylight detector does not.
            assert_eq!(chest.name(game), None);
            assert!(chest.set_name(game, Some("Loot")).unwrap());
            assert_eq!(chest.name(game), Some("Loot".to_owned()));
            let data = chest.data(game).unwrap();
            assert_eq!(
                data.get(&"CustomName".into()),
                Some(&Tag::String("Loot".into()))
            );
            assert!(!sensor.set_name(game, Some("Nope")).unwrap());
            assert_eq!(sensor.name(game), None);
            assert!(chest.set_name(game, None).unwrap());
            assert_eq!(chest.name(game), None);

            // An unrelated API merge must preserve the loaded rich name, not
            // remove it through the plain-text name accessor's projection.
            let styled = "{text:\"Styled\",color:\"aqua\",extra:[{text:\" chest\",bold:1b}]}";
            let name = Tag::Compound(compound(styled));
            assert!(game.load_block_entity_nbt([0, 64, 0], &format!("{{CustomName:{styled}}}")));
            assert_eq!(game.world.block_names.get(&(0, 64, 0)), Some(&name));
            assert!(chest.merge_data(game, compound(stick)).unwrap());
            assert_eq!(slot3(game), Some(3));
            assert_eq!(game.world.block_names.get(&(0, 64, 0)), Some(&name));
            assert_eq!(
                chest.data(game).unwrap().get(&"CustomName".into()),
                Some(&name)
            );
            assert!(chest.replace_data(game, compound("{}")).unwrap());
            assert_eq!((slot3(game), chest.name(game)), (None, None));

            // An abort puts back data and name; a commit keeps them.
            let host: &mut dyn ServerHost = game;
            let aborted: Result<(), TransactionError<()>> = host.transaction(|host, _| {
                assert!(chest.merge_data(host, compound(stick)).unwrap());
                assert!(chest.set_name(host, Some("Gone")).unwrap());
                assert!(chest.set_changed(host).unwrap());
                Err(())
            });
            assert_eq!(aborted, Err(TransactionError::Aborted(())));
            assert_eq!((slot3(game), chest.name(game)), (None, None));
            assert!(game.transactions.is_empty());
            let host: &mut dyn ServerHost = game;
            let committed: Result<(), TransactionError<()>> = host.transaction(|host, _| {
                assert!(chest.merge_data(host, compound(stick)).unwrap());
                assert!(chest.set_name(host, Some("Kept")).unwrap());
                Ok(())
            });
            assert_eq!(committed, Ok(()));
            assert_eq!(
                (slot3(game), chest.name(game)),
                (Some(3), Some("Kept".to_owned()))
            );

            // Replacement uses the same journal and restores the original payload on abort.
            let aborted: Result<(), TransactionError<()>> = (game as &mut dyn ServerHost)
                .transaction(|host, _| {
                    assert!(chest.replace_data(host, compound("{}")).unwrap());
                    Err(())
                });
            assert_eq!(aborted, Err(TransactionError::Aborted(())));
            assert_eq!(
                (slot3(game), chest.name(game)),
                (Some(3), Some("Kept".to_owned()))
            );

            // Merge preserves omitted data; replacement clears the name and items.
            assert!(
                chest
                    .merge_data(game, compound("{CustomName:\"Changed\"}"))
                    .unwrap()
            );
            assert_eq!(slot3(game), Some(3));
            assert_eq!(chest.name(game), Some("Changed".to_owned()));
            game.world.block_log = Some(Vec::new());
            assert!(chest.replace_data(game, compound("{}")).unwrap());
            assert_eq!((slot3(game), chest.name(game)), (None, None));
            assert_eq!(game.world.block_log, Some(vec![(0, 64, 0)]));

            // Clients are told: the block log names the position, and the chunk is dirty.
            game.world.block_log = Some(Vec::new());
            game.world.modified.clear();
            assert!(chest.set_name(game, Some("Told")).unwrap());
            assert_eq!(game.world.block_log, Some(vec![(0, 64, 0)]));
            assert!(game.world.modified.contains(&(0, 0)));
            game.world.modified.clear();
            assert!(chest.set_changed(game).unwrap());
            assert!(game.world.modified.contains(&(0, 0)));

            // A removed block entity is a stale handle.
            game.world.block_entities.remove(&(0, 64, 0));
            let stale = Err(BlockEntityError::Stale);
            assert_eq!(chest.replace_data(game, compound("{}")), stale);
            assert_eq!(chest.set_changed(game), stale);
            assert_eq!(chest.merge_data(game, compound(stick)), stale);
            assert_eq!(chest.set_name(game, Some("x")), stale);
            assert_eq!(chest.name(game), None);
        });
    }

    /// `LevelWriteHost::set_block` in a level hook: the block entity goes with
    /// its block and comes with the new one, as the Host's write does.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_level_hook_set_block_replaces_whole_states_and_their_block_entities() {
        use super::super::native_entity_refs::tests::{with_client_game, with_game};
        use mod_api::{BlockUpdateFlags, LevelWriteHost};
        use sunrise_gameplay_common::game::{
            GameGameExt1 as _, SideGame, mod_api_host::LevelTicks,
        };
        use sunrise_world::chunk::BlockState;

        fn write<'g>(
            game: &mut impl SideGame<'g>,
            pos: [i32; 3],
            state: mod_api::BlockState,
            flags: BlockUpdateFlags,
        ) -> bool {
            game.with_level(|_, level, ticks| {
                LevelWriteHost::set_block(&mut LevelTicks::new(level, ticks), pos, state, flags)
            })
        }
        let flags = BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::KNOWN_SHAPE;
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let state = |name: &str, age: Option<&str>| {
                registry
                    .resolve(&BlockState {
                        name: name.into(),
                        properties: age
                            .map(|age| ("age".into(), age.into()))
                            .into_iter()
                            .collect(),
                    })
                    .unwrap()
            };
            let stone = state("minecraft:stone", None);
            let chest = state("minecraft:chest", None);
            let young = state("minecraft:wheat", Some("0"));
            let grown = state("minecraft:wheat", Some("7"));
            let api_stone = stone.try_into_api().unwrap();
            let api_chest = chest.try_into_api().unwrap();
            let api_young = young.try_into_api().unwrap();
            let api_grown = grown.try_into_api().unwrap();
            let key = (0, 64, 0);

            assert!(write(game, [0, 64, 0], api_stone, flags));
            assert!(game.world.block_entities.get(&key).is_none());
            assert!(write(game, [0, 64, 0], api_chest, flags));
            assert!(matches!(
                game.world.block_entities.get(&key),
                Some(BlockEntity::Chest(_))
            ));
            assert!(write(game, [0, 64, 0], api_stone, flags));
            assert_eq!(game.world.block(0, 64, 0), stone);
            assert!(game.world.block_entities.get(&key).is_none());

            // A crop's whole-state growth: no block entity, and a repeat changes nothing.
            assert!(write(game, [0, 64, 0], api_young, flags));
            assert!(write(game, [0, 64, 0], api_grown, flags));
            assert_eq!(game.world.block(0, 64, 0), grown);
            assert!(game.world.block_entities.get(&key).is_none());
            assert!(!write(game, [0, 64, 0], api_grown, flags));

            // Unloaded, outside the build height and invalid: refused, nothing loaded.
            let chunks = game.world.chunks.len();
            assert!(!write(game, [16, 64, 0], api_stone, flags));
            assert!(!write(game, [0, 10000, 0], api_stone, flags));
            assert!(!write(
                game,
                [0, 64, 0],
                mod_api::__private::block_state_from_ordinal(api_stone.block(), u16::MAX),
                flags
            ));
            assert_eq!(game.world.chunks.len(), chunks);
            assert_eq!(game.world.block(0, 64, 0), grown);

            // The same Host write still creates the block entity.
            assert!(game.native_set_block("minecraft:overworld", [1, 64, 0], chest, flags.bits()));
            assert!(matches!(
                game.world.block_entities.get(&(1, 64, 0)),
                Some(BlockEntity::Chest(_))
            ));
        });

        with_client_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let grown = registry
                .resolve(&BlockState {
                    name: "minecraft:wheat".into(),
                    properties: [("age".into(), "7".into())].into(),
                })
                .unwrap();
            game.world.set(0, 64, 0, grown).unwrap();
            let api_stone = registry
                .default_state("minecraft:stone")
                .unwrap()
                .try_into_api()
                .unwrap();
            assert!(!write(game, [0, 64, 0], api_stone, flags));
            assert_eq!(game.world.block(0, 64, 0), grown);
        });
    }
}
