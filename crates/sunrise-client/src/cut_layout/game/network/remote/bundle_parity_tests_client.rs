use crate::game::client_side::remote_client::ingress_state::bundle_parity_tests::bundle_witness;
use crate::game::client_side::remote_client::ingress_state::bundle_parity_tests::hex_bytes;
use crate::game::client_side::remote_client::ingress_state::bundle_parity_tests::wait_pong_in_game;
use crate::game::client_side::remote_client::ingress_state::bundle_parity_tests::witness_json;
use std::time::{Duration, Instant};
use sunrise_client::{level_loading::Reason, multiplayer, net::Session, title::Mode};
use sunrise_core::{geometry::Vec3, nbt::Tag};
use sunrise_protocol::{
    connection::Pipe,
    entity_data::{PaintingHolder, PaintingVariant, Value, serializer},
    packets::{
        Empty, configuration as cfg,
        play::{self, Clientbound as C, Serverbound as S, SpawnInfo},
    },
};
use sunrise_sim::motion::PlayerMotion;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry,
};

pub(crate) fn dependent_entity_bundle_body() {
    let witness = bundle_witness();
    let case = &witness["dependent_entity_bundle"];
    let entity_type = case["entity_type"].as_str().unwrap();
    let expected_exists_before = case["entity_exists_before_close"].as_bool().unwrap();
    let expected_cam_before = case["camera_entity_id_before_close"].as_i64().unwrap() as i32;
    let expected_exists_after = case["entity_exists_after_close"].as_bool().unwrap();
    let expected_width_bits =
        u32::from_str_radix(case["entity_width_bits"].as_str().unwrap(), 16).unwrap();
    let expected_height_bits =
        u32::from_str_radix(case["entity_height_bits"].as_str().unwrap(), 16).unwrap();
    let expected_cam_after = case["camera_entity_id_after_close"].as_i64().unwrap() as i32;

    with_client_game(|game| {
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "DependentBundle").unwrap();
        let spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 3,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        let (session, mut peer) = Session::play_loop_probe();
        let remote = game.client.remote.as_mut().unwrap();
        remote.session = session;
        remote.joined = true;
        remote.logged_in = true;
        remote.entity_id = expected_cam_before;
        let entity_types = remote.builtin.get("minecraft:entity_type").unwrap();
        let expected_eye_height_bits =
            u32::from_str_radix(case["entity_eye_height_bits"].as_str().unwrap(), 16).unwrap();
        let interaction_kind = entity_types.id(entity_type).unwrap();
        let creeper_kind = entity_types.id("minecraft:creeper").unwrap();

        // First half of bundle: AddEntity without closing delimiter.
        peer.send(&C::BundleDelimiter(Empty).encode()).unwrap();
        peer.send(
            &C::AddEntity(play::AddEntity {
                id: expected_cam_after,
                uuid: expected_cam_after as u128,
                kind: interaction_kind,
                position: [0.5, 300.0, 0.5],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            })
            .encode(),
        )
        .unwrap();
        let hold_until = Instant::now() + Duration::from_millis(80);
        while Instant::now() < hold_until {
            game.poll_remote().unwrap();
            assert_eq!(
                game.client.interactions.contains_key(&expected_cam_after),
                expected_exists_before,
                "unclosed bundle spawned entity before closing delimiter"
            );
            assert_eq!(
                game.remote_camera_id()
                    .map(|id| id as i32)
                    .unwrap_or(expected_cam_before),
                expected_cam_before
            );
            std::thread::yield_now();
        }

        // Second half: SetEntityData + SetCamera + closing delimiter apply atomically on the same Interaction.
        peer.send(
            &C::SetEntityData(play::SetEntityData {
                id: expected_cam_after,
                values: vec![
                    (8, serializer::FLOAT, Value::Float(0.1)),
                    (9, serializer::FLOAT, Value::Float(1.9059)),
                ],
            })
            .encode(),
        )
        .unwrap();
        peer.send(
            &C::SetCamera(play::SetCamera {
                camera_id: expected_cam_after,
            })
            .encode(),
        )
        .unwrap();
        peer.send(&C::BundleDelimiter(Empty).encode()).unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut applied = false;
        while Instant::now() < deadline {
            game.poll_remote().unwrap();
            let exists = game.client.interactions.contains_key(&expected_cam_after);
            let cam = game.remote_camera_id().map(|id| id as i32);
            if exists || cam.is_some() {
                assert!(exists && expected_exists_after);
                let interaction = game.client.interactions.get(&expected_cam_after).unwrap();
                assert_eq!(interaction.width.to_bits(), expected_width_bits);
                assert_eq!(interaction.height.to_bits(), expected_height_bits);
                assert_eq!(cam, Some(expected_cam_after));
                let view = game
                    .spectated_camera_snapshot()
                    .expect("Interaction camera view");
                assert_eq!(view.eye_height.to_bits(), expected_eye_height_bits);
                applied = true;
                break;
            }
            std::thread::yield_now();
        }
        assert!(
            applied,
            "closed interaction AddEntity+SetEntityData+SetCamera bundle never applied"
        );

        // Switching camera from a creeper (with creeper shader) to the resident Interaction clears the shader.
        let creeper_id = expected_cam_after + 1;
        peer.send(
            &C::AddEntity(play::AddEntity {
                id: creeper_id,
                uuid: creeper_id as u128,
                kind: creeper_kind,
                position: [2.5, 300.0, 2.5],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            })
            .encode(),
        )
        .unwrap();
        peer.send(
            &C::SetCamera(play::SetCamera {
                camera_id: creeper_id,
            })
            .encode(),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            game.poll_remote().unwrap();
            if game.remote_camera_id() == Some(creeper_id as u32) {
                break;
            }
            std::thread::yield_now();
        }
        assert_eq!(game.remote_camera_id(), Some(creeper_id as u32));
        assert_eq!(game.client.spectated_post_effect, Some("minecraft:creeper"));
        assert!(game.client.spectated_post_effect_active);

        peer.send(
            &C::SetCamera(play::SetCamera {
                camera_id: expected_cam_after,
            })
            .encode(),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            game.poll_remote().unwrap();
            if game.remote_camera_id() == Some(expected_cam_after as u32) {
                break;
            }
            std::thread::yield_now();
        }
        assert_eq!(game.remote_camera_id(), Some(expected_cam_after as u32));
        assert_eq!(game.client.spectated_post_effect, None);
        assert!(!game.client.spectated_post_effect_active);
    });
}

