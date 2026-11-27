#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::respawn_anchor::RespawnConfig;
use sunrise_gameplay::game::respawn_anchor::look_at_yaw;
use sunrise_gameplay_common::game::Dim;
#[cfg(test)]
pub(crate) mod connected_spawnpoint_rotation_tests {
    use std::sync::{Arc, atomic::AtomicI32, mpsc};
    use sunrise_gameplay::game::lan::LanSettings;
    use sunrise_level::chunk::Chunk;
    use sunrise_protocol::packets::{
        configuration::ClientInformation,
        play::{self, Clientbound as C, Serverbound as S},
    };
    use sunrise_sim::lan::{
        Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig, playerdata,
    };

    fn command(game: &mut sunrise_gameplay::game::ServerGame<'_>, text: &str) {
        game.server.chat_capture = Some(Vec::new());
        game.ui.registries.permission = 4;
        let dispatcher = sunrise_command::vanilla::build_for(false, true);
        let parsed = dispatcher.parse(text, &game.ui.registries);
        let (ctx, _) = parsed
            .executable()
            .expect("actual registered dedicated command");
        let (root, ctx) = (parsed.context.clone(), ctx.clone());
        assert_eq!(game.run_chain(&root, &ctx).unwrap(), 1, "{text}");
        assert!(
            !game.server.chat_capture.take().unwrap().is_empty(),
            "actual command feedback {text}"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn command_forced_free_point_survives_real_guest_death_and_respawn_packet() {
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        game.set_allow_commands(true);
        let scratch = std::env::temp_dir().join(format!(
            "sunrise-forced-spawn-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&scratch).unwrap();
        game.server.world_root = scratch.clone();
        // Fixture setup through the existing column insertion API; admit every lit column below.
        for x in -2..=2 {
            for z in -2..=2 {
                game.server
                    .streaming
                    .insert_remote(&mut game.common.world, Chunk::empty(x, z));
                assert!(game.server.streaming.is_lit(x, z));
            }
        }
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let air = game
            .pack
            .assets
            .registry
            .default_state("minecraft:air")
            .unwrap();
        game.world.set(12, 99, 12, stone).unwrap();
        for y in [100, 101] {
            game.world.set(12, y, 12, air).unwrap();
        }
        assert_eq!(game.world.block(12, 99, 12), stone);
        assert!(game.world.loaded(12, 12));
        game.level.world_spawn = Some([4, 100, 4]);
        game.level.world_spawn_angle = [-30.0, -5.0];
        game.lan_host(
            LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                listen: false,
                announce: false,
                ..NetConfig::loopback(0, String::new(), 2)
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
                conn: 7,
                uuid: 77,
                name: "Alice".into(),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:7".parse().unwrap(),
                expected: false,
            },
            7,
            [4.5, 100.0, 4.5],
            2,
        );
        guest.dimension = "minecraft:overworld".into();
        guest.placed = true;
        guest.loaded = true;
        guest.health = 20.0;
        guest.game_mode = 0;
        guest.yaw = 0.0;
        guest.pitch = 0.0;
        game.server.lan.as_mut().unwrap().hub.guests.push(guest);
        let clock = game.world.game_time;
        command(&mut game, "spawnpoint Alice 12 100 12 45 10");
        let host = game.server.lan.as_ref().unwrap();
        assert!(host.owner.is_none());
        let guest = &host.hub.guests[0];
        assert_eq!(
            (guest.uuid, guest.health, guest.game_mode, guest.position),
            (77, 20.0, 0, [4.5, 100.0, 4.5])
        );
        let tag = playerdata::player_tag(guest, &Default::default());
        let respawn = tag
            .get("respawn")
            .expect("actual serialized personal configuration")
            .clone();
        assert_eq!(
            respawn.get("yaw"),
            Some(&sunrise_core::nbt::Tag::Float(45f32.to_bits()))
        );
        assert_eq!(
            respawn.get("pos"),
            Some(&sunrise_core::nbt::Tag::IntArray(vec![12, 100, 12]))
        );
        command(&mut game, "kill Alice");
        game.tick_lan(false);
        let host = game.server.lan.take().unwrap();
        assert_eq!(host.hub.guests[0].uuid, 77);
        assert_eq!(host.hub.guests[0].health, 0.0);
        assert!(
            host.hub.guests[0].dead,
            "actual command death before request"
        );
        for _ in rx.try_iter() {}
        game.server.lan = Some(host);
        game.lan_guest_packet(7, S::ClientCommand(play::ClientCommand { action: 0 }));
        game.tick_server_travel();
        let host = game.server.lan.take().unwrap();
        assert!(
            !game.on_server_trip(7),
            "admitted lit columns permit actual arrival"
        );
        let guest = &host.hub.guests[0];
        assert_eq!(guest.uuid, 77);
        assert_eq!(guest.health, 20.0);
        assert!(!guest.dead);
        assert_eq!(guest.position, [12.5, 100.1, 12.5]);
        assert_eq!(game.level.world_spawn, Some([4, 100, 4]));
        assert_eq!(game.level.world_spawn_angle, [-30.0, -5.0]);
        assert_eq!(game.world.game_time, clock);
        let physical = [guest.yaw.to_bits(), guest.pitch.to_bits()];
        let awaiting_id = guest
            .awaiting_teleport
            .expect("actual final arrival awaits its acknowledgement")
            .0;
        let mut packets = Vec::new();
        for out in rx.try_iter() {
            match out {
                Out::Packet(p) => packets.push(p),
                Out::Packets(p) => packets.extend(p),
                Out::Disconnect(_) => panic!("unexpected disconnect"),
            }
        }
        let mut positions = Vec::new();
        let mut respawns = 0;
        for p in packets {
            println!("complete actual Hub frame {p:?}");
            let mut r = sunrise_protocol::buf::Reader::new(&p);
            let id = r.varint().unwrap();
            match C::decode(id, r.rest()).unwrap() {
                C::PlayerPosition(p) => positions.push(p),
                C::Respawn(_) => respawns += 1,
                _ => {}
            }
        }
        assert_eq!(respawns, 1);
        assert!(!positions.is_empty());
        println!("all actual arrival positions {positions:?}");
        let selected: Vec<_> = positions
            .iter()
            .filter(|p| p.teleport == awaiting_id)
            .collect();
        assert_eq!(
            selected.len(),
            1,
            "select by actual authoritative pending id, never expected position/pitch"
        );
        let p = selected[0];
        assert_eq!(p.position, [12.5, 100.1, 12.5]);
        // All successful command/death/arrival/identity/world controls precede the intended regression.
        assert_eq!(
            (
                respawn.get("pitch"),
                physical,
                [p.yaw.to_bits(), p.pitch.to_bits()]
            ),
            (
                Some(&sunrise_core::nbt::Tag::Float(10f32.to_bits())),
                [45f32.to_bits(), 10f32.to_bits()],
                [45f32.to_bits(), 10f32.to_bits()]
            )
        );
        drop(host);
        drop(game);
        std::fs::remove_dir_all(scratch).unwrap();
    }
}

#[cfg(test)]
pub(crate) mod forced_respawn_space_tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn forced_respawn_checks_parts_for_both_cells() {
        use mod_api::{BlockPart, Hooks, Host, IsPossibleToRespawnInEvent, Registrar};
        struct Space(bool);
        #[mod_api::sunrise_part]
        impl BlockPart for Space {
            fn is_possible_to_respawn_in(
                &self,
                _: &dyn Host,
                _: &mut IsPossibleToRespawnInEvent,
            ) -> Option<bool> {
                Some(self.0)
            }
        }
        struct Parts;
        #[mod_api::__subscriptions]
        impl Hooks for Parts {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let stone = r.block_key("minecraft:stone");
                r.block_part(stone, Space(true));
                let anchor = r.block_key("minecraft:respawn_anchor");
                r.block_part(anchor, Space(true));
                let air = r.block_key("minecraft:air");
                r.block_part(air, Space(false));
            }
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("space", Box::new(Parts))].into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let registry = game.pack.assets.registry;
                let stone = registry.default_state("minecraft:stone").unwrap();
                let anchor = registry.default_state("minecraft:respawn_anchor").unwrap();
                let air = registry.default_state("minecraft:air").unwrap();
                let glass = registry.default_state("minecraft:glass").unwrap();
                // Parts can accept solid space and reject normally empty space.
                assert!(game.possible_to_respawn_in(stone));
                assert!(!game.possible_to_respawn_in(air));
                assert!(!game.possible_to_respawn_in(glass));
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                game.world.set(5, 64, 5, stone).unwrap();
                game.world.set(5, 65, 5, stone).unwrap();
                let config = RespawnConfig {
                    dim: Dim::Overworld,
                    pos: [5, 64, 5],
                    yaw: 45.0,
                    pitch: 10.0,
                    forced: true,
                };
                assert!(game.find_respawn_and_use(config, false).unwrap().is_some());
                game.world.set(5, 65, 5, air).unwrap();
                assert!(game.find_respawn_and_use(config, false).unwrap().is_none());
                game.world.set(5, 65, 5, stone).unwrap();
                game.world.set(5, 64, 5, air).unwrap();
                assert!(game.find_respawn_and_use(config, false).unwrap().is_none());

                // The Overworld anchor cannot set spawn, so its part answers
                // the ordinary forced-column predicate instead.
                assert!(!game.anchor_works());
                game.world.set(5, 64, 5, anchor).unwrap();
                assert_eq!(
                    game.find_respawn_and_use(config, false).unwrap(),
                    Some((Vec3::new(5.5, 64.1, 5.5), 45.0, 10.0)),
                );
            });
        });
    }
}

