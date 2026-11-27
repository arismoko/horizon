#[cfg(test)]
pub(in crate::game) mod swing_echo_tests {
    use std::sync::{Arc, atomic::AtomicI32, mpsc};
    use sunrise_core::position::ChunkPos;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_items::items::ItemStack;
    use sunrise_level::chunk::Chunk;
    use sunrise_protocol::packets::{
        configuration::ClientInformation,
        play::{self, Clientbound, Serverbound},
    };
    use sunrise_sim::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig};
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    /// handleUseItemOn swings the player's arm, but sends the swing to the
    /// player's own client only when the client did not predict it: a block
    /// placed (SUCCESS) is the client's own swing, a bed's use
    /// (SUCCESS_SERVER) is not (#9887).
    #[test]
    #[ignore = "daily"]
    fn only_a_server_only_use_swings_back_to_the_player() {
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
        options.game_dir = Some(
            std::env::temp_dir().join(format!("sunrise-swing-echo-unit-{}", std::process::id())),
        );
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
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap()
            .0;
        let mut chunk = Chunk::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                chunk.set(x, 199, z, stone);
            }
        }
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("setblock 6 200 4 red_bed[facing=north,part=foot]"));
        assert!(game.run_command("setblock 6 200 3 red_bed[facing=north,part=head]"));
        game.lan_host(
            sunrise_gameplay::game::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "swing-echo-test".into(), 2)
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
                uuid: 331,
                name: "SwingGuest".into(),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            2,
            [4.5, 200.0, 4.5],
            2,
        );
        guest.dimension = "minecraft:overworld".into();
        guest.placed = true;
        guest.loaded = true;
        guest.inventory[0] = Some(ItemStack::new("minecraft:stone", 16));
        guest.selected = 0;
        let mut host = game.server.lan.take().unwrap();
        host.hub.guests.push(guest);
        let id = host.hub.guests[0].entity_id;
        let swings_back = || {
            let mut n = 0;
            for out in rx.try_iter() {
                let bodies = match out {
                    Out::Packet(body) => vec![body],
                    Out::Packets(bodies) => bodies,
                    Out::Disconnect(_) => panic!("unexpected disconnect"),
                };
                for body in bodies {
                    let mut r = sunrise_protocol::buf::Reader::new(&body);
                    let packet_id = r.varint().unwrap();
                    if let Ok(Clientbound::SwingAnimation(p)) =
                        Clientbound::decode(packet_id, r.rest())
                        && p.id == id
                    {
                        n += 1;
                    }
                }
            }
            n
        };
        let use_on = |pos: [i32; 3], cursor: [f32; 3], sequence: i32| {
            Serverbound::UseItemOn(play::UseItemOn {
                hand: 0,
                pos,
                face: 1,
                cursor,
                inside: false,
                world_border: false,
                sequence,
            })
        };
        game.server.lan = Some(host);
        // BlockItem.place: SUCCESS, the client's own swing.
        game.lan_guest_packet(2, use_on([4, 199, 6], [0.5, 1.0, 0.5], 1));
        assert_eq!(game.world.block(4, 200, 6).0, stone, "the stone is placed");
        assert_eq!(swings_back(), 0, "a placement is not swung back");
        // The swing ends before the next use can start one.
        for _ in 0..8 {
            game.lan_swings();
        }
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.selected = 1);
        // BedBlock.useWithoutItem: SUCCESS_SERVER, the server's swing only.
        game.lan_guest_packet(2, use_on([6, 200, 4], [0.5, 0.5625, 0.5], 2));
        assert_eq!(swings_back(), 1, "a bed's use is swung back");
    }
}

