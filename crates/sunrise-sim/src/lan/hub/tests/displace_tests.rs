//! A mod's displacement of a player (`Hub::displace`, `Hub::settle_conn`; docs/modding/src/how-to/
//! native-mods.md): a relative teleport the client adds to where it is, confirmed in the order a
//! client sends it, with the server's checks unchanged.
use super::carried_check_tests::{DECK, Guestside, Rig, first, honest};
use super::*;
use sunrise_core::geometry::Vec3;

const START: [f64; 3] = [20.5, 5.0, 12.0];

fn standing<'a>(
    fixture: &'a crate::test_fixtures::Vanilla,
    walls: &[(i32, i32)],
) -> (Rig, Guestside<'a>) {
    let rig = Rig::new(Vec::new());
    let mut side = Guestside::new(fixture, walls, 100);
    side.hub.guests.update(0, |g| g.position = START);
    (rig, side)
}

fn plus(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// The position packet the hub last sent.
fn position_sent(side: &Guestside<'_>) -> play::PlayerPosition {
    side.sent()
        .into_iter()
        .rev()
        .find_map(|p| match p {
            play::Clientbound::PlayerPosition(p) => Some(p),
            _ => None,
        })
        .expect("a position packet")
}

/// The client's report in the tick `tick` of its walk, after one tick end.
fn walk(side: &mut Guestside<'_>, rig: &Rig, to: [f64; 3], yaw: f32) {
    side.tick(
        rig,
        vec![
            S::MovePlayerPosRot(play::MovePlayerPosRot {
                x: to[0],
                y: to[1],
                z: to[2],
                yaw,
                pitch: 5.0,
                flags: 1,
            }),
            S::ClientTickEnd(Default::default()),
        ],
    );
}

fn accept(id: i32, at: [f64; 3], yaw: f32) -> S {
    S::AcceptTeleportation(play::AcceptTeleportation {
        id,
        x: at[0],
        y: at[1],
        z: at[2],
        yaw,
        pitch: 5.0,
    })
}

fn warned(events: &[HostEvent]) -> bool {
    events.iter().any(|e| matches!(e, HostEvent::Warn(_)))
}

/// A displacement goes out as one relative teleport (position, motion and look), the player's
/// own reports while it is awaited lose their position and keep their rotation with nothing
/// held against it, and the confirmation is an ordinary report of where the client is (the
/// client's walk plus the displacement), accepted from the target the server snapped to; the
/// walk on from there is accepted as an ordinary move.
#[test]
#[ignore = "daily"]
fn a_displacement_is_a_relative_teleport_that_drops_position_keeps_rotation_and_lands_where_the_client_walked()
 {
    use play::relative::{DELTA_X, DELTA_Y, DELTA_Z, ROTATE_DELTA, X, X_ROT, Y, Y_ROT, Z};
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (rig, mut side) = standing(fixture, &[]);
    let conn = side.guest().conn;
    let by = [2.0, 0.0, 0.0];
    let target = plus(START, by);
    side.hub.displace(conn, target, by);
    let packet = position_sent(&side);
    let all = X | Y | Z | DELTA_X | DELTA_Y | DELTA_Z | Y_ROT | X_ROT | ROTATE_DELTA;
    assert_eq!(
        packet.relatives, all,
        "position, motion and look are relative"
    );
    assert_eq!(
        packet.position, by,
        "a relative axis carries the displacement"
    );
    assert_eq!(
        side.guest().awaiting_teleport,
        Some((packet.teleport, target))
    );
    assert_eq!(
        side.guest().position,
        target,
        "the server copy moved at once"
    );
    // Three client ticks of walking on, each reported and each dropped, rotation kept.
    for k in 1..=3 {
        walk(
            &mut side,
            &rig,
            plus(START, [0.0, 0.0, 0.2 * f64::from(k)]),
            30.0,
        );
        assert_eq!(
            side.guest().position,
            target,
            "the report's position is lost"
        );
        assert_eq!((side.guest().yaw, side.guest().pitch), (30.0, 5.0));
        assert!(
            !side.guest().disconnect_pending,
            "nothing is held against it"
        );
    }
    // The client applied the packet to where it was, and confirms.
    let landed = plus(plus(START, [0.0, 0.0, 0.6]), by);
    let events = side.tick(&rig, vec![accept(packet.teleport, landed, 30.0)]);
    assert!(!warned(&events));
    assert!(side.guest().awaiting_teleport.is_none(), "confirmed");
    assert_eq!(
        side.guest().position,
        landed,
        "the confirmation's own position, the walk kept"
    );
    // The walk on from the landing is accepted as the ordinary move it is.
    let next = plus(landed, [0.0, 0.0, 0.2]);
    walk(&mut side, &rig, next, 30.0);
    assert_eq!(side.guest().position, next);
    assert!(side.guest().awaiting_teleport.is_none());
}

/// A confirmation and the next report in one server tick are never a duplicate, and a report
/// that was sent before the teleport, in the same tick, is dropped without a kick.
#[test]
#[ignore = "daily"]
fn a_confirmation_and_the_next_report_in_one_tick_are_not_a_duplicate() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (rig, mut side) = standing(fixture, &[]);
    let conn = side.guest().conn;
    let by = [0.0, 0.0, 1.5];
    let target = plus(START, by);
    side.hub.displace(conn, target, by);
    let id = position_sent(&side).teleport;
    let tick_end = || S::ClientTickEnd(Default::default());
    let report = |to: [f64; 3]| {
        S::MovePlayerPos(play::MovePlayerPos {
            x: to[0],
            y: to[1],
            z: to[2],
            flags: 1,
        })
    };
    let last = plus(START, [0.0, 0.0, 0.2]);
    let landed = plus(last, by);
    let next = plus(landed, [0.0, 0.0, 0.2]);
    // The client's order: the report of the tick before, its tick end, the confirmation at the
    // start of a frame, the report at the end of the next tick, its tick end.
    let events = side.tick(
        &rig,
        vec![
            report(last),
            tick_end(),
            accept(id, landed, 0.0),
            report(next),
            tick_end(),
        ],
    );
    assert!(!warned(&events));
    assert!(!side.guest().disconnect_pending, "no duplicate report");
    assert_eq!(side.guest().position, next);
}

