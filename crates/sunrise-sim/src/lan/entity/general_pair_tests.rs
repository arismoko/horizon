//! A native mod's general rows in the tracker: introduced on `sunrise:general_entity` to the
//! clients that read it, at the declaration's range and interval, and never to another client.
use super::*;
use crate::fast_hash::FxHashSet;
use crate::lan::Body;
use sunrise_protocol::buf::Reader;

/// A live general row of type `kind` at x.
fn row(id: i32, kind: &str, x: f64) -> EntityState {
    let (tracking_chunks, update_interval) = match kind {
        "mymod:near" => (2, 1),
        "mymod:far" => (8, 10),
        "mymod:wide" => (320, 3),
        // Also exercise a row whose key cannot be encoded.
        _ => (5, 3),
    };
    let declared = mod_api::__private::GeneralEntityTypeDeclaration {
        tracking_chunks,
        update_interval,
        ..Default::default()
    };
    EntityState {
        id,
        uuid: 0x1000 + id as u128,
        kind: kind.into(),
        position: [x, 64.0, 0.5],
        yaw: 45.0,
        pitch: -10.0,
        velocity: [0.125, 0.0, 0.0],
        general: Some((&declared).into()),
        ..EntityState::default()
    }
}

fn decode(bytes: &[u8]) -> play::Clientbound {
    let mut r = Reader::new(bytes);
    let id = r.varint().unwrap();
    play::Clientbound::decode(id, r.rest()).unwrap()
}

/// The general spawns in `out`, decoded.
fn spawns(out: &[Body]) -> Vec<general_entity::Message> {
    out.iter()
        .filter_map(|b| match decode(b) {
            play::Clientbound::CustomPayload(p) if p.channel == general_entity::CHANNEL => {
                Some(general_entity::decode(&p.data).unwrap())
            }
            _ => None,
        })
        .collect()
}

fn spawn_ids(out: &[Body]) -> Vec<i32> {
    spawns(out)
        .into_iter()
        .map(|general_entity::Message::Spawn { id, .. }| id)
        .collect()
}

/// Whether any packet in `out` names entity `id` (an add, a spawn, a move, a sync or a removal).
fn names(out: &[Body], id: i32) -> bool {
    out.iter().any(|b| match decode(b) {
        play::Clientbound::MoveEntityPos(play::MoveEntityPos(m))
        | play::Clientbound::MoveEntityPosRot(play::MoveEntityPosRot(m))
        | play::Clientbound::MoveEntityRot(play::MoveEntityRot(m)) => m.id == id,
        play::Clientbound::EntityPositionSync(p) => p.id == id,
        play::Clientbound::RemoveEntities(p) => p.ids.contains(&id),
        play::Clientbound::AddEntity(p) => p.id == id,
        play::Clientbound::CustomPayload(p) => p.channel == general_entity::CHANNEL,
        _ => false,
    })
}

/// Where a client that got `out` puts entity `id`, decoding as the client does: the spawn's
/// position, then each move's delta from the position before it, or a sync's absolute one.
fn decoded(out: &[Body], id: i32) -> Option<[f64; 3]> {
    let mut at = None;
    for body in out {
        match decode(body) {
            play::Clientbound::CustomPayload(p) if p.channel == general_entity::CHANNEL => {
                let general_entity::Message::Spawn {
                    id: of, position, ..
                } = general_entity::decode(&p.data).unwrap();
                if of == id {
                    at = Some(position);
                }
            }
            play::Clientbound::MoveEntityPos(play::MoveEntityPos(m))
            | play::Clientbound::MoveEntityPosRot(play::MoveEntityPosRot(m))
                if m.id == id =>
            {
                if let (Some(base), Some(delta)) = (at, &m.delta) {
                    at = Some(delta.end(base));
                }
            }
            play::Clientbound::EntityPositionSync(p) if p.id == id => {
                at = Some(match p.path {
                    play::PositionPath::Linear(to) => to,
                    play::PositionPath::Stepped(steps) => steps.last().unwrap().0,
                });
            }
            _ => {}
        }
    }
    at
}

/// One client of the level, with what it was sent tick by tick.
struct Client {
    general_entity: bool,
    sources: Option<Vec<((i32, i32), i32)>>,
    view_distance: i32,
    tracker: Tracker,
    sent: Vec<Vec<Body>>,
}

impl Client {
    /// A client in the level that reads general spawns, or one that doesn't.
    fn new(general_entity: bool) -> Self {
        Self {
            general_entity,
            sources: None,
            view_distance: 10,
            tracker: Tracker::default(),
            sent: Vec::new(),
        }
    }

    fn all(&self) -> Vec<Body> {
        self.sent.concat()
    }
}

