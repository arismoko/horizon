//! The server's check of a player a moving collider carries (`lan/hub/carried_check.rs`): a
//! client moves its player with the collider as it last heard of it, some ticks old, and the
//! server judges each report in the collider's frames of the last ticks.
use super::*;
use std::{cell::RefCell, rc::Rc, sync::Arc, sync::atomic::Ordering};
use sunrise_core::geometry::{
    Aabb, Vec3,
    carried::Carried,
    oriented::{ColliderBoxes, Frame, OrientedCollider},
};
use sunrise_world::moving_colliders::{History, HistoryScope, Placed, Prepared, in_level};

/// Where the deck's origin starts, above a floor whose top is at y 5.
pub(super) const ORIGIN: [f64; 3] = [20.5, 9.0, 12.0];
/// The collider entities: not the guest's own entity id.
pub(super) const DECK: u32 = 100;
const OTHER: u32 = 101;

/// A stone floor with its top at y 5, and `walls`: blocks (x, z) filled from the floor up.
pub(super) fn scene(fixture: &crate::test_fixtures::Vanilla, walls: &[(i32, i32)]) -> World {
    let assets = &fixture.assets;
    let mut scene = World::test_scene_with_placements(assets, &[], 10).unwrap();
    let stone = assets.registry.default_state("minecraft:stone").unwrap();
    let air = assets.registry.default_state("minecraft:air").unwrap();
    for x in 10..=30 {
        for z in 0..=170 {
            scene.set(x, 4, z, stone).unwrap();
            for y in 5..=14 {
                let wall = walls.contains(&(x, z)) && y < 14;
                scene.set(x, y, z, if wall { stone } else { air }).unwrap();
            }
        }
    }
    scene
}

pub(super) fn deck(length: f64) -> Arc<ColliderBoxes> {
    Arc::new(ColliderBoxes::new(vec![Aabb::new(
        Vec3::new(-1.5, -1.0, -length / 2.0),
        Vec3::new(1.5, 0.0, length / 2.0),
    )]))
}

/// The server's side of one level: colliders that move, and the history of where they were.
pub(super) struct Rig {
    history: Rc<RefCell<History>>,
    /// Each collider's entity, boxes and frame now.
    colliders: Vec<(u32, Arc<ColliderBoxes>, Frame)>,
}

impl Rig {
    pub(super) fn new(colliders: Vec<(u32, Arc<ColliderBoxes>, Frame)>) -> Self {
        let mut rig = Self {
            history: Rc::default(),
            colliders,
        };
        rig.record();
        rig
    }

    fn prepared(&self) -> Prepared {
        Prepared::new(
            self.colliders
                .iter()
                .map(|(id, boxes, frame)| Placed {
                    id: *id,
                    collider: OrientedCollider::new(*frame, boxes.clone()),
                    exempt: vec![*id],
                })
                .collect(),
        )
    }

    fn record(&mut self) {
        self.history.borrow_mut().record(&self.prepared());
    }

    /// A server tick: every collider moves to `frame(entity)` and the level records them.
    pub(super) fn tick(&mut self, frame: impl Fn(u32) -> Frame) {
        for (id, _, at) in &mut self.colliders {
            *at = frame(*id);
        }
        self.record();
    }

    /// Runs `f` as the level's tick holds its scopes.
    pub(super) fn within<R>(&self, f: impl FnOnce() -> R) -> R {
        let _history = HistoryScope::enter(self.history.clone());
        in_level(self.prepared(), f)
    }

    /// The frame `age` ticks ago.
    pub(super) fn frame(&self, id: u32, age: f64) -> Frame {
        let history = self.history.borrow();
        let (log, _) = history.get(id).unwrap();
        log.frame_at(history.now(), age).unwrap()
    }
}

pub(super) fn along(z: f64) -> Frame {
    Frame::translation(Vec3::new(ORIGIN[0], ORIGIN[1], z))
}

/// A tiny deterministic generator.
struct Lcg(u64);

impl Lcg {
    fn unit(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

pub(super) struct Guestside<'a> {
    pub(super) hub: Hub,
    fixture: &'a crate::test_fixtures::Vanilla,
    pub(super) world: World,
    border: crate::worldborder::WorldBorder,
    /// What the hub warned of in the last report (`moved wrongly`, `moved too quickly`).
    warnings: Vec<String>,
    /// Where the last refusal put the player back.
    corrected: Option<[f64; 3]>,
    /// Whether the client answers a correction at once (the default); a test that looks at the
    /// server while the correction is on its way turns it off and calls [`Self::confirm`].
    answers: bool,
    /// What the hub sent the guest: see [`Self::sent`].
    rx: std::sync::mpsc::Receiver<crate::lan::Out>,
}

impl<'a> Guestside<'a> {
    pub(super) fn new(
        fixture: &'a crate::test_fixtures::Vanilla,
        walls: &[(i32, i32)],
        rtt_ms: i32,
    ) -> Self {
        let (mut hub, rx) = hub_with_a_guest();
        hub.guests.update(0, |g| {
            g.latency.store(rtt_ms, Ordering::Relaxed);
            g.on_ground = true;
            // The client has loaded: its moves are read.
            g.load_timeout = 0;
        });
        Self {
            hub,
            fixture,
            world: scene(fixture, walls),
            border: Default::default(),
            warnings: Vec::new(),
            corrected: None,
            answers: true,
            rx,
        }
    }

    /// One server tick of the hub with these packets in the guest's inbox: the same handling a
    /// client's packets get (the speed test, the move check, the commit, the teleport handshake).
    pub(super) fn tick(&mut self, rig: &Rig, packets: Vec<S>) -> Vec<HostEvent> {
        for packet in packets {
            self.hub
                .guests
                .inbox_mut(0)
                .unwrap()
                .push_back(crate::lan::guest::InboxItem::uncredited(packet));
        }
        let mut next = 2;
        let mut ctx = TickCtx {
            sea_level: 63,
            world: &mut self.world,
            entities: &[],
            next_entity: &mut next,
            spawn: [20.5, 9.0, 12.0],
            spawn_angle: [0.0; 2],
            spawn_dimension: "minecraft:overworld",
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: Default::default(),
            collision: Some(&self.fixture.collision),
            border: &self.border,
            streaming: None,
        };
        let hub = &mut self.hub;
        rig.within(|| hub.drain_packets(&mut ctx))
    }

