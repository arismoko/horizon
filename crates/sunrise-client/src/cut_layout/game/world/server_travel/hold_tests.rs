//! A `tp` inside a level to chunks not lit yet holds the player (their client's moves are ignored,
//! `Guest::landing`) until the chunks are lit; however the hold ends, the moves are heard again (#15727).
use std::sync::{Arc, atomic::AtomicI32, mpsc};
use sunrise_core::geometry::Vec3;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::packets::configuration::ClientInformation;
use sunrise_sim::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig};
use {sunrise_gameplay::game::lan::LanHost, sunrise_gameplay::game::lan::LanSettings};

const CONN: u32 = 7;
const FAR: f64 = 96_000.5;

fn light(game: &mut sunrise_gameplay::game::ServerGame<'_>, around: i32) {
    for x in around - 2..=around + 2 {
        for z in around - 2..=around + 2 {
            game.server
                .streaming
                .insert_remote(&mut game.common.world, Chunk::empty(x, z));
        }
    }
}

/// A server with one guest standing on lit land at the origin.
fn server<'a>(
    resources: &'a mut Option<Arc<sunrise_gameplay_common::game::Resources>>,
) -> (sunrise_gameplay::game::ServerGame<'a>, mpsc::Receiver<Out>) {
    let mut game = sunrise_gameplay::game::game_tests::tests::server_game(resources);
    light(&mut game, 0);
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
            conn: CONN,
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
        CONN as i32,
        [0.5, 100.0, 0.5],
        2,
    );
    guest.dimension = "minecraft:overworld".into();
    guest.placed = true;
    guest.loaded = true;
    guest.load_timeout = 0;
    guest.health = 20.0;
    game.server.lan.as_mut().unwrap().hub.guests.push(guest);
    (game, rx)
}

fn landing(host: &LanHost) -> bool {
    host.hub.guests[0].landing
}

#[test]
#[ignore = "daily"]
fn a_hold_ignores_the_clients_moves_until_a_tp_to_lit_land_replaces_it() {
    let mut resources = None;
    let (mut game, _rx) = server(&mut resources);
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(0, |g| g.position = [FAR, 150.0, FAR]);
    game.hold_for_chunks(CONN, Vec3::new(FAR, 150.0, FAR));
    game.tick_server_travel();
    assert!(
        landing(game.server.lan.as_ref().unwrap()),
        "held while the far chunks are not lit"
    );
    // A second `tp`, to land that is lit, ends the wait: the moves are heard again.
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(0, |g| g.position = [0.5, 100.0, 0.5]);
    game.hold_for_chunks(CONN, Vec3::new(0.5, 100.0, 0.5));
    game.tick_server_travel();
    assert!(!game.on_server_trip(CONN));
    assert!(
        !landing(game.server.lan.as_ref().unwrap()),
        "no hold left, so no moves ignored"
    );
}

#[test]
#[ignore = "daily"]
fn a_hold_ended_by_a_trip_elsewhere_hears_the_client_again() {
    let mut resources = None;
    let (mut game, _rx) = server(&mut resources);
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(0, |g| g.position = [FAR, 150.0, FAR]);
    game.hold_for_chunks(CONN, Vec3::new(FAR, 150.0, FAR));
    game.tick_server_travel();
    assert!(landing(game.server.lan.as_ref().unwrap()));
    // What a death, a respawn or a `tp` to another level does to the trip waiting for the player.
    game.server.server_trips.retain(|t| !t.is(CONN));
    game.tick_server_travel();
    assert!(!landing(game.server.lan.as_ref().unwrap()));
}

#[test]
#[ignore = "daily"]
fn a_hold_places_the_player_once_the_chunks_are_lit() {
    let mut resources = None;
    let (mut game, _rx) = server(&mut resources);
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(0, |g| g.position = [FAR, 150.0, FAR]);
    game.hold_for_chunks(CONN, Vec3::new(FAR, 150.0, FAR));
    game.tick_server_travel();
    assert!(landing(game.server.lan.as_ref().unwrap()));
    light(&mut game, (FAR as i32) >> 4);
    game.tick_server_travel();
    let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
    assert!(!game.on_server_trip(CONN));
    assert!(!guest.landing);
    assert_eq!(guest.position, [FAR, 150.0, FAR]);
    // A `tp` inside a level sets no portal cooldown.
    assert_eq!(
        game.server
            .lan
            .as_mut()
            .unwrap()
            .play
            .entry(CONN)
            .or_default()
            .portal
            .cooldown,
        0
    );
}