/// One tracking pass of `states`, each client at its `at`. A registry knows `minecraft:` keys
/// (as id 5) and no general one.
fn pass(level: &mut Entities, clients: &mut [Client], at: &[[f64; 3]], states: &[EntityState]) {
    let chunks: FxHashSet<(i32, i32)> = (-40..=40)
        .flat_map(|x| (-40..=40).map(move |z| (x, z)))
        .collect();
    let mut outs: Vec<Vec<Body>> = clients.iter().map(|_| Vec::new()).collect();
    let mut viewers: Vec<Viewer> = clients
        .iter_mut()
        .zip(outs.iter_mut())
        .zip(at)
        .map(|((c, out), &position)| Viewer {
            entity_id: 1,
            spectator: false,
            permission: 0,
            sunrise: true,
            general_entity: c.general_entity,
            attachment_sender: None,
            moved: true,
            position,
            view_distance: c.view_distance,
            chunks: &chunks,
            sources: c.sources.as_deref(),
            far: &[],
            tracker: &mut c.tracker,
            out,
        })
        .collect();
    let refs: Vec<&EntityState> = states.iter().collect();
    level.tick(&refs, &Riding::default(), &mut viewers, None, |kind| {
        kind.starts_with("minecraft:").then_some(5)
    });
    drop(viewers);
    for (c, out) in clients.iter_mut().zip(outs) {
        c.sent.push(out);
    }
}

/// A client that reads general spawns and one that doesn't, both at `at`, over `ticks` passes
/// of `rows(tick)`.
fn run(at: [f64; 3], ticks: u32, rows: impl Fn(u32) -> Vec<EntityState>) -> [Client; 2] {
    let mut level = Entities::default();
    let mut clients = [Client::new(true), Client::new(false)];
    for tick in 0..ticks {
        pass(&mut level, &mut clients, &[at, at], &rows(tick));
    }
    clients
}

#[test]
fn a_reading_client_in_range_gets_one_spawn_and_another_client_nothing() {
    let [sunrise, vanilla] = run([0.5, 64.0, 0.5], 3, |_| vec![row(7, "mymod:far", 4.5)]);
    let all = sunrise.all();
    assert_eq!(
        spawns(&all),
        [general_entity::Message::Spawn {
            id: 7,
            uuid: 0x1007,
            type_key: "mymod:far".into(),
            position: [4.5, 64.0, 0.5],
            rotation: [45.0, -10.0],
            velocity: [0.125, 0.0, 0.0],
        }]
    );
    assert!(
        !all.iter()
            .any(|b| matches!(decode(b), play::Clientbound::AddEntity(_)))
    );
    assert!(!names(&vanilla.all(), 7));
    assert!(!vanilla.tracker.is_tracking(7));
}

/// A saved tree whose root type is installed but a rider's is not is quarantined whole: its root
/// keeps the type's key, yet is no row (`EntityState::general` unset), and is never paired.
#[test]
fn a_quarantined_record_with_a_declared_key_is_never_paired() {
    let [sunrise, vanilla] = run([0.5, 64.0, 0.5], 3, |_| {
        vec![EntityState {
            general: None,
            ..row(7, "mymod:far", 4.5)
        }]
    });
    for client in [&sunrise, &vanilla] {
        assert!(!names(&client.all(), 7));
        assert!(!client.tracker.is_tracking(7));
    }
}

#[test]
fn the_declared_range_decides_who_pairs() {
    // 40 blocks away: past `near`'s 2 chunks (32 blocks), within `far`'s 8 (128).
    let [sunrise, _] = run([40.5, 64.0, 0.5], 1, |_| {
        vec![row(7, "mymod:near", 0.5), row(8, "mymod:far", 0.5)]
    });
    assert_eq!(spawn_ids(&sunrise.sent[0]), [8]);
}

/// `entity-broadcast-range-percentage` scales the declared range before anything caps it:
/// `mymod:wide`'s 320 chunks at 10% reach 512 blocks, past what a byte of chunks scaled
/// (25 chunks, 408 blocks) would.
#[test]
fn the_broadcast_percentage_scales_a_wide_declared_range() {
    let mut level = Entities {
        range_percent: Some(10),
        ..Entities::default()
    };
    let mut clients = [Client::new(true), Client::new(true)];
    for c in &mut clients {
        c.view_distance = 40;
    }
    let at = [[500.5, 64.0, 0.5], [520.5, 64.0, 0.5]];
    pass(&mut level, &mut clients, &at, &[row(7, "mymod:wide", 0.5)]);
    assert_eq!(spawn_ids(&clients[0].sent[0]), [7]);
    assert!(spawn_ids(&clients[1].sent[0]).is_empty());
}

