use sunrise_core::geometry::Vec3;

#[test]
#[ignore = "daily"]
fn warning_dispatch_uses_neutral_at_tick_four() {
    use sunrise_sim::motion::PlayerMotion;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };
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
        "sunrise-ominous-sound-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&scratch).unwrap();
    game.server.world_root = scratch.clone();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 100.0, 4.5)).unwrap());

    const WARNING: &str = "minecraft:block.trial_spawner.about_to_spawn_item";
    let center = [6.5, 102.5, 4.5];
    game.start_audio_capture();
    game.server_play_sound("minecraft:block.stone.break", center, 1.0, 1.0);
    game.server_play_sound_on(6, "minecraft:block.note_block.pling", center, 1.0, 1.0);
    assert_eq!(
        game.server
            .pending_sounds
            .iter()
            .map(|s| s.source)
            .collect::<Vec<_>>(),
        [4, 6]
    );
    game.server.pending_sounds.clear();
    let (tag, _) = sunrise_core::nbt::parse_snbt(r#"{id:"minecraft:ominous_item_spawner",UUID:[I;128,0,0,1],Pos:[6.25d,102.75d,4.875d],item:{id:"minecraft:diamond",count:1},spawn_item_after_ticks:40L}"#).unwrap();
    game.load_ominous_item_spawner(tag);
    assert_eq!(game.ominous_spawners.len(), 1);
    let s = &game.ominous_spawners[0];
    assert_eq!(s.uuid, [128, 0, 0, 1]);
    assert_eq!(s.position, [6.25, 102.75, 4.875]);
    assert_eq!(&*s.item.item, "minecraft:diamond");
    assert_eq!(s.item.count, 1);
    assert_eq!(s.spawn_item_after, 40);
    assert_eq!(s.tick_count, 0);
    for tick in 1..=5 {
        game.tick_ominous_item_spawners();
        assert_eq!(game.ominous_spawners[0].tick_count, tick);
        let warning: Vec<_> = game
            .server
            .pending_sounds
            .iter()
            .filter(|s| s.event == WARNING)
            .collect();
        assert_eq!(warning.len(), if tick >= 4 { 1 } else { 0 });
        if tick == 4 {
            let sound = warning[0];
            assert_eq!(sound.position, center);
            assert_eq!(sound.volume.to_bits(), 1.0f32.to_bits());
            assert_eq!(sound.pitch.to_bits(), 1.0f32.to_bits());
            assert_eq!(
                sound.source, 6,
                "OminousItemSpawner.tickServer explicitly selects NEUTRAL"
            );
        }
    }
    game.server_play_sound("minecraft:block.stone.break", center, 1.0, 1.0);
    assert_eq!(
        game.server.pending_sounds.last().unwrap().source,
        4,
        "the explicit warning source must not change later event-family inference"
    );
    game.stop_audio_capture();
    drop(game);
    std::fs::remove_dir_all(scratch).unwrap();
}

#[test]
#[ignore = "daily"]
fn residency_spawner_load_expiry_and_spawn_order() {
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        game.ominous_spawners.clear();
        game.dropped.clear();
        for (item, delay) in [("diamond", 1), ("emerald", 3), ("gold_ingot", 1)] {
            let (tag, _) = sunrise_core::nbt::parse_snbt(&format!(
                "{{Pos:[0d,80d,0d],item:{{id:\"minecraft:{item}\",count:1}},spawn_item_after_ticks:{delay}L}}"
            )).unwrap();
            game.load_ominous_item_spawner(tag);
        }
        let survivor = game.ominous_spawners[1].id;
        game.tick_ominous_item_spawners();
        assert_eq!(game.ominous_spawners.len(), 1);
        assert_eq!(game.ominous_spawners[0].id, survivor);
        assert_eq!(game.ominous_spawners[0].tick_count, 1);
        assert_eq!(
            game.dropped
                .iter()
                .map(|e| e.stack.item.as_ref())
                .collect::<Vec<_>>(),
            ["minecraft:diamond", "minecraft:gold_ingot"]
        );
        game.tick_ominous_item_spawners();
        game.tick_ominous_item_spawners();
        assert!(game.ominous_spawners.is_empty());
        assert_eq!(
            game.dropped.last().unwrap().stack.item.as_ref(),
            "minecraft:emerald"
        );
    });
}