#[test]
#[ignore = "daily"]
fn a_hold_follows_the_destination_a_mod_chose() {
    let mut resources = None;
    let (mut game, _rx) = server(&mut resources);
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(0, |g| g.position = [FAR, 150.0, FAR]);
    game.hold_for_chunks(CONN, Vec3::new(FAR, 150.0, FAR));
    // A mod redirected the move to another unlit place: the hub put the player there, and the wait is for it.
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(0, |g| g.position = [-FAR, 150.0, 0.5]);
    game.tick_server_travel();
    let trips: Vec<_> = game
        .server
        .server_trips
        .iter()
        .filter(|t| t.is(CONN))
        .collect();
    assert_eq!(trips.len(), 1);
    assert_eq!(trips[0].target, Vec3::new(-FAR, 150.0, 0.5));
    // A mod canceled it: the player stays on lit land, so there is nothing to wait for.
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(0, |g| g.position = [0.5, 100.0, 0.5]);
    game.tick_server_travel();
    game.tick_server_travel();
    assert!(!game.on_server_trip(CONN));
    assert!(!landing(game.server.lan.as_ref().unwrap()));
}

/// The hub reads the packets the guests queued, as the network tick does.
fn drain_packets(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    let mut host = game.server.lan.take().unwrap();
    host.hub.drain_packets(&mut game.server_ctx());
    game.server.lan = Some(host);
}

#[test]
#[ignore = "daily"]
fn a_command_arrival_holds_movement_until_lit_then_ticks_damage_again() {
    use sunrise_gameplay::game::server_side::server_travel::LevelTeleport;
    use sunrise_protocol::packets::play::{self, Serverbound as S};
    use sunrise_sim::lan::guest::InboxItem;

    let mut resources = None;
    let (mut game, _rx) = server(&mut resources);
    let at = [FAR, 150.0, FAR];
    let chunk = (FAR as i32) >> 4;
    // The client can already stand and move here, but the arrival's full 5x5 is not lit.
    for x in chunk - 1..=chunk + 1 {
        for z in chunk - 1..=chunk + 1 {
            game.server
                .streaming
                .insert_remote(&mut game.common.world, Chunk::empty(x, z));
        }
    }
    // Exercise the queued command arrival without opening a second saved level.
    game.server.level_teleports.push(LevelTeleport {
        conn: CONN,
        dim: game.dim,
        at,
        rotation: None,
        vehicle: None,
    });
    game.run_level_teleports();
    game.tick_server_travel();
    assert!(game.on_server_trip(CONN));
    let id = game.server.lan.as_ref().unwrap().hub.guests[0].teleport_counter;
    let step = play::MovePlayerPos {
        x: at[0] + 1.0,
        y: at[1],
        z: at[2],
        flags: 0,
    };
    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.inbox.push_back(InboxItem::Packet(
            S::AcceptTeleportation(play::AcceptTeleportation {
                id,
                x: at[0],
                y: at[1],
                z: at[2],
                yaw: 0.0,
                pitch: 0.0,
            }),
            None,
        ));
        g.inbox
            .push_back(InboxItem::Packet(S::MovePlayerPos(step.clone()), None));
    });
    drain_packets(&mut game);
    assert_eq!(
        game.server.lan.as_ref().unwrap().hub.guests[0].position,
        at,
        "a pending arrival cannot be left behind"
    );
    assert!(landing(game.server.lan.as_ref().unwrap()));

    let pos = [at[0] as i32, at[1] as i32 - 1, at[2] as i32];
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(pos[0], pos[1], pos[2], stone).unwrap();
    let host = game.server.lan.as_mut().unwrap();
    host.hub.ticks = 20_000;
    host.play
        .entry(CONN)
        .or_default()
        .arm_mining_for_test(pos, host.hub.ticks, Some((pos, 0)));
    game.tick_guests();
    assert!(game.on_server_trip(CONN));
    assert_eq!(
        game.as_guest(CONN, |g| g.mining.as_ref().map(|m| m.ticks)),
        Some(Some(1)),
        "the held tick did not advance active mining"
    );
    assert_eq!(
        game.world.block(pos[0], pos[1], pos[2]),
        stone,
        "the held tick did not complete the due destroy"
    );

    light(&mut game, chunk);
    game.tick_server_travel();
    assert!(!game.on_server_trip(CONN));
    assert!(!landing(game.server.lan.as_ref().unwrap()));
    // End the preceding client tick's position-packet budget before the next move.
    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.inbox.push_back(InboxItem::Packet(
            S::ClientTickEnd(sunrise_protocol::packets::Empty),
            None,
        ));
        g.inbox
            .push_back(InboxItem::Packet(S::MovePlayerPos(step), None));
    });
    drain_packets(&mut game);
    assert_eq!(
        game.server.lan.as_ref().unwrap().hub.guests[0].position,
        [at[0] + 1.0, at[1], at[2]]
    );

    let hit = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
        game.mob_hits_player(6.0, at, "minecraft:player_attack")
    };
    assert_eq!(game.as_guest(CONN, hit), Some(true));
    let health = game.server.lan.as_ref().unwrap().hub.guests[0].health;
    for _ in 0..20 {
        game.tick_guests();
    }
    assert_eq!(game.as_guest(CONN, hit), Some(true));
    assert!(
        game.server.lan.as_ref().unwrap().hub.guests[0].health < health,
        "the same melee hit hurts again after its cooldown"
    );
}

