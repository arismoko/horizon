//! Differential witness for tools/test_server_player_packets_parity.py. Packet bodies enter
//! the server's ordinary inbox and run through the real Hub and Game dispatch.
use serde_json::{Map, Value, json};
use std::sync::Arc;
use std::{
    collections::HashSet,
    sync::{atomic::AtomicI32, mpsc},
};
use sunrise_core::{
    geometry::Vec3,
    nbt::{self, Tag},
};
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_gameplay_common::game::lan_common::LanSettings;
use sunrise_level::chunk::Chunk;
#[cfg(test)]
use sunrise_protocol::packets::play;
use sunrise_protocol::packets::play::Clientbound as C;
use sunrise_protocol::{
    buf::Reader,
    packets::{configuration::ClientInformation, play::Serverbound as S},
};
use sunrise_sim::lan::NetConfig;
use sunrise_sim::lan::hub::HubConfig;
use sunrise_sim::lan::{
    Joined, Out, PlayerSender,
    guest::{Guest, InboxItem},
};
use {
    sunrise_gameplay::game::native_entity_refs::tests, sunrise_gameplay_common::game::Dim,
    sunrise_gameplay_common::game::execution_role::ExecutionRole,
};

const VIEWER: u32 = 1;
const START: [f64; 3] = [0.5, 100.0, 0.5];
const REPORT: &str = "SUNRISE_PLAYER_PACKET_REPORT";
fn witness(rows: &mut Map<String, Value>, name: &str, actual: Value, expected: Value) {
    assert_eq!(actual, expected, "{name}");
    rows.insert(name.into(), actual);
}

fn join(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    conn: u32,
    loaded: bool,
) -> mpsc::Receiver<Out> {
    let (tx, rx) = mpsc::channel();
    let id = 1000 + conn as i32;
    game.server.next_entity = game.server.next_entity.max(id as u32 + 1);
    let mut guest = Guest::new(
        Joined {
            conn,
            uuid: u128::from(conn),
            name: format!("PacketPlayer{conn}"),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "fixture".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:1".parse().unwrap(),
            expected: false,
        },
        id,
        START,
        2,
    );
    guest.dimension = game.dim.id().to_owned();
    guest.placed = true;
    guest.loaded = loaded;
    guest.load_timeout = if loaded { 0 } else { 60 };
    (guest.yaw, guest.pitch) = (17.0, -12.0);
    game.server.lan.as_mut().unwrap().hub.guests.push(guest);
    game.reset_entity_residency();
    rx
}

fn packet_tick(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    conn: u32,
    packets: impl IntoIterator<Item = S>,
) {
    let back = game.dim;
    let level = Dim::from_id(&guest(game, conn).dimension);
    game.enter_level(level).unwrap();
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .update_guest(conn, |guest| {
            guest.inbox.extend(packets.into_iter().map(|packet| {
                let body = packet.encode();
                let mut reader = Reader::new(&body);
                let id = reader.varint().unwrap();
                InboxItem::Packet(S::decode(id, reader.rest()).unwrap(), None)
            }));
        });
    game.server.level_pass = Default::default();
    game.tick_lan_packets();
    assert_eq!(
        game.dim, level,
        "packet pass did not restore its source level"
    );
    guest(game, conn).sender.flush();
    game.enter_level(back).unwrap();
}

fn guest<'a>(game: &'a sunrise_gameplay::game::ServerGame<'_>, conn: u32) -> &'a Guest {
    game.server.lan.as_ref().unwrap().hub.guest(conn).unwrap()
}

fn pose(guest: &Guest) -> Value {
    let [x, y, z] = guest.position.map(f64::to_bits);
    json!([
        x,
        y,
        z,
        guest.yaw.to_bits() as i32,
        guest.pitch.to_bits() as i32
    ])
}