pub(crate) fn entity_data_validation_inline_painting_and_pre_login_bundle_case(case_name: &str) {
    let bundle = bundle_witness();
    let entity_data = witness_json("entity_data.json");
    let assignment = &entity_data["metadata_assignment"];

    match case_name {
        "before_login_metadata" => {
            // 1. SetEntityData before Login disconnects with disconnect.packetError.
            with_client_game(|game| {
                game.client.title.0 = None;
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "MetadataBeforeLogin").unwrap();
                let (session, mut peer) = Session::play_loop_probe();
                let remote = game.client.remote.as_mut().unwrap();
                remote.session = session;
                remote.joined = true;
                remote.logged_in = false;
                peer.reader
                    .set_read_timeout(Some(Duration::from_millis(20)))
                    .unwrap();
                peer.send(
                    &C::SetEntityData(play::SetEntityData {
                        id: 1,
                        values: vec![(0, serializer::BYTE, Value::Byte(0x40))],
                    })
                    .encode(),
                )
                .unwrap();
                let deadline = Instant::now() + Duration::from_secs(5);
                while game.client.remote.is_some() && Instant::now() < deadline {
                    game.poll_remote().unwrap();
                    std::thread::yield_now();
                }
                assert!(
                    game.client.remote.is_none(),
                    "SetEntityData before Login must disconnect"
                );
                let Some(Mode::Multiplayer(multiplayer::Screen::Disconnected { reason, .. })) =
                    game.client.title.0.as_ref().map(|t| &t.mode)
                else {
                    panic!("missing Disconnected screen for SetEntityData before Login");
                };
                assert_eq!(
                    reason.translation_key(),
                    assignment["before_login_disconnect_key"].as_str()
                );
            });
        }
        "metadata_validation_and_inline_painting" => {
            // 2. Absent entity, repeated valid flags, valid prefix before invalid entry, bundle failure, and inline painting.
            with_client_game(|game| {
                game.client.title.0 = None;
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "MetadataValidation").unwrap();
                let spawn = SpawnInfo {
                    dimension_type: 0,
                    dimension: "minecraft:overworld".into(),
                    seed: 0,
                    game_type: 0,
                    previous_game_type: None,
                    debug: false,
                    flat: true,
                    last_death: None,
                    portal_cooldown: 0,
                    sea_level: 63,
                };
                game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
                let (session, mut peer) = Session::play_loop_probe();
                let remote = game.client.remote.as_mut().unwrap();
                remote.session = session;
                remote.joined = true;
                remote.logged_in = true;
                remote.entity_id = 1000;
                let entity_types = remote.builtin.get("minecraft:entity_type").unwrap();
                let interaction_kind = entity_types.id("minecraft:interaction").unwrap();
                let painting_kind = entity_types.id("minecraft:painting").unwrap();
                peer.reader
                    .set_read_timeout(Some(Duration::from_millis(20)))
                    .unwrap();

                // Absent entity is ignored and never enters the glowing set.
                assert_eq!(
                    assignment["absent_entity_disconnect_key"].as_str(),
                    Some("none")
                );
                peer.send(
                    &C::SetEntityData(play::SetEntityData {
                        id: 77_777,
                        values: vec![(0, serializer::BYTE, Value::Byte(0x40))],
                    })
                    .encode(),
                )
                .unwrap();
                wait_pong_in_game(game, &mut peer, 1);
                assert!(game.client.remote.is_some());
                assert!(
                    !game
                        .client
                        .remote
                        .as_ref()
                        .unwrap()
                        .glowing
                        .contains(&77_777)
                );

                // Spawn an Interaction and a Painting.
                peer.send(
                    &C::AddEntity(play::AddEntity {
                        id: 1001,
                        uuid: 1001,
                        kind: interaction_kind,
                        position: [0.5, 64.0, 0.5],
                        velocity: [0.0; 3],
                        pitch: 0,
                        yaw: 0,
                        head_yaw: 0,
                        data: 0,
                    })
                    .encode(),
                )
                .unwrap();
                peer.send(
                    &C::AddEntity(play::AddEntity {
                        id: 2001,
                        uuid: 2001,
                        kind: painting_kind,
                        position: [2.0, 65.0, 2.0],
                        velocity: [0.0; 3],
                        pitch: 0,
                        yaw: 0,
                        head_yaw: 0,
                        data: 2,
                    })
                    .encode(),
                )
                .unwrap();

                // Repeated valid flag entries: last update wins.
                peer.send(
                    &C::SetEntityData(play::SetEntityData {
                        id: 1001,
                        values: vec![
                            (0, serializer::BYTE, Value::Byte(0x40)),
                            (0, serializer::BYTE, Value::Byte(0x00)),
                        ],
                    })
                    .encode(),
                )
                .unwrap();

                // Inline PaintingVariant updates asset and geometry (2x1).
                peer.send(
                    &C::SetEntityData(play::SetEntityData {
                        id: 2001,
                        values: vec![(
                            9,
                            serializer::PAINTING_VARIANT,
                            Value::PaintingVariant(PaintingHolder::Direct(PaintingVariant {
                                width: 2,
                                height: 1,
                                asset_id: "minecraft:alban".into(),
                                title: Some(Tag::String("Alban".into())),
                                author: None,
                            })),
                        )],
                    })
                    .encode(),
                )
                .unwrap();
                wait_pong_in_game(game, &mut peer, 2);
                assert_eq!(
                    game.client.remote.as_ref().unwrap().glowing.contains(&1001),
                    assignment["repeated_flags_last_wins"]["glowing_after"]
                        .as_bool()
                        .unwrap()
                );
                let painting = game.paintings.iter().find(|p| p.id == 2001).unwrap();
                assert_eq!(painting.asset.as_deref(), Some("alban"));
                assert_eq!((painting.width, painting.height), (2, 1));
                let (lo, hi) = painting.bounds();
                assert!((hi[0] - lo[0] - 2.0).abs() < 1e-6);
                assert!((hi[1] - lo[1] - 1.0).abs() < 1e-6);

                // Out-of-range accessor preserves valid prefix and raises PacketError.
                let err = game
                    .apply_set_entity_data_packet(play::SetEntityData {
                        id: 1001,
                        values: vec![
                            (0, serializer::BYTE, Value::Byte(0x40)),
                            (99, serializer::BYTE, Value::Byte(1)),
                        ],
                    })
                    .unwrap_err();
                assert!(err.is::<sunrise_client_gameplay::game::client_side::remote_client::ingress_state::PacketError>());
                assert_eq!(
                    game.client.remote.as_ref().unwrap().glowing.contains(&1001),
                    assignment["out_of_range_preserves_prefix"]["glowing_after"]
                        .as_bool()
                        .unwrap()
                );
                game.client.remote.as_mut().unwrap().glowing.remove(&1001);

                // Mismatched serializer preserves valid prefix and raises PacketError.
                let err = game
                    .apply_set_entity_data_packet(play::SetEntityData {
                        id: 1001,
                        values: vec![
                            (0, serializer::BYTE, Value::Byte(0x40)),
                            (0, serializer::INT, Value::Int(1)),
                        ],
                    })
                    .unwrap_err();
                assert!(err.is::<sunrise_client_gameplay::game::client_side::remote_client::ingress_state::PacketError>());
                assert_eq!(
                    game.client.remote.as_ref().unwrap().glowing.contains(&1001),
                    assignment["mismatched_serializer_preserves_prefix"]["glowing_after"]
                        .as_bool()
                        .unwrap()
                );

                // Inside a bundle, invalid entity metadata stops before a trailing Ping and disconnects.
                for packet in [
                    C::BundleDelimiter(Empty),
                    C::SetEntityData(play::SetEntityData {
                        id: 1001,
                        values: vec![
                            (0, serializer::BYTE, Value::Byte(0x40)),
                            (0, serializer::INT, Value::Int(1)),
                        ],
                    }),
                    C::Ping(cfg::Ping { id: 55 }),
                    C::BundleDelimiter(Empty),
                ] {
                    peer.send(&packet.encode()).unwrap();
                }
                let mut saw_pong = false;
                let deadline = Instant::now() + Duration::from_secs(5);
                while game.client.remote.is_some() && Instant::now() < deadline {
                    game.poll_remote().unwrap();
                    while let Ok((id, body)) = peer.recv() {
                        if matches!(S::decode(id, &body), Ok(S::Pong(p)) if p.id == 55) {
                            saw_pong = true;
                        }
                    }
                    std::thread::yield_now();
                }
                assert!(
                    game.client.remote.is_none(),
                    "invalid SetEntityData in bundle must disconnect"
                );
                assert!(!saw_pong, "bundle must stop before trailing Ping(55)");
                let Some(Mode::Multiplayer(multiplayer::Screen::Disconnected { reason, .. })) =
                    game.client.title.0.as_ref().map(|t| &t.mode)
                else {
                    panic!("missing Disconnected screen after bundle metadata error");
                };
                assert_eq!(
                    reason.translation_key(),
                    assignment["mismatched_serializer_preserves_prefix"]["disconnect_key"].as_str()
                );
            });
        }
        "synced_painting_override" => {
            // 3. Synchronized painting_variant override beats same-name local variant ("minecraft:kebab").
            let sync_override = &entity_data["painting_variant_sync_override"];
            let synced_name = sync_override["resolved_name"].as_str().unwrap();
            let synced_width = sync_override["resolved_width"].as_i64().unwrap() as i32;
            let synced_height = sync_override["resolved_height"].as_i64().unwrap() as i32;
            let synced_asset = sync_override["resolved_asset_id"].as_str().unwrap();
            let mut reg = sunrise_protocol::registry::Dynamic::default();
            reg.registries.insert(
                "minecraft:painting_variant".into(),
                vec![(
                    synced_name.to_owned(),
                    Some(Tag::compound([
                        ("width", Tag::Int(synced_width)),
                        ("height", Tag::Int(synced_height)),
                        ("asset_id", Tag::String(synced_asset.into())),
                    ])),
                )],
            );
            let registries = std::sync::Arc::new(reg);
            with_client_game(|game| {
                game.client.title.0 = None;
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "SyncedPaintingOverride")
                    .unwrap();
                let spawn = SpawnInfo {
                    dimension_type: 0,
                    dimension: "minecraft:overworld".into(),
                    seed: 0,
                    game_type: 0,
                    previous_game_type: None,
                    debug: false,
                    flat: true,
                    last_death: None,
                    portal_cooldown: 0,
                    sea_level: 63,
                };
                game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
                let (session, mut peer) =
                    Session::play_loop_probe_with_registries(registries.clone());
                let remote = game.client.remote.as_mut().unwrap();
                remote.session = session;
                remote.joined = true;
                remote.logged_in = true;
                remote.entity_id = 1000;
                let painting_kind = remote
                    .builtin
                    .get("minecraft:entity_type")
                    .unwrap()
                    .id("minecraft:painting")
                    .unwrap();
                peer.reader
                    .set_read_timeout(Some(Duration::from_millis(20)))
                    .unwrap();

                peer.send(
                    &C::AddEntity(play::AddEntity {
                        id: 2002,
                        uuid: 2002,
                        kind: painting_kind,
                        position: [2.0, 65.0, 2.0],
                        velocity: [0.0; 3],
                        pitch: 0,
                        yaw: 0,
                        head_yaw: 0,
                        data: 2,
                    })
                    .encode(),
                )
                .unwrap();
                wait_pong_in_game(game, &mut peer, 2);
                let before = game.paintings.iter().find(|p| p.id == 2002).unwrap();
                assert_eq!((before.width, before.height), (1, 1));
                // Send the recorded wire bytes for holder id 1 (0-based index 0: overridden minecraft:kebab).
                let mut frame = sunrise_core::vanilla_bytes::Writer::new();
                frame.varint(sunrise_protocol::ids::play::clientbound::SET_ENTITY_DATA);
                frame.varint(2002);
                frame.bytes(&hex_bytes(
                    sync_override["entity_data_hex"].as_str().unwrap(),
                ));
                peer.send(&frame.data).unwrap();
                wait_pong_in_game(game, &mut peer, 3);

                let painting = game.paintings.iter().find(|p| p.id == 2002).unwrap();
                assert_eq!(&*painting.variant, synced_name);
                assert_eq!(
                    painting.asset.as_deref(),
                    Some(
                        synced_asset
                            .strip_prefix("minecraft:")
                            .unwrap_or(synced_asset)
                    )
                );
                assert_eq!(
                    (painting.width, painting.height),
                    (synced_width, synced_height)
                );

                // Default spawn variant (index 0) also prefers synced data over same-name local variant.
                game.client.remote.as_mut().unwrap().registries = registries.clone();
                peer.send(
                    &C::AddEntity(play::AddEntity {
                        id: 2003,
                        uuid: 2003,
                        kind: painting_kind,
                        position: [8.0, 65.0, 2.0],
                        velocity: [0.0; 3],
                        pitch: 0,
                        yaw: 0,
                        head_yaw: 0,
                        data: 2,
                    })
                    .encode(),
                )
                .unwrap();
                wait_pong_in_game(game, &mut peer, 4);
                let spawned = game.paintings.iter().find(|p| p.id == 2003).unwrap();
                assert_eq!(&*spawned.variant, synced_name);
                assert_eq!(
                    spawned.asset.as_deref(),
                    Some(
                        synced_asset
                            .strip_prefix("minecraft:")
                            .unwrap_or(synced_asset)
                    )
                );
                assert_eq!(
                    (spawned.width, spawned.height),
                    (synced_width, synced_height)
                );
            });
        }
        "pre_login_disconnect_bundle" => {
            // 4. Pre-Login bundle [Disconnect("server kick"), PostEffects, Ping(88)] preserves first reason and stops.
            with_client_game(|game| {
                game.client.title.0 = None;
                game.client.post_effects.clear();
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "PreLoginDisconnectBundle")
                    .unwrap();
                let (session, mut peer) = Session::play_loop_probe();
                let remote = game.client.remote.as_mut().unwrap();
                remote.session = session;
                remote.joined = true;
                remote.logged_in = false;
                peer.reader
                    .set_read_timeout(Some(Duration::from_millis(20)))
                    .unwrap();
                for packet in [
                    C::BundleDelimiter(Empty),
                    C::Disconnect(cfg::Disconnect {
                        reason: Tag::String("server kick".into()),
                    }),
                    C::PostEffects(play::PostEffects {
                        effects: vec!["minecraft:invert".into()],
                    }),
                    C::Ping(cfg::Ping { id: 88 }),
                    C::BundleDelimiter(Empty),
                ] {
                    peer.send(&packet.encode()).unwrap();
                }
                let mut saw_pong = false;
                let deadline = Instant::now() + Duration::from_secs(5);
                while game.client.remote.is_some() && Instant::now() < deadline {
                    game.poll_remote().unwrap();
                    while let Ok((id, body)) = peer.recv() {
                        if matches!(S::decode(id, &body), Ok(S::Pong(p)) if p.id == 88) {
                            saw_pong = true;
                        }
                    }
                    std::thread::yield_now();
                }
                assert!(game.client.remote.is_none());
                assert!(!saw_pong, "bundle must stop before Ping(88)");
                assert!(game.client.post_effects.is_empty());
                let Some(Mode::Multiplayer(multiplayer::Screen::Disconnected { reason, .. })) =
                    game.client.title.0.as_ref().map(|t| &t.mode)
                else {
                    panic!("missing Disconnected screen");
                };
                assert_eq!(
                    Some(reason.plain().as_str()),
                    bundle["pre_login_disconnect_then_post_effects_bundle"]["disconnect_text"]
                        .as_str()
                );
            });
        }
        "pre_login_post_effects_bundle" => {
            // 5. Pre-Login bundle [PostEffects, Ping(89)] stops before Ping(89) with disconnect.packetError.
            with_client_game(|game| {
                game.client.title.0 = None;
                game.client.post_effects.clear();
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "PreLoginPostEffectsBundle")
                    .unwrap();
                let (session, mut peer) = Session::play_loop_probe();
                let remote = game.client.remote.as_mut().unwrap();
                remote.session = session;
                remote.joined = true;
                remote.logged_in = false;
                peer.reader
                    .set_read_timeout(Some(Duration::from_millis(20)))
                    .unwrap();
                for packet in [
                    C::BundleDelimiter(Empty),
                    C::PostEffects(play::PostEffects {
                        effects: vec!["minecraft:invert".into()],
                    }),
                    C::Ping(cfg::Ping { id: 89 }),
                    C::BundleDelimiter(Empty),
                ] {
                    peer.send(&packet.encode()).unwrap();
                }
                let mut saw_pong = false;
                let deadline = Instant::now() + Duration::from_secs(5);
                while game.client.remote.is_some() && Instant::now() < deadline {
                    game.poll_remote().unwrap();
                    while let Ok((id, body)) = peer.recv() {
                        if matches!(S::decode(id, &body), Ok(S::Pong(p)) if p.id == 89) {
                            saw_pong = true;
                        }
                    }
                    std::thread::yield_now();
                }
                assert!(game.client.remote.is_none());
                assert!(!saw_pong, "bundle must stop before Ping(89)");
                assert!(game.client.post_effects.is_empty());
                let Some(Mode::Multiplayer(multiplayer::Screen::Disconnected { reason, .. })) =
                    game.client.title.0.as_ref().map(|t| &t.mode)
                else {
                    panic!("missing Disconnected screen");
                };
                assert_eq!(
                    reason.translation_key(),
                    bundle["pre_login_post_effects_then_ping_bundle"]["disconnect_key"].as_str()
                );
            });
        }
        "disconnect_notify_after_shutdown" => {
            // 6. Hooks::ended runs only after preceding replies drained and transport shutdown completed.
            use std::sync::{
                Mutex,
                atomic::{AtomicBool, AtomicUsize, Ordering},
            };
            static PEER: Mutex<Option<sunrise_protocol::connection::Connection>> = Mutex::new(None);
            static NOTIFY_COUNT: AtomicUsize = AtomicUsize::new(0);
            static REASON: Mutex<Option<String>> = Mutex::new(None);
            static PONG_DRAINED: AtomicBool = AtomicBool::new(false);
            static CHANNEL_CLOSED: AtomicBool = AtomicBool::new(false);

            struct ShutdownProbeHook;
            impl sunrise_hooks::Hooks for ShutdownProbeHook {
                fn ended(&self, end: &sunrise_hooks::connection::Ended<'_>) {
                    NOTIFY_COUNT.fetch_add(1, Ordering::SeqCst);
                    *REASON.lock().unwrap() = Some(end.reason.to_owned());
                    if let Some(peer) = PEER.lock().unwrap().as_mut() {
                        let mut saw_pong = false;
                        let closed = loop {
                            match peer.recv() {
                                Ok((id, body)) => {
                                    if matches!(S::decode(id, &body), Ok(S::Pong(p)) if p.id == 12)
                                    {
                                        saw_pong = true;
                                    }
                                }
                                Err(err) => break err.is_end_of_stream(),
                            }
                        };
                        PONG_DRAINED.store(saw_pong, Ordering::SeqCst);
                        CHANNEL_CLOSED.store(closed, Ordering::SeqCst);
                    }
                }
            }
            static HOOK: ShutdownProbeHook = ShutdownProbeHook;
            sunrise_hooks::register(&HOOK).unwrap();

            let expected = &bundle["disconnect_notify_after_shutdown"];
            let expected_reason = expected["reason_text"].as_str().unwrap();
            with_client_game(|game| {
                game.client.title.0 = None;
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "ShutdownOrder").unwrap();
                game.connection_starts("play.example.test", "ShutdownOrder", false);
                let (session, mut peer) = Session::play_loop_probe();
                let remote = game.client.remote.as_mut().unwrap();
                remote.session = session;
                remote.joined = true;
                remote.logged_in = true;
                peer.reader
                    .set_read_timeout(Some(Duration::from_millis(50)))
                    .unwrap();
                for packet in [
                    C::BundleDelimiter(Empty),
                    C::Ping(cfg::Ping { id: 12 }),
                    C::Disconnect(cfg::Disconnect {
                        reason: Tag::String(expected_reason.into()),
                    }),
                    C::BundleDelimiter(Empty),
                ] {
                    peer.send(&packet.encode()).unwrap();
                }
                *PEER.lock().unwrap() = Some(peer);

                let deadline = Instant::now() + Duration::from_secs(5);
                while game.client.remote.is_some() && Instant::now() < deadline {
                    game.poll_remote().unwrap();
                    std::thread::yield_now();
                }
                assert!(game.client.remote.is_none());
            });
            *PEER.lock().unwrap() = None;
            assert_eq!(
                NOTIFY_COUNT.load(Ordering::SeqCst),
                expected["notify_count"].as_u64().unwrap() as usize
            );
            assert_eq!(
                REASON.lock().unwrap().as_deref(),
                Some(expected_reason),
                "disconnect hook reason"
            );
            assert!(
                PONG_DRAINED.load(Ordering::SeqCst),
                "preceding Pong(12) must drain before Hooks::ended runs"
            );
            assert_eq!(
                !CHANNEL_CLOSED.load(Ordering::SeqCst),
                expected["channel_open_during_on_disconnect"]
                    .as_bool()
                    .unwrap(),
                "transport must be shut down before Hooks::ended runs"
            );
        }
        other => panic!("unknown bundle parity case {other}"),
    }
}
