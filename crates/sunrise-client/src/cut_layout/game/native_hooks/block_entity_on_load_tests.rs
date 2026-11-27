#![cfg(test)]

#[cfg(test)]
mod tests {
    use mod_api::{
        Block, BlockEntityOnLoadEvent, BlockEntityPart, BlockEntitySideScope, BlocksEvent, Hooks,
        ModEntry, Registrar, RegistrationContext,
    };
    use std::sync::{Arc, Mutex};
    #[cfg(feature = "client")]
    use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_gameplay_common::game::native_hooks::block_entity_on_load::BlockEntityOnLoadGameExt1;
    use sunrise_gameplay_common::world_biomes;
    use sunrise_level::{
        chunk::Chunk,
        containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields},
    };
    use {
        sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
        sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
        sunrise_gameplay::game::native_entity_refs::tests::with_game,
        sunrise_gameplay::game::native_entity_refs::tests::with_game_at,
    };

    type Log = Arc<Mutex<Vec<(i32, i32)>>>;
    struct Loaded(Log);
    #[mod_api::sunrise_part]
    impl mod_api::BlockPart for Loaded {}
    #[mod_api::sunrise_part]
    impl BlockEntityPart for Loaded {
        fn block_entity_tick(&self, event: &mut mod_api::BlockEntityTickEvent<'_>) {
            event.subject.with_view(|entity| {
                self.0.lock().unwrap().push((entity.position()[0], -1));
            });
        }

        fn block_entity_on_load(&self, event: &mut BlockEntityOnLoadEvent<'_>) {
            let (pos, count) = match &mut event.subject {
                BlockEntitySideScope::Server(scope) => scope.with_view(|view| {
                    let Some(mod_api::Tag::Int(count)) = view.nbt().get(&"count".into()) else {
                        panic!("saved count missing")
                    };
                    (view.position(), *count)
                }),
                BlockEntitySideScope::Client(scope) => scope.with_view(|view| {
                    let Some(mod_api::Tag::Int(count)) = view.nbt().get(&"count".into()) else {
                        panic!("saved count missing")
                    };
                    (view.position(), *count)
                }),
            }
            .expect("live resident block entity");
            self.0.lock().unwrap().push((pos[0], count));
        }
    }
    struct User(Log, &'static [&'static str]);
    #[mod_api::__subscriptions]
    impl Hooks for User {
        fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
            let kind = registrar.block_entity_type("holder").blocks(self.1).build();
            registrar
                .block_entity_part(&kind, Loaded(self.0.clone()))
                .unwrap();
        }

        fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
            for &id in self.1 {
                event
                    .register(Block::new(id, "minecraft:stone").with_item(false))
                    .unwrap();
            }
        }
    }
    fn holder(count: i32) -> BlockEntity {
        BlockEntity::Mod(Box::new(ModBlockEntity::new(
            "test:holder".into(),
            ModBlockEntityFields {
                nbt: sunrise_core::nbt::Compound::from([(
                    "count".into(),
                    sunrise_core::nbt::Tag::Int(count),
                )]),
                ..Default::default()
            },
        )))
    }

    #[test]
    #[ignore = "daily"]
    fn block_entity_on_load_saved_data_placement_rewrite_and_reload() {
        const NAME: &str = "cut_layout::game::native_hooks::block_entity_on_load_tests::tests::block_entity_on_load_saved_data_placement_rewrite_and_reload";
        // Content startup freezes the engine type table, including ticker routing.
        if run_fresh_cases(NAME, "SUNRISE_ON_LOAD_TICK_CHILD", &["game"]) {
            return;
        }
        let log = Log::default();
        let mods: mod_api::Mods = vec![ModEntry::new(
            "test",
            Box::new(User(log.clone(), &["test:holder_block"])),
        )]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            let root = sunrise_gameplay_common::game::mods::content_root(
                &fixture_root(),
                &Default::default(),
            )
            .unwrap();
            with_game_at(root.prepared().to_path_buf(), |game| {
                game.options.dedicated = Some(Vec::new());
                let registry = game.pack.assets.registry;
                let block = registry.default_state("test:holder_block").unwrap();
                let mut chunk = Chunk::empty(0, 0);
                for (x, count) in [(1, 7), (2, 9)] {
                    chunk.set(x, 64, 1, block.0);
                    chunk.block_entities.push(((x, 64, 1), holder(count)));
                }
                game.test_save()
                    .save(&[Arc::new(chunk)], registry, world_biomes!(game))
                    .unwrap();

                // The first resident instances come from a saved region, not from placement.
                let mut chunk = game
                    .test_save()
                    .load(0, 0, registry, world_biomes!(game))
                    .unwrap();
                game.world.install_chunk_block_entities(&mut chunk);
                game.world.chunks.insert((0, 0), Arc::new(chunk));
                assert!(game.server.streaming.set_forced(0, 0, true));
                game.update_chunk_tickets();
                game.tick_block_entities().unwrap();
                assert_eq!(*log.lock().unwrap(), [(2, 9), (1, 7), (2, -1), (1, -1)]);
                log.lock().unwrap().clear();
                game.fire_pending_block_entity_loads();
                game.world.block_entities.insert((1, 64, 1), holder(11));
                game.fire_pending_block_entity_loads();
                assert!(log.lock().unwrap().is_empty());
                game.world.set(3, 64, 1, block).unwrap();
                game.world.block_entities.insert((3, 64, 1), holder(0));
                game.fire_pending_block_entity_loads();
                game.fire_pending_block_entity_loads();
                assert_eq!(*log.lock().unwrap(), [(3, 0)]);
                game.tick_mod_block_entity((3, 64, 1));
                game.fire_pending_block_entity_loads();
                game.tick_mod_block_entity((3, 64, 1));
                assert_eq!(*log.lock().unwrap(), [(3, 0), (3, -1), (3, -1)]);
                game.world.block_entities.remove(&(3, 64, 1));
                game.world.block_entities.insert((3, 64, 1), holder(12));
                // No pass-level drain: tick this new identity immediately.
                game.tick_mod_block_entity((3, 64, 1));
                assert_eq!(
                    *log.lock().unwrap(),
                    [(3, 0), (3, -1), (3, -1), (3, 12), (3, -1)]
                );
                game.world
                    .block_entities
                    .load_chunk(vec![((1, 64, 1), holder(7))]);
                game.fire_pending_block_entity_loads();
                assert_eq!(
                    *log.lock().unwrap(),
                    [(3, 0), (3, -1), (3, -1), (3, 12), (3, -1), (1, 7)]
                );
            });
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn block_entity_on_load_client_instance_reads_received_fields() {
        use crate::game::server_side::native_entity_refs::tests::with_client_game;
        use sunrise_client::level_loading::Reason;
        use sunrise_protocol::packets::play::SpawnInfo;
        let log = Log::default();
        mod_platform::testing::with_mods(
            vec![ModEntry::new("test", Box::new(User(log.clone(), &[])))].into(),
            || {
                with_client_game(|game| {
                    mod_api::__private::resolve_block_entity_types(["test:holder"], |_| Some(()))
                        .unwrap();
                    game.native_lifecycle =
                        sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycle::default();
                    let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                    game.connect_local(client, "OnLoadTest").unwrap();
                    let spawn = SpawnInfo {
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
                    };
                    game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
                    game.client.remote.as_mut().unwrap().logged_in = true;
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    game.world.block_entities.insert((1, 64, 1), holder(0));
                    let Some(BlockEntity::Mod(entity)) =
                        game.world.block_entities.get_mut(&(1, 64, 1))
                    else {
                        panic!()
                    };
                    entity.merge_update_tag(&sunrise_core::nbt::Compound::from([(
                        "count".into(),
                        sunrise_core::nbt::Tag::Int(7),
                    )]));
                    game.tick_remote_level_with(|game| {
                        game.fire_pending_block_entity_loads();
                        Ok(())
                    })
                    .unwrap();
                    assert_eq!(*log.lock().unwrap(), [(1, 7)]);
                    game.fire_pending_block_entity_loads();
                    assert_eq!(*log.lock().unwrap(), [(1, 7)]);
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn block_entity_on_load_without_listener_preserves_data_and_never_queues() {
        mod_platform::testing::with_mods(vec![].into(), || {
            with_game(|game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.world
                    .block_entities
                    .load_chunk(vec![((1, 64, 1), holder(7)), ((2, 64, 1), holder(9))]);
                assert_eq!(game.world.block_entities.pending_on_load_len(), 0);
                game.fire_pending_block_entity_loads();
                assert_eq!(game.world.block_entities.next_on_load(), None);
                let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&(1, 64, 1))
                else {
                    panic!()
                };
                assert_eq!(
                    entity.nbt.get(&"count".into()),
                    Some(&sunrise_core::nbt::Tag::Int(7))
                );
            })
        });
    }
}