fn receive(rx: &mpsc::Receiver<Out>) -> Vec<C> {
    rx.try_iter()
        .flat_map(|packet| match packet {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => Vec::new(),
        })
        .map(|body| {
            let mut reader = Reader::new(&body);
            let id = reader.varint().unwrap();
            C::decode(id, reader.rest()).unwrap()
        })
        .collect()
}

fn credits(game: &mut sunrise_gameplay::game::ServerGame<'_>, rows: &mut Map<String, Value>) {
    game.enter_level(Dim::End).unwrap();
    game.server
        .streaming
        .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
    assert!(
        game.set_state_named([0, 100, 0], "minecraft:end_portal", &[])
            .unwrap()
    );
    let rx = join(game, 7, true);
    game.handle_server_portal(7);
    assert!(
        receive(&rx)
            .iter()
            .any(|p| matches!(p, C::GameEvent(event) if event.event == 4))
    );
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .update_guest(7, |g| g.last_action_tick = 0);
    packet_tick(
        game,
        7,
        [S::MovePlayerPosRot(play::MovePlayerPosRot {
            x: 0.75,
            y: 100.0,
            z: 0.5,
            yaw: 60.0,
            pitch: 30.0,
            flags: 1,
        })],
    );
    rows.insert("141/credits_pose".into(), pose(guest(game, 7)));
    witness(
        rows,
        "141/credits_idle_credit",
        json!(guest(game, 7).last_action_tick != 0),
        json!(false),
    );
    rows.insert(
        "141/credits_move_count".into(),
        json!(guest(game, 7).received_moves),
    );
    packet_tick(
        game,
        7,
        [S::MovePlayerPos(play::MovePlayerPos {
            x: 0.75,
            y: 100.0,
            z: 0.5,
            flags: 1,
        })],
    );
    rows.insert(
        "141/repeated_position".into(),
        json!(rx.try_iter().any(|p| matches!(p, Out::Disconnect(_)))),
    );

    let rx = join(game, 8, true);
    game.handle_server_portal(8);
    packet_tick(
        game,
        8,
        [S::MovePlayerRot(play::MovePlayerRot {
            yaw: f32::NAN,
            pitch: 0.0,
            flags: 1,
        })],
    );
    rows.insert(
        "141/invalid_rotation".into(),
        json!(rx.try_iter().any(|p| matches!(p, Out::Disconnect(_)))),
    );
    game.enter_level(Dim::Overworld).unwrap();
    let _rx = join(game, 9, true);
    packet_tick(
        game,
        9,
        [S::MovePlayerPos(play::MovePlayerPos {
            x: 0.75,
            y: 100.0,
            z: 0.5,
            flags: 0,
        })],
    );
    witness(
        rows,
        "141/ordinary_idle_credit",
        json!(guest(game, 9).last_action_tick != 0),
        json!(true),
    );
}

fn use_item(game: &mut sunrise_gameplay::game::ServerGame<'_>, rows: &mut Map<String, Value>) {
    for kind in ["empty", "disabled", "survival", "spectator"] {
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(VIEWER, |g| {
                (g.yaw, g.pitch) = (17.0, -12.0);
                g.last_action_tick = 0;
                g.ack_up_to.set(-1);
                g.game_mode = if kind == "spectator" { 3 } else { 0 };
                g.offhand = (kind != "empty")
                    .then(|| sunrise_items::items::ItemStack::new("minecraft:snowball", 3));
            });
        let saved = game.level.enabled_features.clone();
        if kind == "disabled" {
            game.level.enabled_features = Vec::<String>::new().into();
        }
        packet_tick(
            game,
            VIEWER,
            [S::UseItem(play::UseItem {
                hand: 1,
                sequence: 11,
                yaw: 725.0,
                pitch: -485.0,
            })],
        );
        let g = guest(game, VIEWER);
        rows.insert(
            format!("142/{kind}"),
            json!([
                g.yaw.to_bits() as i32,
                g.pitch.to_bits() as i32,
                g.offhand.as_ref().map_or(0, |s| s.count),
                g.ack_up_to.get(),
                i32::from(g.last_action_tick != 0)
            ]),
        );
        game.level.enabled_features = saved;
    }
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .update_guest(VIEWER, |g| g.game_mode = 0);
}

