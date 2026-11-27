#![cfg(test)]
use sunrise_gameplay_common::game::lan_common::LanSettings;
use sunrise_protocol::packets::play;
use sunrise_protocol::packets::play::Clientbound as C;
use sunrise_sim::lan::NetConfig;
use sunrise_sim::lan::hub::HubConfig;
#[cfg(test)]
pub(crate) mod player_packet_tests;

#[cfg(test)]
pub(crate) mod world_spawn_sync_tests {

    use sunrise_gameplay_common::game::server_side::lan::world_spawn_sync_tests::spawns;

    use sunrise_gameplay::game::server_side::lan::world_spawn_sync_tests::host;

    use super::*;

    use std::sync::Arc;

    #[test]
    #[ignore = "daily"]
    fn join_and_respawn_contexts_use_world_spawn_not_a_personal_spawn() {
        run_join_and_respawn_contexts_use_world_spawn_not_a_personal_spawn();
    }
    pub(crate) fn run_join_and_respawn_contexts_use_world_spawn_not_a_personal_spawn() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.level.world_spawn = Some([100, 140, -20]);
            game.spawn = Some(sunrise_core::geometry::Vec3::new(96.5, 145.0, -31.5));
            let ctx = game.server_ctx();
            let block = ctx.spawn.map(|v| v.floor() as i32);
            println!(
                "world-spawn\tcontext\tminecraft:overworld\t{}\t{}\t{}",
                block[0], block[1], block[2]
            );
            assert_eq!(ctx.spawn, [100.5, 140.0, -19.5]);
            game.level.world_spawn_angle = [45.0, -10.0];
            assert_eq!(game.server_ctx().spawn_angle, [45.0, -10.0]);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn setworldspawn_reaches_guests_with_the_dimension_it_ran_in() {
        run_setworldspawn_reaches_guests_with_the_dimension_it_ran_in();
    }
    pub(crate) fn run_setworldspawn_reaches_guests_with_the_dimension_it_ran_in() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            game.level.world_spawn = None;
            let personal = sunrise_core::geometry::Vec3::new(96.5, 145.0, -31.5);
            game.spawn = Some(personal);
            for (x, z) in [(0, 0), (6, -2), (-1, 0)] {
                game.world
                    .chunks
                    .insert((x, z), Arc::new(sunrise_level::chunk::Chunk::empty(x, z)));
            }
            let rx = host(game);
            // Materializing the effective fallback does not change the packet data.
            game.as_console(|game| assert!(game.run_command("setworldspawn 0 80 0")));
            assert!(spawns(&rx).is_empty());
            game.as_console(|game| assert!(game.run_command("setworldspawn 100 140 -20")));
            assert_eq!(
                game.spawn,
                Some(personal),
                "a personal respawn point stays personal"
            );
            let sent = spawns(&rx);
            assert_eq!(
                sent.len(),
                1,
                "a placed Nether guest must hear the world spawn change"
            );
            assert_eq!(sent[0].pos.dimension, "minecraft:overworld");
            assert_eq!(sent[0].pos.pos, [100, 140, -20]);
            println!(
                "world-spawn\tchanged\t{}\t{}\t{}\t{}",
                sent[0].pos.dimension, sent[0].pos.pos[0], sent[0].pos.pos[1], sent[0].pos.pos[2]
            );
            game.as_console(|game| assert!(game.run_command("setworldspawn 100 140 -20")));
            assert!(
                spawns(&rx).is_empty(),
                "unchanged respawn data is not broadcast again"
            );
            // SetWorldSpawnCommand keeps the level it runs in (RespawnData.of(level.dimension(), ..)):
            // the same block in another dimension is changed respawn data, and reaches the guests
            // with that dimension.
            game.as_console(|game| {
                assert!(
                    game.run_command("execute in minecraft:the_end run setworldspawn 100 140 -20")
                )
            });
            let sent = spawns(&rx);
            assert_eq!(sent.len(), 1);
            assert_eq!(sent[0].pos.dimension, "minecraft:the_end");
            assert_eq!(sent[0].pos.pos, [100, 140, -20]);
            println!(
                "world-spawn\tother_dimension\t{}\t{}\t{}\t{}",
                sent[0].pos.dimension, sent[0].pos.pos[0], sent[0].pos.pos[1], sent[0].pos.pos[2]
            );
            game.as_console(|game| assert!(game.run_command("setworldspawn -7 90 12")));
            let sent = spawns(&rx);
            assert_eq!(sent.len(), 1);
            assert_eq!(sent[0].pos.dimension, "minecraft:overworld");
            assert_eq!(sent[0].pos.pos, [-7, 90, 12]);
            // Rotation alone is a changed RespawnData, even at the same block.
            game.as_console(|game| assert!(game.run_command("setworldspawn -7 90 12 45 -10")));
            let sent = spawns(&rx);
            assert_eq!(sent.len(), 1);
            assert_eq!(sent[0].pos.pos, [-7, 90, 12]);
            assert_eq!([sent[0].yaw, sent[0].pitch], [45.0, -10.0]);
            println!(
                "world-spawn\tangle\t{}\t{}\t{}\t{}\t{:.1}\t{:.1}",
                sent[0].pos.dimension,
                sent[0].pos.pos[0],
                sent[0].pos.pos[1],
                sent[0].pos.pos[2],
                sent[0].yaw,
                sent[0].pitch
            );
            game.as_console(|game| assert!(game.run_command("setworldspawn -7 90 12 45 -10")));
            assert!(spawns(&rx).is_empty());
            // RespawnData.of wraps yaw and clamps pitch before storing or comparing.
            game.as_console(|game| assert!(game.run_command("setworldspawn -7 90 12 405 100")));
            let sent = spawns(&rx);
            assert_eq!(sent.len(), 1);
            assert_eq!(game.level.world_spawn_angle, [45.0, 90.0]);
            assert_eq!([sent[0].yaw, sent[0].pitch], [45.0, 90.0]);
            println!(
                "world-spawn\tnormalized\t{}\t{}\t{}\t{}\t{:.1}\t{:.1}",
                sent[0].pos.dimension,
                sent[0].pos.pos[0],
                sent[0].pos.pos[1],
                sent[0].pos.pos[2],
                sent[0].yaw,
                sent[0].pitch
            );
            for command in [
                "setworldspawn -7 90 12 45 90",
                "setworldspawn -7 90 12 405 100",
            ] {
                game.as_console(|game| assert!(game.run_command(command)));
                assert!(
                    spawns(&rx).is_empty(),
                    "equivalent normalized rotations are unchanged"
                );
            }
            // Clamp raw pitch: wrapping -270 first would incorrectly produce +90.
            game.as_console(|game| assert!(game.run_command("setworldspawn -7 90 12 405 -270")));
            let sent = spawns(&rx);
            assert_eq!(sent.len(), 1);
            assert_eq!(game.level.world_spawn_angle, [45.0, -90.0]);
            assert_eq!([sent[0].yaw, sent[0].pitch], [45.0, -90.0]);
            println!(
                "world-spawn\tclamped\t{}\t{}\t{}\t{}\t{:.1}\t{:.1}",
                sent[0].pos.dimension,
                sent[0].pos.pos[0],
                sent[0].pos.pos[1],
                sent[0].pos.pos[2],
                sent[0].yaw,
                sent[0].pitch
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn guest_respawn_without_a_known_world_spawn_never_advertises_the_hosts_personal_point() {
        run_guest_respawn_without_a_known_world_spawn_never_advertises_the_hosts_personal_point();
    }
    pub(crate) fn run_guest_respawn_without_a_known_world_spawn_never_advertises_the_hosts_personal_point()
     {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.level.world_spawn = None;
            game.level.world_spawn_angle = [45.0, -10.0];
            let host_personal = sunrise_core::geometry::Vec3::new(96.5, 145.0, -31.5);
            let guest_personal = [5.5, 81.0, 5.5];
            game.spawn = Some(host_personal);
            let rx = host(game);
            let mut host = game.server.lan.take().unwrap();
            host.hub.guests.update(0, |guest| {
                guest.dimension = "minecraft:overworld".into();
                guest.spawn = Some(guest_personal);
                guest.health = 0.0;
                guest.dead = true;
                guest.loaded = true;
            });
            game.server.lan = Some(host);
            game.guest_respawn(1);
            let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(
                guest.position, guest_personal,
                "the guest's landing stays personal"
            );
            assert!(!guest.dead);
            assert_eq!(game.spawn, Some(host_personal));
            let sent = spawns(&rx);
            assert_eq!(sent.len(), 2, "respawn and level info both send world data");
            for point in &sent {
                assert_eq!(point.pos.dimension, "minecraft:overworld");
                assert_eq!(
                    point.pos.pos,
                    [0, 80, 0],
                    "the host's personal point is not world data"
                );
                assert_eq!([point.yaw, point.pitch], [45.0, -10.0]);
            }
            println!(
                "world-spawn\trespawn\t{}\t{}\t{}\t{}\t{:.1}\t{:.1}",
                sent[0].pos.dimension,
                sent[0].pos.pos[0],
                sent[0].pos.pos[1],
                sent[0].pos.pos[2],
                sent[0].yaw,
                sent[0].pitch
            );
        });
    }
}