#[test]
#[ignore = "daily"]
fn respawn_arrivals_hold_movement_then_resume_damage_and_pickups() {
    use sunrise_items::items::ItemStack;
    use sunrise_protocol::packets::play::{self, Serverbound as S};
    use sunrise_sim::lan::guest::InboxItem;

    mod_platform::testing::with_mods(Vec::new().into(), || {
        for personal in [false, true] {
            let mut resources = None;
            let (mut game, _rx) = server(&mut resources);
            game.options.dedicated = Some(Vec::new());
            let pos = [FAR as i32, 150, FAR as i32];
            let chunk = pos[0] >> 4;
            game.level.world_spawn = Some(pos);
            assert!(game.ui.rules.set_string("respawn_radius", "0"));
            let elsewhere = personal.then(|| (game.dim.id().into(), pos, 93.0, 12.0, true));
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| {
                    guest.dead = true;
                    guest.health = 0.0;
                    if let Some(elsewhere) = elsewhere {
                        guest.respawn_elsewhere = Some(elsewhere);
                    }
                });
            // The client can render its immediate surroundings before all 5x5 arrival chunks are lit.
            for x in chunk - 1..=chunk + 1 {
                for z in chunk - 1..=chunk + 1 {
                    game.server
                        .streaming
                        .insert_remote(&mut game.common.world, Chunk::empty(x, z));
                }
            }
            game.guest_respawn(CONN);
            game.tick_server_travel();
            assert!(game.on_server_trip(CONN));
            assert!(
                landing(game.server.lan.as_ref().unwrap()),
                "respawn must hold movement while loading"
            );
            let at = game.server.lan.as_ref().unwrap().hub.guests[0].position;
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| {
                    // The old body is still present. There's no teleport to acknowledge yet.
                    assert!(guest.awaiting_teleport.is_none());
                    guest.inbox.push_back(InboxItem::Packet(
                        S::MovePlayerPos(play::MovePlayerPos {
                            x: at[0] + 1.0,
                            y: at[1],
                            z: at[2],
                            flags: 0,
                        }),
                        None,
                    ));
                });
            drain_packets(&mut game);
            let guest = game.command_guest(CONN).unwrap();
            assert!(guest.left_reason.is_empty());
            assert_eq!(guest.position, at, "cannot abandon the respawn destination");
            // A second pass must preserve the respawn search, even if its eventual stand-up spot
            // differs from the original target; it must not turn into an ordinary teleport hold.
            game.tick_server_travel();
            assert!(landing(game.server.lan.as_ref().unwrap()));
            light(&mut game, chunk);
            game.tick_server_travel();
            assert!(!game.on_server_trip(CONN));
            assert!(!landing(game.server.lan.as_ref().unwrap()));
            if personal {
                let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
                assert_eq!(guest.position, [FAR, 150.1, FAR]);
                assert_eq!([guest.yaw, guest.pitch], [93.0, 12.0]);
            }
            let host = game.server.lan.as_mut().unwrap();
            let at = host.hub.guests.update(0, |g| {
                assert!(
                    !g.client_loaded(),
                    "PlayerList respawn restarts the client load timer"
                );
                let [x, y, z] = g.position;
                let (id, _) = g.awaiting_teleport.expect("the replacement was placed");
                for packet in [
                    S::AcceptTeleportation(play::AcceptTeleportation {
                        id,
                        x,
                        y,
                        z,
                        yaw: g.yaw,
                        pitch: g.pitch,
                    }),
                    S::PlayerLoaded(sunrise_protocol::packets::Empty),
                ] {
                    g.inbox.push_back(InboxItem::Packet(packet, None));
                }
                g.position
            });
            game.tick_lan_packets();
            let guest = game.command_guest(CONN).unwrap();
            assert!(guest.awaiting_teleport.is_none());
            assert!(guest.client_loaded());
            let hit = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                game.mob_hits_player(6.0, at, "minecraft:player_attack")
            };
            assert_eq!(game.as_guest(CONN, hit), Some(true));
            let health = game.server.lan.as_ref().unwrap().hub.guests[0].health;
            game.add_item_entity(ItemStack::new("minecraft:stick", 1), at, Some([0.0; 3]), 0);
            for _ in 0..20 {
                game.tick_guests();
            }
            assert!(game.dropped.is_empty(), "pickups resume after arrival");
            assert!(
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .inventory
                    .iter()
                    .flatten()
                    .any(|s| &*s.item == "minecraft:stick" && s.count == 1)
            );
            assert_eq!(game.as_guest(CONN, hit), Some(true));
            assert!(
                game.server.lan.as_ref().unwrap().hub.guests[0].health < health,
                "the damage cooldown must expire"
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_world_spawn_search_keeps_movement_held_between_candidate_chunks() {
    use sunrise_sim::world_spawn::PlayerSpawnFinder;

    mod_platform::testing::with_mods(Vec::new().into(), || {
        let mut resources = None;
        let (mut game, _rx) = server(&mut resources);
        // The old body stays on the origin; the effective spawn sits one chunk away,
        // with only four of its rows lit when the player requests a respawn.
        let spawn = [16, 100, 16];
        game.level.world_spawn = Some(spawn);
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(CONN, |g| {
                g.dead = true;
                g.health = 0.0;
            });
        game.guest_respawn(CONN);
        // A deterministic search starts at (-48, -48), outside the newly lit square.
        // This is the search state an arrival pass keeps when a candidate is missing.
        game.server.server_trips[0].finder = Some(PlayerSpawnFinder::new(spawn, 64, |_| 0));
        light(&mut game, 1);
        for _ in 0..2 {
            game.tick_server_travel();
            assert!(game.on_server_trip(CONN));
            let host = game.server.lan.as_ref().unwrap();
            assert!(
                landing(host),
                "waiting for a candidate must not release movement"
            );
            assert!(
                host.hub.guests[0].dead,
                "the replacement must wait for the actual spawn"
            );
        }
        // No column has a floor, so after loading the search area it falls back to
        // the original suggestion. The pending search must complete and release movement.
        for x in -3..=5 {
            for z in -3..=5 {
                game.server
                    .streaming
                    .insert_remote(&mut game.common.world, Chunk::empty(x, z));
            }
        }
        game.tick_server_travel();
        assert!(!game.on_server_trip(CONN));
        let host = game.server.lan.as_ref().unwrap();
        assert!(!landing(host));
        assert!(!host.hub.guests[0].dead);
    });
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