/// A second displacement while the first is awaited starts from the awaited target, and the
/// first confirmation, now stale, is ignored.
#[test]
#[ignore = "daily"]
fn a_second_displacement_while_one_is_awaited_composes_and_the_old_id_is_ignored() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (rig, mut side) = standing(fixture, &[]);
    let conn = side.guest().conn;
    let (first_by, second_by) = ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    let one = plus(START, first_by);
    side.hub.displace(conn, one, first_by);
    let old = position_sent(&side).teleport;
    let two = plus(one, second_by);
    side.hub.displace(conn, two, second_by);
    let new = position_sent(&side).teleport;
    assert_ne!(old, new);
    assert_eq!(side.guest().awaiting_teleport, Some((new, two)));
    side.tick(&rig, vec![accept(old, one, 0.0)]);
    assert_eq!(side.guest().awaiting_teleport, Some((new, two)), "stale");
    assert!(!side.guest().disconnect_pending);
    side.tick(&rig, vec![accept(new, two, 0.0)]);
    assert!(side.guest().awaiting_teleport.is_none());
    assert_eq!(side.guest().position, two);
}

/// A displacement nobody confirms is sent again after 20 ticks, absolute and with no velocity,
/// at the awaited position.
#[test]
#[ignore = "daily"]
fn an_unconfirmed_displacement_is_resent_absolute_with_no_velocity() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (rig, mut side) = standing(fixture, &[]);
    let conn = side.guest().conn;
    let by = [3.0, 0.0, 0.0];
    let target = plus(START, by);
    side.hub.displace(conn, target, by);
    let old = position_sent(&side).teleport;
    side.hub.ticks += 21;
    walk(&mut side, &rig, START, 0.0);
    let packet = position_sent(&side);
    assert_ne!(packet.teleport, old, "a new id");
    assert_eq!(packet.relatives, 0, "no relative flag");
    assert_eq!(packet.position, target, "the awaited position, absolute");
    assert_eq!(packet.delta, [0.0; 3]);
    // The old id is stale now.
    side.tick(&rig, vec![accept(old, target, 0.0)]);
    assert_eq!(
        side.guest().awaiting_teleport,
        Some((packet.teleport, target))
    );
}