    /// A client's position report, with the age of its copy of the collider if it says, and the
    /// ground flag. Then, if the server refused it, the client's answer to the correction.
    /// Answers (moved wrongly, refused, stands).
    fn send(
        &mut self,
        rig: &Rig,
        to: [f64; 3],
        hint: Option<u8>,
        on_ground: bool,
    ) -> (bool, bool, bool) {
        self.hub.guests.update(0, |g| g.position_this_tick = false);
        let mut packets = Vec::new();
        if let Some(ticks) = hint {
            packets.push(S::CustomPayload(
                sunrise_protocol::packets::configuration::CustomPayload {
                    channel: sunrise_protocol::carry_age::CHANNEL.into(),
                    data: sunrise_protocol::carry_age::encode(sunrise_protocol::carry_age::Hint {
                        collider: DECK,
                        ticks,
                    }),
                },
            ));
        }
        packets.push(S::MovePlayerPos(play::MovePlayerPos {
            x: to[0],
            y: to[1],
            z: to[2],
            flags: u8::from(on_ground),
        }));
        let handled = (
            self.hub.guests[0].received_moves,
            self.hub.guests[0].moves.len(),
        );
        let events = self.tick(rig, packets);
        // The move was read: counted, and either committed or answered with a teleport to wait
        // for. A guest that silently ignores its input fails here, not by passing.
        let g = &self.hub.guests[0];
        assert_eq!(g.received_moves, handled.0 + 1, "the move was read");
        assert!(
            g.moves.len() == handled.1 + 1 || g.awaiting_teleport.is_some(),
            "the move was committed or a teleport is awaited"
        );
        self.warnings = events
            .into_iter()
            .filter_map(|e| match e {
                HostEvent::Warn(line) => Some(line),
                _ => None,
            })
            .collect();
        let refused = self.hub.guests[0].awaiting_teleport.is_some();
        self.corrected = None;
        if refused {
            // A move while the correction is unanswered is ignored.
            let before = self.hub.guests[0].position;
            self.hub.guests.update(0, |g| g.position_this_tick = false);
            self.tick(
                rig,
                vec![S::MovePlayerPos(play::MovePlayerPos {
                    x: to[0] + 1.0,
                    y: to[1],
                    z: to[2],
                    flags: 1,
                })],
            );
            assert_eq!(self.hub.guests[0].position, before, "ignored while awaited");
            self.corrected = self.hub.guests[0].awaiting_teleport.map(|(_, at)| at);
            if self.answers {
                self.confirm(rig);
            }
        }
        let wrongly = self.warnings.iter().any(|w| w.contains("moved wrongly"));
        (wrongly, refused, self.hub.guests[0].rests_on_ground)
    }

    /// The client's answer to the server's teleport: it stands where the server put it.
    pub(super) fn confirm(&mut self, rig: &Rig) {
        let g = &self.hub.guests[0];
        let (id, at) = g.awaiting_teleport.expect("a teleport is awaited");
        let (yaw, pitch) = (g.yaw, g.pitch);
        self.tick(
            rig,
            vec![S::AcceptTeleportation(play::AcceptTeleportation {
                id,
                x: at[0],
                y: at[1],
                z: at[2],
                yaw,
                pitch,
            })],
        );
        assert!(self.hub.guests[0].awaiting_teleport.is_none(), "confirmed");
    }

    /// [`Self::send`] on the ground, no hint.
    pub(super) fn report(
        &mut self,
        rig: &Rig,
        to: [f64; 3],
        hint: Option<u8>,
    ) -> (bool, bool, bool) {
        self.send(rig, to, hint, true)
    }

    /// The packets the hub sent the guest since the last call, in order.
    pub(super) fn sent(&self) -> Vec<play::Clientbound> {
        let mut out = Vec::new();
        while let Ok(sent) = self.rx.try_recv() {
            let bodies = match sent {
                crate::lan::Out::Packet(body) => vec![body],
                crate::lan::Out::Packets(bodies) => bodies,
                crate::lan::Out::Disconnect(_) => Vec::new(),
            };
            for body in bodies {
                let mut reader = sunrise_protocol::buf::Reader::new(&body);
                let id = reader.varint().unwrap();
                if let Ok(packet) = play::Clientbound::decode(id, &body[reader.position()..]) {
                    out.push(packet);
                }
            }
        }
        out
    }

    pub(super) fn guest(&self) -> &Guest {
        &self.hub.guests[0]
    }
}

/// Where an honest client puts its player: the deck as it saw it `age` ticks ago.
pub(super) fn honest(rig: &Rig, age: f64, local: Vec3) -> [f64; 3] {
    let at = rig.frame(DECK, age).to_world(local);
    [at.x, at.y, at.z]
}

pub(super) fn first(fixture: &crate::test_fixtures::Vanilla) -> (Rig, Guestside<'_>) {
    let mut rig = Rig::new(vec![(DECK, deck(20.0), along(ORIGIN[2]))]);
    let mut side = Guestside::new(fixture, &[], 100);
    // Ten ticks standing still, the player on the deck.
    for _ in 0..10 {
        rig.tick(|_| along(ORIGIN[2]));
    }
    let to = honest(&rig, 4.0, Vec3::new(0.0, 0.0, 0.0));
    side.hub.guests.update(0, |g| g.position = to);
    (rig, side)
}

/// An honest rider is never refused, at any speed Create's trains run at, any round trip the
/// check allows for, jitter of a tick either way and several seeds: it attaches, stays
/// attached, is never counted as floating and no report is rejected.
#[test]
#[ignore = "daily"]
fn an_honest_rider_at_train_speed_is_never_rejected() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (mut spent, mut reports) = (std::time::Duration::ZERO, 0u32);
    for seed in [1, 2, 3] {
        for speed in [0.5, 1.4, 2.0] {
            for rtt in [50, 100, 200] {
                let mut rng = Lcg(seed * 7919 + (speed * 10.0) as u64 + rtt as u64);
                let mut rig = Rig::new(vec![(DECK, deck(20.0), along(ORIGIN[2]))]);
                let mut side = Guestside::new(fixture, &[], rtt);
                let trip = f64::from(rtt) / 50.0;
                let age = trip + 2.0;
                let mut local = Vec3::new(0.0, 0.0, 0.0);
                for t in 1..=60 {
                    rig.tick(|_| along(ORIGIN[2] + speed * f64::from(t)));
                    if t < 25 {
                        continue;
                    }
                    local = Vec3::new(
                        (local.x + (rng.unit() - 0.5) * 0.4).clamp(-1.2, 1.2),
                        0.0,
                        (local.z + (rng.unit() - 0.5) * 0.4).clamp(-8.0, 8.0),
                    );
                    // The jitter moves the copy the client held by a tick either way.
                    let jitter = (rng.unit() * 3.0).floor() - 1.0;
                    let to = honest(&rig, age + jitter, local);
                    if t == 25 {
                        side.hub.guests.update(0, |g| g.position = to);
                    }
                    let began = std::time::Instant::now();
                    let (_, refused, stands) = side.report(&rig, to, None);
                    spent += began.elapsed();
                    reports += 1;
                    assert!(
                        !refused,
                        "seed {seed} speed {speed} rtt {rtt} tick {t}: refused at {to:?}"
                    );
                    assert!(stands, "supported");
                    assert!(side.guest().on_collider.is_some());
                    assert!(!side.guest().client_is_floating, "held up by the deck");
                }
                assert_eq!(
                    side.guest().carry_rejects,
                    0,
                    "seed {seed} speed {speed} rtt {rtt}"
                );
            }
        }
    }
    eprintln!(
        "carried check: {reports} honest reports, {:.1} us each (the whole move check, a deck of one box)",
        spent.as_secs_f64() * 1.0e6 / f64::from(reports)
    );
}

