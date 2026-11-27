#![cfg(test)]

#[cfg(test)]
pub(crate) mod controller_tests {
    use std::sync::{Arc, atomic::AtomicI32, mpsc};
    use sunrise_core::position::ChunkPos;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_level::chunk::Chunk;
    use sunrise_protocol::packets::{configuration::ClientInformation, play};
    use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    fn guest(conn: u32, uuid: u128) -> Guest {
        guest_with_packets(conn, uuid).0
    }

    /// A guest and what is sent to it.
    fn guest_with_packets(conn: u32, uuid: u128) -> (Guest, mpsc::Receiver<sunrise_sim::lan::Out>) {
        let (tx, rx) = mpsc::channel();
        let mut g = Guest::new(
            Joined {
                conn,
                uuid,
                name: format!("Rider{conn}"),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            1000 + conn as i32,
            [4.5, 200.0, 4.5],
            2,
        );
        g.dimension = "minecraft:overworld".into();
        g.placed = true;
        g.loaded = true;
        // PlayerLoaded came in: its moves count.
        g.load_timeout = 0;
        (g, rx)
    }

    /// Two players in one boat: the one that got in first drives it
    /// (`getControllingPassenger`), whichever joined the server first. Its
    /// client hears it sits first, and only its moves count; the second
    /// seat's client, which still sends where it sees the boat, used to put
    /// the boat back every tick (#11541).
    #[test]
    #[ignore = "daily"]
    fn the_first_rider_in_drives_the_boat() {
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
            std::env::temp_dir().join(format!("sunrise-boat-driver-unit-{}", std::process::id())),
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
        assert!(game.run_command("summon minecraft:oak_boat 4.5 200 4.5"));
        let boat = game.boats[0].id;
        let start = game.boats[0].position;
        game.lan_host(
            sunrise_gameplay::game::server_side::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "boat-driver-test".into(), 3)
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
                max_players: 3,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();
        let mut host = game.server.lan.take().unwrap();
        // The second seat's player joined the server first; the driver got
        // into the boat first.
        let (mut passenger, mut driver) = (guest(2, 21), guest(3, 22));
        (driver.vehicle, driver.vehicle_order) = (Some(boat), 1);
        (passenger.vehicle, passenger.vehicle_order) = (Some(boat), 2);
        let (passenger_id, driver_id) = (passenger.entity_id, driver.entity_id);
        host.hub.guests.push(passenger);
        host.hub.guests.push(driver);

        let mut seated: Vec<_> = host
            .hub
            .guests
            .iter()
            .filter_map(|g| Some((g.vehicle_order, g.vehicle?, g.entity_id)))
            .collect();
        seated.sort_by_key(|&(order, ..)| order);
        let guests: Vec<_> = seated
            .into_iter()
            .map(|(_, vehicle, id)| (vehicle, id))
            .collect();
        game.lan_riding(&guests, &mut host.hub.riding);
        assert_eq!(
            host.hub.riding.passengers(boat as i32),
            [driver_id, passenger_id],
            "the driver sits first"
        );

        let move_to = |x: f64| play::MoveVehicleRequest {
            position: [x, start.y, start.z],
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
        };
        game.server.lan = Some(host);
        game.guest_move_vehicle(2, &move_to(start.x + 0.3));
        assert_eq!(
            game.boats[0].position, start,
            "the second seat moves nothing"
        );
        game.guest_move_vehicle(3, &move_to(start.x + 0.3));
        assert!(
            (game.boats[0].position.x - (start.x + 0.3)).abs() < 1.0e-9,
            "the driver moves the boat: {:?}",
            game.boats[0].position
        );
        // handleMoveVehicle's `vehicle.move` runs AbstractBoat.checkFallDamage:
        // a boat that lands on the ground (a fall loaded from its save, here)
        // has no fall left.
        game.boats.update(0, |b| b.fall_distance = 5.0);
        game.guest_move_vehicle(3, &move_to(start.x + 0.6));
        assert_eq!(
            game.boats[0].fall_distance, 0.0,
            "a guest's boat on the ground"
        );
        // `AbstractBoat.tick` still runs `applyEffectsFromBlocks` on the server
        // for the boat its guest drives: `WebBlock.entityInside` resets the fall
        // (`Entity.makeStuckInBlock`).
        let cobweb = game
            .pack
            .assets
            .registry
            .default_state("minecraft:cobweb")
            .unwrap();
        let p = game.boats[0].position;
        game.world
            .set(
                p.x.floor() as i32,
                p.y.floor() as i32,
                p.z.floor() as i32,
                cobweb,
            )
            .unwrap();
        game.boats.update(0, |b| b.fall_distance = 4.0);
        game.guest_move_vehicle(
            3,
            &play::MoveVehicleRequest {
                position: [p.x, p.y, p.z],
                yaw: 0.0,
                pitch: 0.0,
                on_ground: false,
            },
        );
        assert_eq!(game.boats[0].fall_distance, 4.0, "still falling");
        game.tick_boats();
        assert_eq!(
            game.boats[0].fall_distance, 0.0,
            "a cobweb resets the fall of a boat its guest drives"
        );
    }

    /// A guest drives a horse with an unobstructed ten-block fall to stone.
    pub(crate) fn guest_horse_at_start_of_fall(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
    ) -> (u32, mpsc::Receiver<sunrise_sim::lan::Out>) {
        game.build_registries();
        game.set_allow_commands(true);
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap()
            .0;
        let cobweb = game
            .pack
            .assets
            .registry
            .default_state("minecraft:cobweb")
            .unwrap()
            .0;
        let mut chunk = Chunk::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                chunk.set(x, 199, z, stone);
            }
        }
        // Off the fall's line: only the long moves at the end cross them.
        chunk.set(6, 205, 4, cobweb);
        chunk.set(8, 205, 4, cobweb);
        for y in 205..=207 {
            chunk.set(10, y, 4, stone);
        }
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("summon minecraft:horse 4.5 210 4.5 {NoAI:1b,Tame:1b}"));
        let horse = game.mobs[0].id;
        game.mobs[0].extra.set_saddled(true);
        game.mobs[0].rider = Some(sunrise_sim::mobs::equine::Rider {
            client: true,
            ..Default::default()
        });
        game.lan_host(
            sunrise_gameplay::game::server_side::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "horse-fall-test".into(), 3)
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
                max_players: 3,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();

        let (mut rider, packets) = guest_with_packets(2, 21);
        (rider.vehicle, rider.vehicle_order) = (Some(horse), 1);
        game.server.lan.as_mut().unwrap().hub.guests.push(rider);
        (horse, packets)
    }

    #[test]
    #[ignore = "daily"]
    fn a_guests_mount_uses_its_totem_during_the_landing() {
        use sunrise_gameplay::game::native_entity_refs::tests::with_game;

        with_game(|game| {
            let (horse, _packets) = guest_horse_at_start_of_fall(game);
            game.mobs.update(0, |mob| {
                mob.health = 1.0;
                mob.add_effect("minecraft:poison", 25, 0);
                mob.extra
                    .set_main_hand_stack(Some(sunrise_items::items::ItemStack::new(
                        "minecraft:totem_of_undying",
                        1,
                    )));
            });
            for (y, on_ground) in [
                (209.0, false),
                (207.0, false),
                (204.0, false),
                (200.0, true),
            ] {
                game.guest_move_vehicle(
                    2,
                    &play::MoveVehicleRequest {
                        position: [4.5, y, 4.5],
                        yaw: 0.0,
                        pitch: 0.0,
                        on_ground,
                    },
                );
            }
            let mob = game.mobs.iter().find(|mob| mob.id == horse).unwrap();
            assert!(mob.alive());
            assert!(mob.extra.main_hand_stack().is_none());
            assert!(!mob.has_effect("minecraft:poison"));
            assert!(mob.has_effect("minecraft:regeneration"));
            assert_eq!(game.server.lan.as_ref().unwrap().hub.guests[0].health, 13.0);
        });
    }

    /// handleMoveVehicle for a horse its guest's client moves: `vehicle.move`
    /// leaves the fall alone (the server isn't the mount's authority), then
    /// `doCheckFallDamage` with the client's move and ground adds each drop and,
    /// on the ground, lands the horse: `AbstractHorse.causeFallDamage` of the
    /// whole fall. The horse used to keep no fall at all (#13042).
    #[test]
    #[ignore = "daily"]
    fn a_guests_horse_falls_and_lands() {
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
            std::env::temp_dir().join(format!("sunrise-horse-fall-unit-{}", std::process::id())),
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
        let (_, packets) = guest_horse_at_start_of_fall(&mut game);
        // What the landing should do: the horse's own fall damage for the whole drop.
        let mut twin = game.mobs[0].clone();
        twin.cause_fall_damage_from(
            &game.loop_context,
            &mut sunrise_sim::health::TestLevel::new(0).level(),
            10.0,
            1.0,
            &mut Vec::new(),
        );
        assert!(
            twin.health < game.mobs[0].health,
            "a 10-block fall hurts a horse"
        );

        let move_to = |y: f64, on_ground: bool| play::MoveVehicleRequest {
            position: [4.5, y, 4.5],
            yaw: 0.0,
            pitch: 0.0,
            on_ground,
        };
        for y in [209.0, 207.0, 204.0] {
            game.guest_move_vehicle(2, &move_to(y, false));
        }
        assert_eq!(game.mobs[0].position.y, 204.0, "the guest moves the horse");
        assert_eq!(
            game.mobs[0].fall_distance, 6.0,
            "each drop adds to the fall"
        );
        game.guest_move_vehicle(2, &move_to(200.0, true));
        assert_eq!(game.mobs[0].fall_distance, 0.0, "the landing ends the fall");
        assert_eq!(
            game.mobs[0].health, twin.health,
            "the landing hurts as the fall does"
        );
        // propagateFallToPassengers: the rider's own causeFallDamage of the fall,
        // ceil((10 - 3) * 1) = 7.
        assert_eq!(
            game.server.lan.as_mut().unwrap().hub.guests[0].health,
            13.0,
            "the rider takes the fall too"
        );
        // LivingEntity.checkFallDamage sends the landing's block crumbs to every
        // player near, the rider too.
        let dust = packets.try_iter().any(|out| match out {
            sunrise_sim::lan::Out::Packet(body) => {
                let mut r = sunrise_protocol::buf::Reader::new(&body);
                let id = r.varint().unwrap();
                matches!(
                    play::Clientbound::decode(id, r.rest()),
                    Ok(play::Clientbound::LevelParticles(_))
                )
            }
            _ => false,
        });
        assert!(dust, "the guest sees the landing dust");

        // Entity.move in `vehicle.move`: a move of a block or more casts along
        // it (FALLDAMAGE_RESETTING, WATER), and the cobweb it crosses resets
        // the fall (a fall loaded from its save, here) before the move counts.
        game.guest_move_vehicle(2, &move_to(205.5, false));
        assert_eq!(game.mobs[0].position.y, 205.5, "the guest lifts the horse");
        game.mobs[0].fall_distance = 4.0;
        game.guest_move_vehicle(
            2,
            &play::MoveVehicleRequest {
                position: [6.5, 205.5, 4.5],
                yaw: 0.0,
                pitch: 0.0,
                on_ground: false,
            },
        );
        assert_eq!(game.mobs[0].position.x, 6.5, "the guest moves the horse on");
        assert_eq!(
            game.mobs[0].fall_distance, 0.0,
            "a long move through a cobweb resets the fall"
        );
        // handleMoveVehicle moves the horse before it checks the move: one
        // into a wall is put back (moved wrongly), and keeps the reset.
        game.mobs[0].fall_distance = 4.0;
        game.guest_move_vehicle(
            2,
            &play::MoveVehicleRequest {
                position: [11.5, 205.5, 4.5],
                yaw: 0.0,
                pitch: 0.0,
                on_ground: false,
            },
        );
        assert_eq!(game.mobs[0].position.x, 6.5, "the move is put back");
        assert_eq!(
            game.mobs[0].fall_distance, 0.0,
            "a move put back keeps the cobweb's reset"
        );
    }
}
