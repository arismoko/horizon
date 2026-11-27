#![cfg(test)]
#[cfg(test)]
pub(crate) mod allay_owner_damage_tests {
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game_at;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_sim::mobs::{Damage, Mob};

    #[test]
    #[ignore = "daily"]
    fn allay_owner_adapters_compare_the_actual_player_uuid() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            eprintln!("Prepared resources unavailable; Allay Game damage coverage did not run");
            return;
        }
        with_game_at(root, |game| {
            game.build_registries();
            let actual = [101, 102, 103, 104];
            let other = [10, 20, 30, 40];
            let mut player =
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            player.entity_id = 7;
            game.player = Some(player);
            game.level.player_uuid = Some(actual);
            assert_eq!(game.attack_of(7).player_uuid, Some(actual));
            game.server.lan_acting = Some(12);
            game.server.lan_acting_uuid = Some(sunrise_sim::lan::playerdata::ints_uuid(other));
            assert_eq!(
                game.attack_of(7).player_uuid,
                Some(other),
                "the acting guest is the causing player"
            );
            game.server.lan_acting_uuid = None;
            game.level.player_uuid = None;
            assert_eq!(
                game.attack_of(7).player_uuid,
                None,
                "no invented UUID for an unknown source"
            );
            game.level.player_uuid = Some(actual);
            game.server.lan_acting = None;
            let kind = game.res.mob_types().get("minecraft:allay").unwrap().clone();
            let before = game.world.game_time;
            for melee in [true, false] {
                for liked in [Some(actual), None, Some(other)] {
                    game.mobs.clear();
                    let mut mob = Mob::new(1, kind.clone(), Vec3::new(22.5, 101.0, 22.5), 0.0);
                    mob.no_ai = true;
                    mob.extra.tame = liked.is_some();
                    mob.extra.owner = liked;
                    assert_eq!((mob.health, mob.hurt_time), (20.0, 0));
                    game.mobs.push(mob);
                    let source = Damage::player([20.5, 102.0, 20.5]);
                    let accepted = if melee {
                        game.hurt_by_player(0, 2.0, source).0
                    } else {
                        // The command entry point lends its resolved attacker to the damage path.
                        let attack = game.attack_of_who(source.attacker);
                        game.with_attack(attack, |g| g.command_damage_mob(1, 2.0, source))
                    };
                    let expected = liked != Some(actual);
                    assert_eq!(accepted, expected, "melee={melee}, liked={liked:?}");
                    assert_eq!(game.mobs[0].health, if expected { 18.0 } else { 20.0 });
                    assert_eq!(game.mobs[0].extra.owner, liked);
                    assert_eq!(game.world.game_time, before);
                }
            }
        });
    }
}

#[cfg(test)]
#[path = "registered_damage_tests_server.rs"]
pub(crate) mod registered_damage_tests;

#[cfg(test)]
pub(crate) mod datapack_damage_tests {

    use sunrise_gameplay::game::server_side::commands::world_cmds::datapack_damage_tests::with_pack;

    use sunrise_gameplay::game::server_side::commands_server::world_cmds::datapack_damage_tests::pig;

    use sunrise_gameplay_common::game::commands_common::Target;

    use sunrise_gameplay::game::native_entity_refs::tests::with_game;

    use sunrise_core::geometry::Vec3;

    pub(crate) fn run_a_datapack_explosion_type_removes_a_crystal_quietly_and_hurts_the_dragon() {
        with_game(|game| {
            with_pack(game);
            let crystal = |game: &mut sunrise_gameplay::game::ServerGame<'_>, id: u32| {
                game.server.next_entity = game.server.next_entity.max(id);
                game.crystals
                    .push(sunrise_gameplay_common::game::dragon_common::Crystal::new(
                        id,
                        Vec3::new(0.5, 64.0, 0.5),
                        false,
                        0,
                    ));
            };
            pig(game, 1, Vec3::new(2.5, 64.0, 0.5));
            crystal(game, 50);
            assert!(game.hurt_target(Target::Crystal(50), 1.0, "dmgtest:boom", None, None, None));
            assert!(game.crystals.is_empty());
            assert_eq!(game.mobs[0].health, 10.0, "no blast");
            crystal(game, 51);
            assert!(game.hurt_target(Target::Crystal(51), 1.0, "dmgtest:zap", None, None, None));
            assert!(game.crystals.is_empty());
            assert!(
                game.mobs.first().is_none_or(|pig| pig.health < 10.0),
                "the crystal's blast"
            );

            let dragon = game
                .spawn_dragon_spawned(
                    [0.5, 80.0, 0.5],
                    sunrise_gameplay_common::game::creatures::SpawnSite::Plain,
                )
                .unwrap();
            let before = game.dragon.as_ref().unwrap().health;
            assert!(game.hurt_target(
                Target::Dragon(dragon),
                4.0,
                "dmgtest:boom",
                None,
                None,
                None
            ));
            assert!(game.dragon.as_ref().unwrap().health < before);
        });
    }
}