/// The same on a curve: the deck turns 4 degrees a tick and its origin follows its heading,
/// at the speeds Create's trains turn at and beyond them.
#[test]
#[ignore = "daily"]
fn an_honest_rider_on_a_curve_is_never_rejected() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    for seed in [1, 2] {
        for speed in [0.7, 1.0, 2.0] {
            for rtt in [50, 100, 200] {
                let mut rng = Lcg(seed * 104_729 + (speed * 10.0) as u64 + rtt as u64);
                let mut rig = Rig::new(vec![(DECK, deck(20.0), along(ORIGIN[2]))]);
                let mut side = Guestside::new(fixture, &[], rtt);
                let age = f64::from(rtt) / 50.0 + 2.0;
                let (mut x, mut z) = (ORIGIN[0], ORIGIN[2]);
                let mut local = Vec3::new(0.0, 0.0, 0.0);
                for t in 1..=60 {
                    let heading = (4.0 * f64::from(t)).to_radians();
                    x += speed * heading.sin();
                    z += speed * heading.cos();
                    let half = heading / 2.0;
                    rig.tick(|_| {
                        Frame::from_quaternion(
                            Vec3::new(x, ORIGIN[1], z),
                            [0.0, half.sin(), 0.0, half.cos()],
                        )
                    });
                    if t < 25 {
                        continue;
                    }
                    local = Vec3::new(
                        (local.x + (rng.unit() - 0.5) * 0.4).clamp(-1.2, 1.2),
                        0.0,
                        (local.z + (rng.unit() - 0.5) * 0.4).clamp(-8.0, 8.0),
                    );
                    let jitter = (rng.unit() * 3.0).floor() - 1.0;
                    let to = honest(&rig, age + jitter, local);
                    if t == 25 {
                        side.hub.guests.update(0, |g| g.position = to);
                    }
                    let (_, refused, _) = side.report(&rig, to, None);
                    assert!(
                        !refused,
                        "seed {seed} speed {speed} rtt {rtt} tick {t}: refused at {to:?}"
                    );
                }
                assert_eq!(side.guest().carry_rejects, 0);
            }
        }
    }
}

/// A player who jumps onto a deck going 2.0 blocks a tick, two reports in a tick at times: the
/// fall is judged in the world the client saw, and it attaches the tick it lands.
#[test]
#[ignore = "daily"]
fn jumping_onto_a_deck_at_two_blocks_a_tick_attaches_on_landing() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    let mut rig = Rig::new(vec![(DECK, deck(20.0), along(ORIGIN[2]))]);
    let mut side = Guestside::new(fixture, &[], 100);
    for t in 1..=30 {
        rig.tick(|_| along(ORIGIN[2] + 2.0 * f64::from(t)));
    }
    // 1.2 blocks over the deck and 5 blocks to its side, falling and drifting onto it.
    let steps = [
        (4.0, 1.2, 1.9),
        (4.0, 0.9, 1.6),
        (4.0, 0.5, 1.2),
        (4.0, 0.2, 0.8),
        (4.0, 0.0, 0.5),
    ];
    for (t, (i, (age, y, x))) in (31..).zip(steps.into_iter().enumerate()) {
        rig.tick(|_| along(ORIGIN[2] + 2.0 * f64::from(t)));
        let to = honest(&rig, age, Vec3::new(x, y, 0.0));
        if i == 0 {
            side.hub.guests.update(0, |g| g.position = to);
        }
        let (_, refused, _) = side.report(&rig, to, None);
        assert!(!refused, "step {i} at {to:?}");
        assert_eq!(
            side.guest().on_collider.is_some(),
            y.abs() < 0.05,
            "step {i}"
        );
    }
    assert!(!side.guest().client_is_floating, "held up by the deck");
    assert_eq!(side.guest().carry_rejects, 0);
}

/// A report that is refused while a collider carries the player is corrected to where the server
/// last had the player on it, placed under the collider as it is now, counted, and answered by the
/// client's confirmation; the server's own position of an accepted rider is the same local
/// position under the collider as it is now, never the frame the client held.
#[test]
#[ignore = "daily"]
fn a_refused_rider_is_corrected_under_the_transform_now_and_an_accepted_one_is_kept_there() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    let mut rig = Rig::new(vec![(DECK, deck(20.0), along(ORIGIN[2]))]);
    let mut side = Guestside::new(fixture, &[], 100);
    for t in 1..=40 {
        rig.tick(|_| along(ORIGIN[2] + 2.0 * f64::from(t)));
    }
    let local = Vec3::new(0.5, 0.0, 1.0);
    let to = honest(&rig, 4.0, local);
    side.hub.guests.update(0, |g| g.position = to);
    assert!(!side.report(&rig, to, None).1);
    // Accepted: the server has the player at its local position under the deck as it is now,
    // 8 blocks ahead of the position in the frame the client held.
    let kept = side.guest().position;
    let now = rig.frame(DECK, 0.0).to_world(local);
    assert!(
        (kept[0] - now.x).abs() < 0.01
            && (kept[1] - now.y).abs() < 0.01
            && (kept[2] - now.z).abs() < 0.01,
        "kept {kept:?} vs the deck now {now:?}"
    );
    assert!(
        (kept[2] - to[2] - 8.0).abs() < 0.5,
        "{kept:?} vs the reported {to:?}"
    );
    // The next tick's honest report moves it on with the deck.
    rig.tick(|_| along(ORIGIN[2] + 2.0 * 41.0));
    let local = Vec3::new(0.5, 0.0, 1.2);
    let to = honest(&rig, 4.0, local);
    assert!(!side.report(&rig, to, None).1);
    let now = rig.frame(DECK, 0.0).to_world(local);
    let kept = side.guest().position;
    assert!((kept[2] - now.z).abs() < 0.01, "kept {kept:?} vs {now:?}");
    // A report inside the deck is refused: the player goes back to where the server last had it
    // on the deck, under the deck as it is now.
    let held = side.guest().on_collider.expect("attached");
    rig.tick(|_| along(ORIGIN[2] + 2.0 * 42.0));
    let inside = honest(&rig, 4.0, Vec3::new(0.5, -0.6, 1.2));
    let (_, refused, _) = side.report(&rig, inside, None);
    assert!(refused, "inside the deck");
    let corrected = side.corrected.expect("a correction was sent");
    let now = rig.frame(DECK, 0.0).to_world(held.local);
    assert!(
        (corrected[0] - now.x).abs() < 1e-9
            && (corrected[1] - now.y).abs() < 1e-9
            && (corrected[2] - now.z).abs() < 1e-9,
        "{corrected:?} vs {now:?}"
    );
    assert_eq!(side.guest().carry_rejects, 1);
    assert!(side.guest().on_collider.is_some(), "still carried");
    // The player's confirmed position is the correction, and it rides on from there.
    assert_eq!(side.guest().position, corrected);
    rig.tick(|_| along(ORIGIN[2] + 2.0 * 43.0));
    let to = honest(&rig, 4.0, Vec3::new(0.5, 0.0, 1.2));
    assert!(
        !side.report(&rig, to, None).1,
        "carried on after the correction"
    );
}

