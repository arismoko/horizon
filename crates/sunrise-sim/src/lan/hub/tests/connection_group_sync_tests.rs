use super::*;
use crate::lan::viewed_levels::areas::{ChunkArea, ChunkAreaMode};
use sunrise_protocol::{
    buf::Reader,
    connection_groups::{self, Cell, Entry, Section},
    packets::play::Clientbound as C,
    viewed_levels,
};

fn transport(with_groups: bool) -> Arc<crate::raw_payloads::Transport> {
    let transport = crate::raw_payloads::Transport::capture(
        crate::raw_payloads::Side::Server,
        crate::lan::PAYLOAD_RECORD_OVERHEAD,
    )
    .unwrap();
    let entries: Vec<_> = transport
        .local
        .manifest
        .entries()
        .iter()
        .filter(|entry| with_groups || entry.id() != connection_groups::CHANNEL)
        .cloned()
        .collect();
    transport.accept(&entries).unwrap();
    transport.finish_configuration().unwrap();
    transport
}

fn bodies(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<Vec<u8>> {
    rx.try_iter()
        .flat_map(|out| match out {
            crate::lan::Out::Packet(body) => vec![body],
            crate::lan::Out::Packets(bodies) => bodies,
            crate::lan::Out::Disconnect(_) => panic!("unexpected disconnect"),
        })
        .collect()
}

fn decode(body: &[u8]) -> C {
    let mut r = Reader::new(body);
    let id = r.varint().unwrap();
    C::decode(id, r.rest()).unwrap()
}

fn groups(packet: &C) -> Section {
    let C::CustomPayload(payload) = packet else {
        panic!("missing group data");
    };
    assert_eq!(payload.channel, connection_groups::CHANNEL);
    Section::decode(&payload.data).unwrap()
}

fn section(at: [i32; 3], group: Option<u64>) -> Section {
    Section {
        pos: at.map(|v| v >> 4),
        entries: vec![Entry {
            cell: Cell::of(at),
            group,
        }],
    }
}

/// Vanilla body, followed by its group's section, in one wire bundle.
fn pair(bodies: &[Vec<u8>], expect: &Section) -> (Vec<u8>, C) {
    let [open, vanilla, group, close] = bodies else {
        panic!("not one pair: {} frames", bodies.len());
    };
    for delimiter in [open, close] {
        assert!(matches!(decode(delimiter), C::BundleDelimiter(_)));
    }
    assert_eq!(&groups(&decode(group)), expect);
    (vanilla.clone(), decode(vanilla))
}

fn hub_with_sunrise_and_vanilla() -> (
    Hub,
    std::sync::mpsc::Receiver<crate::lan::Out>,
    std::sync::mpsc::Receiver<crate::lan::Out>,
) {
    let (mut hub, _old) = hub_with_a_guest();
    let (tx_sunrise, rx_sunrise) = std::sync::mpsc::channel();
    let sunrise_sender = crate::lan::PlayerSender::new_with_payload_transport(
        tx_sunrise,
        Arc::default(),
        transport(true),
    );
    hub.guests.update(0, |g| {
        g.sender = sunrise_sender;
        g.max_unacknowledged = 64;
    });
    let (tx_vanilla, rx_vanilla) = std::sync::mpsc::channel();
    let mut vanilla = Guest::new(
        crate::lan::Joined {
            conn: 2,
            uuid: 2,
            name: "Vanilla".into(),
            properties: Vec::new(),
            info: sunrise_protocol::packets::configuration::ClientInformation::new(8),
            brand: "vanilla".into(),
            sender: crate::lan::PlayerSender::new(tx_vanilla, Arc::default()),
            latency: Arc::new(std::sync::atomic::AtomicI32::new(0)),
            address: "127.0.0.1:25566".parse().unwrap(),
            expected: false,
        },
        2,
        [0.5, 64.0, 0.5],
        8,
    );
    vanilla.placed = true;
    vanilla.dimension = "minecraft:overworld".into();
    vanilla.max_unacknowledged = 64;
    hub.guests.push(vanilla);
    (hub, rx_sunrise, rx_vanilla)
}

fn grouped_world() -> World {
    let mut world = World::new(sunrise_world::registry::BlockStateId(0), 0);
    for cz in -1..=1 {
        for cx in -1..=1 {
            let mut chunk = Chunk::empty(cx, cz);
            chunk.lit = true;
            if (cx, cz) == (0, 0) {
                chunk.set(3, 64, 5, 1);
                chunk.set_connection_group(3, 64, 5, Some(77));
            }
            world.chunks.insert((cx, cz), Arc::new(chunk));
        }
    }
    world
}

fn in_tick(world: &mut World, f: impl FnOnce(&mut TickCtx<'_>)) {
    f(&mut TickCtx {
        sea_level: 63,
        world,
        entities: &[],
        next_entity: &mut 10,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &crate::worldborder::WorldBorder::default(),
        streaming: None,
    });
}

#[test]
fn chunk_send_keeps_the_vanilla_body_for_both_kinds_of_unnegotiated_peer() {
    for vanilla in [true, false] {
        let (mut hub, rx_sunrise, mut rx_plain) = hub_with_sunrise_and_vanilla();
        if !vanilla {
            let (tx, rx) = std::sync::mpsc::channel();
            hub.guests.update(1, |g| {
                g.sender = crate::lan::PlayerSender::new_with_payload_transport(
                    tx,
                    Arc::default(),
                    transport(false),
                );
            });
            rx_plain = rx;
        }
        hub.ticks = 10;
        let mut world = grouped_world();
        in_tick(&mut world, |ctx| hub.update_chunks(ctx));
        let ours = bodies(&rx_sunrise);
        let plain = bodies(&rx_plain);
        let group = ours.iter().position(|body| matches!(decode(body), C::CustomPayload(p) if p.channel == connection_groups::CHANNEL)).unwrap();
        let (column_bytes, column) =
            pair(&ours[group - 2..=group + 1], &section([3, 64, 5], Some(77)));
        assert!(matches!(column, C::LevelChunkWithLight(p) if (p.x, p.z) == (0, 0)));
        assert!(
            plain
                .iter()
                .all(|p| !matches!(decode(p), C::BundleDelimiter(_)))
        );
        let vanilla_column = plain
            .iter()
            .find(|p| matches!(decode(p), C::LevelChunkWithLight(p) if (p.x, p.z) == (0, 0)))
            .unwrap();
        assert_eq!(&column_bytes, vanilla_column);
        assert!(
            ours[group + 2..]
                .iter()
                .any(|body| matches!(decode(body), C::ChunkBatchFinished(_)))
        );
    }
}

#[test]
fn corrective_block_update_bundles_its_group_or_clear_only_for_the_negotiated_holder() {
    let (mut hub, rx_sunrise, rx_vanilla) = hub_with_sunrise_and_vanilla();
    let mut world = grouped_world();
    let pos = [3, 64, 5];
    world.connection_group_log = Some(Default::default());
    hub.guests.update(0, |g| {
        g.sent_chunks.insert((0, 0));
    });
    for group in [Some(77), None] {
        if group.is_none() {
            assert!(world.invalidate_replaced_group((3, 64, 5)));
        }
        hub.send_authoritative_block(hub.guests[0].conn, &world, pos);
        hub.send_authoritative_block(hub.guests[1].conn, &world, pos);
        let (body, packet) = pair(&bodies(&rx_sunrise), &section(pos, group));
        assert!(matches!(packet, C::BlockUpdate(block) if block.pos == pos));
        assert_eq!(bodies(&rx_vanilla), [body]);
    }
}

#[test]
fn group_only_changes_publish_current_blocks_and_never_add_vanilla_traffic() {
    let (mut hub, rx_groups, rx_plain) = hub_with_sunrise_and_vanilla();
    let mut world = grouped_world();
    world.block_log = Some(Vec::new());
    world.connection_group_log = Some(Default::default());
    for i in 0..2 {
        hub.guests.update(i, |g| {
            g.sent_chunks.insert((0, 0));
        });
    }
    let at = [3, 64, 5];
    let state = sunrise_world::registry::BlockStateId(2);
    // This replacement uses no vanilla notification. The next real group
    // change must nevertheless arrive with the cell's authoritative state.
    world.set_with_notification(3, 64, 5, state, false).unwrap();
    assert!(world.set_connection_group(3, 64, 5, Some(100), true));
    assert!(world.set_connection_group(3, 64, 5, Some(101), true));
    // This group's column has not been sent to either player.
    assert!(world.set_connection_group(17, 64, 1, Some(99), true));
    hub.broadcast_block_changes(&mut world);
    let (_, p) = pair(&bodies(&rx_groups), &section(at, Some(101)));
    assert!(matches!(p, C::BlockUpdate(p) if p.pos == at && p.state == state.0 as i32));
    assert!(bodies(&rx_plain).is_empty());
    assert!(world.connection_group_log.as_ref().unwrap().is_empty());

    // A normal scheduled update consumes that cell's group change in its own
    // bundle. The ordinary recipient sees the same bytes, once, no delimiters.
    world.block_log.as_mut().unwrap().push((3, 64, 5));
    assert!(world.invalidate_replaced_group((3, 64, 5)));
    hub.broadcast_block_changes(&mut world);
    let (body, _) = pair(&bodies(&rx_groups), &section(at, None));
    assert_eq!(bodies(&rx_plain), [body]);

    // A scheduled block in the next tick uses the existing held block log.
    world.block_log_held.push((3, 64, 5));
    assert!(world.set_connection_group(3, 64, 5, Some(40), true));
    hub.broadcast_block_changes(&mut world);
    assert!(bodies(&rx_groups).is_empty());
    world.block_log = Some(std::mem::take(&mut world.block_log_held));
    hub.broadcast_block_changes(&mut world);
    let (body, _) = pair(&bodies(&rx_groups), &section(at, Some(40)));
    assert_eq!(bodies(&rx_plain), [body]);

    // A bare bulk column is authoritative for an empty group map.
    assert!(world.invalidate_replaced_group((3, 64, 5)));
    world.bulk_log.insert((0, 0));
    hub.broadcast_block_changes(&mut world);
    let got = bodies(&rx_groups);
    assert!(matches!(got.as_slice(), [body] if matches!(decode(body), C::LevelChunkWithLight(_))));
    assert_eq!(bodies(&rx_plain), got);
}

#[test]
fn chunk_snapshot_has_at_most_one_payload_for_each_occupied_section() {
    let mut chunk = Chunk::empty(0, 0);
    for i in 0..4097 {
        chunk.set_connection_group(i & 15, i >> 8, (i >> 4) & 15, Some(i as u64));
    }
    let bodies = block_changes::encode_chunk_connection_groups(&chunk);
    let [a, b] = bodies.as_slice() else {
        panic!("expected two sections");
    };
    let (a, b) = (groups(&decode(a)), groups(&decode(b)));
    assert_eq!((a.pos, a.entries.len()), ([0, 0, 0], 4096));
    assert_eq!(b, section([0, 16, 0], Some(4096)));
}

#[test]
fn viewed_group_bundle_keeps_a_normal_scoped_run_first_and_omits_metadata_without_negotiation() {
    let (mut hub, rx_groups, rx_plain) = hub_with_sunrise_and_vanilla();
    hub.ticks = 10;
    let levels = hub.level_keys();
    for i in 0..2 {
        hub.guests.update(i, |g| {
            g.sender.viewed_levels = viewed_levels::Answer {
                revision: viewed_levels::REVISION,
                max_levels: 2,
            };
            g.batch_quota = 9.0;
            g.viewed.logged_in_with(levels.clone());
        });
        hub.set_viewed_areas(
            hub.guests[i].uuid,
            "test:portal",
            vec![ChunkArea {
                level: "minecraft:the_nether".into(),
                center: (0, 0),
                radius: 0,
                mode: ChunkAreaMode::Loaded,
            }],
        )
        .unwrap();
    }
    hub.set_dimension("minecraft:the_nether");
    let mut world = grouped_world();
    in_tick(&mut world, |ctx| assert!(hub.viewed_pass(ctx)));
    let scoped = |body: &[u8]| {
        let C::CustomPayload(cp) = decode(body) else {
            return None;
        };
        let Ok(viewed_levels::Message::Run { level, packets }) =
            viewed_levels::Message::decode(&cp.data)
        else {
            return None;
        };
        let mut vanilla = Vec::new();
        let mut sections = Vec::new();
        for packet in packets {
            match packet {
                viewed_levels::ScopedPacket::Play(id, data) => {
                    let mut w = sunrise_core::vanilla_bytes::Writer::new();
                    w.varint(id);
                    w.bytes(data);
                    vanilla.push(w.data);
                }
                viewed_levels::ScopedPacket::ConnectionGroups(body) => {
                    sections.push(Section::decode(body).unwrap());
                }
            }
        }
        Some((level, vanilla, sections))
    };
    let our = bodies(&rx_groups);
    let plain = bodies(&rx_plain);
    let start = our
        .iter()
        .position(|b| matches!(decode(b), C::BundleDelimiter(_)))
        .unwrap();
    let (level, column, no_metadata) = scoped(&our[start + 1]).unwrap();
    let (group_level, no_vanilla, metadata) = scoped(&our[start + 2]).unwrap();
    assert_eq!(level, group_level);
    assert!(no_metadata.is_empty() && no_vanilla.is_empty());
    assert_eq!(metadata, [section([3, 64, 5], Some(77))]);
    assert!(matches!(decode(&our[start + 3]), C::BundleDelimiter(_)));
    assert!(
        plain
            .iter()
            .all(|p| !matches!(decode(p), C::BundleDelimiter(_)))
    );
    let plain_column = plain
        .iter()
        .filter_map(|p| scoped(p))
        .flat_map(|(_, packets, sections)| {
            assert!(sections.is_empty());
            packets
        })
        .find(|p| matches!(decode(p), C::LevelChunkWithLight(_)))
        .unwrap();
    assert_eq!(column, [plain_column]);
}