#[test]
#[ignore = "daily"]
fn taken_spawner_tick_keeps_survivors_before_both_spawns() {
    use sunrise_items::items::ItemStack;
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        game.ominous_spawners.clear();
        for _ in 0..2 {
            game.add_ominous_item_spawner(
                ItemStack::new("minecraft:air", 1),
                [0.0, 80.0, 0.0],
                100,
            );
        }
        let first = game.ominous_spawners[0].id;
        let second = game.ominous_spawners[1].id;
        let next = game.server.next_entity;
        crate::game::server_side::entity_server::taken_pass_tests::with_spawn(
            |game| {
                assert!(game.ominous_spawners.is_empty());
                for delay in [200, 300] {
                    game.add_ominous_item_spawner(
                        ItemStack::new("minecraft:air", 1),
                        [0.0, 80.0, 0.0],
                        delay,
                    );
                }
            },
            || game.tick_ominous_item_spawners(),
        );
        assert_eq!(
            game.ominous_spawners
                .iter()
                .map(|s| (s.id, s.tick_count))
                .collect::<Vec<_>>(),
            [(first, 1), (second, 1), (next + 1, 0), (next + 2, 0)]
        );
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn lifecycle_spawner_commands_unload_park_and_remote_metadata() {
    use crate::game::server_side::entity_server::taken_pass_tests::{
        LifecycleFixture, RemoteLifecycle, run_remote_lifecycle,
    };
    use sunrise_gameplay_common::game::commands_common::Target;
    use sunrise_protocol::{
        connection::Pipe,
        entity_data::{Value, serializer},
        item::Stack,
    };
    LifecycleFixture {
        seed: |game, entries| {
            game.ominous_spawners.clear();
            for &(id, at) in entries {
                game.add_remote_ominous_item_spawner(id, at);
            }
        },
        ids: |game| game.ominous_spawners.iter().map(|s| s.id).collect(),
        target: Target::Spawner,
        unloads_with_chunk: true,
        remote: None,
    }
    .run();
    run_remote_lifecycle(
        |game, entries| {
            game.ominous_spawners.clear();
            for &(id, at) in entries {
                game.add_remote_ominous_item_spawner(id, at);
            }
        },
        |game| game.ominous_spawners.iter().map(|s| s.id).collect(),
        RemoteLifecycle {
            prepare: |game| {
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "Lifecycle").unwrap();
            },
            apply: |game, remove, update| {
                let item = game
                    .client
                    .remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:item")
                    .unwrap()
                    .id("minecraft:emerald")
                    .unwrap();
                game.remote_object_data(
                    update as i32,
                    &[(
                        8,
                        serializer::ITEM_STACK,
                        Value::Item(Some(Stack::new(item, 7))),
                    )],
                );
                let changed = game
                    .ominous_spawners
                    .iter()
                    .find(|s| s.id == update)
                    .unwrap();
                assert_eq!(changed.item.item.as_ref(), "minecraft:emerald");
                assert_eq!(changed.item.count, 7);
                game.remove_remote_object(remove);
            },
        },
    );
}

#[test]
#[ignore = "daily"]
fn residency_command_positions_and_client_spawner_clock() {
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        game.set_allow_commands(true);
        game.add_remote_ominous_item_spawner(701, [1.0, 80.0, 1.0]);
        game.spawn_creeper_cloud([2.0, 80.0, 2.0], Vec::new(), Vec::new(), Vec::new(), 0);
        assert!(game.run_command("tp @e[type=minecraft:ominous_item_spawner] 64.0 80.0 64.0"));
        assert_eq!(game.ominous_spawners[0].position, [64.0, 80.0, 64.0]);
        assert!(game.run_command("tp @e[type=minecraft:area_effect_cloud] -32.0 90.0 -48.0"));
        assert_eq!(game.breath_clouds[0].at, [-32.0, 90.0, -48.0]);
        #[cfg(feature = "client")]
        with_spawner_client(game, |client| {
            client.tick_remote_ominous_item_spawners();
            assert_eq!(client.ominous_spawners[0].tick_count, 1);
        });
    });
}

#[cfg(feature = "client")]
fn with_spawner_client(
    server: &mut sunrise_gameplay::game::ServerGame<'_>,
    test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
) {
    crate::game::server_side::native_entity_refs::tests::with_client_game(|client| {
        client.world.game_time = server.world.game_time;
        client.ominous_spawners.append(&mut server.ominous_spawners);
        test(client);
    });
}