/// The speed test holds in the collider's frame: a client that changes the age it says its copy
/// is, to turn the same world position into a move of 12 blocks along the deck, is refused as
/// having moved too quickly, and the same report with the honest age is not.
#[test]
#[ignore = "daily"]
fn a_changing_age_cannot_hide_a_large_local_move_behind_a_small_world_one() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    for cheat in [false, true] {
        let mut rig = Rig::new(vec![(DECK, deck(40.0), along(ORIGIN[2]))]);
        let mut side = Guestside::new(fixture, &[], 100);
        for t in 1..=40 {
            rig.tick(|_| along(ORIGIN[2] + 2.0 * f64::from(t)));
        }
        let local = Vec3::new(0.0, 0.0, -6.0);
        let to = honest(&rig, 2.0, local);
        side.hub.guests.update(0, |g| g.position = to);
        assert!(!side.report(&rig, to, Some(0)).1, "attached");
        rig.tick(|_| along(ORIGIN[2] + 2.0 * 41.0));
        // The honest report of this tick, as the client's copy of age 2 puts it; the cheat tells
        // the server the copy is 6 ticks older, which puts the same world position 12 blocks along.
        let to = honest(&rig, 2.0, local);
        let (_, refused, _) = side.report(&rig, to, Some(if cheat { 6 } else { 0 }));
        assert_eq!(refused, cheat, "cheat {cheat}");
        if cheat {
            assert!(
                side.warnings
                    .iter()
                    .any(|w| w.contains("moved too quickly")),
                "{:?}",
                side.warnings
            );
            assert_eq!(side.guest().carry_rejects, 1);
        }
    }
}

/// A body that already overlaps one box of a collider may not walk into another it did not
/// overlap, a box of the same collider or of another.
#[test]
#[ignore = "daily"]
fn walking_from_one_box_into_another_is_a_new_collision_even_when_already_inside_one() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    let two = |gap: f64| {
        Arc::new(ColliderBoxes::new(vec![
            Aabb::new(Vec3::new(-6.0, -1.0, -2.0), Vec3::new(-1.0, 3.0, 2.0)),
            Aabb::new(Vec3::new(1.0 + gap, -1.0, -2.0), Vec3::new(6.0, 3.0, 2.0)),
        ]))
    };
    for (id, other) in [(DECK, false), (OTHER, true)] {
        // The body is inside the first box (a collider that moved into it); the second box is a
        // box of the same collider, or of another collider.
        let boxes = if other {
            Arc::new(ColliderBoxes::new(vec![Aabb::new(
                Vec3::new(-6.0, -1.0, -2.0),
                Vec3::new(-1.0, 3.0, 2.0),
            )]))
        } else {
            two(0.0)
        };
        let mut colliders = vec![(id, boxes, along(ORIGIN[2]))];
        if other {
            colliders.push((
                102,
                Arc::new(ColliderBoxes::new(vec![Aabb::new(
                    Vec3::new(1.0, -1.0, -2.0),
                    Vec3::new(6.0, 3.0, 2.0),
                )])),
                along(ORIGIN[2]),
            ));
        }
        let mut rig = Rig::new(colliders);
        for _ in 0..10 {
            rig.tick(|_| along(ORIGIN[2]));
        }
        let mut side = Guestside::new(fixture, &[], 100);
        // In the first box at its feet, 0.3 inside, and not carried.
        let at = |x: f64| {
            let p = rig.frame(id, 0.0).to_world(Vec3::new(x, 0.0, 0.0));
            [p.x, p.y, p.z]
        };
        side.hub.guests.update(0, |g| g.position = at(-1.2));
        // Staying inside it, or leaving into the open, is allowed: the gap between the boxes is 2.
        assert!(
            !side.send(&rig, at(-1.0), None, true).1,
            "other={other}: stays"
        );
        // Walking on into the second box: a collision with something new, refused.
        side.hub.guests.update(0, |g| g.position = at(-0.5));
        assert!(
            side.send(&rig, at(0.95), None, true).1,
            "other={other}: into the next box"
        );
    }
}

/// An attached rider that jumps across a grid cell boundary of the collider keeps its support:
/// the deck's top is 3.9 local, in the cell below the one the feet reach at 4.3.
#[test]
#[ignore = "daily"]
fn an_attached_jump_across_a_grid_cell_boundary_stays_carried() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    let wide = Arc::new(ColliderBoxes::new(vec![Aabb::new(
        Vec3::new(-20.0, 3.0, -20.0),
        Vec3::new(20.0, 3.9, 20.0),
    )]));
    let mut rig = Rig::new(vec![(DECK, wide, along(ORIGIN[2]))]);
    let mut side = Guestside::new(fixture, &[], 100);
    for t in 1..=30 {
        rig.tick(|_| along(ORIGIN[2] + 2.0 * f64::from(t)));
    }
    let mut t = 30;
    for (y, ground) in [
        (3.9, true),
        (4.15, false),
        (4.3, false),
        (4.4, false),
        (4.1, false),
        (3.9, true),
    ] {
        t += 1;
        rig.tick(|_| along(ORIGIN[2] + 2.0 * f64::from(t)));
        let to = honest(&rig, 4.0, Vec3::new(0.0, y, 0.0));
        if y == 3.9 && t == 31 {
            side.hub.guests.update(0, |g| g.position = to);
        }
        assert!(!side.send(&rig, to, None, ground).1, "y {y}");
        assert!(side.guest().on_collider.is_some(), "carried at y {y}");
    }
}

