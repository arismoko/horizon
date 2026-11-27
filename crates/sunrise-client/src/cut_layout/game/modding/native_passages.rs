#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1 as _;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::native_passages::tests::RemoveOnSecondSent;

    use sunrise_gameplay_common::game::native_passages::tests::doorway;

    use sunrise_gameplay_common::game::native_passages::tests::doorway_form;

    use sunrise_gameplay_common::game::native_passages::tests::packets;

    use sunrise_gameplay_common::game::native_passages::tests::passage_messages;

    use sunrise_gameplay::game::native_passages::tests::listen;

    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::native_passages::HeldRefusal;

    #[cfg(feature = "client")]
    use sunrise_protocol::passage_sync::PassageForm;

    #[cfg(feature = "client")]
    use mod_api::PassageRegistrationError as Refused;

    use crate::game::Dim;

    #[cfg(feature = "client")]
    use crate::game::native_entity_refs::tests::{with_client_game, with_game};

    use crate::game::native_guest_turn_tests::with_guests;

    use mod_api::{DimensionKey, Passage, PassageId};
    #[cfg(feature = "client")]
    use mod_api::{LevelRef, Vec3, WorldEpoch};

    use std::sync::{Arc, mpsc};

    use sunrise_level::chunk::Chunk;

    use sunrise_protocol::packets::play::Clientbound;

    use sunrise_protocol::passage_sync::Message;

    #[cfg(feature = "client")]
    use sunrise_protocol::passage_sync::encode;

    /// Passages are numbered from 1 in order; a chunk takes 8 and a level 256, and the next
    /// is refused with nothing registered; a removal frees its place once. A stale handle
    /// and a client register nothing.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_level_numbers_its_passages_and_refuses_past_its_caps() {
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let in_chunk = |cx: i32, i: u32| Passage {
                origin: Vec3::new(f64::from(cx * 16) + 0.5, 65.5, f64::from(i) + 0.5),
                ..doorway()
            };
            for i in 0..8 {
                let id = game.level_register_passage(level, in_chunk(0, i)).unwrap();
                assert_eq!(id, PassageId::from_raw(u64::from(i) + 1));
            }
            assert_eq!(
                game.level_register_passage(level, in_chunk(0, 8)),
                Err(Refused::ChunkFull { limit: 8 })
            );
            for cx in 1..32 {
                for i in 0..8 {
                    game.level_register_passage(level, in_chunk(cx, i)).unwrap();
                }
            }
            assert_eq!(
                game.level_register_passage(level, in_chunk(40, 0)),
                Err(Refused::LevelFull { limit: 256 })
            );
            assert!(game.level_remove_passage(level, PassageId::from_raw(3)));
            assert!(!game.level_remove_passage(level, PassageId::from_raw(3)));
            assert!(!game.level_remove_passage(level, PassageId::from_raw(999)));
            assert_eq!(
                game.level_register_passage(level, in_chunk(0, 8)),
                Ok(PassageId::from_raw(257)),
                "the freed place, under a new number"
            );
            assert_eq!(
                game.level_register_passage(level, in_chunk(40, 0)),
                Err(Refused::LevelFull { limit: 256 })
            );
            let stale = LevelRef::new(WorldEpoch::new(0), game.dim.key());
            assert_eq!(
                game.level_register_passage(stale, doorway()),
                Err(Refused::Stale)
            );
            assert!(!game.level_remove_passage(stale, PassageId::from_raw(1)));
            // The last id is given once; then registration is refused, nothing given twice.
            game.passages.last = u64::MAX - 1;
            assert!(game.level_remove_passage(level, PassageId::from_raw(4)));
            assert_eq!(
                game.level_register_passage(level, in_chunk(0, 9)),
                Ok(PassageId::from_raw(u64::MAX))
            );
            assert!(game.level_remove_passage(level, PassageId::from_raw(5)));
            assert_eq!(
                game.level_register_passage(level, in_chunk(0, 10)),
                Err(Refused::NoIdsLeft)
            );
        });
        // Client refusal is enforced at compile time (`ClientGame` lacks server passage writes).
    }
    /// A level's passages and their untold changes go when it closes: reopened, it starts
    /// with no passage and its whole caps, and numbering goes on (1 is not given again).
    /// A failed open retires everything, the window's copy too.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn passages_go_with_their_level_and_world() {
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(
                game.level_register_passage(level, doorway()),
                Ok(PassageId::from_raw(1))
            );
            game.close_loaded_levels_for_stop();
            assert!(game.passages.index.is_empty() && game.passages.dirty.is_empty());
            assert!(!game.level_remove_passage(level, PassageId::from_raw(1)));
            game.native_level_opened(game.dim);
            for i in 0..8 {
                let id = game.level_register_passage(level, doorway()).unwrap();
                assert_eq!(id, PassageId::from_raw(i + 2));
            }
            game.passages.held.insert(PassageId::from_raw(1), doorway());
            game.native_lifecycle_world_closed();
            assert!(game.passages.levels.is_empty() && game.passages.index.is_empty());
            assert!(game.passages.dirty.is_empty() && game.passages.held.is_empty());
        });
    }
    /// A destination in another level is registered only while that level is open: one that
    /// is not open, and the passage's own level, are refused `UnknownLevel`, with nothing
    /// registered.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_destination_level_must_be_open_and_another_level() {
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let to = |key| Passage {
                destination_level: Some(key),
                ..doorway()
            };
            assert_eq!(
                game.level_register_passage(level, to(DimensionKey::NETHER)),
                Err(Refused::UnknownLevel),
                "not open"
            );
            assert_eq!(
                game.level_register_passage(level, to(level.dimension())),
                Err(Refused::UnknownLevel),
                "its own level"
            );
            assert!(game.passages.index.is_empty());
            game.enter_level(Dim::Nether).unwrap();
            assert_eq!(
                game.level_register_passage(level, to(DimensionKey::NETHER)),
                Ok(PassageId::from_raw(1))
            );
        });
    }
    /// A passage into another level is registered but not sent: neither at the tick's end
    /// nor after its chunk. The client hears only of the passage within the level.
    #[test]
    #[ignore = "daily"]
    fn a_passage_into_another_level_is_not_sent_to_the_clients() {
        with_guests(|game| {
            game.enter_level(Dim::Nether).unwrap();
            game.enter_level(Dim::Overworld).unwrap();
            game.native_lifecycle.world_open_ready().unwrap();
            let receivers = listen(game, ["sunrise", "vanilla"]);
            let level = game.level(game.dim.id()).unwrap();
            let far = Passage {
                destination_level: Some(DimensionKey::NETHER),
                ..doorway()
            };
            assert_eq!(
                game.level_register_passage(level, far),
                Ok(PassageId::from_raw(1))
            );
            assert_eq!(
                game.level_register_passage(level, doorway()),
                Ok(PassageId::from_raw(2))
            );
            for cz in -1..=1 {
                for cx in -1..=1 {
                    let mut chunk = Chunk::empty(cx, cz);
                    chunk.lit = true;
                    game.world.chunks.insert((cx, cz), Arc::new(chunk));
                }
            }
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.ticks = 10;
            hub.guests.update_all(|g| g.max_unacknowledged = 64);
            let mut sent = Vec::new();
            for _ in 0..5 {
                game.tick_lan(false);
                sent.extend(packets(&receivers[0]));
            }
            assert_eq!(
                passage_messages(&sent),
                [Message::Add(vec![doorway_form(2)])]
            );
        });
    }
    /// In the real order of a server tick: a registration with no client holding the
    /// chunk is told to nobody; the hub then sends the chunk and the passage follows it, to
    /// the Sunrise client only. A removal reaches it at the next tick's end.
    #[test]
    #[ignore = "daily"]
    fn a_passage_follows_its_chunk_to_the_sunrise_clients() {
        with_guests(|game| {
            game.native_lifecycle.world_open_ready().unwrap();
            let receivers = listen(game, ["sunrise", "vanilla"]);
            let level = game.level(game.dim.id()).unwrap();
            let id = game.level_register_passage(level, doorway()).unwrap();
            game.flush_passages();
            assert!(
                receivers
                    .iter()
                    .all(|rx| passage_messages(&packets(rx)).is_empty())
            );
            for cz in -1..=1 {
                for cx in -1..=1 {
                    let mut chunk = Chunk::empty(cx, cz);
                    chunk.lit = true;
                    game.world.chunks.insert((cx, cz), Arc::new(chunk));
                }
            }
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.ticks = 10;
            hub.guests.update_all(|g| g.max_unacknowledged = 64);
            let mut sent: Vec<Vec<Clientbound>> = vec![Vec::new(), Vec::new()];
            for _ in 0..5 {
                game.tick_lan(false);
                for (rx, sent) in receivers.iter().zip(&mut sent) {
                    sent.extend(packets(rx));
                }
            }
            let chunk_at = |sent: &[Clientbound]| {
                sent.iter().position(
                    |p| matches!(p, Clientbound::LevelChunkWithLight(c) if (c.x, c.z) == (0, 0)),
                )
            };
            let sunrise = &sent[0];
            let chunk = chunk_at(sunrise).expect("the hub sent chunk (0, 0)");
            let told = sunrise
                .iter()
                .position(|p| matches!(p, Clientbound::CustomPayload(p) if p.channel == "sunrise:passage_sync"))
                .expect("the passage followed");
            assert!(chunk < told, "the chunk comes first");
            assert_eq!(
                passage_messages(sunrise),
                [Message::Add(vec![doorway_form(1)])]
            );
            assert!(chunk_at(&sent[1]).is_some());
            assert!(
                passage_messages(&sent[1]).is_empty(),
                "never to a vanilla client"
            );

            assert!(game.level_remove_passage(level, id));
            game.tick_lan(false);
            assert_eq!(
                passage_messages(&packets(&receivers[0])),
                [Message::Remove(vec![1])]
            );
        });
    }
    /// A passage registered and removed before the tick's end is never told and leaves no
    /// trace, however often; the changes kept stay within the live passages and the told
    /// ones removed.
    #[test]
    #[ignore = "daily"]
    fn churn_before_the_tick_end_keeps_and_sends_nothing() {
        with_guests(|game| {
            game.native_lifecycle.world_open_ready().unwrap();
            let receivers = listen(game, ["sunrise", "sunrise"]);
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.guests.update_all(|g| {
                g.center = Some((0, 0));
                g.sent_chunks.insert((0, 0));
            });
            let level = game.level(game.dim.id()).unwrap();
            for _ in 0..1000 {
                let id = game.level_register_passage(level, doorway()).unwrap();
                assert!(game.level_remove_passage(level, id));
            }
            assert!(game.passages.dirty.is_empty() && game.passages.index.is_empty());
            game.flush_passages();
            assert!(
                receivers
                    .iter()
                    .all(|rx| passage_messages(&packets(rx)).is_empty())
            );

            let kept = game.level_register_passage(level, doorway()).unwrap();
            game.flush_passages();
            for _ in 0..1000 {
                let id = game.level_register_passage(level, doorway()).unwrap();
                assert!(game.level_remove_passage(level, id));
            }
            assert!(game.level_remove_passage(level, kept));
            assert_eq!(game.passages.dirty.len(), 1, "only the told one's removal");
            game.flush_passages();
            for rx in &receivers {
                assert_eq!(
                    passage_messages(&packets(rx)),
                    [
                        Message::Add(vec![doorway_form(1001)]),
                        Message::Remove(vec![1001])
                    ]
                );
            }
        });
    }
    /// The LAN closes and opens again with a passage live: the new hub records the chunks
    /// it sends from its start, so a Sunrise guest that joins it is sent the passage with
    /// the chunk the first tick sends, before that tick's `flush_passages`.
    #[test]
    #[ignore = "daily"]
    fn a_reopened_lan_records_chunks_for_the_live_passages() {
        use sunrise_gameplay::game::lan::LanSettings;
        use sunrise_protocol::packets::configuration::ClientInformation;
        use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
        with_guests(|game| {
            game.native_lifecycle.world_open_ready().unwrap();
            let level = game.level(game.dim.id()).unwrap();
            game.level_register_passage(level, doorway()).unwrap();
            game.flush_passages();
            game.lan_disconnect(false);
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: game.dim.id().to_owned(),
                allow_flight: true,
                seed: 0,
                max_players: 2,
                integrated: true,
                levels: game.server.level_shapes.clone(),
                level_seeds: game.server.levels.0.runtime.seeds(),
            };
            game.lan_host(settings, net, config).unwrap();
            assert!(
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .chunk_watches
                    .recording()
            );
            for cz in -1..=1 {
                for cx in -1..=1 {
                    let mut chunk = Chunk::empty(cx, cz);
                    chunk.lit = true;
                    game.world.chunks.insert((cx, cz), Arc::new(chunk));
                }
            }
            let (tx, rx) = mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 14,
                    uuid: 14,
                    name: "late".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(2),
                    brand: "sunrise".into(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::default(),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                14,
                [3.0, 64.0, 5.0],
                2,
            );
            guest.placed = true;
            guest.max_unacknowledged = 64;
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.ticks = 10;
            hub.guests.push(guest);
            // One tick: its level pass sends the chunks, its end flushes the passages.
            game.tick_lan(false);
            let sent = packets(&rx);
            let chunk = sent
                .iter()
                .position(
                    |p| matches!(p, Clientbound::LevelChunkWithLight(c) if (c.x, c.z) == (0, 0)),
                )
                .expect("the first tick sent chunk (0, 0)");
            let told = sent
                .iter()
                .position(|p| matches!(p, Clientbound::CustomPayload(p) if p.channel == "sunrise:passage_sync"))
                .expect("the passage went with its chunk");
            assert!(chunk < told);
            assert_eq!(
                passage_messages(&sent),
                [Message::Add(vec![doorway_form(1)])]
            );
        });
    }
    /// A Sunrise arrival held for a hand-over is sent its chunks, so it is told their
    /// passages too; a held vanilla client is not.
    #[test]
    #[ignore = "daily"]
    fn a_held_arrival_is_told_its_chunks_passages() {
        with_guests(|game| {
            game.native_lifecycle.world_open_ready().unwrap();
            let receivers = listen(game, ["sunrise", "vanilla"]);
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.guests.update_all(|g| {
                g.placed = false;
                g.held = true;
                g.center = Some((0, 0));
                g.sent_chunks.insert((0, 0));
            });
            let level = game.level(game.dim.id()).unwrap();
            game.level_register_passage(level, doorway()).unwrap();
            game.flush_passages();
            assert_eq!(
                passage_messages(&packets(&receivers[0])),
                [Message::Add(vec![doorway_form(1)])]
            );
            assert!(passage_messages(&packets(&receivers[1])).is_empty());
        });
    }
    /// Two chunk sends in one tick: the first gives a new passage to guest 12 with its
    /// chunk, and the mod removes it at the second, before the tick's end told it to the
    /// rest. The removal still reaches 12, which ends with no passage, and 13 is told
    /// only the removal.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_passage_sent_with_its_chunk_and_removed_in_the_same_tick_leaves_no_ghost() {
        use sunrise_sim::lan::chunk_watch::{
            ChunkWatch, ChunkWatchKind, ChunkWatchLog, LevelWatches,
        };
        let mods = vec![mod_api::ModEntry::new(
            "remover",
            Box::new(RemoveOnSecondSent {
                heard: Default::default(),
                id: PassageId::from_raw(1),
            }),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            let mut twelve = Vec::new();
            with_guests(|game| {
                game.native_lifecycle.world_open_ready().unwrap();
                let receivers = listen(game, ["sunrise", "sunrise"]);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let dimension = game.dim.id().to_owned();
                let hub = &mut game.server.lan.as_mut().unwrap().hub;
                hub.guests.update_all(|g| {
                    g.center = Some((0, 0));
                    g.sent_chunks.insert((0, 0));
                });
                let sent = |uuid: u128| ChunkWatch {
                    kind: ChunkWatchKind::Sent,
                    uuid,
                    entity_id: uuid as i32,
                    chunk: [0, 0],
                };
                hub.chunk_watches = ChunkWatchLog::new(vec![LevelWatches {
                    dimension,
                    watches: vec![sent(12), sent(13)],
                }]);
                let level = game.level(game.dim.id()).unwrap();
                let id = game.level_register_passage(level, doorway()).unwrap();
                assert_eq!(id, PassageId::from_raw(1));
                game.run_chunk_watches();
                game.flush_passages();
                twelve = packets(&receivers[0]);
                assert_eq!(
                    passage_messages(&twelve),
                    [
                        Message::Add(vec![doorway_form(1)]),
                        Message::Remove(vec![1])
                    ]
                );
                assert_eq!(
                    passage_messages(&packets(&receivers[1])),
                    [Message::Remove(vec![1])]
                );
            });
            // Guest 12's client, given what it was sent, holds nothing.
            with_client_game(|client| {
                client
                    .world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                for p in &twelve {
                    if let Clientbound::CustomPayload(p) = p
                        && p.channel == "sunrise:passage_sync"
                    {
                        assert!(client.remote_passage_sync(&p.data).is_empty());
                    }
                }
                assert!(client.passages.held.is_empty());
            });
        });
    }
    /// The window keeps what its server sent for a chunk it holds, by number, until the
    /// server removes it or the window forgets the chunk. It refuses by name a passage the
    /// server could not have registered, one for a chunk it does not hold, and a 257th,
    /// across packets; a number sent again replaces its passage.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn the_window_keeps_only_passages_it_can_hold() {
        with_client_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let send = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                        message: Message| {
                game.remote_passage_sync(&encode(&message).unwrap())
            };
            let bad = PassageForm {
                width: f64::NAN,
                ..doorway_form(2)
            };
            let skewed = PassageForm {
                axis_h: [1.0, 0.0, 0.0],
                ..doorway_form(3)
            };
            let elsewhere = PassageForm {
                origin: [80.5, 65.5, 8.5],
                ..doorway_form(4)
            };
            let refused = send(
                game,
                Message::Add(vec![doorway_form(1), bad, skewed, elsewhere]),
            );
            assert_eq!(
                refused,
                [
                    (2, HeldRefusal::Bad(Refused::NotFinite)),
                    (3, HeldRefusal::Bad(Refused::BadAxes)),
                    (4, HeldRefusal::ChunkNotHeld),
                ]
            );
            assert_eq!(
                game.passages.held.get(&PassageId::from_raw(1)),
                Some(&doorway())
            );

            let many = |ids: std::ops::Range<u64>| Message::Add(ids.map(doorway_form).collect());
            assert!(send(game, many(10..210)).is_empty());
            let refused = send(game, many(210..270));
            assert_eq!(game.passages.held.len(), 256);
            assert_eq!(refused.len(), 5);
            assert!(refused.iter().all(|(_, why)| *why == HeldRefusal::Full));
            assert_eq!(refused[0].0, 265);
            assert!(send(game, many(10..11)).is_empty(), "a number sent again");

            send(game, Message::Remove(vec![1]));
            assert_eq!(game.passages.held.len(), 255);
            game.client_chunk_unload(1, 0, true);
            assert_eq!(game.passages.held.len(), 255);
            game.client_chunk_unload(0, 0, true);
            assert!(game.passages.held.is_empty());
        });
    }
}