/// The collision a displacement is made through: a wall stops it, and the player ends this side.
#[test]
#[ignore = "daily"]
fn the_move_stops_at_a_wall() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    let walls: Vec<(i32, i32)> = (0..=170).map(|z| (23, z)).collect();
    let (_, side) = standing(fixture, &walls);
    let conn = side.guest().conn;
    let to = plus(START, [10.0, 0.0, 0.0]);
    let reached = side
        .hub
        .settle_conn(conn, START, to, &side.world, &fixture.collision, &[])
        .expect("the level answers");
    // The block at x 23 is the wall: the player's box ends against it.
    assert!((reached[0] - 22.7).abs() < 1e-6, "{reached:?}");
    assert_eq!((reached[1], reached[2]), (START[1], START[2]));
    // Free air: all of it.
    let free = plus(START, [-5.0, 0.0, 0.0]);
    assert_eq!(
        side.hub
            .settle_conn(conn, START, free, &side.world, &fixture.collision, &[]),
        Some(free)
    );
}

/// A carried player displaced along its deck stays carried, at the same place on it; displaced
/// off the deck it lets go. Either way its next honest report is accepted and counts nothing.
#[test]
#[ignore = "daily"]
fn a_carried_player_stays_on_its_deck_or_lets_go_and_nothing_is_counted() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    for (by, stays) in [([0.0, 0.0, 3.0], true), ([4.0, 0.0, 0.0], false)] {
        let (rig, mut side) = first(fixture);
        let to = honest(&rig, 4.0, Vec3::new(0.0, 0.0, 0.0));
        assert!(!side.report(&rig, to, None).1);
        let held = side.guest().on_collider.expect("attached");
        let conn = side.guest().conn;
        let at = side.guest().position;
        let target = plus(at, by);
        rig.within(|| side.hub.displace(conn, target, by));
        match (stays, side.guest().on_collider) {
            (true, Some(now)) => {
                assert_eq!(now.collider, DECK);
                assert!((now.local.z - (held.local.z + 3.0)).abs() < 1e-9, "{now:?}");
                assert_eq!(now.committed, Vec3::new(target[0], target[1], target[2]));
            }
            (false, None) => {}
            (stays, now) => panic!("stays {stays}: {now:?}"),
        }
        side.confirm(&rig);
        assert_eq!(side.guest().position, target);
        // The deck stands still: an honest report from where the player is.
        let (_, refused, _) = side.report(&rig, plus(target, [0.0, 0.0, 0.1]), None);
        assert!(!refused, "{by:?}");
        assert_eq!(side.guest().carry_rejects, 0, "{by:?}");
    }
}

/// A development build warns once for a player displaced on 20 ticks in a row: the streak counts
/// ticks, a gap starts it again.
#[test]
#[ignore = "daily"]
fn the_streak_of_displaced_ticks_counts_in_a_row_and_a_gap_starts_it_again() {
    // Only a development build keeps the streak.
    if !cfg!(debug_assertions) {
        return;
    }
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    let (_, mut side) = standing(fixture, &[]);
    let conn = side.guest().conn;
    let mut at = START;
    let step = |side: &mut Guestside<'_>, at: &mut [f64; 3]| {
        let by = [0.01, 0.0, 0.0];
        *at = plus(*at, by);
        side.hub.displace(conn, *at, by);
    };
    for _ in 0..25 {
        side.hub.ticks += 1;
        step(&mut side, &mut at);
    }
    assert_eq!(side.guest().displaced_streak.1, 25, "one streak");
    // Two calls in a tick are one tick of the streak.
    step(&mut side, &mut at);
    assert_eq!(side.guest().displaced_streak.1, 25);
    side.hub.ticks += 5;
    step(&mut side, &mut at);
    assert_eq!(side.guest().displaced_streak.1, 1, "a gap starts it again");
}