/// A rider that was airborne when its last report was accepted is counted as floating; a
/// refusal of the next report clears that, so the server's floating ticks do not run on and kick
/// a player it is correcting.
#[test]
#[ignore = "daily"]
fn a_refused_rider_is_not_counted_toward_the_flying_kick() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    let mut rig = Rig::new(vec![(DECK, deck(20.0), along(ORIGIN[2]))]);
    let mut side = Guestside::new(fixture, &[], 100);
    for t in 1..=30 {
        rig.tick(|_| along(ORIGIN[2] + 2.0 * f64::from(t)));
    }
    let to = honest(&rig, 4.0, Vec3::new(0.0, 0.0, 0.0));
    side.hub.guests.update(0, |g| g.position = to);
    assert!(!side.send(&rig, to, None, true).1);
    // A jump: above the deck, no step up, nothing but air around: floating.
    rig.tick(|_| along(ORIGIN[2] + 2.0 * 31.0));
    let up = honest(&rig, 4.0, Vec3::new(0.0, 1.5, 0.0));
    assert!(!side.send(&rig, up, None, false).1);
    assert!(side.guest().client_is_floating, "an airborne rider floats");
    // The next report is refused: inside the deck. The correction is on its way, unanswered.
    rig.tick(|_| along(ORIGIN[2] + 2.0 * 32.0));
    let inside = honest(&rig, 4.0, Vec3::new(0.0, -0.6, 0.0));
    side.answers = false;
    assert!(side.send(&rig, inside, None, true).1);
    assert!(
        !side.guest().client_is_floating,
        "a refusal is not floating"
    );
    let mut events = Vec::new();
    for _ in 0..120 {
        side.hub.tick_floating(0, &mut events);
    }
    assert!(
        events.is_empty(),
        "the server warned of {} things",
        events.len()
    );
    assert!(side.hub.pending_kicks.is_empty(), "nobody kicked for it");
    assert_eq!(side.guest().carry_rejects, 1);
}

/// Honest riders on a collider that rises, falls, rocks about each horizontal axis and does all of
/// them at once are never refused, and the server keeps them under the collider as it is now.
#[test]
#[ignore = "daily"]
fn an_honest_rider_on_a_rising_pitching_and_rolling_deck_is_never_refused() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    // (rise per tick, pitch amplitude, roll amplitude), degrees.
    for (rise, pitch, roll) in [
        (0.2_f64, 0.0_f64, 0.0_f64),
        (-0.05, 0.0, 0.0),
        (0.0, 5.0, 0.0),
        (0.0, 0.0, 5.0),
        (0.1, 4.0, 4.0),
    ] {
        let frame = |t: f64| {
            let (p, r) = (
                pitch.to_radians() * (t * 0.5).sin() / 2.0,
                roll.to_radians() * (t * 0.37).sin() / 2.0,
            );
            // Pitch about x, roll about z, composed as quaternions [x, y, z, w].
            let (qx, qz) = ([p.sin(), 0.0, 0.0, p.cos()], [0.0, 0.0, r.sin(), r.cos()]);
            let q = [
                qz[3] * qx[0] + qz[0] * qx[3] + qz[1] * qx[2] - qz[2] * qx[1],
                qz[3] * qx[1] - qz[0] * qx[2] + qz[1] * qx[3] + qz[2] * qx[0],
                qz[3] * qx[2] + qz[0] * qx[1] - qz[1] * qx[0] + qz[2] * qx[3],
                qz[3] * qx[3] - qz[0] * qx[0] - qz[1] * qx[1] - qz[2] * qx[2],
            ];
            Frame::from_quaternion(
                Vec3::new(ORIGIN[0], ORIGIN[1] + rise * t, ORIGIN[2] + 0.5 * t),
                q,
            )
        };
        let mut rig = Rig::new(vec![(DECK, deck(20.0), frame(0.0))]);
        let mut side = Guestside::new(fixture, &[], 100);
        let mut rng = Lcg(5);
        let mut local = Vec3::new(0.0, 0.0, 0.0);
        for t in 1..=50 {
            rig.tick(|_| frame(f64::from(t)));
            if t < 20 {
                continue;
            }
            local = Vec3::new(
                (local.x + (rng.unit() - 0.5) * 0.4).clamp(-1.0, 1.0),
                0.0,
                (local.z + (rng.unit() - 0.5) * 0.4).clamp(-6.0, 6.0),
            );
            // The client rests the box on the tilted top: the feet are a little over it.
            let lift = Vec3::new(local.x, 0.3 * 5.0_f64.to_radians().tan() + 0.002, local.z);
            let to = honest(&rig, 4.0, lift);
            if t == 20 {
                side.hub.guests.update(0, |g| g.position = to);
            }
            let (_, refused, _) = side.report(&rig, to, None);
            assert!(!refused, "rise {rise} pitch {pitch} roll {roll} tick {t}");
            let kept = side.guest().position;
            let now = rig.frame(DECK, 0.0).to_world(lift);
            assert!(
                (kept[0] - now.x).abs() < 0.02
                    // The tilted top lifts or sinks the feet by a little as it rocks.
                    && (kept[1] - now.y).abs() < 0.15
                    && (kept[2] - now.z).abs() < 0.02,
                "rise {rise} pitch {pitch} roll {roll} tick {t}: kept {kept:?} vs {now:?}"
            );
        }
        assert_eq!(side.guest().carry_rejects, 0);
    }
}

/// Stepping from one collider to another detaches from the first and attaches to the second,
/// each way, with no report refused; stepping onto the ground detaches too.
#[test]
#[ignore = "daily"]
fn stepping_between_colliders_hands_the_player_over_each_way() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    // Two decks side by side, 4 apart in x, moving at different speeds.
    let frames = |t: f64| {
        move |id: u32| match id {
            DECK => along(ORIGIN[2] + 1.0 * t),
            // 0.2 a tick faster, alongside at tick 31 and drifting two blocks either way.
            _ => Frame::translation(Vec3::new(
                ORIGIN[0] + 4.0,
                ORIGIN[1],
                ORIGIN[2] + 1.2 * t - 0.2 * 31.0,
            )),
        }
    };
    let mut rig = Rig::new(vec![
        (DECK, deck(20.0), along(ORIGIN[2])),
        (
            OTHER,
            deck(20.0),
            Frame::translation(Vec3::new(ORIGIN[0] + 4.0, ORIGIN[1], ORIGIN[2])),
        ),
    ]);
    let mut side = Guestside::new(fixture, &[], 100);
    for t in 1..=30 {
        rig.tick(frames(f64::from(t)));
    }
    let on = |rig: &Rig, id: u32, age: f64, local: Vec3| {
        let at = rig.frame(id, age).to_world(local);
        [at.x, at.y, at.z]
    };
    // On the first deck.
    let to = on(&rig, DECK, 4.0, Vec3::new(1.0, 0.0, 0.0));
    side.hub.guests.update(0, |g| g.position = to);
    assert!(!side.report(&rig, to, None).1);
    assert_eq!(side.guest().on_collider.map(|c| c.collider), Some(DECK));
    // Walks east off its edge (the margin ends at local x 1.8): airborne between the decks,
    // then on the second.
    let mut t = 30;
    for (id, local) in [
        (DECK, Vec3::new(1.6, 0.0, 0.0)),
        (DECK, Vec3::new(1.9, 0.4, 0.0)),
        (OTHER, Vec3::new(-1.4, 0.4, 0.0)),
        (OTHER, Vec3::new(-1.2, 0.0, 0.0)),
        (OTHER, Vec3::new(-1.0, 0.0, 0.0)),
    ] {
        t += 1;
        rig.tick(frames(f64::from(t)));
        let to = on(&rig, id, 4.0, local);
        let (_, refused, _) = side.report(&rig, to, None);
        assert!(!refused, "{id} {local:?}");
    }
    assert_eq!(side.guest().on_collider.map(|c| c.collider), Some(OTHER));
    // And back west onto the first.
    for (id, local) in [
        (OTHER, Vec3::new(-1.6, 0.0, 0.0)),
        (OTHER, Vec3::new(-1.9, 0.4, 0.0)),
        (DECK, Vec3::new(1.4, 0.4, 0.0)),
        (DECK, Vec3::new(1.2, 0.0, 0.0)),
        (DECK, Vec3::new(1.0, 0.0, 0.0)),
    ] {
        t += 1;
        rig.tick(frames(f64::from(t)));
        let to = on(&rig, id, 4.0, local);
        let (_, refused, _) = side.report(&rig, to, None);
        assert!(!refused, "{id} {local:?}");
    }
    assert_eq!(side.guest().on_collider.map(|c| c.collider), Some(DECK));
    assert_eq!(side.guest().carry_rejects, 0);
    // Off the deck's end onto the ground: it lets go.
    t += 1;
    rig.tick(frames(f64::from(t)));
    let ground = [to[0] + 8.0, 5.0, to[2] + 6.0];
    assert!(!side.report(&rig, ground, None).1);
    assert!(side.guest().on_collider.is_none());
}

