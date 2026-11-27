#![cfg(test)]
#[cfg(test)]
mod owner_attachment_save_tests {
    use sunrise_core::nbt::Tag;
    use sunrise_mod_runtime::attachments::{
        AttachmentCodec, AttachmentError, AttachmentType, Attachments,
    };

    static PROGRESS: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "progress",
        default: || 0,
        codec: Some(AttachmentCodec {
            encode: |n: &i32| Ok([("n".into(), Tag::Int(*n))].into()),
            decode: |c| match c.get(&"n".into()) {
                Some(Tag::Int(n)) => Ok(*n),
                _ => Err(AttachmentError::Decode),
            },
        }),
        copy_on_death: true,
        sync_codec: None,
    };

    /// The live value as the named owner file would hold it.
    fn saved_value(game: &sunrise_gameplay::game::ServerGame<'_>) -> Option<i32> {
        let Some(Tag::Compound(map)) = game.level.player_extra.as_ref() else {
            return None;
        };
        let data = sunrise_mod_runtime::attachments::attachment_compound(
            map.get(&sunrise_mod_runtime::attachments::ATTACHMENTS_KEY.into()),
        )
        .ok()
        .flatten()?;
        PROGRESS
            .get_existing(
                &mut Attachments::from_nbt(data.clone()),
                sunrise_mod_runtime::attachments::Holder::Level,
            )
            .ok()
            .flatten()
    }

    /// Set `value` in the map of the player `conn`, as a mod's write does.
    fn set_value(game: &mut sunrise_gameplay::game::ServerGame<'_>, conn: u32, value: i32) {
        let host = game.server.lan.as_mut().unwrap();
        PROGRESS
            .set(
                host.hub.guests.attachments_mut(conn).unwrap(),
                sunrise_mod_runtime::attachments::Holder::Level,
                value,
            )
            .unwrap();
    }

    /// An integrated host whose owner is its placed player (conn 1, uuid 1).
    fn owned_host(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
        let _rx = sunrise_gameplay::game::server_side::lan::world_spawn_sync_tests::host_for(game);
        game.server.lan.as_mut().unwrap().owner = Some(sunrise_gameplay::game::lan::Owner {
            uuid: 1,
            saved: None,
        });
    }

    /// Each test loads resources, which replaces process-wide tables: it runs
    /// in a fresh process of its own.
    const HERE: &str = "cut_layout::game::server_side::lan::owner_attachment_connected_statistics_save_tests::owner_attachment_save_tests";

    #[test]
    #[ignore = "daily"]
    fn an_autosave_writes_the_owners_live_attachments_to_the_file_it_loads() {
        sunrise_gameplay_common::game::in_own_process(
            &format!("{HERE}::an_autosave_writes_the_owners_live_attachments_to_the_file_it_loads"),
            || {
                let mut resources = None;
                let mut game =
                    sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
                owned_host(&mut game);
                // The integrated server has no player of its own: outside a turn, where
                // the autosave and the pause save run, the owner's map is their record's.
                set_value(&mut game, 1, 7);
                assert!(game.store_owner().is_some());
                assert_eq!(
                    saved_value(&game),
                    Some(7),
                    "the autosave stores the live value"
                );
                // A later change is what the next autosave writes.
                set_value(&mut game, 1, 9);
                game.store_owner();
                assert_eq!(saved_value(&game), Some(9));
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_save_in_another_players_turn_writes_the_owners_attachments() {
        sunrise_gameplay_common::game::in_own_process(
            &format!("{HERE}::a_save_in_another_players_turn_writes_the_owners_attachments"),
            || {
                let mut resources = None;
                let mut game =
                    sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
                owned_host(&mut game);
                let other =
                    sunrise_gameplay_common::game::native_player_death::tests::new_guest(2, 2);
                game.server.lan.as_mut().unwrap().hub.guests.push(other);
                set_value(&mut game, 1, 7);
                set_value(&mut game, 2, 5);
                // For their turn, the other player's map is the game's.
                let saved = game
                    .as_guest(2, |game| {
                        assert!(game.store_owner().is_some());
                        saved_value(game)
                    })
                    .unwrap();
                assert_eq!(saved, Some(7), "the owner's value, not the acting player's");
            },
        );
    }
}

#[cfg(test)]
mod connected_statistics_save_tests {
    use std::fs;

    fn connected_stats_save_control(stale: bool) {
        use std::sync::{Arc, Mutex};
        use {
            sunrise_gameplay::game::server_side::stats,
            sunrise_gameplay_common::game::execution_role::ExecutionRole,
        };
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        game.options.execution_role = ExecutionRole::DedicatedServer;
        // The command's dedicated-only guard also checks the launch mode.
        game.options.dedicated = Some(Vec::new());
        let scratch = tempfile::tempdir().unwrap();
        let dir = scratch.path().join(if stale {
            "stats-stale"
        } else {
            "stats-missing"
        });
        fs::create_dir_all(&dir).unwrap();
        game.server.world_root = dir.clone();
        game.server.save.0 = Arc::new(sunrise_sim::save::SaveDir::new(
            dir.clone(),
            sunrise_sim::save::LevelShape::OVERWORLD,
            Some(Default::default()),
        ));
        use sunrise_server_files::user_lists::{BanList, IpBanList, OpList, WhiteList};
        game.server.server_lists.0 = Some(Arc::new(Mutex::new(
            sunrise_gameplay::game::server_side::dedicated::Lists {
                ops: OpList::new(dir.join("ops.json")),
                white: WhiteList::new(dir.join("whitelist.json")),
                bans: BanList::new(dir.join("banned-players.json")),
                ip_bans: IpBanList::new(dir.join("banned-ips.json")),
                use_whitelist: false,
                enforce_whitelist: false,
                op_level: 4,
                player_idle_timeout: 0,
                stop: false,
                autosave: true,
            },
        )));
        game.set_allow_commands(true);
        let _rx =
            sunrise_gameplay::game::server_side::lan::world_spawn_sync_tests::host_for(&mut game);
        let host = game.server.lan.as_mut().unwrap();
        host.owner = None;
        host.hub
            .guests
            .update(0, |guest| guest.dimension = "minecraft:overworld".into());
        assert!(host.hub.guests[0].placed);
        assert!(!host.hub.guests[0].released);
        assert_eq!(host.hub.guests[0].uuid, 1);
        let path = stats::player_json(&dir, "stats", [0, 0, 0, 1]);
        assert!(!path.exists());
        let play = host.play.entry(1).or_default();
        play.stats.loaded = true;
        if stale {
            play.stats.add("used", "minecraft:diamond_sword", 4);
            play.stats.write(&path);
        }
        play.stats.add("used", "minecraft:diamond_sword", 1);
        let expected = if stale { 5 } else { 1 };
        // Packet dirty state is not the file's authority: a stats response can
        // consume it before the connected player's next server save.
        assert!(!play.stats.take_dirty().is_empty());
        assert_eq!(play.stats.get("used", "minecraft:diamond_sword"), expected);
        assert!(play.stats.take_dirty().is_empty());
        for command in ["save-all", "save-all flush"] {
            game.server.chat_capture = Some(Vec::new());
            // Match dedicated registration without changing the process-wide
            // tree used by unrelated integrated-server tests.
            let dispatcher = sunrise_command::vanilla::build_for(false, true);
            game.ui.registries.permission = 4;
            let parsed = dispatcher.parse(command, &game.ui.registries);
            let (ctx, _) = parsed.executable().expect("registered dedicated command");
            let (root, ctx) = (parsed.context.clone(), ctx.clone());
            assert_eq!(game.run_chain(&root, &ctx).unwrap(), 1);
            let messages = game.server.chat_capture.take().unwrap();
            assert!(
                messages
                    .iter()
                    .any(|m| m.translation_key() == Some("commands.save.success")),
                "actual save completion"
            );
            let host = game.server.lan.as_ref().unwrap();
            assert_eq!(host.hub.guests.len(), 1, "saved while still connected");
            assert_eq!(
                host.play[&1].stats.get("used", "minecraft:diamond_sword"),
                expected
            );
            let raw = fs::read_to_string(&path)
                .expect("save-all must write the connected guest's actual stats file");
            let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
            assert_eq!(
                json["stats"]["minecraft:used"]["minecraft:diamond_sword"], expected,
                "{command}: current live value, not stale disk data"
            );
        }
    }

    #[test]
    #[ignore = "daily"]
    fn save_all_creates_the_connected_guests_statistics_file() {
        connected_stats_save_control(false);
    }

    #[test]
    #[ignore = "daily"]
    fn save_all_replaces_stale_connected_guest_statistics() {
        connected_stats_save_control(true);
    }
}

#[cfg(test)]
mod connected_statistics_save_exclusion_tests {

    /// Source controls for the bounded eligibility gates; these excluded
    /// guests are not counted as successfully saved players. A placed guest of an
    /// integrated server is saved, as on a dedicated one (#5780).
    #[test]
    #[ignore = "daily"]
    fn excluded_guest_records_are_not_invented_or_written() {
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        let _rx =
            sunrise_gameplay::game::server_side::lan::world_spawn_sync_tests::host_for(&mut game);
        let scratch = tempfile::tempdir().unwrap();
        game.server.world_root = scratch.path().join("stats-excluded");
        std::fs::create_dir_all(&game.server.world_root).unwrap();
        let path = sunrise_gameplay::game::server_side::stats::player_json(
            &game.server.world_root,
            "stats",
            [0, 0, 0, 1],
        );
        let mut host = game.server.lan.take().unwrap();
        host.play
            .entry(1)
            .or_default()
            .stats
            .add("used", "minecraft:diamond_sword", 1);
        host.hub.guests.update(0, |guest| guest.placed = false);
        game.save_connected_guest_statistics(&host);
        assert!(!path.exists(), "not placed: no statistics file");
        host.hub.guests.update(0, |guest| guest.placed = true);
        host.hub.guests.update(0, |guest| guest.released = true);
        game.save_connected_guest_statistics(&host);
        assert!(!path.exists(), "released: no statistics file");
        host.hub.guests.update(0, |guest| guest.released = false);
        let play = host.play.remove(&1).unwrap();
        game.save_connected_guest_statistics(&host);
        assert!(!path.exists(), "no play record: no invented statistics");
        host.play.insert(1, play);
        game.server.lan = Some(host);
        // An integrated server saves its connected guests too: `IntegratedPlayerList.save`
        // keeps the owner's data for level.dat and hands every player to `PlayerList.save`,
        // which saves their stats.
        game.options.dedicated = None;
        game.save_game_metadata().unwrap();
        assert!(
            path.exists(),
            "an integrated server writes a placed guest's statistics"
        );
        std::fs::remove_file(&path).unwrap();
        game.options.dedicated = Some(Vec::new());
        game.options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
        game.save_game_metadata().unwrap();
        assert!(!path.exists(), "the existing nonclient gate is preserved");
        game.options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::DedicatedServer;
        let host = game.server.lan.take().unwrap();
        game.save_game_metadata().unwrap();
        assert!(!path.exists(), "no hub: no substituted record");
        assert_eq!(
            host.play[&1].stats.get("used", "minecraft:diamond_sword"),
            1
        );
    }
}