#[cfg(test)]
pub(crate) mod vehicle_damage_command_tests {
    use crate::game::server_side::commands::world_cmds::vehicle_damage_command_tests::attribute_chat_count;
    use crate::game::server_side::commands::world_cmds::vehicle_damage_command_tests::with_game;
    use std::sync::Arc;
    use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2;

    pub(crate) fn attribute_guests(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
    ) -> Vec<std::sync::mpsc::Receiver<sunrise_sim::lan::Out>> {
        use std::sync::{atomic::AtomicI32, mpsc};
        use sunrise_protocol::packets::configuration::ClientInformation;
        use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
        game.server.world_root = game.options.game_dir.as_ref().unwrap().join("h082-world");
        std::fs::create_dir_all(&game.server.world_root).unwrap();
        game.lan_host(
            sunrise_gameplay::game::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "attribute-test".into(), 2)
            },
            HubConfig {
                default_game_mode: 1,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 3,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();
        let mut receivers = Vec::new();
        for (conn, name, position) in [
            (2, "Bob", [7.5, 100.0, 4.5]),
            (3, "Carol", [10.5, 100.0, 4.5]),
        ] {
            let (tx, rx) = mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn,
                    uuid: sunrise_protocol::crypto::offline_uuid(name),
                    name: name.into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(2),
                    brand: "test".into(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:25565".parse().unwrap(),
                    expected: false,
                },
                conn as i32,
                position,
                2,
            );
            guest.dimension = "minecraft:overworld".into();
            guest.placed = true;
            guest.loaded = true;
            guest.game_mode = 1;
            game.server.lan.as_mut().unwrap().hub.guests.push(guest);
            receivers.push(rx);
        }
        let host = game.server.lan.as_mut().unwrap();
        for name in ["Bob", "Carol"] {
            host.hub
                .ops
                .insert(sunrise_protocol::crypto::offline_uuid(name), 2);
        }
        host.hub.refresh_permissions();
        assert!(host.hub.guests.iter().all(|guest| guest.permission == 2));
        receivers
    }

    #[test]
    #[ignore = "daily"]
    fn direct_guest_attributes_adopt_the_target_and_restore_the_issuer() {
        use sunrise_core::geometry::Vec3;
        use sunrise_gameplay_common::game::commands_common::Target;
        use sunrise_sim::motion::PlayerMotion;
        with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 100.0, 4.5)).unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 1, 99);
            game.creative = true;
            assert!(game.run_command("fill 0 99 0 15 99 15 stone"));
            assert!(game.run_command("attribute @s minecraft:armor base set 2"));
            let host_uuid = game.acting_uuid();
            let clock = game.world.game_time;
            let dimension = game.dim;
            let receivers = attribute_guests(game);
            for (conn, name) in [(2, "Bob"), (3, "Carol")] {
                let g = game.command_guest(conn).unwrap();
                assert_eq!(g.uuid, sunrise_protocol::crypto::offline_uuid(name));
                assert_eq!(g.health, 20.0);
                assert!(g.loaded && g.placed);
                assert_eq!(g.game_mode, 1);
                assert_eq!(
                    game.for_player(Target::Guest(conn), |g| g
                        .player
                        .as_mut()
                        .unwrap()
                        .attribute("minecraft:armor")
                        .unwrap()),
                    Some(0.0)
                );
            }
            // Existing guest-issued Player path is positive before direct Guest dispatch.
            assert_eq!(
                game.for_player(Target::Guest(2), |g| g
                    .run_command("attribute @s minecraft:armor base set 3")),
                Some(true)
            );
            assert_eq!(
                game.for_player(Target::Guest(2), |g| g
                    .player
                    .as_mut()
                    .unwrap()
                    .attribute("minecraft:armor")
                    .unwrap()),
                Some(3.0)
            );
            for rx in &receivers {
                attribute_chat_count(rx);
            }
            game.server.chat_capture = Some(Vec::new());
            let accepted = game.run_command("attribute Bob minecraft:armor base set 4");
            let issuer = game.server.chat_capture.take().unwrap();
            // This assertion fails old on the actual guest model, not feedback alone.
            assert_eq!(
                game.for_player(Target::Guest(2), |g| g
                    .player
                    .as_mut()
                    .unwrap()
                    .attribute("minecraft:armor")
                    .unwrap()),
                Some(4.0)
            );
            assert!(accepted);
            assert_eq!(issuer.len(), 1);
            assert!(issuer[0].plain().contains("Bob"));
            assert_eq!(
                attribute_chat_count(&receivers[0]),
                0,
                "success belongs to issuer, not Bob"
            );
            assert_eq!(
                game.player
                    .as_mut()
                    .unwrap()
                    .attribute("minecraft:armor")
                    .unwrap(),
                2.0
            );
            assert_eq!(
                game.for_player(Target::Guest(3), |g| g
                    .player
                    .as_mut()
                    .unwrap()
                    .attribute("minecraft:armor")
                    .unwrap()),
                Some(0.0)
            );
            assert_eq!(
                game.for_player(Target::Guest(2), |g| g
                    .player
                    .as_mut()
                    .unwrap()
                    .attribute("minecraft:max_health")
                    .unwrap()),
                Some(20.0)
            );
            game.server.chat_capture = Some(Vec::new());
            assert!(
                !game.run_command("attribute Bob minecraft:armor modifier remove h082:missing")
            );
            let errors = game.server.chat_capture.take().unwrap();
            assert_eq!(errors.len(), 1);
            assert!(errors[0].plain().contains("h082:missing"));
            assert_eq!(
                attribute_chat_count(&receivers[0]),
                0,
                "error returns to issuer"
            );
            game.server.chat_capture = Some(Vec::new());
            assert!(game.run_command("attribute Bob minecraft:armor base reset"));
            assert_eq!(game.server.chat_capture.take().unwrap().len(), 1);
            assert_eq!(
                game.for_player(Target::Guest(2), |g| g
                    .player
                    .as_mut()
                    .unwrap()
                    .attribute("minecraft:armor")
                    .unwrap()),
                Some(0.0)
            );
            assert_eq!(game.acting_uuid(), host_uuid);
            assert_eq!(game.server.lan_acting, None);
            assert_eq!(game.dim, dimension);
            assert_eq!(game.world.game_time, clock);
            assert_eq!(game.player.as_ref().unwrap().health.health(), 20.0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn existing_attribute_operations_keep_their_exact_success_messages() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::motion::PlayerMotion;
        with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 200.0, 4.5)).unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 1, 99);
            assert!(
                game.run_command("summon pig 6.5 200 4.5 {UUID:[I;82,0,0,1],NoAI:1b,NoGravity:1b}")
            );
            assert!(
                game.run_command("summon armor_stand 8.5 200 4.5 {UUID:[I;82,0,0,2],NoGravity:1b}")
            );
            for target in [
                "@s",
                "00000052-0000-0000-0000-000000000001",
                "00000052-0000-0000-0000-000000000002",
            ] {
                for operation in [
                    "get",
                    "base get",
                    "base set 2",
                    "base get",
                    "base reset",
                    "modifier add h082:reply 1 add_value",
                    "modifier value get h082:reply",
                    "get",
                    "modifier remove h082:reply",
                ] {
                    let command = format!("attribute {target} minecraft:armor {operation}");
                    game.server.chat_capture = Some(Vec::new());
                    game.server.cmds.0.results = Some(Vec::new());
                    assert!(game.run_command(&command), "{command}");
                    let messages = game.server.chat_capture.take().unwrap();
                    assert_eq!(messages.len(), 1, "{command}");
                    let components: Vec<_> = messages.iter().map(|m| m.component()).collect();
                    println!(
                        "H082_EXISTING {}",
                        serde_json::json!({"command":command,"messages":components,"results":game.server.cmds.0.results.take().unwrap()})
                    );
                }
            }
        });
    }
}

#[cfg(test)]
#[path = "effect_attribute_tests_server.rs"]
pub(crate) mod effect_attribute_tests;