/// A collider that brakes, one whose stream stalls and then bursts, and a round trip the
/// server mis-estimates by two ticks either way: the honest rider is not refused.
#[test]
#[ignore = "daily"]
fn braking_a_stall_with_a_burst_and_a_wrong_estimate_still_pass() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    // Braking from 2.0 blocks a tick to a stop over 10 ticks.
    let mut z = ORIGIN[2];
    let mut speed: f64 = 2.0;
    let mut rig = Rig::new(vec![(DECK, deck(20.0), along(z))]);
    let mut side = Guestside::new(fixture, &[], 100);
    let local = Vec3::new(0.0, 0.0, 0.0);
    for t in 1..=60 {
        if t > 30 {
            speed = (speed - 0.2).max(0.0);
        }
        z += speed;
        rig.tick(|_| along(z));
        if t >= 20 {
            let to = honest(&rig, 4.0, local);
            if t == 20 {
                side.hub.guests.update(0, |g| g.position = to);
            }
            assert!(!side.report(&rig, to, None).1, "braking tick {t}");
        }
    }
    // A stall: the client's copy stays where it was for 6 ticks while the server's deck goes
    // on at 2.0, then catches up. The client says its copy is that many ticks older.
    let mut rig = Rig::new(vec![(DECK, deck(20.0), along(ORIGIN[2]))]);
    let mut side = Guestside::new(fixture, &[], 100);
    for t in 1..=40 {
        rig.tick(|_| along(ORIGIN[2] + 2.0 * f64::from(t)));
    }
    let to = honest(&rig, 4.0, local);
    side.hub.guests.update(0, |g| g.position = to);
    assert!(!side.report(&rig, to, None).1);
    for stalled in 1..=3u8 {
        let t = 40 + u32::from(stalled);
        rig.tick(|_| along(ORIGIN[2] + 2.0 * f64::from(t)));
        // Its copy is still the one it had: another tick older each time.
        let to = honest(&rig, 4.0 + f64::from(stalled), local);
        assert!(
            !side.report(&rig, to, Some(2 + stalled)).1,
            "stalled {stalled}"
        );
    }
    // Every wrong estimate within two ticks passes: the server believes 100 ms, the truth is
    // 0 ms to 200 ms.
    for off in -2i32..=2 {
        let mut rig = Rig::new(vec![(DECK, deck(20.0), along(ORIGIN[2]))]);
        let mut side = Guestside::new(fixture, &[], 100);
        for t in 1..=40 {
            rig.tick(|_| along(ORIGIN[2] + 2.0 * f64::from(t)));
            if t >= 20 {
                let to = honest(&rig, 4.0 + f64::from(off), local);
                if t == 20 {
                    side.hub.guests.update(0, |g| g.position = to);
                }
                assert!(!side.report(&rig, to, None).1, "off {off} tick {t}");
            }
        }
    }
}

/// A collider that is gone lets its rider fall, and a teleport (a portal, a command) lets go.
#[test]
#[ignore = "daily"]
fn a_disassembled_collider_and_a_teleport_let_the_rider_go() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (mut rig, mut side) = first(fixture);
    let to = honest(&rig, 4.0, Vec3::new(0.0, 0.0, 0.0));
    assert!(!side.report(&rig, to, None).1);
    assert!(side.guest().on_collider.is_some());
    // The collider is removed: the history forgets it and the rider falls like anyone.
    rig.colliders.clear();
    rig.record();
    let fall = [to[0], to[1] - 0.08, to[2]];
    let (wrongly, refused, _) = side.report(&rig, fall, None);
    assert_eq!((wrongly, refused), (false, false));
    assert!(side.guest().on_collider.is_none());
    // A teleport lets go of a rider.
    let (rig, mut side) = first(fixture);
    let to = honest(&rig, 4.0, Vec3::new(0.0, 0.0, 0.0));
    assert!(!side.report(&rig, to, None).1);
    assert!(side.guest().on_collider.is_some());
    side.hub.resend_teleport(0, Some([50.5, 5.0, 50.5]));
    assert!(side.guest().on_collider.is_none());
}

/// Blocks beside the track still stop an honest rider, and a train through a tunnel is no
/// reason to refuse one; a report inside the deck is a collision with something new.
#[test]
#[ignore = "daily"]
fn walls_beside_the_track_and_the_deck_itself_still_stop_an_honest_rider() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    // A tunnel: walls at x 18 and x 22 (the deck spans x 19 to 22, so they touch it) along the whole track.
    let walls: Vec<(i32, i32)> = (0..=170).flat_map(|z| [(18, z), (22, z)]).collect();
    let mut rig = Rig::new(vec![(DECK, deck(20.0), along(ORIGIN[2]))]);
    let mut side = Guestside::new(fixture, &walls, 100);
    for t in 1..=60 {
        rig.tick(|_| along(ORIGIN[2] + 2.0 * f64::from(t)));
        if t >= 20 {
            let to = honest(
                &rig,
                4.0,
                Vec3::new(1.0 - 0.02 * f64::from(t % 10), 0.0, 0.0),
            );
            if t == 20 {
                side.hub.guests.update(0, |g| g.position = to);
            }
            assert!(!side.report(&rig, to, None).1, "the tunnel, tick {t}");
        }
    }
    // The player walks east into the wall at x 22: its client stops it there, so a report
    // beyond that is a wall the server refuses.
    let into = honest(&rig, 4.0, Vec3::new(1.45, 0.0, 0.0));
    let (_, refused, _) = side.report(&rig, into, None);
    assert!(refused, "through the wall at {into:?}");
    // Inside the deck: a collision the body was not in a moment ago.
    let (mut rig, mut side) = (
        Rig::new(vec![(DECK, deck(20.0), along(ORIGIN[2]))]),
        Guestside::new(fixture, &[], 100),
    );
    for _ in 0..10 {
        rig.tick(|_| along(ORIGIN[2]));
    }
    let on = honest(&rig, 4.0, Vec3::new(0.0, 0.0, 0.0));
    side.hub.guests.update(0, |g| g.position = on);
    assert!(!side.report(&rig, on, None).1);
    let inside = honest(&rig, 4.0, Vec3::new(0.0, -0.6, 0.0));
    assert!(side.report(&rig, inside, None).1, "inside the deck");
}

