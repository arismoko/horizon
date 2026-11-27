#[cfg(test)]
mod language_tests;

#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay::game::server_side::dedicated::capture_log;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;

    use std::fs;

    #[test]
    #[ignore = "daily"]
    fn dedicated_checkpoints_continue_only_after_record_refusals() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::dedicated::tests::dedicated_checkpoints_continue_only_after_record_refusals",
            || {
                use mod_api::{Authority, Hooks, ModEntry, ServerHost, ServerStoppedEvent};
                use std::sync::{Arc, Mutex};
                use sunrise_core::nbt::{DISK_WRITE_NESTING_GUARD, Tag};
                use sunrise_sim::save::{LevelShape, SaveDir};
                use {
                    sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
                    sunrise_gameplay::game::native_entity_refs::tests::with_game_role,
                    sunrise_gameplay_common::game::execution_role::ExecutionRole,
                };

                struct CloseEvents(Arc<Mutex<usize>>);
                #[mod_api::__subscriptions]
                impl Hooks for CloseEvents {
                    fn server_stopped(&self, _: &mut dyn ServerHost, _: &mut ServerStoppedEvent) {
                        *self.0.lock().unwrap() += 1;
                    }
                }

                let events = Arc::new(Mutex::new(0usize));
                let mods = vec![ModEntry::new(
                    "crash_close_events",
                    Box::new(CloseEvents(events.clone())),
                )]
                .into();
                mod_platform::testing::with_mods(mods, || {
                    with_game_role(fixture_root(), ExecutionRole::DedicatedServer, |game| {
                        game.native_lifecycle
                            .begin_open(Some(Authority::DedicatedServer));
                        game.native_lifecycle.world_open_ready().unwrap();
                        let dir = tempfile::tempdir().unwrap();
                        game.server.world_root = dir.path().to_path_buf();
                        game.server.save.0 = Arc::new(SaveDir::new(
                            game.server.world_root.clone(),
                            LevelShape::OVERWORLD,
                            Some(Default::default()),
                        ));
                        game.server.session_lock.0 =
                            Some(sunrise_sim::save::SessionLock::acquire(dir.path()).unwrap());
                        game.server.world_adopted = true;
                        assert_eq!(
                            game.save_dedicated_checkpoint("startup fixture").unwrap(),
                            0
                        );
                        let fail_next_save = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                            let key = (2, 0);
                            let mut chunk = sunrise_level::chunk::Chunk::with_layout(
                                key.0,
                                key.1,
                                game.world.layout(),
                            );
                            let invalid = (0..=DISK_WRITE_NESTING_GUARD)
                                .fold(Tag::Int(0), |nested, _| Tag::List(vec![nested]));
                            Arc::make_mut(chunk.saved.get_or_insert_with(Default::default))
                                .root
                                .insert("test:invalid".into(), invalid);
                            game.world.chunks.insert(key, Arc::new(chunk));
                            game.world.modified.insert(key);
                        };
                        for phase in ["startup fixture", "pause fixture"] {
                            fail_next_save(game);
                            assert_eq!(game.save_dedicated_checkpoint(phase).unwrap(), 1);
                        }
                        assert_eq!(game.server.save.0.record_save_failures(), 2);
                        assert_eq!(game.save_dedicated_checkpoint("no new edits").unwrap(), 0);

                        fail_next_save(game);
                        let cause = "injected dedicated crash";
                        let error = game
                            .run_server_lifecycle(true, |_| panic!("{cause}"))
                            .unwrap_err();
                        assert!(error.to_string().contains(cause));
                        assert_eq!(*events.lock().unwrap(), 1);
                        assert_eq!(game.server.save.0.record_save_failures(), 3);

                        let blocked = dir.path().join("level.dat_new");
                        std::fs::create_dir(&blocked).unwrap();
                        let expected = std::fs::write(blocked, b"test").unwrap_err();
                        let error = game.save_dedicated_checkpoint("pause fixture").unwrap_err();
                        assert_eq!(
                            error.downcast_ref::<std::io::Error>().unwrap().kind(),
                            expected.kind()
                        );
                        game.native_lifecycle
                            .begin_open(Some(Authority::DedicatedServer));
                        game.native_lifecycle.world_open_ready().unwrap();
                        game.server.session_lock.0 =
                            Some(sunrise_sim::save::SessionLock::acquire(dir.path()).unwrap());
                        game.server.world_adopted = true;
                        let error = game
                            .run_server_lifecycle(true, |_| panic!("{cause}"))
                            .unwrap_err();
                        assert!(error.to_string().contains(cause));
                        assert_eq!(*events.lock().unwrap(), 2);
                    });
                });
            },
        );
    }
    #[test]
    #[ignore = "daily"]
    fn broadcast_console_destination_follows_role_not_launch_marker() {
        use sunrise_gameplay_common::game::execution_role::ExecutionRole;
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        game.options.dedicated = Some(Vec::new());
        for (role, expected) in [
            (
                ExecutionRole::DedicatedServer,
                vec!["destination probe".to_owned()],
            ),
            (ExecutionRole::IntegratedServer, Vec::new()),
            (ExecutionRole::Client, Vec::new()),
        ] {
            game.options.execution_role = role;
            let lines = capture_log(|| {
                game.native_broadcast(sunrise_game_data::text::Text::literal("destination probe"))
            });
            assert_eq!(lines, expected, "{role:?}");
        }
    }
    #[test]
    #[ignore = "daily"]
    fn console_name_lookup_skips_fake_players() {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            game.fake_player(level, 100, "robot").unwrap();
            assert!(game.online_player("robot").is_none());
            assert_eq!(game.online_player("GUEST12").unwrap().id, 12);
            // A fake with the same name must not hide a real player later in join order.
            let guests = &mut game.server.lan.as_mut().unwrap().hub.guests;
            let index = guests.iter().position(|g| g.uuid == 12).unwrap();
            guests.update(index, |g| g.name = "robot".into());
            guests.reverse();
            assert_eq!(game.online_player("ROBOT").unwrap().id, 12);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn console_and_management_ip_ban_sets_skip_fake_players() {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            game.fake_player(level, 100, "robot").unwrap();
            assert!(game.ip_ban_players("0.0.0.0").is_empty());
            assert_eq!(game.ip_ban_players("127.0.0.1"), [12, 13]);
            // Even with a matching real peer, the fake is not counted as affected.
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| guest.set_address("0.0.0.0:1".parse().unwrap()))
                .unwrap();
            assert_eq!(game.ip_ban_players("0.0.0.0"), [12]);
            assert_eq!(game.ip_ban_players("127.0.0.1"), [13]);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_missing_level_file_keeps_the_existing_world_generator() {
        use sunrise_java_properties::Properties;
        use sunrise_sim::save::Level;

        let world = tempfile::tempdir().unwrap();
        let dir = world.path();
        Level {
            seed: 12345,
            ..Level::default()
        }
        .write_without_player(dir)
        .unwrap();
        let settings_path = dir.join("data/minecraft/world_gen_settings.dat");
        assert!(settings_path.is_file(), "test world has no saved generator");
        fs::rename(dir.join("level.dat"), dir.join("level.dat_old")).unwrap();
        let mut properties = Properties::default();
        properties.put("level-seed", "54321".into());
        let props = sunrise_server_files::properties::ServerProperties::new(
            properties,
            &mut sunrise_server_files::properties::SystemEntropy::default(),
        )
        .unwrap();
        let resources = std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap().into();
        sunrise_gameplay::game::native_entity_refs::tests::with_game_role(
            resources,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::DedicatedServer,
            |game| {
                game.prepare_world_metadata(dir, &props, false).unwrap();
            },
        );
        let generation = sunrise_sim::save::read_dat(&settings_path).unwrap();
        assert_eq!(
            generation.get("data").and_then(|data| data.get("seed")),
            Some(&sunrise_core::nbt::Tag::Long(12345)),
        );
    }
}