#[cfg(test)]
pub(crate) mod pitch_preservation_controls {
    use super::*;

    #[test]
    fn wrapped_existing_callers_default_to_positive_zero_pitch() {
        let c = RespawnConfig::wrapped(Dim::Overworld, [5, 100, 5], 45.0, false);
        assert_eq!(c.yaw.to_bits(), 45f32.to_bits());
        assert_eq!(c.pitch.to_bits(), 0.0f32.to_bits());
        assert!(!c.forced);
    }

    #[test]
    #[ignore = "daily"]
    fn bed_and_anchor_facing_ignore_stored_pitch() {
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for x in 0..16 {
            for z in 0..16 {
                game.world.set(x, 99, z, stone).unwrap();
            }
        }
        let state = |game: &sunrise_gameplay::game::ServerGame<'_>,
                     name: &str,
                     properties: &[(&str, &str)]| {
            let registry = game.pack.assets.registry;
            let id = registry.default_state(name).unwrap();
            let mut value = registry.state(id).unwrap().clone();
            for (key, value_) in properties {
                value.properties.insert((*key).into(), (*value_).into());
            }
            registry.resolve(&value).unwrap()
        };
        let head = state(
            &game,
            "minecraft:red_bed",
            &[("facing", "north"), ("part", "head")],
        );
        let foot = state(
            &game,
            "minecraft:red_bed",
            &[("facing", "north"), ("part", "foot")],
        );
        game.world.set(5, 100, 5, head).unwrap();
        game.world.set(5, 100, 6, foot).unwrap();
        assert!(game.is_bed(head) && game.bed_sets_spawn(head));
        let bed = RespawnConfig {
            dim: Dim::Overworld,
            pos: [5, 100, 5],
            yaw: 45.0,
            pitch: 0.0,
            forced: true,
        };
        let zero = game
            .find_respawn_and_use(bed, false)
            .unwrap()
            .expect("valid bed geometry");
        let stored = game
            .find_respawn_and_use(RespawnConfig { pitch: 75.0, ..bed }, false)
            .unwrap()
            .unwrap();
        assert_eq!(stored, zero);
        assert_eq!(stored.2.to_bits(), 0.0f32.to_bits());
        assert_eq!(stored.1.to_bits(), look_at_yaw(stored.0, bed.pos).to_bits());

        // Existing built-in dimension facts select the anchor rule; no new world or actor API.
        game.dim = Dim::Nether;
        let anchor = state(&game, "minecraft:respawn_anchor", &[("charges", "2")]);
        game.world.set(10, 100, 10, anchor).unwrap();
        assert!(game.anchor_works() && game.is_anchor(anchor));
        let c = RespawnConfig {
            dim: Dim::Nether,
            pos: [10, 100, 10],
            yaw: 45.0,
            pitch: 0.0,
            forced: true,
        };
        let zero = game
            .find_respawn_and_use(c, false)
            .unwrap()
            .expect("valid anchor geometry");
        let stored = game
            .find_respawn_and_use(RespawnConfig { pitch: 75.0, ..c }, true)
            .unwrap()
            .unwrap();
        assert_eq!(stored, zero);
        assert_eq!(stored.2.to_bits(), 0.0f32.to_bits());
        assert_eq!(stored.1.to_bits(), look_at_yaw(stored.0, c.pos).to_bits());
        assert_eq!(
            game.world.block(10, 100, 10),
            anchor,
            "existing forced-charge behavior"
        );
    }
}