#[test]
fn moves_go_out_at_the_declared_interval() {
    // Both rows move every tick; `near` updates every tick, `far` every 10.
    let moving = |tick: u32| {
        let x = 0.5 + f64::from(tick) * 0.25;
        vec![row(7, "mymod:near", x), row(8, "mymod:far", x)]
    };
    let [sunrise, _] = run([0.5, 64.0, 0.5], 21, moving);
    let moved = |id: i32| -> Vec<u32> {
        (1..21)
            .filter(|&t| {
                sunrise.sent[t as usize].iter().any(|b| match decode(b) {
                    play::Clientbound::MoveEntityPos(play::MoveEntityPos(m))
                    | play::Clientbound::MoveEntityPosRot(play::MoveEntityPosRot(m)) => m.id == id,
                    play::Clientbound::EntityPositionSync(p) => p.id == id,
                    _ => false,
                })
            })
            .collect()
    };
    assert_eq!(moved(7), (1..21).collect::<Vec<_>>());
    assert_eq!(moved(8).len(), 2, "{:?}", moved(8));
}

#[test]
fn leaving_range_removes_the_row_and_coming_back_spawns_it_afresh() {
    // The row walks out of `near`'s 32 blocks and back.
    let x = |tick: u32| match tick {
        0 => 0.5,
        1 => 100.5,
        _ => 1.5,
    };
    let [sunrise, _] = run([0.5, 64.0, 0.5], 3, |t| vec![row(7, "mymod:near", x(t))]);
    assert_eq!(spawn_ids(&sunrise.sent[0]), [7]);
    assert!(
        sunrise.sent[1]
            .iter()
            .any(|b| matches!(decode(b), play::Clientbound::RemoveEntities(p) if p.ids == [7]))
    );
    assert_eq!(spawn_ids(&sunrise.sent[2]), [7]);
}

/// A key the protocol spawn message cannot carry (past `general_entity::MAX_KEY`): no client is
/// introduced to the row, so none is recorded as tracking it or sent its moves.
#[test]
fn a_row_whose_spawn_cannot_be_written_is_tracked_by_nobody() {
    let long = format!("mymod:{}", "l".repeat(general_entity::MAX_KEY));
    let [sunrise, _] = run([0.5, 64.0, 0.5], 3, |tick| {
        vec![row(7, &long, 0.5 + f64::from(tick))]
    });
    assert!(!sunrise.tracker.is_tracking(7));
    assert!(!names(&sunrise.all(), 7));
}

/// A client that starts tracking a row between its updates is introduced at the position the
/// others were last told, so the next move's delta lands both on the same point.
#[test]
fn a_client_paired_between_updates_decodes_the_next_move_as_the_others_do() {
    let mut level = Entities::default();
    let mut clients = [Client::new(true), Client::new(true)];
    let (near, away) = ([0.5, 64.0, 0.5], [600.5, 64.0, 0.5]);
    // `far` updates every 10 ticks; the second client arrives at tick 3, between updates.
    for tick in 0..=21 {
        let at = [near, if tick < 3 { away } else { near }];
        let x = 0.5 + f64::from(tick) * 0.25;
        pass(&mut level, &mut clients, &at, &[row(7, "mymod:far", x)]);
    }
    assert_eq!(spawn_ids(&clients[1].sent[3]), [7]);
    // A move follows the late spawn, and both clients decode it to one point.
    assert!(clients[1].sent[4..].concat().iter().any(|b| matches!(
        decode(b),
        play::Clientbound::MoveEntityPos(_) | play::Clientbound::MoveEntityPosRot(_)
    )));
    let first = decoded(&clients[0].all(), 7).unwrap();
    let second = decoded(&clients[1].all(), 7).unwrap();
    assert_eq!(first, second);
}

/// A client viewing the level from another (#3716) is sent no general spawn, so a general row
/// takes none of the starts its cap allows: every other entity up to the cap is still started.
#[test]
fn general_rows_take_no_start_under_a_viewed_levels_cap() {
    let room = sunrise_protocol::viewed_levels::MAX_ENTITIES;
    let mut states = vec![row(1, "mymod:far", 0.5)];
    states.extend((0..room as i32).map(|i| EntityState {
        id: 100 + i,
        uuid: 0x10_0000 + i as u128,
        kind: "minecraft:item".into(),
        position: [0.5, 64.0, 0.5],
        ..EntityState::default()
    }));
    let mut level = Entities::default();
    let mut viewer = Client::new(false);
    viewer.sources = Some(vec![((0, 0), 10)]);
    let mut clients = [viewer];
    pass(&mut level, &mut clients, &[[1.0e6, 64.0, 1.0e6]], &states);
    let tracker = &clients[0].tracker;
    assert!(!tracker.is_tracking(1));
    assert_eq!(
        (100..100 + room as i32)
            .filter(|&id| tracker.is_tracking(id))
            .count(),
        room
    );
}