#[cfg(test)]
mod console_source_tests {
    use std::sync::{Arc, mpsc};
    use sunrise_core::geometry::Vec3;
    use sunrise_game_data::text::Text;
    use sunrise_gameplay::game::native_guest_turn_tests::with_guests;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_protocol::{buf::Reader, packets::play::Clientbound};
    use sunrise_sim::lan::{Out, PlayerSender, data::component_nbt};
    use sunrise_sim::motion::PlayerMotion;

    /// Runs `command` as the console does and returns whether it worked and every message it printed.
    fn console(game: &mut sunrise_gameplay::game::ServerGame<'_>, command: &str) -> (bool, String) {
        game.server.chat_capture = Some(Vec::new());
        let ok = game.as_console_in_spawn_level(|g| g.run_command(command));
        let said = game.server.chat_capture.take().unwrap_or_default();
        let said = said
            .iter()
            .map(|t| t.component().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        (ok, said)
    }

    /// `MinecraftServer.createCommandSourceStack` and `RconConsoleSource.createCommandSourceStack` use named sources without entities.
    #[test]
    #[ignore = "daily"]
    fn the_console_runs_as_the_server_not_the_player_in_place() {
        with_guests(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
            game.build_registries();
            let (tx, rx) = mpsc::channel();
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.update_guest(12, |guest| {
                guest.sender = PlayerSender::new(tx, Arc::default())
            })
            .unwrap();
            let emote = hub
                .registry_id("minecraft:chat_type", "minecraft:emote_command")
                .unwrap();

            for (rcon, name) in [(false, "Server"), (true, "Rcon")] {
                game.server.rcon_source = rcon;
                assert!(game.as_console_in_spawn_level(|g| g.run_command("me waves")));
                let message = rx
                    .try_iter()
                    .flat_map(|out| match out {
                        Out::Packet(body) => vec![body],
                        Out::Packets(bodies) => bodies,
                        Out::Disconnect(_) => Vec::new(),
                    })
                    .find_map(|body| {
                        let mut reader = Reader::new(&body);
                        match Clientbound::decode(reader.varint().unwrap(), reader.rest()).unwrap()
                        {
                            Clientbound::DisguisedChat(message) => Some(message),
                            _ => None,
                        }
                    })
                    .expect("the guest receives the console's emote");
                assert_eq!(message.chat_type.chat_type, emote);
                assert_eq!(
                    message.chat_type.name,
                    component_nbt(&Text::literal(name).component())
                );
                assert_eq!(
                    message.message,
                    component_nbt(&Text::literal("waves").component())
                );
            }

            game.server.rcon_source = false;
            let (ok, said) = console(game, "teammsg hi");
            assert!(
                !ok && said.contains("permissions.requires.entity"),
                "{said}"
            );
            assert!(game.server.command_source.is_none());
        });
    }
}

pub(crate) mod report;

mod missing_mods;

mod management;