#[cfg(test)]
mod cross_tests {
    use mod_api::{DimensionKey, Passage, PassageCrossRefusal as Refusal, PassageId, Vec3};
    use std::sync::{Arc, mpsc};
    use sunrise_gameplay::game::native_guest_turn_tests::with_guests;
    use sunrise_gameplay_common::game::Dim;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_level::chunk::Chunk;
    use sunrise_protocol::packets::configuration::CustomPayload;
    use sunrise_protocol::packets::play::{self, Clientbound as C, Serverbound as S, relative};
    use sunrise_protocol::passage_sync::{CROSS_CHANNEL, Cross, Message, encode_cross};
    use sunrise_sim::lan::guest::InboxItem;
    use sunrise_sim::lan::hub::passage_cross::refusal_code;
    use {
        sunrise_gameplay::game::native_passages::tests::listen,
        sunrise_gameplay_common::game::native_passages::tests::doorway,
        sunrise_gameplay_common::game::native_passages::tests::packets,
        sunrise_gameplay_common::game::native_passages::tests::passage_messages,
    };

    /// The doorway comes out 24 blocks east, turned a clockwise quarter.
    fn near_doorway() -> Passage {
        Passage {
            destination: Vec3::new(24.5, 65.5, 8.5),
            ..doorway()
        }
    }