/// Past a second of round trip the window stops growing: a rider whose copy is older than the
/// window reaches is not carried, and is judged as any walker in the frame the cap allows.
#[test]
#[ignore = "daily"]
fn past_a_second_of_round_trip_the_rider_is_no_longer_carried() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    let mut rig = Rig::new(vec![(DECK, deck(20.0), along(ORIGIN[2]))]);
    let mut side = Guestside::new(fixture, &[], 3_000);
    for t in 1..=70 {
        rig.tick(|_| along(ORIGIN[2] + f64::from(t)));
    }
    // The truth is 3 s: the client's copy is 62 ticks old, and the server holds 26.
    let at = along(ORIGIN[2] + 70.0 - 62.0).to_world(Vec3::new(0.0, 0.0, 0.0));
    let to = [at.x, at.y, at.z];
    side.hub.guests.update(0, |g| g.position = to);
    side.report(&rig, to, None);
    assert!(side.guest().on_collider.is_none());
}

/// What the costly paths take, on a collider at the box cap (a 32 x 32 x 32 lump of 32768
/// boxes, its top at its origin), the lump moving 2.0 blocks a tick, so every report meets a new
/// tick's frames: a report that fits no age (the whole search), one that is refused and
/// corrected, and a guest walking beside the lump (near it, on nothing). The numbers go to stderr (`--nocapture`).
#[test]
#[ignore = "daily"]
fn the_costly_paths_are_timed_on_a_collider_at_the_box_cap() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    const SIDE: usize = 32;
    let lump = Arc::new(ColliderBoxes::new(
        (0..SIDE * SIDE * SIDE)
            .map(|i| {
                let (x, y, z) = (i % SIDE, i / SIDE % SIDE, i / SIDE / SIDE);
                Aabb::new(
                    Vec3::new(x as f64 - 16.0, y as f64 - 32.0, z as f64 - 16.0),
                    Vec3::new(x as f64 - 15.0, y as f64 - 31.0, z as f64 - 15.0),
                )
            })
            .collect(),
    ));
    let mut rig = Rig::new(vec![(DECK, lump, along(ORIGIN[2]))]);
    let mut side = Guestside::new(fixture, &[], 100);
    for t in 1..=40 {
        rig.tick(|_| along(ORIGIN[2] + 2.0 * f64::from(t)));
    }
    let held = |local: Vec3| Carried {
        collider: DECK,
        age: 4.0,
        local,
        reported: local,
        committed: local,
    };
    let mut time = |name: &str, report: &mut dyn FnMut(&mut Guestside<'_>, &Rig)| {
        let mut spent = std::time::Duration::ZERO;
        let mut reports = 0u32;
        for t in 41..241u32 {
            rig.tick(|_| along(ORIGIN[2] + 2.0 * f64::from(t)));
            // Hovering beside the lump for 200 ticks is not what is timed: nobody is kicked for it.
            side.hub.pending_kicks.clear();
            side.hub.guests.update(0, |g| {
                g.client_is_floating = false;
                g.above_ground_ticks = 0;
            });
            let began = std::time::Instant::now();
            report(&mut side, &rig);
            spent += began.elapsed();
            reports += 1;
        }
        eprintln!(
            "carried check, collider at the box cap, {name}: {:.1} us per report over {reports}",
            spent.as_secs_f64() * 1.0e6 / f64::from(reports)
        );
    };
    let on = Vec3::new(0.0, 0.0, 0.0);
    // Attached, and the report is 25 blocks along the lump's length: over nothing at any age.
    time("fits no age", &mut |side, rig| {
        let to = honest(rig, 4.0, Vec3::new(0.0, 0.0, 25.0));
        side.hub.guests.update(0, |g| {
            g.position = honest(rig, 4.0, on);
            g.on_collider = Some(held(on));
        });
        side.report(rig, to, None);
    });
    // Attached, and the report is inside the lump: refused and corrected.
    time("refused and corrected", &mut |side, rig| {
        let to = honest(rig, 4.0, Vec3::new(0.0, -3.0, 0.0));
        side.hub.guests.update(0, |g| {
            g.position = honest(rig, 4.0, on);
            g.on_collider = Some(held(on));
        });
        side.report(rig, to, None);
    });
    // Not attached, beside the lump, on nothing.
    time("walking beside it", &mut |side, rig| {
        let to = honest(rig, 4.0, Vec3::new(16.8, 0.0, 0.0));
        side.hub.guests.update(0, |g| {
            g.position = honest(rig, 4.0, Vec3::new(16.7, 0.0, 0.0));
            g.on_collider = None;
        });
        side.report(rig, to, None);
    });
}

/// A rider that is attached is moved through the level's current placement: an aged report that is
/// clear, while the deck now carries the player across a wall, is stopped at the wall, the server
/// saves the place on the deck that was reached, and a refused report's correction does not pass
/// the wall.
#[test]
#[ignore = "daily"]
fn a_commit_stopped_by_a_wall_in_the_current_frame_is_what_a_correction_returns_to() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    // A wall across the track at z 71 (the deck is at z 70 at tick 29 and 72 at tick 30, and
    // passes through it as a collider).
    let walls: Vec<(i32, i32)> = (19..=22).map(|x| (x, 71)).collect();
    let mut rig = Rig::new(vec![(DECK, deck(20.0), along(ORIGIN[2]))]);
    let mut side = Guestside::new(fixture, &walls, 100);
    for t in 1..=29 {
        rig.tick(|_| along(ORIGIN[2] + 2.0 * f64::from(t)));
    }
    let on = Vec3::new(0.0, 0.0, 0.0);
    let to = honest(&rig, 4.0, on);
    side.hub.guests.update(0, |g| g.position = to);
    assert!(!side.report(&rig, to, None).1);
    assert!(side.guest().on_collider.is_some(), "attached");
    // The deck moves on to z 72; the report in the age the client saw is clear of the wall.
    rig.tick(|_| along(ORIGIN[2] + 2.0 * 30.0));
    let to = honest(&rig, 4.0, on);
    assert!(to[2] < 70.0, "{to:?} is before the wall in the aged frame");
    assert!(!side.report(&rig, to, None).1);
    let kept = side.guest().position;
    assert!(
        kept[2] < 71.0,
        "stopped at the wall, not through it: {kept:?}"
    );
    let held = side.guest().on_collider.expect("still attached");
    let now = rig.frame(DECK, 0.0).to_world(held.local);
    assert!(
        (now.z - kept[2]).abs() < 1e-9,
        "the saved place on the deck is the one reached: {now:?} vs {kept:?}"
    );
    // The deck stands still; the next report is inside it and refused: the correction is the
    // place reached, on this side of the wall.
    rig.tick(|_| along(ORIGIN[2] + 2.0 * 30.0));
    let inside = honest(&rig, 4.0, Vec3::new(0.0, -0.6, 0.0));
    let (_, refused, _) = side.report(&rig, inside, None);
    assert!(refused);
    let corrected = side.corrected.expect("a correction");
    assert!(
        corrected[2] < 71.0,
        "the correction stops at the wall: {corrected:?}"
    );
}