fn input(shift: bool) -> S {
    S::PlayerInput(play::PlayerInput {
        flags: if shift { 32 } else { 0 },
    })
}

fn interact(game: &mut sunrise_gameplay::game::ServerGame<'_>, rows: &mut Map<String, Value>) {
    let interact = |id, secondary| {
        S::Interact(play::Interact {
            id,
            hand: 0,
            location: [0.0; 3],
            secondary,
        })
    };
    for shift in [true, false] {
        packet_tick(game, VIEWER, [input(!shift), interact(-1, shift)]);
        rows.insert(
            format!("143/missing_{shift}"),
            json!(guest(game, VIEWER).sneaking),
        );
    }
    for (conn, shift) in [(3, true), (4, false)] {
        let _out = join(game, conn, true);
        assert!(game.as_console(|g| g.run_command("summon oak_boat 1.5 100 0.5")));
        let boat = game.boats.last().unwrap().id;
        packet_tick(
            game,
            conn,
            [input(!shift), interact(boat as i32, shift), input(!shift)],
        );
        rows.insert(
            format!("143/boat_{shift}"),
            json!(guest(game, conn).vehicle == Some(boat)),
        );
        if !shift {
            rows.insert("143/next_input".into(), json!(guest(game, conn).sneaking));
        }
    }
}

