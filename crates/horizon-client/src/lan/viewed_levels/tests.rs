//! The hub's side of viewed levels: what a viewing client is sent, the linger, the refusals, and
//! that an area is never one of a level's players.
use super::areas::{ChunkArea, ChunkAreaMode, ChunkAreaRefusal, ChunkCap, ViewedCaps};
use super::view::LINGER;
use crate::lan::{
    Out,
    data::SyncData,
    guest::Guest,
    hub::{Hub, HubConfig, TickCtx},
};
use crate::world::World;
use horizon_protocol::{
    buf::{Reader, Writer},
    ids::play::clientbound as ids,
    packets::play::Clientbound as C,
    viewed_levels::{CHANNEL, Message},
};
use std::sync::mpsc::Receiver;

const NETHER: &str = "minecraft:the_nether";
/// The Nether's index in the Login packet's levels (overworld, the_end, the_nether).
const NETHER_INDEX: i32 = 2;

/// A hub with the players `uuids`, each placed in the Overworld at (0.5, 64, 0.5) with a client
/// that views up to `max_levels` other levels and a batch quota of 9; each one's packets.
fn hub_with(uuids: &[u128], max_levels: u8) -> (Hub, Vec<Receiver<Out>>) {
    let config = crate::lan::NetConfig {
        announce: false,
        ..crate::lan::NetConfig::loopback(0, "ViewedLevelsTest".into(), 4)
    };
    let sync = std::sync::Arc::new(SyncData::empty());
    let lan = crate::lan::Lan::start_with(sync.clone(), config).unwrap();
    let encoder = crate::lan::chunks::ChunkEncoder::new(
        [] as [(std::sync::Arc<str>, bool); 0],
        &horizon_protocol::registry::Builtin::default(),
        &[],
        &[],
    );
    let hub_config = HubConfig {
        default_game_mode: 0,
        guest_commands: false,
        force_game_mode: false,
        hardcore: false,
        view_distance: 10,
        simulation_distance: 10,
        dimension: "minecraft:overworld".into(),
        allow_flight: false,
        seed: 0,
        max_players: 20,
        integrated: false,
        levels: Default::default(),
    };
    let mut hub = Hub::new(
        lan,
        sync,
        encoder,
        hub_config,
        crate::test_resources::loop_context(horizon_world::native_load_context::LoopSide::Server),
    );
    hub.ticks = 10;
    let mut receivers = Vec::new();
    for &uuid in uuids {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut sender = crate::lan::PlayerSender::new(tx, std::sync::Arc::default());
        sender.max_viewed_levels = max_levels;
        let mut g = Guest::new(
            crate::lan::Joined {
                conn: uuid as u32,
                uuid,
                name: format!("P{uuid}"),
                properties: Vec::new(),
                info: horizon_protocol::packets::configuration::ClientInformation::new(8),
                brand: "horizon".into(),
                sender,
                latency: std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            uuid as i32,
            [0.5, 64.0, 0.5],
            8,
        );
        g.placed = true;
        g.dimension = "minecraft:overworld".into();
        g.batch_quota = 9.0;
        g.max_unacknowledged = 10;
        hub.guests.push(g);
        receivers.push(rx);
    }
    (hub, receivers)
}

fn area(level: &str, center: (i32, i32), radius: u32, mode: ChunkAreaMode) -> ChunkArea {
    ChunkArea {
        level: level.into(),
        center,
        radius,
        mode,
    }
}

fn nether(radius: u32) -> Vec<ChunkArea> {
    vec![area(NETHER, (0, 0), radius, ChunkAreaMode::Loaded)]
}

/// A Nether with lit columns out to `r` around (0, 0).
fn lit_columns(r: i32) -> World {
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    for z in -r..=r {
        for x in -r..=r {
            let mut chunk = crate::chunk::Chunk::empty(x, z);
            chunk.lit = true;
            world.chunks.insert((x, z), std::sync::Arc::new(chunk));
        }
    }
    world
}

/// One level pass of the level in place.
fn run_level(hub: &mut Hub, world: &mut World) {
    let mut next_entity = 100;
    let border = crate::worldborder::WorldBorder::default();
    let mut ctx = TickCtx {
        sea_level: 63,
        world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    };
    let _ = hub.tick_level(&mut ctx);
}

/// A packet a client was sent: a plain one, or what `horizon:levels` carried.
#[derive(Debug, PartialEq)]
enum Got {
    Plain(i32),
    Open(i32),
    Wrapped { level: i32, id: i32, body: Vec<u8> },
    Close(i32),
}

fn got(rx: &Receiver<Out>) -> Vec<Got> {
    let mut out = Vec::new();
    let bodies = rx.try_iter().flat_map(|o| match o {
        Out::Packet(body) => vec![body],
        Out::Packets(bodies) => bodies,
        Out::Disconnect(_) => Vec::new(),
    });
    for body in bodies {
        let mut r = Reader::new(&body);
        let id = r.varint().unwrap();
        let payload = match C::decode(id, r.rest()) {
            Ok(C::CustomPayload(p)) if p.channel == CHANNEL => p,
            _ => {
                out.push(Got::Plain(id));
                continue;
            }
        };
        match Message::decode(&payload.data).unwrap() {
            Message::Open { level, .. } => out.push(Got::Open(level)),
            Message::Close { level } => out.push(Got::Close(level)),
            Message::Run { level, packets } => {
                out.extend(packets.into_iter().map(|(id, body)| Got::Wrapped {
                    level,
                    id,
                    body: body.to_vec(),
                }))
            }
        }
    }
    out
}

/// The columns of the wrapped packets of kind `id` (chunks and forgets), in order.
fn wrapped_columns(got: &[Got], kind: i32) -> Vec<(i32, i32)> {
    got.iter()
        .filter_map(|g| match g {
            Got::Wrapped { level, id, body } if *id == kind => {
                assert_eq!(*level, NETHER_INDEX);
                match C::decode(*id, body).unwrap() {
                    C::LevelChunkWithLight(p) => Some((p.x, p.z)),
                    C::ForgetLevelChunk(p) => Some((p.pos[0], p.pos[1])),
                    other => panic!("{other:?}"),
                }
            }
            _ => None,
        })
        .collect()
}

/// A player in the Overworld whose client views 2 levels, with a Nether area: the Nether is
/// opened with its clock, then its nearest 9 chunks go wrapped in a plain batch; a block change
/// in a sent column follows them wrapped.
#[test]
fn a_nether_area_is_sent_wrapped_in_a_plain_batch() {
    let (mut hub, rx) = hub_with(&[1], 2);
    hub.set_viewed_areas(1, "test:portal", nether(2)).unwrap();
    hub.set_dimension(NETHER);
    let mut world = lit_columns(3);
    run_level(&mut hub, &mut world);
    let sent = got(&rx[0]);
    assert_eq!(sent[0], Got::Open(NETHER_INDEX), "{sent:?}");
    assert!(
        matches!(sent[1], Got::Wrapped { level: NETHER_INDEX, id, .. } if id == ids::SET_TIME),
        "{sent:?}"
    );
    assert_eq!(sent[2], Got::Plain(ids::CHUNK_BATCH_START));
    assert_eq!(sent.last(), Some(&Got::Plain(ids::CHUNK_BATCH_FINISHED)));
    let columns = wrapped_columns(&sent, ids::LEVEL_CHUNK_WITH_LIGHT);
    assert_eq!(columns.len(), 9, "the batch quota");
    assert_eq!(columns[0], (0, 0));
    assert!(columns.iter().all(|&(x, z)| x.abs() <= 1 && z.abs() <= 1));
    assert!(!sent.contains(&Got::Plain(ids::LEVEL_CHUNK_WITH_LIGHT)));
    let g = &hub.guests[0];
    assert!(g.sent_chunks.is_empty(), "its own level's chunks are apart");
    assert_eq!(g.unacknowledged, 1);
    // A change in a sent column reaches it wrapped; one in a column not sent does not.
    world.block_log = Some(vec![(3, 64, 3), (40, 64, 40)]);
    hub.guests.update(0, |g| g.batch_quota = 0.0);
    run_level(&mut hub, &mut world);
    let updates: Vec<_> = got(&rx[0])
        .into_iter()
        .filter_map(|g| match g {
            Got::Wrapped { level, id, body } if id == ids::BLOCK_UPDATE => {
                let Ok(C::BlockUpdate(p)) = C::decode(id, &body) else {
                    panic!("not a block update");
                };
                Some((level, p.pos))
            }
            Got::Plain(id) => panic!("plain packet {id}"),
            _ => None,
        })
        .collect();
    assert_eq!(updates, vec![(NETHER_INDEX, [3, 64, 3])]);
}

/// A vanilla client (no capability answer, so 0 levels) gets no area in another level and
/// nothing of it; an area in its own level holds its ticket and sends nothing past its view.
#[test]
fn a_vanilla_client_gets_nothing() {
    let (mut hub, rx) = hub_with(&[1], 0);
    assert_eq!(
        hub.set_viewed_areas(1, "test:portal", nether(1)),
        Err(ChunkAreaRefusal::TooManyLevels { asked: 1, cap: 0 })
    );
    let own = vec![area(
        "minecraft:overworld",
        (40, 40),
        1,
        ChunkAreaMode::Loaded,
    )];
    hub.set_viewed_areas(1, "test:portal", own).unwrap();
    assert_eq!(hub.viewed_tickets("minecraft:overworld").len(), 1);
    hub.set_dimension(NETHER);
    let mut world = lit_columns(2);
    run_level(&mut hub, &mut world);
    hub.set_dimension("minecraft:overworld");
    run_level(&mut hub, &mut world);
    let sent = got(&rx[0]);
    assert!(
        sent.iter().all(|g| matches!(g, Got::Plain(_))),
        "nothing on horizon:levels: {sent:?}"
    );
}

/// A withdrawn area's chunks stay on the client 52 ticks, then are forgotten, then the level is
/// closed; nothing more follows.
#[test]
fn a_withdrawn_areas_chunks_are_forgotten_after_the_linger_then_closed() {
    assert_eq!(LINGER, 52);
    let (mut hub, rx) = hub_with(&[1], 2);
    hub.set_viewed_areas(1, "test:portal", nether(1)).unwrap();
    hub.set_dimension(NETHER);
    let mut world = lit_columns(2);
    run_level(&mut hub, &mut world);
    assert_eq!(
        wrapped_columns(&got(&rx[0]), ids::LEVEL_CHUNK_WITH_LIGHT).len(),
        9
    );
    hub.set_viewed_areas(1, "test:portal", Vec::new()).unwrap();
    hub.ticks = 11;
    run_level(&mut hub, &mut world);
    hub.ticks = 11 + 51;
    run_level(&mut hub, &mut world);
    assert!(
        wrapped_columns(&got(&rx[0]), ids::FORGET_LEVEL_CHUNK).is_empty(),
        "not before 52 ticks"
    );
    hub.ticks = 11 + 52;
    run_level(&mut hub, &mut world);
    let last = got(&rx[0]);
    let mut forgotten = wrapped_columns(&last, ids::FORGET_LEVEL_CHUNK);
    forgotten.sort_unstable();
    let expected: Vec<_> = (-1..=1)
        .flat_map(|x| (-1..=1).map(move |z| (x, z)))
        .collect();
    assert_eq!(forgotten, expected);
    assert_eq!(last.last(), Some(&Got::Close(NETHER_INDEX)));
    assert!(hub.guests[0].viewed.view(NETHER).is_none());
    assert!(!hub.viewed.active(), "no area nor view left on the server");
    hub.ticks += 1;
    run_level(&mut hub, &mut world);
    assert_eq!(got(&rx[0]), Vec::new());
}

/// Each limit refuses with its cap's name and value, and the areas asked before stay.
#[test]
fn each_limit_refuses_with_its_cap_and_keeps_the_old_areas() {
    use ChunkAreaMode::{Loaded, Ticking};
    let (mut hub, _rx) = hub_with(&[1, 2], 2);
    let refused = |hub: &mut Hub, uuid, areas| hub.set_viewed_areas(uuid, "test:portal", areas);
    hub.set_viewed_areas(1, "test:portal", nether(4)).unwrap();
    assert_eq!(
        refused(&mut hub, 9, nether(1)),
        Err(ChunkAreaRefusal::NotOnline)
    );
    assert_eq!(
        refused(&mut hub, 1, nether(33)),
        Err(ChunkAreaRefusal::RadiusTooLarge {
            radius: 33,
            max: 32
        })
    );
    let many: Vec<_> = (0..33)
        .map(|i| area(NETHER, (i * 3, 0), 0, Loaded))
        .collect();
    assert_eq!(
        refused(&mut hub, 1, many),
        Err(ChunkAreaRefusal::TooManyAreas { asked: 33, max: 32 })
    );
    let three = vec![
        area(NETHER, (0, 0), 0, Loaded),
        area("minecraft:the_end", (0, 0), 0, Loaded),
        area("test:third", (0, 0), 0, Loaded),
    ];
    assert_eq!(
        refused(&mut hub, 1, three),
        Err(ChunkAreaRefusal::TooManyLevels { asked: 3, cap: 2 })
    );
    let chunks = |cap, asked, limit| {
        Err(ChunkAreaRefusal::TooManyChunks {
            cap,
            level: NETHER.into(),
            asked,
            limit,
        })
    };
    // Radius 21 holds (2r + 3)² = 2025 chunks; a ticking chunk far away adds the 24 of its rings.
    hub.set_viewed_areas(2, "test:portal", vec![area(NETHER, (0, 0), 21, Loaded)])
        .unwrap();
    assert_eq!(
        refused(
            &mut hub,
            1,
            vec![
                area(NETHER, (0, 0), 21, Loaded),
                area(NETHER, (100, 0), 0, Ticking)
            ]
        ),
        chunks(ChunkCap::LoadedPerPlayer, 2049, 2048)
    );
    assert_eq!(
        refused(&mut hub, 1, vec![area(NETHER, (0, 0), 11, Ticking)]),
        chunks(ChunkCap::TickingPerPlayer, 529, 441)
    );
    hub.viewed.caps = ViewedCaps {
        loaded_per_level: 4000,
        ticking_per_level: 800,
        ..ViewedCaps::default()
    };
    assert_eq!(
        refused(&mut hub, 1, vec![area(NETHER, (0, 0), 21, Loaded)]),
        chunks(ChunkCap::LoadedPerLevel, 2025 + 2025, 4000)
    );
    hub.set_viewed_areas(2, "test:portal", vec![area(NETHER, (0, 0), 10, Ticking)])
        .unwrap();
    assert_eq!(
        refused(&mut hub, 1, vec![area(NETHER, (0, 0), 10, Ticking)]),
        chunks(ChunkCap::TickingPerLevel, 441 + 441, 800)
    );
    assert_eq!(hub.viewed_areas(1, "test:portal"), nether(4));
}

/// A player in the Nether with an Overworld area is not one of the Overworld's players: it is
/// not here, not watching, holds no player ticket and is not counted where the level's players
/// are; its area is its own ticket.
#[test]
fn an_area_is_not_a_player() {
    let (mut hub, _rx) = hub_with(&[1], 2);
    hub.guests.update(0, |g| g.dimension = NETHER.into());
    let overworld = vec![area(
        "minecraft:overworld",
        (40, 40),
        2,
        ChunkAreaMode::Ticking,
    )];
    hub.set_viewed_areas(1, "test:portal", overworld).unwrap();
    let g = &hub.guests[0];
    assert!(g.views_level("minecraft:overworld"));
    assert!(!hub.here(g) && !hub.watches(g) && !g.placed_in("minecraft:overworld"));
    assert_eq!(hub.loading_chunks(|_| true).count(), 0);
    assert_eq!(
        hub.viewed_tickets("minecraft:overworld"),
        vec![crate::ticket_levels::ViewedTicket {
            center: (40, 40),
            radius: 2,
            ticking: true
        }]
    );
    assert_eq!(hub.viewed_level("minecraft:overworld"), Some(true));
}

/// A player that disconnects takes its areas, their tickets and their squares with it.
#[test]
fn disconnect_drops_the_areas_and_their_tickets() {
    let (mut hub, _rx) = hub_with(&[1], 2);
    hub.set_viewed_areas(1, "test:portal", nether(3)).unwrap();
    assert_eq!(hub.viewed_extra(NETHER), vec![((0, 0), 3)]);
    hub.remove_guest(0, "left");
    assert!(!hub.viewed.active());
    assert!(hub.viewed_tickets(NETHER).is_empty() && hub.viewed_extra(NETHER).is_empty());
    assert_eq!(hub.viewed_level(NETHER), None);
}

/// A client keeps at most 2048 columns of a viewed level in this protocol revision: once its
/// view holds that many, nothing more of the level is sent, though the areas may cover more.
#[test]
fn a_view_holding_2048_columns_is_sent_no_more() {
    let (mut hub, rx) = hub_with(&[1], 2);
    hub.set_viewed_areas(1, "test:portal", nether(2)).unwrap();
    hub.set_dimension(NETHER);
    let mut world = lit_columns(3);
    run_level(&mut hub, &mut world);
    assert_eq!(
        wrapped_columns(&got(&rx[0]), ids::LEVEL_CHUNK_WITH_LIGHT).len(),
        9
    );
    hub.guests.update(0, |g| {
        let view = g.viewed.views.get_mut(NETHER).unwrap();
        view.sent.extend((0..2048 - 9).map(|i| (1000 + i, 1000)));
        g.batch_quota = 9.0;
        g.unacknowledged = 0;
    });
    hub.ticks += 1;
    run_level(&mut hub, &mut world);
    let sent = got(&rx[0]);
    assert!(
        wrapped_columns(&sent, ids::LEVEL_CHUNK_WITH_LIGHT).is_empty(),
        "{sent:?}"
    );
    assert_eq!(hub.guests[0].viewed.view(NETHER).unwrap().queue.len(), 16);
}

/// A centre past the world border's 1,875,000 chunks on either axis is refused with that limit,
/// whatever the radius, and changes nothing; centres on the limit hold their squares and
/// tickets there.
#[test]
fn a_centre_past_the_world_border_is_refused() {
    use ChunkAreaMode::{Loaded, Ticking};
    let (mut hub, _rx) = hub_with(&[1], 2);
    hub.set_viewed_areas(1, "test:portal", nether(1)).unwrap();
    for center in [
        (i32::MAX, 0),
        (0, i32::MIN),
        (i32::MIN, i32::MAX),
        (1_875_001, 0),
        (0, -1_875_001),
    ] {
        assert_eq!(
            hub.set_viewed_areas(1, "test:portal", vec![area(NETHER, center, 0, Loaded)]),
            Err(ChunkAreaRefusal::CenterOutOfRange {
                center,
                max: 1_875_000
            }),
            "{center:?}"
        );
    }
    assert_eq!(hub.viewed_areas(1, "test:portal"), nether(1));
    let edge = vec![
        area(NETHER, (1_875_000, -1_875_000), 19, Loaded),
        area(NETHER, (-1_875_000, 1_875_000), 10, Ticking),
    ];
    hub.set_viewed_areas(1, "test:portal", edge.clone())
        .unwrap();
    assert_eq!(
        hub.viewed_extra(NETHER),
        vec![((1_875_000, -1_875_000), 19), ((-1_875_000, 1_875_000), 11)]
    );
    let mut tickets = crate::ticket_levels::ChunkTickets::default();
    tickets.update(Some(crate::ticket_levels::Sources {
        viewed: hub.viewed_tickets(NETHER),
        ..Default::default()
    }));
    let levels = tickets.levels();
    assert!(levels.entity_ticking(-1_874_990, 1_875_010));
    assert!(!levels.entity_ticking(-1_874_989, 1_875_000));
    assert!(
        !levels.block_ticking(1_875_000, -1_875_000),
        "Loaded ticks nothing"
    );
}

/// A column still waiting for its postprocessing (vanilla runs `postProcessGeneration` before
/// a chunk is sent) is held back until it is done; the others go meanwhile.
#[test]
fn a_column_waiting_for_its_postprocessing_is_not_sent() {
    let (mut hub, rx) = hub_with(&[1], 2);
    hub.set_viewed_areas(1, "test:portal", nether(1)).unwrap();
    hub.set_dimension(NETHER);
    let mut world = lit_columns(2);
    let mark = |world: &mut World, marks: Vec<[i32; 3]>| {
        std::sync::Arc::make_mut(world.chunks.get_mut(&(0, 0)).unwrap()).postprocess = marks;
    };
    mark(&mut world, vec![[3, 64, 3]]);
    run_level(&mut hub, &mut world);
    let mut first = wrapped_columns(&got(&rx[0]), ids::LEVEL_CHUNK_WITH_LIGHT);
    first.sort_unstable();
    assert_eq!(
        first,
        vec![
            (-1, -1),
            (-1, 0),
            (-1, 1),
            (0, -1),
            (0, 1),
            (1, -1),
            (1, 0),
            (1, 1)
        ]
    );
    mark(&mut world, Vec::new());
    hub.guests.update(0, |g| {
        g.batch_quota = 9.0;
        g.unacknowledged = 0;
    });
    hub.ticks += 1;
    run_level(&mut hub, &mut world);
    assert_eq!(
        wrapped_columns(&got(&rx[0]), ids::LEVEL_CHUNK_WITH_LIGHT),
        vec![(0, 0)]
    );
}

/// A column whose packet is too large to wrap is not counted in the batch, stays queued and is
/// not recorded as sent; a batch of none of them sends nothing.
#[test]
fn a_column_too_large_to_wrap_is_not_counted_as_sent() {
    use super::send::batch_runs;
    let (hub, _rx) = hub_with(&[1], 2);
    let world = lit_columns(1);
    let body = |key| hub.encoder.body_in(&world.chunks[&key], &world);
    // A LevelChunkWithLight id and 2 MiB after it: past a frame's limit once wrapped.
    let mut oversized = Writer::new();
    oversized.varint(ids::LEVEL_CHUNK_WITH_LIGHT);
    oversized.bytes(&vec![0; 1 << 21]);
    let taken = [(3, ()), (5, ()), (8, ())];
    let bodies = [body((0, 0)), oversized.data.clone(), body((1, 0))];
    let (payloads, sent) = batch_runs(NETHER_INDEX, &taken, &bodies);
    assert_eq!(sent, vec![3, 8], "the queue places of the columns sent");
    let mut columns = Vec::new();
    for payload in &payloads {
        let mut r = Reader::new(payload);
        let id = r.varint().unwrap();
        let Ok(C::CustomPayload(p)) = C::decode(id, r.rest()) else {
            panic!("not a custom payload");
        };
        let Message::Run { level, packets } = Message::decode(&p.data).unwrap() else {
            panic!("not a run");
        };
        assert_eq!(level, NETHER_INDEX);
        for (id, body) in packets {
            let Ok(C::LevelChunkWithLight(c)) = C::decode(id, body) else {
                panic!("not a chunk");
            };
            columns.push((c.x, c.z));
        }
    }
    assert_eq!(columns, vec![(0, 0), (1, 0)]);
    let (payloads, sent) = batch_runs(NETHER_INDEX, &[(0, ())], &[oversized.data]);
    assert!(payloads.is_empty() && sent.is_empty(), "no empty batch");
}
