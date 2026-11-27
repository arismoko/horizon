/// Sleeping metadata and the local tick use the loaded bed's shape, not
/// a red-bed constant. Missing client data must not replace a server pose
/// or decide to wake the player.
#[test]
#[ignore = "daily"]
fn client_sleep_metadata_and_tick_preserve_shape_and_authoritative_positions() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use sunrise_core::geometry::Vec3;
    use sunrise_level::chunk::Chunk;
    use sunrise_protocol::{
        buf::Reader,
        connection::Pipe,
        entity_data::{Value, serializer},
        packets::play::{self, Clientbound as C},
    };
    use sunrise_sim::{mobs::Mob, motion::PlayerMotion};

    let deliver = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>, packet: C| {
        let body = packet.encode();
        let mut reader = Reader::new(&body);
        let id = reader.varint().unwrap();
        game.handle_packet(C::decode(id, reader.rest()).unwrap())
            .unwrap();
    };
    let metadata = |id, bed: Option<[i32; 3]>| {
        C::SetEntityData(play::SetEntityData {
            id,
            values: vec![
                (
                    6,
                    serializer::POSE,
                    Value::Id(if bed.is_some() { 2 } else { 0 }),
                ),
                (14, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(bed)),
            ],
        })
    };
    let teleport = |position: Vec3| {
        C::PlayerPosition(play::PlayerPosition {
            teleport: 1,
            position: [position.x, position.y, position.z],
            delta: [0.0; 3],
            yaw: 0.0,
            pitch: 30.0,
            relatives: 0,
        })
    };

    with_client_game(|game| {
        let (client, server) = Pipe::pair();
        game.connect_local(client, "sleep-client").unwrap();
        let remote = game.client.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.loaded_sent = true;
        remote.entity_id = 42;
        remote.level_players.insert(66, 789);
        let kind = remote.player_type.as_ref().unwrap().clone();
        let authoritative = Vec3::new(8.5, 70.0, 8.5);
        game.player = Some(PlayerMotion::new(game.motion, authoritative).unwrap());
        game.mobs.push(Mob::new(66, kind, authoritative, 0.0));
        game.client
            .streaming
            .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
        let registry = game.pack.assets.registry;
        let air = registry.default_state("minecraft:air").unwrap();
        let stone = registry.default_state("minecraft:stone").unwrap();

        // Straw's foot is 4/16 high; red's shape is 9/16. These expected
        // positions are independent of sleeping_position and its helpers.
        for (name, complete, head, expected) in [
            (
                "minecraft:straw_bed",
                true,
                [5, 64, 5],
                Some(Vec3::new(5.5, 64.375, 5.5)),
            ),
            (
                "minecraft:red_bed",
                true,
                [5, 64, 5],
                Some(Vec3::new(5.5, 64.6875, 5.5)),
            ),
            ("minecraft:straw_bed", false, [5, 64, 5], None),
            ("minecraft:stone", false, [5, 64, 5], None),
            ("minecraft:straw_bed", false, [37, 64, 5], None),
        ] {
            let case = (name, complete, head);
            game.world.set(5, 64, 5, air).unwrap();
            game.world.set(5, 64, 6, air).unwrap();
            if head[0] == 5 {
                let state = if name == "minecraft:stone" {
                    stone
                } else {
                    registry
                        .resolve(&sunrise_world::chunk::BlockState {
                            name: name.into(),
                            properties: [
                                ("facing", "north"),
                                ("occupied", "false"),
                                ("part", "head"),
                            ]
                            .into_iter()
                            .map(|(k, v)| (k.into(), v.into()))
                            .collect(),
                        })
                        .unwrap()
                };
                game.world.set(5, 64, 5, state).unwrap();
                if complete {
                    let mut foot = registry.state(state).unwrap().clone();
                    foot.properties.insert("part".into(), "foot".into());
                    game.world
                        .set(5, 64, 6, registry.id(&foot).unwrap())
                        .unwrap();
                }
            } else {
                assert!(!game.world.chunks.contains_key(&(2, 0)));
            }
            deliver(game, teleport(authoritative));
            let other = game
                .common
                .mob_index
                .get_mut(&mut game.common.mobs, 66)
                .unwrap();
            other.position = authoritative;
            other.previous = authoritative;
            other.head_pitch = 30.0;
            other.previous_head_pitch = 30.0;
            game.sleep_timer = 17;
            deliver(game, metadata(66, Some(head)));
            deliver(game, metadata(42, Some(head)));
            let at = expected.unwrap_or(authoritative);
            assert_eq!(
                game.player.as_ref().unwrap().position,
                at,
                "own metadata: {case:?}"
            );
            assert_eq!(game.previous_position, at, "own old position: {case:?}");
            let other = game.common.mob_index.get(&game.common.mobs, 66).unwrap();
            assert_eq!(other.position, at, "other metadata: {case:?}");
            assert_eq!(other.previous, at, "other old position: {case:?}");
            assert_eq!((other.head_pitch, other.previous_head_pitch), (0.0, 0.0));
            assert_eq!(game.sleeping, Some(head));
            assert_eq!(game.sleep_timer, 0);

            // A correct authoritative teleport cannot be overwritten by
            // the next sleeping tick, even while this client's bed is absent.
            deliver(game, teleport(at));
            game.player.as_mut().unwrap().velocity = Vec3::new(0.3, -0.4, 0.2);
            for timer in 1..=2 {
                game.tick_remote_player(true, false).unwrap();
                assert_eq!(game.sleep_timer, timer, "local tick must execute: {case:?}");
                assert_eq!(game.sleeping, Some(head));
                let player = game.player.as_ref().unwrap();
                assert_eq!(player.position, at, "own tick: {case:?}");
                assert_eq!(player.velocity, Vec3::ZERO);
                assert_eq!(player.pitch, 0.0);
                assert_eq!(
                    game.common
                        .mob_index
                        .get(&game.common.mobs, 66)
                        .unwrap()
                        .position,
                    at,
                    "other tick: {case:?}"
                );
            }

            deliver(game, metadata(66, None));
            deliver(game, metadata(42, None));
            assert_eq!(game.sleeping, None);
            assert_eq!(
                game.player.as_ref().unwrap().position,
                at,
                "clearing metadata does not relocate"
            );
            let other = game.common.mob_index.get(&game.common.mobs, 66).unwrap();
            assert_eq!(other.position, at);
            assert_eq!(other.extra.ints[1], 0);
            assert_eq!(other.extra.synced_pose, 0);
            deliver(game, C::Animate(play::Animate { id: 42, action: 0 }));
            assert_eq!(game.sleep_timer, 100);
            let wake = Vec3::new(9.5, 70.1, 9.5);
            deliver(game, teleport(wake));
            assert_eq!(game.player.as_ref().unwrap().position, wake);
            game.tick_remote_player(true, false).unwrap();
            assert_eq!(game.sleeping, None);
            assert_eq!(game.sleep_timer, 101);
            let player = game.player.as_ref().unwrap();
            assert_eq!(
                (player.position.x, player.position.z),
                (wake.x, wake.z),
                "wake tick must not snap to bed: {case:?}"
            );
        }
        server.shutdown();
    });
}