#[cfg(test)]
pub(crate) mod world_spawn_join_tests {
    /// `PrepareSpawnTask.start`: a returning player whose saved level the world no longer has
    /// joins in the world spawn's level (`respawnData.dimension()`), at its saved position and
    /// rotation, not in the Overworld.
    #[test]
    #[ignore = "daily"]
    fn a_returning_player_from_a_missing_level_joins_in_the_world_spawn_level() {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            game.server.world_root = game.server.save.0.root.clone();
            game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
                .unwrap();
            game.enter_level(sunrise_gameplay_common::game::Dim::Overworld)
                .unwrap();
            game.level.world_spawn_dimension = Some("minecraft:the_nether".into());
            let guests = &mut game.server.lan.as_mut().unwrap().hub.guests;
            guests.update(0, |g| {
                g.dimension = "example:gone".into();
                g.position = [40.5, 70.0, -12.5];
                g.yaw = 135.0;
                g.pitch = -20.0;
            });
            sunrise_sim::lan::playerdata::save(
                &guests[0],
                &game.server.world_root,
                &Default::default(),
            );
            guests.update(0, |g| g.placed = false);
            game.lan_guest_joined(12, &[], [0.5, 64.0, 0.5], None);
            let g = &game.server.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(g.dimension, "minecraft:the_nether");
            assert_eq!(g.position, [40.5, 70.0, -12.5]);
            assert_eq!((g.yaw, g.pitch), (135.0, -20.0));
        });
    }
}