    /// Guests 12 and 13 on Sunrise clients on a stone floor (y 64) over chunks x 0 to 1,
    /// tracking them; 12 stands loaded at `at`. The doorway is passage 1.
    fn scene(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        at: [f64; 3],
    ) -> mpsc::Receiver<sunrise_sim::lan::Out> {
        game.native_lifecycle.world_open_ready().unwrap();
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap()
            .0;
        for cx in 0..=1 {
            let mut chunk = Chunk::empty(cx, 0);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set(x, 64, z, stone);
                }
            }
            game.world.chunks.insert((cx, 0), Arc::new(chunk));
        }
        let dimension = game.dim.id().to_owned();
        let mut receivers = listen(game, ["sunrise", "sunrise"]);
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_all(|g| {
                g.dimension = dimension.clone();
                g.loaded = true;
                g.load_timeout = 0;
                g.center = Some((0, 0));
                g.sent_chunks.extend([(0, 0), (1, 0)]);
                g.position = at;
                g.previous_position = at;
                g.first_good = at;
                g.on_ground = true;
            });
        let level = game.level(game.dim.id()).unwrap();
        let id = game.level_register_passage(level, near_doorway()).unwrap();
        assert_eq!(id, PassageId::from_raw(1));
        game.tick_lan(false);
        receivers.truncate(1);
        let rx = receivers.pop().unwrap();
        packets(&rx);
        rx
    }

    /// Guest 12's next tick: `inbox`, then the rest of the server's tick.
    fn tick(game: &mut sunrise_gameplay::game::ServerGame<'_>, inbox: Vec<S>) {
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.inbox
                .extend(inbox.into_iter().map(|p| InboxItem::Packet(p, None)));
        });
        game.tick_lan(false);
    }

    fn cross(id: u64, from: [f64; 3]) -> S {
        S::CustomPayload(CustomPayload {
            channel: CROSS_CHANNEL.into(),
            data: encode_cross(&Cross { id, from }),
        })
    }

    fn position_packets(sent: &[C]) -> Vec<play::PlayerPosition> {
        sent.iter()
            .filter_map(|p| match p {
                C::PlayerPosition(p) => Some(p.clone()),
                _ => None,
            })
            .collect()
    }

    /// The refusals among `sent` (the passages sent with chunks are left out).
    fn refusals(sent: &[C]) -> Vec<Message> {
        passage_messages(sent)
            .into_iter()
            .filter(|m| matches!(m, Message::Refused { .. }))
            .collect()
    }

    fn guest<'a>(
        game: &'a sunrise_gameplay::game::ServerGame<'_>,
    ) -> &'a sunrise_sim::lan::guest::Guest {
        &game.server.lan.as_ref().unwrap().hub.guests[0]
    }

    const ALL: i32 = relative::X
        | relative::Y
        | relative::Z
        | relative::Y_ROT
        | relative::X_ROT
        | relative::DELTA_X
        | relative::DELTA_Y
        | relative::DELTA_Z
        | relative::ROTATE_DELTA;

    /// The server follows a guest's crossing to where the doorway comes out, with the yaw
    /// turned, and answers with one position packet that is all relative and zero. The
    /// confirmation, from a step further on, and the next move are accepted as they are:
    /// no correction follows. A passage that leads into the air: the confirmation, checked
    /// off the ground, and the fall after it pass too, and the player is not floating.
    #[test]
    #[ignore = "daily"]
    fn the_server_follows_a_crossing_without_a_snap() {
        for (destination_y, steps) in [(65.5, [65.0, 65.0]), (75.5, [74.9216, 74.6862])] {
            with_guests(|game| {
                let rx = scene(game, [0.5, 65.0, 8.25]);
                if destination_y != 65.5 {
                    let level = game.level(game.dim.id()).unwrap();
                    assert!(game.level_remove_passage(level, PassageId::from_raw(1)));
                    let high = Passage {
                        destination: Vec3::new(24.5, destination_y, 8.5),
                        ..near_doorway()
                    };
                    assert_eq!(
                        game.level_register_passage(level, high),
                        Ok(PassageId::from_raw(2))
                    );
                }
                let id = if destination_y == 65.5 { 1 } else { 2 };
                tick(game, vec![cross(id, [0.5, 65.0, 8.25])]);
                let to = [24.75, destination_y - 0.5, 8.5];
                let g = guest(game);
                assert_eq!(g.position, to);
                assert_eq!(g.yaw, 90.0);
                let (teleport, awaited) = g.awaiting_teleport.expect("the acceptance waits");
                assert_eq!(awaited, to);
                let sent = packets(&rx);
                assert_eq!(
                    position_packets(&sent),
                    [play::PlayerPosition {
                        teleport,
                        position: [0.0; 3],
                        delta: [0.0; 3],
                        yaw: 0.0,
                        pitch: 0.0,
                        relatives: ALL,
                    }]
                );
                assert!(refusals(&sent).is_empty(), "no refusal");

                let on_floor = u8::from(destination_y == 65.5);
                tick(
                    game,
                    vec![
                        S::AcceptTeleportation(play::AcceptTeleportation {
                            id: teleport,
                            x: 24.5,
                            y: steps[0],
                            z: 8.5,
                            yaw: 90.0,
                            pitch: 0.0,
                        }),
                        S::ClientTickEnd(sunrise_protocol::packets::Empty),
                    ],
                );
                tick(
                    game,
                    vec![
                        S::MovePlayerPos(play::MovePlayerPos {
                            x: 24.25,
                            y: steps[1],
                            z: 8.5,
                            flags: on_floor,
                        }),
                        S::ClientTickEnd(sunrise_protocol::packets::Empty),
                    ],
                );
                let g = guest(game);
                assert_eq!(g.awaiting_teleport, None);
                assert_eq!(g.position, [24.25, steps[1], 8.5]);
                assert!(!g.client_is_floating);
                assert!(position_packets(&packets(&rx)).is_empty(), "no correction");
            });
        }
    }

    /// Two crossings in one inbox from different places, no move behind them: the first is
    /// decided from its own `from` before the hub reads the second, and is followed; the
    /// second, read the next tick while the first's acceptance waits, is refused `Pending`.
    #[test]
    #[ignore = "daily"]
    fn two_crossings_in_a_row_are_decided_each_from_its_own_source() {
        with_guests(|game| {
            let rx = scene(game, [0.5, 65.0, 8.25]);
            tick(
                game,
                vec![cross(1, [0.5, 65.0, 8.25]), cross(1, [0.5, 65.0, 8.0])],
            );
            let g = guest(game);
            assert_eq!(g.position, [24.75, 65.0, 8.5]);
            let (teleport, awaited) = g.awaiting_teleport.expect("the first is followed");
            assert_eq!(awaited, [24.75, 65.0, 8.5]);
            let sent = packets(&rx);
            assert!(refusals(&sent).is_empty());
            let accepted = position_packets(&sent);
            assert_eq!(accepted.len(), 1);
            assert_eq!(
                (accepted[0].teleport, accepted[0].relatives),
                (teleport, ALL)
            );
            tick(game, Vec::new());
            let sent = packets(&rx);
            let reason = refusal_code(Refusal::Pending);
            assert_eq!(refusals(&sent), [Message::Refused { id: 1, reason }]);
            let back = position_packets(&sent);
            assert_eq!(back.len(), 1);
            assert_eq!(
                (back[0].position, back[0].relatives),
                ([24.75, 65.0, 8.5], 0)
            );
        });
    }

    /// Each refusal is told by name, then the player is put back where the server has it:
    /// a passage the level lacks, a crossing while a teleport waits, a far end the guest
    /// does not track, feet more than 20 blocks from the opening, and a move to where it
    /// crossed from that the move checks turn down.
    #[test]
    #[ignore = "daily"]
    fn a_crossing_the_server_cannot_follow_is_refused_by_name() {
        with_guests(|game| {
            let rx = scene(game, [0.5, 65.0, 8.25]);
            let level = game.level(game.dim.id()).unwrap();
            let away = Passage {
                destination: Vec3::new(100.5, 65.5, 8.5),
                ..doorway()
            };
            assert_eq!(
                game.level_register_passage(level, away),
                Ok(PassageId::from_raw(2))
            );
            let refused = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                           id: u64,
                           why: Refusal,
                           at: [f64; 3]| {
                let sent = packets(&rx);
                let reason = refusal_code(why);
                assert_eq!(
                    refusals(&sent),
                    [Message::Refused { id, reason }],
                    "{why:?}"
                );
                let back = position_packets(&sent);
                assert_eq!(back.len(), 1, "{why:?}");
                assert_eq!((back[0].position, back[0].relatives), (at, 0), "{why:?}");
                assert_eq!(guest(game).position, at, "{why:?}");
                back[0].teleport
            };
            let here = [0.5, 65.0, 8.25];
            tick(game, vec![cross(9, here)]);
            refused(game, 9, Refusal::UnknownPassage, here);
            tick(game, vec![cross(1, here)]);
            refused(game, 1, Refusal::Pending, here);
            let accept = |id| {
                S::AcceptTeleportation(play::AcceptTeleportation {
                    id,
                    x: here[0],
                    y: here[1],
                    z: here[2],
                    yaw: 0.0,
                    pitch: 0.0,
                })
            };
            let latest = guest(game).awaiting_teleport.unwrap().0;
            tick(game, vec![accept(latest), cross(2, here)]);
            let teleport = refused(game, 2, Refusal::FarEndNotTracked, here);
            tick(game, vec![accept(teleport), cross(1, [0.5, 65.0, 40.0])]);
            refused(game, 1, Refusal::MoveRefused, here);

            let far = [0.5, 65.0, 29.0];
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.awaiting_teleport = None;
                g.position = far;
                g.previous_position = far;
                g.first_good = far;
            });
            tick(game, vec![cross(1, far)]);
            refused(game, 1, Refusal::TooFar { limit: 20 }, far);
        });
    }

    /// A passage to chunk (300, 0), far past the view, in the player's own level: with a mod's
    /// area there, a client that answered no revision 5 of `sunrise:levels` is refused
    /// `FarEndNotTracked`, as the area's chunk is not sent to it; once its client answers 5, the
    /// chunk is sent past the view and the crossing is followed.
    #[test]
    #[ignore = "daily"]
    fn a_far_end_an_own_level_area_sends_is_tracked() {
        use sunrise_sim::lan::viewed_levels::areas::{ChunkArea, ChunkAreaMode};
        with_guests(|game| {
            let rx = scene(game, [0.5, 65.0, 8.25]);
            let level = game.level(game.dim.id()).unwrap();
            let portal = Passage {
                destination: Vec3::new(4800.5, 65.5, 8.5),
                ..near_doorway()
            };
            assert_eq!(
                game.level_register_passage(level, portal),
                Ok(PassageId::from_raw(2))
            );
            let area = ChunkArea {
                level: game.dim.id().to_owned(),
                center: (300, 0),
                radius: 0,
                mode: ChunkAreaMode::Loaded,
            };
            let uuid = guest(game).uuid;
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.set_viewed_areas(uuid, "test:portal", vec![area])
                .unwrap();
            for cx in 299..=301 {
                for cz in -1..=1 {
                    let mut chunk = Chunk::empty(cx, cz);
                    chunk.lit = true;
                    game.common.world.chunks.insert((cx, cz), Arc::new(chunk));
                }
            }
            let here = [0.5, 65.0, 8.25];
            let send_far = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                for _ in 0..10 {
                    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                        g.unacknowledged = 0;
                    });
                    tick(game, Vec::new());
                }
                guest(game).sent_chunks.contains(&(300, 0))
            };
            assert!(!send_far(game), "nothing past the view of an older client");
            packets(&rx);
            tick(game, vec![cross(2, here)]);
            let sent = packets(&rx);
            let reason = refusal_code(Refusal::FarEndNotTracked);
            assert_eq!(refusals(&sent), [Message::Refused { id: 2, reason }]);
            let teleport = position_packets(&sent)[0].teleport;
            let accepted = S::AcceptTeleportation(play::AcceptTeleportation {
                id: teleport,
                x: here[0],
                y: here[1],
                z: here[2],
                yaw: 0.0,
                pitch: 0.0,
            });
            tick(game, vec![accepted]);

            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.sender.viewed_levels = sunrise_protocol::viewed_levels::Answer {
                    revision: sunrise_protocol::viewed_levels::REVISION_5,
                    max_levels: 2,
                };
            });
            assert!(send_far(game), "the area's chunk past the view");
            packets(&rx);
            tick(game, vec![cross(2, here)]);
            let sent = packets(&rx);
            assert!(refusals(&sent).is_empty(), "{:?}", refusals(&sent));
            assert_eq!(guest(game).position, [4800.75, 65.0, 8.5]);
            assert_eq!(position_packets(&sent)[0].relatives, ALL);
        });
    }

    /// What `player_passage_cross` heard.
    static HEARD: std::sync::Mutex<Vec<(PassageId, Vec3)>> = std::sync::Mutex::new(Vec::new());

    /// Cancels every crossing it hears.
    struct Veto;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Veto {
        fn player_passage_cross(
            &self,
            _: &mut dyn mod_api::ServerHost,
            e: &mut mod_api::PlayerPassageCrossEvent,
        ) -> mod_api::Flow {
            HEARD.lock().unwrap().push((e.passage, e.from));
            mod_api::Flow::Cancel
        }
    }

    /// A passage into another level is refused `FarLevel` however near the player is, and
    /// the player stays: crossing another level waits for clients that can follow.
    #[test]
    #[ignore = "daily"]
    fn a_crossing_into_another_level_is_refused_far_level() {
        with_guests(|game| {
            game.enter_level(Dim::Nether).unwrap();
            game.enter_level(Dim::Overworld).unwrap();
            let rx = scene(game, [0.5, 65.0, 8.25]);
            let level = game.level(game.dim.id()).unwrap();
            let far = Passage {
                destination_level: Some(DimensionKey::NETHER),
                ..near_doorway()
            };
            assert_eq!(
                game.level_register_passage(level, far),
                Ok(PassageId::from_raw(2))
            );
            tick(game, vec![cross(2, [0.5, 65.0, 8.25])]);
            let sent = packets(&rx);
            let reason = refusal_code(Refusal::FarLevel);
            assert_eq!(refusals(&sent), [Message::Refused { id: 2, reason }]);
            assert_eq!(position_packets(&sent)[0].relatives, 0);
            assert_eq!(guest(game).position, [0.5, 65.0, 8.25]);
        });
    }

    /// A mod hears the crossing, its passage and the feet before it, before the server
    /// follows, and cancels it: refused as `Vetoed`, and the player stays.
    #[test]
    #[ignore = "daily"]
    fn a_mod_vetoes_a_crossing_before_the_server_follows() {
        let mods = vec![mod_api::ModEntry::new("veto", Box::new(Veto))];
        mod_platform::testing::with_mods(mods.into(), || {
            with_guests(|game| {
                let rx = scene(game, [0.5, 65.0, 8.25]);
                tick(game, vec![cross(1, [0.5, 65.0, 8.25])]);
                let sent = packets(&rx);
                let reason = refusal_code(Refusal::Vetoed);
                assert_eq!(refusals(&sent), [Message::Refused { id: 1, reason }]);
                assert_eq!(position_packets(&sent)[0].relatives, 0);
                assert_eq!(guest(game).position, [0.5, 65.0, 8.25]);
            });
        });
        assert_eq!(
            *HEARD.lock().unwrap(),
            [(PassageId::from_raw(1), Vec3::new(0.5, 65.0, 8.25))]
        );
    }
}