fn spectate(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    rx: &mpsc::Receiver<Out>,
    rows: &mut Map<String, Value>,
) {
    let shoot = |uuid| S::TeleportToEntity(play::TeleportToEntity { uuid });
    let _peer = join(game, 2, true);
    let host = game.server.lan.as_mut().unwrap();
    host.hub.update_guest(VIEWER, |g| g.game_mode = 3);
    host.hub.update_guest(2, |g| {
        g.place([10.5, 104.0, 7.5]);
        (g.yaw, g.pitch) = (-35.0, 20.0);
    });
    packet_tick(game, VIEWER, [shoot(2)]);
    rows.insert("144/player".into(), pose(guest(game, VIEWER)));
    receive(rx);
    packet_tick(game, VIEWER, [shoot((1u128 << 64) | 1)]);
    rows.insert(
        "144/missing".into(),
        json!(
            receive(rx)
                .iter()
                .filter(|p| matches!(p, C::PlayerPosition(_)))
                .count()
        ),
    );
    assert!(game.as_console(|g| g.run_command("summon oak_boat 2.5 100 3.5")));
    let boat = game.boats.last().unwrap().id;
    for active in [false, true] {
        let i = game.guest_index(VIEWER).unwrap();
        if active {
            game.guest_set_camera(i, Some(boat as i32));
            assert!(guest(game, VIEWER).watching);
        }
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(VIEWER, |g| {
                g.place(START);
                g.vehicle = Some(boat);
            });
        game.as_guest(VIEWER, |g| {
            let body = g.player.as_mut().unwrap();
            body.fall_distance = 7.25;
            body.velocity = Vec3::new(0.125, -0.25, 0.5);
        });
        guest(game, VIEWER).sender.flush();
        receive(rx);
        packet_tick(game, VIEWER, [shoot(2)]);
        let viewer = guest(game, VIEWER);
        let body = viewer.motion.as_ref().unwrap();
        let packets: Vec<_> = receive(rx)
            .iter()
            .filter_map(|p| match p {
                C::SetPassengers(p) if p.vehicle == boat as i32 && p.passengers.is_empty() => {
                    Some("dismount")
                }
                C::PlayerPosition(_) => Some("position"),
                C::SetCamera(p) if p.camera_id == viewer.entity_id => Some("camera_self"),
                _ => None,
            })
            .collect();
        let expected_packets = if active {
            vec!["dismount", "position", "camera_self", "position"]
        } else {
            vec!["dismount", "position"]
        };
        witness(
            rows,
            &format!("144/same_level_{active}"),
            json!([
                packets,
                viewer.vehicle.is_none(),
                !viewer.watching && !game.server.cmds.0.guest_cameras.contains_key(&VIEWER),
                body.fall_distance.to_bits() as i64,
                vector_bits(body.velocity),
                viewer.first_good == viewer.position,
            ]),
            json!([
                expected_packets,
                true,
                true,
                7.25_f64.to_bits() as i64,
                [0, 0, 0],
                true
            ]),
        );
    }
    for (word, dim, name, x) in [
        (90u128, Dim::Overworld, "overworld", 6.25),
        (10, Dim::Nether, "the_nether", 8.25),
    ] {
        let from = game.dim;
        game.enter_level(dim).unwrap();
        assert!(game.as_console(|g| g.run_command(&format!(
            "summon armor_stand {x} 101 2.75 {{UUID:[I;0,0,0,{word}],Rotation:[63f,0f]}}"
        ))));
        game.enter_level(from).unwrap();
        packet_tick(game, VIEWER, [shoot(word)]);
        let viewer = guest(game, VIEWER);
        rows.insert(format!("144/stand_{name}"), pose(viewer));
        rows.insert(
            format!("144/level_{name}"),
            json!(viewer.dimension == dim.id()),
        );
    }
    for (word, dim, name, command) in [
        (
            11,
            Dim::Nether,
            "arrow",
            "summon arrow 5.125 101.5 3.25 {UUID:[I;0,0,0,11],Rotation:[-123.25f,-32.5f]}",
        ),
        (
            12,
            Dim::Overworld,
            "upward_frame",
            "summon item_frame 7.5 100.5 3.5 {UUID:[I;0,0,0,12],block_pos:[I;7,100,3],Facing:1b,Fixed:1b}",
        ),
    ] {
        let from = game.dim;
        game.enter_level(dim).unwrap();
        assert!(game.as_console(|g| g.run_command(command)));
        game.enter_level(from).unwrap();
        packet_tick(game, VIEWER, [shoot(word)]);
        rows.insert(format!("144/{name}"), pose(guest(game, VIEWER)));
    }

    receive(rx);
    type HeardTransition = (String, String, bool, String, [f64; 3]);
    struct DimRecorder(std::sync::Arc<std::sync::Mutex<Vec<HeardTransition>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for DimRecorder {
        fn player_changed_dimension(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::PlayerChangedDimensionEvent<'_>,
        ) {
            let player = host.players().acting_player().unwrap();
            let entity = player.entity();
            let level = entity.level(host).unwrap();
            let in_place = level.in_place(host);
            let dim = level.dimension().id(host).unwrap().to_owned();
            let pos = entity.position(host).unwrap();
            self.0.lock().unwrap().push((
                event.from.to_owned(),
                event.to.to_owned(),
                in_place,
                dim,
                pos,
            ));
        }
    }
    // A return is a second level change; a target in the destination is not.
    for (name, target, levels, at, angles) in [
        (
            "mixed_return",
            90,
            vec![Dim::Nether, Dim::Overworld],
            [6.25_f64, 101.0, 2.75],
            [63.0_f32, 0.0],
        ),
        (
            "mixed_stay",
            11,
            vec![Dim::Nether],
            [5.125, 101.5, 3.25],
            [-123.25, -32.5],
        ),
    ] {
        let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "dim-recorder",
            Box::new(DimRecorder(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            packet_tick(game, VIEWER, [shoot(10), shoot(target)]);
        });
        if name == "mixed_return" {
            assert_eq!(
                *heard.lock().unwrap(),
                vec![
                    (
                        "minecraft:overworld".to_owned(),
                        "minecraft:the_nether".to_owned(),
                        true,
                        "minecraft:the_nether".to_owned(),
                        [8.25, 101.0, 2.75],
                    ),
                    (
                        "minecraft:the_nether".to_owned(),
                        "minecraft:overworld".to_owned(),
                        true,
                        "minecraft:overworld".to_owned(),
                        [6.25, 101.0, 2.75],
                    ),
                ]
            );
        } else {
            assert_eq!(
                *heard.lock().unwrap(),
                vec![(
                    "minecraft:overworld".to_owned(),
                    "minecraft:the_nether".to_owned(),
                    true,
                    "minecraft:the_nether".to_owned(),
                    [8.25, 101.0, 2.75],
                )]
            );
        }
        let packets = receive(rx);
        let transitions: Vec<_> = packets
            .iter()
            .filter_map(|p| match p {
                C::Respawn(p) => Some(p.spawn.dimension.as_str()),
                _ => None,
            })
            .collect();
        let corrections = packets
            .iter()
            .filter(|p| matches!(p, C::PlayerPosition(_)))
            .count();
        let expected_levels: Vec<_> = levels.iter().map(|level| level.id()).collect();
        witness(
            rows,
            &format!("144/{name}"),
            json!([
                transitions,
                corrections,
                guest(game, VIEWER).dimension,
                pose(guest(game, VIEWER))
            ]),
            json!([
                expected_levels,
                2,
                expected_levels.last().unwrap(),
                [
                    at[0].to_bits() as i64,
                    at[1].to_bits() as i64,
                    at[2].to_bits() as i64,
                    i64::from(angles[0].to_bits() as i32),
                    i64::from(angles[1].to_bits() as i32)
                ],
            ]),
        );
    }
}

fn vector_bits(v: Vec3) -> [i64; 3] {
    [v.x, v.y, v.z].map(|x| x.to_bits() as i64)
}

fn vehicle_packet() -> S {
    S::MoveVehicle(play::MoveVehicleRequest {
        position: [2.0, 101.0, 3.0],
        yaw: 72.0,
        pitch: 35.0,
        on_ground: true,
    })
}

fn correction_sample(packets: &[C], guest: &Guest) -> Value {
    let corrections: Vec<_> = packets
        .iter()
        .filter_map(|p| match p {
            C::PlayerPosition(p) => Some(json!([
                p.teleport,
                p.yaw.to_bits() as i32,
                p.pitch.to_bits() as i32,
                p.delta.map(|x| x.to_bits() as i64),
            ])),
            _ => None,
        })
        .collect();
    json!([
        corrections,
        guest.yaw.to_bits() as i32,
        guest.pitch.to_bits() as i32,
        guest.awaiting_teleport.map(|(id, _)| id),
    ])
}

fn resend(game: &mut sunrise_gameplay::game::ServerGame<'_>, rows: &mut Map<String, Value>) {
    for (n, route) in ["vehicle", "player", "loading_vehicle", "loading_player"]
        .into_iter()
        .enumerate()
    {
        let conn = 20 + n as u32;
        let rx = join(game, conn, !route.starts_with("loading_"));
        let i = game.guest_index(conn).unwrap();
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .teleport(i, Some(START));
        for elapsed in 1..=22 {
            let mut packet = None;
            if elapsed >= 19 {
                packet = Some(if route.ends_with("vehicle") {
                    vehicle_packet()
                } else {
                    S::MovePlayerRot(play::MovePlayerRot {
                        yaw: 72.0,
                        pitch: 35.0,
                        flags: 1,
                    })
                });
            }
            receive(&rx);
            packet_tick(game, conn, packet);
            let packets = receive(&rx);
            if elapsed >= 19 {
                let angles = |yaw: f32, pitch: f32| (yaw.to_bits() as i32, pitch.to_bits() as i32);
                let turns = route == "player";
                let (yaw, pitch) = if turns {
                    angles(72.0, 35.0)
                } else {
                    angles(17.0, -12.0)
                };
                let resends = route != "loading_player" && elapsed >= 21;
                let sent = if resends && elapsed == 21 {
                    json!([[2, yaw, pitch, [0, 0, 0]]])
                } else {
                    json!([])
                };
                witness(
                    rows,
                    &format!("145/{route}_{elapsed}"),
                    correction_sample(&packets, guest(game, conn)),
                    json!([sent, yaw, pitch, if resends { 2 } else { 1 }]),
                );
            }
        }
    }
    for (conn, kind) in [
        (24, "vehicle_then_accept"),
        (25, "status"),
        (26, "position"),
    ] {
        let rx = join(game, conn, true);
        let yaw: f32 = if conn == 24 { 17.0 } else { 270.0 };
        let pitch = -12.0_f32;
        let i = game.guest_index(conn).unwrap();
        let hub = &mut game.server.lan.as_mut().unwrap().hub;
        hub.guests.update(i, |g| g.yaw = yaw);
        hub.teleport(i, Some(START));
        for _ in 0..20 {
            packet_tick(game, conn, []);
        }
        receive(&rx);
        let packets = match kind {
            "vehicle_then_accept" => vec![
                vehicle_packet(),
                S::AcceptTeleportation(play::AcceptTeleportation {
                    id: 1,
                    x: START[0],
                    y: START[1],
                    z: START[2],
                    yaw,
                    pitch,
                }),
            ],
            "status" => vec![S::MovePlayerStatus(play::MovePlayerStatus { flags: 1 })],
            _ => vec![S::MovePlayerPos(play::MovePlayerPos {
                x: START[0],
                y: START[1],
                z: START[2],
                flags: 1,
            })],
        };
        packet_tick(game, conn, packets);
        let now = if conn == 24 { yaw } else { -90.0 };
        witness(
            rows,
            &format!("145/{kind}"),
            correction_sample(&receive(&rx), guest(game, conn)),
            json!([
                [[2, yaw.to_bits() as i32, pitch.to_bits() as i32, [0, 0, 0]]],
                now.to_bits() as i32,
                pitch.to_bits() as i32,
                2
            ]),
        );
    }
}

fn crystal_round_trip(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    rows: &mut Map<String, Value>,
) {
    let key = (1, 0);
    game.server
        .streaming
        .insert_remote(&mut game.common.world, Chunk::empty(key.0, key.1));
    let tag = nbt::parse_snbt(
        r#"{id:"minecraft:end_crystal",UUID:[I;0,0,0,130],Pos:[18.5d,100d,0.5d],Rotation:[-132.25f,31.5f],Invulnerable:1b,beam_target:[I;0,128,0],Fire:120s,data:{marker:42}}"#,
    ).unwrap().0;
    game.spawn_loaded_entity(key, tag, None);
    let crystal = game.crystals.last_mut().unwrap();
    let id = crystal.id;
    let rotation = [-132.25_f32, 31.5].map(|angle| angle.to_bits() as i32);
    witness(
        rows,
        "146/loaded_crystal",
        json!([
            game.target_rotation(sunrise_gameplay::game::commands::Target::Crystal(id))
                .map(|angle| angle.to_bits() as i32),
            game.crystals.last().unwrap().invulnerable,
            game.crystals.last().unwrap().beam,
            game.crystals.last().unwrap().fire,
        ]),
        json!([rotation, true, [0, 128, 0], 120]),
    );
    let moved_pos = Vec3::new(20.5, 102.0, 2.5);
    let crystal = game.crystals.last_mut().unwrap();
    crystal.position = moved_pos;
    crystal.invulnerable = false;
    crystal.beam = None;
    game.clear_crystal_fire();
    let (_, tags) = game
        .entity_chunks(&HashSet::from([key]), true)
        .pop()
        .unwrap();
    assert!(game.crystals.iter().all(|c| c.id != id));
    let tag = tags
        .into_iter()
        .find(|tag| sunrise_sim::entity_nbt::kind(tag).as_deref() == Some("minecraft:end_crystal"))
        .unwrap();
    let cleared = tag.get("beam_target").is_none()
        && matches!(tag.get("Fire"), Some(Tag::Short(0)))
        && matches!(tag.get("Invulnerable"), Some(Tag::Byte(0)));
    assert_eq!(
        tag.get("data").and_then(|d| d.get("marker")),
        Some(&Tag::Int(42)),
        "custom entity data must survive crystal move and save"
    );
    let binary = nbt::write_network(&tag, nbt::Limits::default()).unwrap();
    let (saved, _) = nbt::read_network(&binary, nbt::Limits::default()).unwrap();
    game.spawn_loaded_entity(key, saved, None);
    let c = game.crystals.last().unwrap();
    assert_eq!(c.position, moved_pos);
    assert_eq!(
        game.crystal_nbt(c)
            .get("data")
            .and_then(|d| d.get("marker")),
        Some(&Tag::Int(42)),
        "custom entity data must survive crystal reload"
    );
    witness(
        rows,
        "146/reloaded_crystal",
        json!([
            game.target_rotation(sunrise_gameplay::game::commands::Target::Crystal(c.id))
                .map(|angle| angle.to_bits() as i32),
            c.invulnerable,
            c.beam,
            c.fire,
            cleared,
        ]),
        json!([rotation, false, null, 0, true]),
    );
}

#[test]
#[ignore = "daily"]
fn server_player_packet_witness() {
    run_server_player_packet_witness();
}

pub(crate) fn run_server_player_packet_witness() {
    // Resource loading installs process-wide registries. Use the standard child
    // boundary, with a larger stack on the thread that loads the prepared game.
    let (_, module) = module_path!().split_once("::").unwrap();
    sunrise_gameplay_common::game::in_own_process(
        &format!("{module}::server_player_packet_witness"),
        || {
            std::thread::Builder::new()
                .stack_size(16 * 1024 * 1024)
                .spawn(run_witness)
                .unwrap()
                .join()
                .unwrap();
        },
    );
}

fn run_witness() {
    tests::with_game_role(
        tests::fixture_root(),
        ExecutionRole::DedicatedServer,
        |game| {
            game.options.dedicated = Some(Vec::new());
            game.build_registries();
            let levels = game.server.level_shapes.clone();
            let level_seeds = game.server.levels.0.runtime.seeds();
            game.lan_host(
                LanSettings {
                    port: 0,
                    guest_commands: true,
                    force_game_mode: false,
                },
                NetConfig {
                    listen: false,
                    announce: false,
                    lod_distance: 0,
                    ..NetConfig::lan(0, String::new(), 16)
                },
                HubConfig {
                    default_game_mode: 0,
                    guest_commands: true,
                    force_game_mode: false,
                    hardcore: false,
                    view_distance: 2,
                    simulation_distance: 2,
                    dimension: Dim::Overworld.id().to_owned(),
                    allow_flight: true,
                    seed: 0,
                    max_players: 16,
                    integrated: false,
                    levels,
                    level_seeds,
                },
            )
            .unwrap();
            game.server
                .streaming
                .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
            let rx = join(game, VIEWER, true);
            let mut rows = Map::new();
            credits(game, &mut rows);
            use_item(game, &mut rows);
            interact(game, &mut rows);
            spectate(game, &rx, &mut rows);
            resend(game, &mut rows);
            crystal_round_trip(game, &mut rows);
            assert_eq!(rows["141/repeated_position"], true);
            assert_eq!(rows["141/invalid_rotation"], true);
            assert_eq!(rows["144/missing"], 0);
            let report = serde_json::to_vec_pretty(&rows).unwrap();
            if let Some(path) = std::env::var_os(REPORT) {
                std::fs::write(path, report).unwrap();
            }
        },
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