#[cfg(test)]
pub(crate) mod mob_pickup_sync_tests {

    use super::*;
    use std::sync::{Arc, atomic::AtomicI32, mpsc};
    use sunrise_items::items::ItemStack;
    use sunrise_sim::lan::{Joined, Out, PlayerSender, guest::Guest};

    /// A server with one placed guest beside the mob.
    fn guest_at(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        at: [f64; 3],
    ) -> mpsc::Receiver<Out> {
        game.lan_host(
            LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig::loopback(0, String::new(), 2),
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
                max_players: 20,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();
        let (tx, rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn: 1,
                uuid: 1,
                name: "PickupGuest".into(),
                properties: Vec::new(),
                info: sunrise_protocol::packets::configuration::ClientInformation::new(2),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            1,
            at,
            2,
        );
        guest.placed = true;
        game.server.lan.as_mut().unwrap().hub.guests.push(guest);
        rx
    }

    fn takes(rx: &mpsc::Receiver<Out>) -> Vec<play::TakeItemEntity> {
        rx.try_iter()
            .flat_map(|out| match out {
                Out::Packet(body) => vec![body],
                Out::Packets(bodies) => bodies,
                Out::Disconnect(_) => panic!("unexpected disconnect"),
            })
            .filter_map(|body| {
                let mut r = sunrise_protocol::buf::Reader::new(&body);
                let id = r.varint().unwrap();
                match C::decode(id, r.rest()).unwrap() {
                    C::TakeItemEntity(p) => Some(p),
                    _ => None,
                }
            })
            .collect()
    }

    /// `ItemEntity.playerTouch` broadcasts the take whoever collects it; the
    /// window is a client too, so a mob's pickup has to reach it or the item
    /// just vanishes (#12493).
    #[test]
    #[ignore = "daily"]
    fn a_mob_pickup_reaches_guests_as_a_take() {
        let mut resources = None;
        let mut game = super::super::game_tests::tests::server_game(&mut resources);
        game.build_registries();
        game.set_allow_commands(true);
        assert!(game.run_command("summon piglin 8.5 80 8.5 {NoAI:1b,PersistenceRequired:1b}"));
        let mob = game.mobs[0].id;
        game.spawn_item(
            ItemStack::new("minecraft:gold_ingot", 3),
            [8.5, 80.0, 8.5],
            [0.0; 3],
            0,
        );
        let item = game.dropped[0].clone();
        let rx = guest_at(&mut game, [8.5, 80.0, 8.5]);
        game.server_mob_take_item(&item, mob);
        let sent = takes(&rx);
        assert!(
            sent.iter()
                .any(|p| p.item == item.id as i32 && p.player == mob as i32 && p.amount == 3),
            "the take: {sent:?}"
        );
    }
}

#[cfg(test)]
pub(crate) mod mob_swing_sync_tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicI32, mpsc};
    use {
        sunrise_sim::first_person_sim::SwingAnimation, sunrise_sim::lan::Joined,
        sunrise_sim::lan::Out, sunrise_sim::lan::PlayerSender, sunrise_sim::lan::guest::Guest,
    };

    /// A server with one placed guest beside the mobs.
    fn guest_at(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        at: [f64; 3],
    ) -> mpsc::Receiver<Out> {
        game.lan_host(
            LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig::loopback(0, String::new(), 2),
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
                max_players: 20,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();
        let (tx, rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn: 1,
                uuid: 1,
                name: "SwingGuest".into(),
                properties: Vec::new(),
                info: sunrise_protocol::packets::configuration::ClientInformation::new(2),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            1,
            at,
            2,
        );
        guest.placed = true;
        game.server.lan.as_mut().unwrap().hub.guests.push(guest);
        rx
    }

    fn swings(rx: &mpsc::Receiver<Out>) -> Vec<play::SwingAnimation> {
        rx.try_iter()
            .flat_map(|out| match out {
                Out::Packet(body) => vec![body],
                Out::Packets(bodies) => bodies,
                Out::Disconnect(_) => panic!("unexpected disconnect"),
            })
            .filter_map(|body| {
                let mut r = sunrise_protocol::buf::Reader::new(&body);
                let id = r.varint().unwrap();
                match C::decode(id, r.rest()).unwrap() {
                    C::SwingAnimation(p) => Some(p),
                    _ => None,
                }
            })
            .collect()
    }

    /// `LivingEntity.swing` names the hand; the flush reads the arm the swing
    /// recorded, so an off-hand throw reaches guests as the off hand (#12512).
    #[test]
    #[ignore = "daily"]
    fn an_off_hand_mob_swing_reaches_guests_as_the_off_hand() {
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        game.build_registries();
        game.set_allow_commands(true);
        let piglin = "{NoAI:1b,PersistenceRequired:1b}";
        assert!(game.run_command(&format!("summon piglin 8.5 80 8.5 {piglin}")));
        assert!(game.run_command(&format!("summon piglin 10.5 80 10.5 {piglin}")));
        let (off, main) = (game.mobs[0].id, game.mobs[1].id);
        let rx = guest_at(&mut game, [8.5, 80.0, 8.5]);
        game.mobs[0].swing_hand_modified(true, SwingAnimation::DEFAULT);
        game.mobs[1].swing_hand_modified(false, SwingAnimation::DEFAULT);
        let host = game.server.lan.take().expect("the hub");
        game.server.lan = Some(host);
        game.lan_flush_effects();
        let host = game.server.lan.take().unwrap();
        game.server.lan = Some(host);
        let sent = swings(&rx);
        assert!(
            sent.iter().any(|p| p.id == off as i32 && p.hand == 1),
            "the off hand: {sent:?}"
        );
        assert!(
            sent.iter().any(|p| p.id == main as i32 && p.hand == 0),
            "the main hand: {sent:?}"
        );
    }
}

#[cfg(test)]
pub(crate) mod owner_attachment_connected_statistics_save_tests;

#[cfg(test)]
pub(crate) mod login_profile_tests;
#[cfg(test)]
pub(crate) mod no_gravity_sync_tests;
#[cfg(test)]
pub(crate) mod tracking_hook_tests;
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

#[cfg(test)]
include!("advancement_save_tests.rs");