#[cfg(test)]
pub(in crate::game) mod item_on_block_cooldown_tests {
    use std::sync::{Arc, atomic::AtomicI32, mpsc};
    use sunrise_core::position::ChunkPos;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_items::items::ItemStack;
    use sunrise_level::chunk::Chunk;
    use sunrise_protocol::packets::{
        configuration::ClientInformation,
        play::{self, Clientbound, Serverbound},
    };
    use sunrise_sim::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig};
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    // Actual registered guest packet route, with real loaded block facts.
    fn placement_control(check_start: Option<bool>) {
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
        options.game_dir = Some(
            std::env::temp_dir().join(format!("sunrise-use-cooldown-unit-{}", std::process::id())),
        );
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
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap()
            .0;
        let mut chunk = Chunk::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                chunk.set(x, 199, z, stone);
            }
        }
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        game.lan_host(
            sunrise_gameplay::game::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "use-cooldown-test".into(), 2)
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
                uuid: 331,
                name: "SwingGuest".into(),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            2,
            [4.5, 200.0, 4.5],
            2,
        );
        guest.dimension = "minecraft:overworld".into();
        guest.placed = true;
        guest.loaded = true;
        guest.inventory[0] = Some(ItemStack::new("minecraft:stone", 16));
        guest.selected = 0;

        game.server.lan.as_mut().unwrap().hub.guests.push(guest);
        assert_eq!(game.server.lan.as_mut().unwrap().hub.guests[0].uuid, 331);
        assert!(
            game.server.lan.as_mut().unwrap().hub.guests[0].placed
                && game.server.lan.as_mut().unwrap().hub.guests[0].loaded
        );
        assert_eq!(game.server.lan.as_mut().unwrap().hub.guests[0].game_mode, 0);
        let mut planks = ItemStack::new("minecraft:oak_planks", 5);
        planks.data_mut().set_other(
            "minecraft:use_cooldown",
            Some(sunrise_core::nbt::Tag::compound([
                ("seconds", sunrise_core::nbt::Tag::Float(30.0f32.to_bits())),
                (
                    "cooldown_group",
                    sunrise_core::nbt::Tag::String("probe:group".into()),
                ),
            ])),
        );
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |guest| guest.inventory[0] = Some(planks));
        let use_on = |x, sequence| {
            Serverbound::UseItemOn(play::UseItemOn {
                hand: 0,
                pos: [x, 199, 6],
                face: 1,
                cursor: [0.5, 1.0, 0.5],
                inside: false,
                world_border: false,
                sequence,
            })
        };
        assert!(game.world.is_air(game.world.block(5, 200, 6)));
        if check_start.is_none() {
            let mut stick = game.server.lan.as_mut().unwrap().hub.guests[0].inventory[0]
                .as_ref()
                .unwrap()
                .clone();
            stick.item = "minecraft:stick".into();
            stick.count = 1;
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| guest.inventory[0] = Some(stick));
            game.lan_guest_packet(2, use_on(5, 1));
            assert!(
                game.world.is_air(game.world.block(5, 200, 6)),
                "failed stick use"
            );
            assert!(
                !game.server.lan.as_mut().unwrap().play[&2]
                    .cooldowns
                    .is_on_cooldown("probe:group")
            );
            let mut planks = game.server.lan.as_mut().unwrap().hub.guests[0].inventory[0]
                .as_ref()
                .unwrap()
                .clone();
            planks.item = "minecraft:oak_planks".into();
            planks.count = 5;
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| guest.inventory[0] = Some(planks));
        }
        game.lan_guest_packet(2, use_on(5, 2));
        let oak = game
            .pack
            .assets
            .registry
            .default_state("minecraft:oak_planks")
            .unwrap();
        assert_eq!(game.world.block(5, 200, 6), oak, "actual first placement");
        assert_eq!(
            game.server.lan.as_mut().unwrap().hub.guests[0].inventory[0]
                .as_ref()
                .unwrap()
                .count,
            4
        );
        assert!(
            game.server.lan.as_mut().unwrap().play.contains_key(&2),
            "real acting guest context installed"
        );
        if check_start == Some(true) {
            assert!(
                game.server.lan.as_mut().unwrap().play[&2]
                    .cooldowns
                    .is_on_cooldown("probe:group"),
                "successful use starts the actual guest's group"
            );
            assert!(rx.try_iter().any(|out| {
                let bodies = match out { Out::Packet(body) => vec![body], Out::Packets(bodies) => bodies, Out::Disconnect(_) => panic!("disconnect") };
                bodies.iter().any(|body| {
                    let mut r = sunrise_protocol::buf::Reader::new(body);
                    matches!(Clientbound::decode(r.varint().unwrap(),r.rest()), Ok(Clientbound::Cooldown(p)) if p.group == "probe:group" && p.duration == 600)
                })
            }), "existing helper's actual cooldown packet");
        } else if check_start == Some(false) {
            game.lan_guest_packet(2, use_on(6, 2));
            assert!(
                game.world.is_air(game.world.block(6, 200, 6)),
                "same-group second placement is rejected"
            );
            assert_eq!(
                game.server.lan.as_mut().unwrap().hub.guests[0].inventory[0]
                    .as_ref()
                    .unwrap()
                    .count,
                4
            );
        }
    }
    #[test]
    #[ignore = "daily"]
    fn failed_stick_use_keeps_group_available_for_placement() {
        placement_control(None);
    }
    #[test]
    #[ignore = "daily"]
    fn actual_guest_placement_starts_its_cooldown() {
        placement_control(Some(true));
    }
    #[test]
    #[ignore = "daily"]
    fn actual_guest_second_placement_is_rejected() {
        placement_control(Some(false));
    }
}

#[cfg(test)]
#[path = "tests/boat_holder_movement_tests.rs"]
pub(in crate::game) mod boat_holder_movement_tests;

#[cfg(test)]
#[path = "tests/crack_feedback_tests.rs"]
pub(in crate::game) mod crack_feedback_tests;

#[cfg(test)]
#[path = "tests/projectile_lifecycle_tests.rs"]
pub(in crate::game) mod projectile_lifecycle_tests;

#[cfg(test)]
#[path = "tests/spawn_protection_tests.rs"]
pub(in crate::game) mod spawn_protection_tests;
