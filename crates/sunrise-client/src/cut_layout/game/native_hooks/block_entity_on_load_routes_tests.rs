#![cfg(test)]

#[cfg(test)]
mod tests {
    #[cfg(feature = "client")]
    use mod_api::*;
    #[cfg(feature = "client")]
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    #[cfg(feature = "client")]
    use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::native_hooks::block_entity_on_load::BlockEntityOnLoadGameExt1;
    #[cfg(feature = "client")]
    use sunrise_level::chunk::Chunk;
    #[cfg(feature = "client")]
    use sunrise_level::containers::BlockEntity;
    #[cfg(feature = "client")]
    use {
        crate::game::server_side::native_entity_refs::tests::with_game_role_client,
        sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
        sunrise_gameplay::game::native_entity_refs::tests::with_game_role,
    };
    #[cfg(feature = "client")]
    type Log = Arc<Mutex<Vec<(&'static str, i32)>>>;
    #[cfg(feature = "client")]
    type Payloads = Arc<Mutex<Vec<(BlockEntityRef, i32, [i32; 3], LevelRef)>>>;
    #[cfg(feature = "client")]
    struct Listener {
        pub air: BlockKey,
        replacement: BlockKey,
        log: Log,
        replace: Arc<AtomicBool>,
        report_payload: bool,
    }
    #[cfg(feature = "client")]
    #[mod_api::sunrise_part]
    impl BlockPart for Listener {}
    #[cfg(feature = "client")]
    #[mod_api::sunrise_part]
    impl BlockEntityPart for Listener {
        fn block_entity(&self, _: LevelSide<'_>, _: &mut BlockEntityEvent<'_>) -> Option<Compound> {
            Some(Compound::from([("count".into(), Tag::Int(7))]))
        }
        fn block_entity_tick(&self, _: &mut BlockEntityTickEvent<'_>) {
            self.log.lock().unwrap().push(("tick", -1));
        }
        fn block_entity_load_additional(&self, event: &mut BlockEntityLoadAdditionalEvent<'_>) {
            if self.report_payload {
                let count = event.tag.get(&"count".into()).unwrap();
                let seen = event
                    .subject
                    .with_view(|view| view.nbt().get(&"count".into()).cloned())
                    .expect("live resident block entity");
                assert_eq!(seen.as_ref(), Some(count));
                let Tag::Int(count) = count else {
                    panic!("save count is an int")
                };
                self.log.lock().unwrap().push(("payload", *count));
                // This Part's event view is local. A later Part still sees the installed compound.
                static OTHER: std::sync::OnceLock<Compound> = std::sync::OnceLock::new();
                event.tag = OTHER.get_or_init(Compound::new);
            }
        }
        fn block_entity_on_load(&self, event: &mut BlockEntityOnLoadEvent<'_>) {
            let (label, count, pos, level) = match &mut event.subject {
                BlockEntitySideScope::Server(scope) => scope
                    .with_view(|view| {
                        let Some(Tag::Int(count)) = view.nbt().get(&"count".into()) else {
                            panic!("missing saved count")
                        };
                        ("load", *count, view.position(), view.level())
                    })
                    .expect("live resident server block entity"),
                BlockEntitySideScope::Client(scope) => scope
                    .with_view(|view| {
                        let Some(Tag::Int(count)) = view.nbt().get(&"count".into()) else {
                            panic!("missing saved count")
                        };
                        ("client_load", *count, view.position(), view.level())
                    })
                    .expect("live resident client block entity"),
            };
            self.log.lock().unwrap().push((label, count));
            if self.replace.load(Ordering::SeqCst) {
                let BlockEntitySideScope::Server(scope) = &mut event.subject else {
                    panic!()
                };
                let host = scope.host();
                let state = BlockState::of(&host.block_states(), self.replacement, &[]).unwrap();
                let air = BlockState::of(&host.block_states(), self.air, &[]).unwrap();
                assert_eq!(
                    level.set_block(host, pos, air, BlockUpdateFlags::KNOWN_SHAPE),
                    Ok(true)
                );
                assert_eq!(
                    level.set_block(host, pos, state, BlockUpdateFlags::KNOWN_SHAPE),
                    Ok(true)
                );
            }
        }
    }
    #[cfg(feature = "client")]
    struct PayloadPeer(Payloads);
    #[cfg(feature = "client")]
    #[mod_api::sunrise_part]
    impl BlockPart for PayloadPeer {}
    #[cfg(feature = "client")]
    #[mod_api::sunrise_part]
    impl BlockEntityPart for PayloadPeer {
        fn block_entity_load_additional(&self, event: &mut BlockEntityLoadAdditionalEvent<'_>) {
            let Some(Tag::Int(count)) = event.tag.get(&"count".into()) else {
                panic!("the second Part did not read the applied payload");
            };
            let (entity, pos, level) = event
                .subject
                .with_view(|view| (view.handle(), view.position(), view.level()))
                .expect("live resident block entity");
            self.0.lock().unwrap().push((entity, *count, pos, level));
        }
    }

    #[cfg(feature = "client")]
    struct User {
        pub log: Log,
        replace: Arc<AtomicBool>,
        payloads: Option<Payloads>,
    }
    #[cfg(feature = "client")]
    #[mod_api::__subscriptions]
    impl Hooks for User {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let key = r
                .block_entity_type("holder")
                .blocks(&["test:a", "test:b", "test:box_shulker_box"])
                .build();
            let air = r.block_key("minecraft:air");
            let replacement = r.block_key("test:b");
            r.block_entity_part(
                &key,
                Listener {
                    air,
                    replacement,
                    log: self.log.clone(),
                    replace: self.replace.clone(),
                    report_payload: self.payloads.is_some(),
                },
            )
            .unwrap();
            if let Some(payloads) = &self.payloads {
                r.block_entity_part(&key, PayloadPeer(payloads.clone()))
                    .unwrap();
            }
        }
        fn items(&self, _: &RegistrationContext, e: &mut ItemsEvent<'_>) {
            let mut item = Item::new("test:box_shulker_box", "minecraft:purple_shulker_box");
            item.block = Some("test:box_shulker_box".into());
            e.register(item).unwrap();
        }
        fn blocks(&self, _: &RegistrationContext, e: &mut BlocksEvent<'_>) {
            // Its item is the shulker box item above, not a block item of its own.
            e.register(Block::new("test:box_shulker_box", "minecraft:barrel").with_item(false))
                .unwrap();
            for block in ["test:a", "test:b"].map(|id| Block::new(id, "minecraft:white_banner")) {
                e.register(block).unwrap();
            }
        }
    }
    #[cfg(feature = "client")]
    fn with_fixture(
        replace: bool,
        f: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, Log, Arc<AtomicBool>),
    ) {
        with_fixture_role(
            replace,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            f,
        );
    }
    #[cfg(feature = "client")]
    fn with_fixture_role(
        replace: bool,
        role: sunrise_gameplay_common::game::execution_role::ExecutionRole,
        f: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, Log, Arc<AtomicBool>),
    ) {
        with_fixture_options(replace, role, false, |game, log, replace, _| {
            f(game, log, replace)
        });
    }

    #[cfg(feature = "client")]
    fn with_fixture_role_client(
        replace: bool,
        role: sunrise_gameplay_common::game::execution_role::ExecutionRole,
        f: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>, Log, Arc<AtomicBool>),
    ) {
        with_fixture_options_client(replace, role, false, |game, log, replace, _| {
            f(game, log, replace)
        });
    }
    #[cfg(feature = "client")]
    fn with_fixture_options(
        replace: bool,
        role: sunrise_gameplay_common::game::execution_role::ExecutionRole,
        report_payload: bool,
        f: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, Log, Arc<AtomicBool>, Payloads),
    ) {
        let log = Log::default();
        let payloads = Payloads::default();
        let replace = Arc::new(AtomicBool::new(replace));
        let mods: Mods = vec![ModEntry::new(
            "test",
            Box::new(User {
                log: log.clone(),
                replace: replace.clone(),
                payloads: report_payload.then(|| payloads.clone()),
            }),
        )]
        .into();
        mod_platform::__private::install(mods.clone());
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root =
            sunrise_gameplay_common::game::mods::content_root(&base, &Default::default()).unwrap();
        with_game_role(root.prepared().to_path_buf(), role, |game| {
            if game.is_logical_server() {
                game.options.dedicated = Some(vec![]);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let level = game.level("minecraft:overworld").unwrap();
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.level_set_block(level, [1, 63, 1], stone, BlockUpdateFlags::NONE)
                    .unwrap();
            }
            f(game, log, replace, payloads);
        });
    }

    #[cfg(feature = "client")]
    fn with_fixture_options_client(
        replace: bool,
        role: sunrise_gameplay_common::game::execution_role::ExecutionRole,
        report_payload: bool,
        f: impl FnOnce(
            &mut sunrise_client_gameplay::game::ClientGame<'_>,
            Log,
            Arc<AtomicBool>,
            Payloads,
        ),
    ) {
        let log = Log::default();
        let payloads = Payloads::default();
        let replace = Arc::new(AtomicBool::new(replace));
        let mods: Mods = vec![ModEntry::new(
            "test",
            Box::new(User {
                log: log.clone(),
                replace: replace.clone(),
                payloads: report_payload.then(|| payloads.clone()),
            }),
        )]
        .into();
        mod_platform::__private::install(mods.clone());
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root =
            sunrise_gameplay_common::game::mods::content_root(&base, &Default::default()).unwrap();
        with_game_role_client(root.prepared().to_path_buf(), role, |game| {
            f(game, log, replace, payloads);
        });
    }
    #[cfg(feature = "client")]
    fn place(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
        let level = game.level("minecraft:overworld").unwrap();
        let state = game.pack.assets.registry.default_state("test:a").unwrap();
        assert_eq!(
            game.level_set_block(level, [1, 64, 1], state, BlockUpdateFlags::NONE),
            Ok(true)
        );
        assert!(matches!(
            game.world.block_entities.get(&(1, 64, 1)),
            Some(BlockEntity::Mod(_))
        ));
    }
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn block_entity_on_load_reentrant_replacement_is_bounded() {
        const NAME: &str = "cut_layout::game::native_hooks::block_entity_on_load_routes_tests::tests::block_entity_on_load_reentrant_replacement_is_bounded";
        if run_fresh_cases(NAME, "SUNRISE_ON_LOAD_REENTRANT", &["run"]) {
            return;
        }
        with_fixture(true, |game, log, replace| {
            place(game);
            game.fire_pending_block_entity_loads();
            assert_eq!(*log.lock().unwrap(), [("load", 7)]);
            assert_eq!(game.world.block_entities.pending_on_load_len(), 1);
            game.tick_mod_block_entity((1, 64, 1));
            assert_eq!(*log.lock().unwrap(), [("load", 7), ("load", 7)]);
            assert_eq!(game.world.block_entities.pending_on_load_len(), 1);
            replace.store(false, Ordering::SeqCst);
            game.tick_mod_block_entity((1, 64, 1));
            assert_eq!(
                *log.lock().unwrap(),
                [("load", 7), ("load", 7), ("load", 7), ("tick", -1)]
            );
            assert_eq!(game.world.block_entities.pending_on_load_len(), 0);
        });
    }
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn block_entity_on_load_shared_type_replacement_has_fresh_handle() {
        const NAME: &str = "cut_layout::game::native_hooks::block_entity_on_load_routes_tests::tests::block_entity_on_load_shared_type_replacement_has_fresh_handle";
        if run_fresh_cases(NAME, "SUNRISE_ON_LOAD_SHARED_TYPE", &["run"]) {
            return;
        }
        with_fixture(false, |game, log, _| {
            place(game);
            let level = game.level("minecraft:overworld").unwrap();
            let old = game.block_entity(level, [1, 64, 1]).unwrap();
            let generation = game.world.block_entities.generation(&(1, 64, 1)).unwrap();
            game.fire_pending_block_entity_loads();
            let b = game.pack.assets.registry.default_state("test:b").unwrap();
            assert_eq!(
                game.level_set_block(level, [1, 64, 1], b, BlockUpdateFlags::NONE),
                Ok(true)
            );
            let new = game.block_entity(level, [1, 64, 1]).unwrap();
            assert_ne!(old, new);
            assert_ne!(
                game.world.block_entities.generation(&(1, 64, 1)),
                Some(generation)
            );
            assert_eq!(old.data(game), None);
            assert_eq!(
                new.data(game).unwrap().get(&"count".into()),
                Some(&Tag::Int(7))
            );
            assert_eq!(game.world.block_entities.pending_on_load_len(), 1);
            game.tick_mod_block_entity((1, 64, 1));
            game.tick_mod_block_entity((1, 64, 1));
            assert_eq!(
                *log.lock().unwrap(),
                [("load", 7), ("load", 7), ("tick", -1), ("tick", -1)]
            );
        });
    }
    #[cfg(feature = "client")]
    fn assert_first_two_ticks(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        log: &Log,
        count: i32,
    ) {
        assert!(log.lock().unwrap().is_empty());
        assert_eq!(game.world.block_entities.pending_on_load_len(), 1);
        game.tick_mod_block_entity((1, 64, 1));
        game.tick_mod_block_entity((1, 64, 1));
        assert_eq!(
            *log.lock().unwrap(),
            [("load", count), ("tick", -1), ("tick", -1)]
        );
    }
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn saved_payload_notices_distinguish_loads_from_attachment_and_api_edits() {
        const NAME: &str = "cut_layout::game::native_hooks::block_entity_on_load_routes_tests::tests::saved_payload_notices_distinguish_loads_from_attachment_and_api_edits";
        if run_fresh_cases(NAME, "SUNRISE_LOAD_PAYLOAD", &["run"]) {
            return;
        }
        with_fixture_options(
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            true,
            |game, log, _, payloads| {
                game.build_registries();
                game.set_allow_commands(true);
                place(game);
                game.fire_pending_block_entity_loads();
                assert!(game.run_command("data merge block 1 64 1 {count:19}"));
                let level = game.level("minecraft:overworld").unwrap();
                let entity = game.block_entity(level, [1, 64, 1]).unwrap();
                assert_eq!(
                    entity.merge_data(game, Compound::from([("count".into(), Tag::Int(29))])),
                    Ok(true)
                );
                assert_eq!(*log.lock().unwrap(), [("load", 7), ("payload", 19)]);
                assert_eq!(*payloads.lock().unwrap(), [(entity, 19, [1, 64, 1], level)]);

                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(2, 63, 1, stone).unwrap();
                assert!(game.run_command("clone 1 64 1 1 64 1 2 64 1"));
                let clone = game.block_entity(level, [2, 64, 1]).unwrap();
                assert_ne!(clone, entity);
                assert_eq!(
                    *payloads.lock().unwrap(),
                    [
                        (entity, 19, [1, 64, 1], level),
                        (clone, 29, [2, 64, 1], level)
                    ]
                );
                game.fire_pending_block_entity_loads();
                assert_eq!(
                    *log.lock().unwrap(),
                    [("load", 7), ("payload", 19), ("payload", 29), ("load", 29),]
                );

                let key = (1, 64, 1);
                let saved = game.world.block_entities.remove(&key).unwrap();
                let mut chunk = Chunk::empty(0, 0);
                chunk.block_entities.push((key, saved));
                game.world.install_chunk_block_entities(&mut chunk);
                game.fire_pending_block_entity_loads();
                assert_eq!(
                    *log.lock().unwrap(),
                    [
                        ("load", 7),
                        ("payload", 19),
                        ("payload", 29),
                        ("load", 29),
                        ("payload", 29),
                        ("load", 29),
                    ]
                );
            },
        );
    }
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn block_entity_on_load_command_placement() {
        const NAME: &str = "cut_layout::game::native_hooks::block_entity_on_load_routes_tests::tests::block_entity_on_load_command_placement";
        if run_fresh_cases(NAME, "SUNRISE_ON_LOAD_COMMAND", &["run"]) {
            return;
        }
        with_fixture(false, |game, log, _| {
            game.build_registries();
            game.set_allow_commands(true);
            assert!(game.run_command("setblock 1 64 1 test:a"));
            assert_first_two_ticks(game, &log, 7);
        });
    }
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn block_entity_on_load_dispenser_placement() {
        const NAME: &str = "cut_layout::game::native_hooks::block_entity_on_load_routes_tests::tests::block_entity_on_load_dispenser_placement";
        if run_fresh_cases(NAME, "SUNRISE_ON_LOAD_DISPENSER", &["run"]) {
            return;
        }
        with_fixture(false, |game, log, _| {
            let left = game
                .dispense_item(
                    (0, 64, 1),
                    sunrise_core::geometry::Direction::East,
                    sunrise_items::items::ItemStack::new("test:box_shulker_box", 1),
                )
                .unwrap();
            assert_eq!(left.continue_value(), Some(None));
            assert!(matches!(
                game.world.block_entities.get(&(1, 64, 1)),
                Some(BlockEntity::Mod(_))
            ));
            assert_first_two_ticks(game, &log, 7);
        });
    }
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn block_entity_on_load_falling_block_placement() {
        const NAME: &str = "cut_layout::game::native_hooks::block_entity_on_load_routes_tests::tests::block_entity_on_load_falling_block_placement";
        if run_fresh_cases(NAME, "SUNRISE_ON_LOAD_FALLING", &["run"]) {
            return;
        }
        with_fixture(false, |game, log, _| {
            let level = game.level("minecraft:overworld").unwrap();
            let state = game.pack.assets.registry.default_state("test:a").unwrap();
            assert!(
                game.level_spawn_falling_block(
                    level,
                    state,
                    [1.5, 66.0, 1.5],
                    [0.0; 3],
                    Falling::default(),
                )
                .unwrap()
                .is_some()
            );
            let join = game.falling[0].join;
            let mut block = game.falling.extract(join).unwrap();
            let id = block.id;
            block.saved = Some(Box::new((
                [1, 2, 3, 4],
                Tag::Compound(Compound::from([(
                    "TileEntityData".into(),
                    Tag::Compound(Compound::from([("count".into(), Tag::Int(29))])),
                )])),
            )));
            game.finish_falling_block(block, [1, 64, 1]);
            game.falling.retire_extracted(id, join);
            assert_first_two_ticks(game, &log, 29);
        });
    }
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn block_entity_on_load_stale_queued_identity_is_skipped() {
        const NAME: &str = "cut_layout::game::native_hooks::block_entity_on_load_routes_tests::tests::block_entity_on_load_stale_queued_identity_is_skipped";
        if run_fresh_cases(NAME, "SUNRISE_ON_LOAD_STALE", &["run"]) {
            return;
        }
        with_fixture(false, |game, log, _| {
            place(game);
            let level = game.level("minecraft:overworld").unwrap();
            let b = game.pack.assets.registry.default_state("test:b").unwrap();
            assert_eq!(
                game.level_set_block(level, [1, 64, 1], b, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(game.world.block_entities.pending_on_load_len(), 2);
            game.fire_pending_block_entity_loads();
            assert_eq!(*log.lock().unwrap(), [("load", 7)]);
            assert_eq!(game.world.block_entities.pending_on_load_len(), 0);
            game.tick_mod_block_entity((1, 64, 1));
            assert_eq!(*log.lock().unwrap(), [("load", 7), ("tick", -1)]);
        });
    }
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn block_entity_on_load_real_client_chunk_receive() {
        use sunrise_protocol::{
            chunk::{BlockEntityInfo, ChunkData, LightData, Paletted, Section, Strategy},
            packets::play::{LevelChunkWithLight, SpawnInfo},
            registry::Builtin,
        };
        const NAME: &str = "cut_layout::game::native_hooks::block_entity_on_load_routes_tests::tests::block_entity_on_load_real_client_chunk_receive";
        if run_fresh_cases(NAME, "SUNRISE_ON_LOAD_CLIENT", &["run"]) {
            return;
        }
        with_fixture_role_client(
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
            |game, log, _| {
                game.native_lifecycle =
                    sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycle::default();
                let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                game.connect_local(client, "OnLoadChunk").unwrap();
                game.enter_remote_dimension(
                    &SpawnInfo {
                        dimension_type: 0,
                        dimension: "minecraft:overworld".into(),
                        seed: 0,
                        game_type: 1,
                        previous_game_type: None,
                        debug: false,
                        flat: true,
                        last_death: None,
                        portal_cooldown: 0,
                        sea_level: 63,
                    },
                    RemoteDimensionEntry::Login,
                    sunrise_client::level_loading::Reason::Other,
                );
                let remote = game.client.remote.as_mut().unwrap();
                remote.logged_in = true;
                let mut builtin = (*remote.builtin).clone();
                builtin.registries.extend(Builtin::parse(r#"{"minecraft:block_entity_type":{"entries":{"test:holder":{"protocol_id":0}}}}"#).unwrap().registries);
                remote.builtin = Arc::new(builtin);
                let air = game.res.facts().air.0 as i32;
                let state = game.pack.assets.registry.default_state("test:a").unwrap().0 as i32;
                let sections: Vec<_> = (0..24)
                    .map(|i| {
                        let mut states = vec![air; 4096];
                        if i == 8 {
                            states[17] = state;
                        }
                        Section {
                            non_empty: if i == 8 { 1 } else { 0 },
                            fluids: 0,
                            states: Paletted::from_values(&states, Strategy::Blocks, 16),
                            biomes: Paletted::from_values(&[0; 64], Strategy::Biomes, 2),
                        }
                    })
                    .collect();
                let packet = LevelChunkWithLight {
                    x: 0,
                    z: 0,
                    chunk: ChunkData {
                        heightmaps: vec![],
                        buffer: ChunkData::from_sections(&sections),
                        block_entities: vec![BlockEntityInfo {
                            packed_xz: 0x11,
                            y: 64,
                            kind: 0,
                            tag: Tag::Compound(Compound::from([("count".into(), Tag::Int(23))])),
                        }],
                    },
                    light: LightData {
                        sky_mask: vec![],
                        block_mask: vec![],
                        empty_sky_mask: vec![],
                        empty_block_mask: vec![],
                        sky: vec![],
                        block: vec![],
                    },
                };
                game.remote_chunk(&packet, None);
                assert!(log.lock().unwrap().is_empty());
                assert_eq!(game.world.block_entities.pending_on_load_len(), 1);
                let level = game.level("minecraft:overworld").unwrap();
                let first = game.block_entity(level, [1, 64, 1]).unwrap();
                game.fire_pending_block_entity_loads();
                assert_eq!(*log.lock().unwrap(), [("client_load", 23)]);
                game.remote_chunk(&packet, None);
                assert_ne!(game.block_entity(level, [1, 64, 1]).unwrap(), first);
                game.fire_pending_block_entity_loads();
                game.fire_pending_block_entity_loads();
                assert_eq!(
                    *log.lock().unwrap(),
                    [("client_load", 23), ("client_load", 23)]
                );
            },
        );
    }
}