/// Two position reports in one tick: the first is judged as any other, the second is the
/// repeated packet the hub refuses by kicking the player, with the age payload in front of each
/// not changing that.
#[test]
#[ignore = "daily"]
fn a_second_position_report_in_a_tick_is_refused_by_the_real_guard() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    let mut rig = Rig::new(vec![(DECK, deck(20.0), along(ORIGIN[2]))]);
    let mut side = Guestside::new(fixture, &[], 100);
    for t in 1..=30 {
        rig.tick(|_| along(ORIGIN[2] + 2.0 * f64::from(t)));
    }
    let to = honest(&rig, 4.0, Vec3::new(0.0, 0.0, 0.0));
    side.hub.guests.update(0, |g| g.position = to);
    assert!(!side.report(&rig, to, None).1);
    rig.tick(|_| along(ORIGIN[2] + 2.0 * 31.0));
    let hint = |ticks| {
        S::CustomPayload(sunrise_protocol::packets::configuration::CustomPayload {
            channel: sunrise_protocol::carry_age::CHANNEL.into(),
            data: sunrise_protocol::carry_age::encode(sunrise_protocol::carry_age::Hint {
                collider: DECK,
                ticks,
            }),
        })
    };
    let movement = |p: [f64; 3]| {
        S::MovePlayerPos(play::MovePlayerPos {
            x: p[0],
            y: p[1],
            z: p[2],
            flags: 1,
        })
    };
    let first = honest(&rig, 4.0, Vec3::new(0.0, 0.0, 0.2));
    let second = honest(&rig, 4.0, Vec3::new(0.0, 0.0, 0.3));
    let before = side.guest().received_moves;
    side.hub.guests.update(0, |g| g.position_this_tick = false);
    // One inbox, the guard intact between the reports.
    side.tick(
        &rig,
        vec![hint(4), movement(first), hint(4), movement(second)],
    );
    let g = side.guest();
    assert_eq!(
        g.received_moves,
        before + 1,
        "only the first report is judged"
    );
    assert!(g.disconnect_pending, "the repeated report kicks the player");
    assert_eq!(g.carry_rejects, 0);
}

/// A player attaching to a deck is stopped by another collider across its path, as by a block:
/// only the deck it attaches to is no obstacle.
#[test]
#[ignore = "daily"]
fn an_attaching_player_is_stopped_by_another_collider_across_its_path() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    // A wall of a collider across the track at z 71 to 72, standing still. The deck is at z 74 now
    // and was at z 66 where the client saw it: attaching with a report that puts the player
    // across the wall in the current frame holds it at the wall.
    let (mut rig, mut side) = (
        Rig::new(vec![
            (DECK, deck(20.0), along(ORIGIN[2])),
            (
                OTHER,
                Arc::new(ColliderBoxes::new(vec![Aabb::new(
                    Vec3::new(-3.0, -4.0, -0.5),
                    Vec3::new(3.0, 4.0, 0.5),
                )])),
                Frame::translation(Vec3::new(ORIGIN[0], ORIGIN[1], 71.5)),
            ),
        ]),
        Guestside::new(fixture, &[], 100),
    );
    for t in 1..=31 {
        rig.tick(|id| {
            if id == DECK {
                along(ORIGIN[2] + 2.0 * f64::from(t))
            } else {
                Frame::translation(Vec3::new(ORIGIN[0], ORIGIN[1], 71.5))
            }
        });
    }
    let to = honest(&rig, 4.0, Vec3::new(0.0, 0.0, 0.0));
    side.hub.guests.update(0, |g| g.position = to);
    assert!(!side.report(&rig, to, None).1);
    assert!(side.guest().on_collider.is_some(), "attached");
    let kept = side.guest().position;
    assert!(
        kept[2] < 71.0,
        "stopped by the other collider, not through it: {kept:?}"
    );
}

/// A block's part is asked about the guest itself in the scan that commits a carried guest, as
/// in its ordinary moves: not about nothing, and not about the collider that carries it.
#[test]
#[ignore = "daily"]
fn a_carried_guests_settle_asks_a_part_about_the_guest() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    crate::native_client_hooks::test_hooks::install();
    let glass = fixture
        .assets
        .registry
        .default_state("minecraft:glass")
        .unwrap();
    let collision = fixture.own_collision();
    collision.set_part_states([glass]);
    let asked = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&asked);
    crate::native_client_hooks::test_hooks::COLLISION_ANSWER.with_borrow_mut(|answer| {
        *answer = Some(Box::new(move |query| {
            seen.borrow_mut().push(query.entity());
            None
        }));
    });
    let rig = Rig::new(vec![(DECK, deck(20.0), along(ORIGIN[2]))]);
    let mut side = Guestside::new(fixture, &[], 100);
    side.world.set(22, 9, 12, glass).unwrap();
    let id = side.hub.guests[0].entity_id;
    let mut next = 2;
    let ctx = TickCtx {
        sea_level: 63,
        world: &mut side.world,
        entities: &[],
        next_entity: &mut next,
        spawn: [20.5, 9.0, 12.0],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: Some(&collision),
        border: &side.border,
        streaming: None,
    };
    let looped = Arc::new(crate::native_client_hooks::test_hooks::loop_context());
    let parts = Rc::new(crate::collision_parts::ModParts::new(looped.clone()));
    let _parts = sunrise_world::part_collision::InParts::enter(parts.clone());
    // A step-up retry can query the wall's part again (#5459).
    let mut first = 0;
    rig.within(|| {
        side.hub
            .settle(0, [20.5, 9.0, 12.5], [22.0, 9.0, 12.5], &ctx);
        first = asked.borrow().len();
        parts.begin_tick(&looped, "minecraft:overworld");
        sunrise_world::moving_colliders::as_mover(DECK, || {
            side.hub
                .settle(0, [20.5, 9.0, 12.5], [22.0, 9.0, 12.5], &ctx)
        });
    });
    let asked = asked.borrow();
    assert!(first > 0, "the guest's own settle asked the wall's part");
    assert!(
        asked.len() > first,
        "the carried settle asked the wall's part: {asked:?}"
    );
    assert!(
        asked.iter().all(|entity| *entity == Some(id as u32)),
        "asked about {asked:?}, not the guest {id}"
    );
}
