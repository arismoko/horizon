//! `player_move`: what is not heard (the first tick in a level, a teleport's confirmation, the veto's
//! correction) and what is (the next tick, the first report after a teleport).
use super::pending_move_tests::*;
use super::*;

fn confirm(hub: &Hub, yaw: f32) -> S {
    let (id, at) = hub.guests[0].awaiting_teleport.expect("a teleport waits");
    S::AcceptTeleportation(play::AcceptTeleportation {
        id,
        x: at[0],
        y: at[1],
        z: at[2],
        yaw,
        pitch: 0.0,
    })
}

fn push(hub: &mut Hub, packets: Vec<S>) {
    hub.guests.update(0, |g| {
        g.inbox
            .extend(packets.into_iter().map(InboxItem::uncredited));
    });
}

fn look(hub: &Hub) -> [f32; 2] {
    [hub.guests[0].yaw, hub.guests[0].pitch]
}

#[test]
fn the_first_tick_after_a_join_is_not_heard_and_the_second_is() {
    let mut hub = standing(true, vec![]);
    hub.guests.update(0, |g| g.placed = false);
    let conn = hub.guests[0].conn;
    with_input_context(|ctx| hub.place_new_player(conn, ctx));
    assert_eq!(
        hub.guests[0].level_entered, 10,
        "it entered its level at tick 10"
    );
    hub.guests.update(0, |g| g.load_timeout = 0);
    hub.ticks = 11;
    // The join's teleport is confirmed in the first tick, and the first report comes with it.
    let confirmation = confirm(&hub, 0.0);
    push(&mut hub, vec![confirmation, turned(10.0)]);
    assert!(moves(&drain(&mut hub)).is_empty(), "tick 11 is the first");
    assert_eq!(look(&hub), [10.0, 0.0], "committed in place");
    hub.ticks = 12;
    push(&mut hub, vec![turned(20.0)]);
    assert_eq!(moves(&drain(&mut hub)), [conn], "tick 12 is the second");
    assert_eq!(look(&hub), [10.0, 0.0]);
}

#[test]
fn a_report_later_in_the_level_after_a_join_is_heard_like_any_other() {
    // The first report after joining may come long after the first tick: it is heard then.
    let mut hub = standing(true, vec![]);
    hub.guests.update(0, |g| {
        g.ticks = 0;
        g.level_entered = 3;
    });
    hub.ticks = 30;
    push(&mut hub, vec![turned(10.0)]);
    let conn = hub.guests[0].conn;
    assert_eq!(moves(&drain(&mut hub)), [conn]);
}

#[test]
fn the_first_tick_after_a_level_change_is_not_heard_and_the_second_is() {
    let mut hub = standing(true, vec![]);
    let conn = hub.guests[0].conn;
    with_input_context(|ctx| hub.change_level(0, [0.0, 64.0, 0.0], 0.0, None, false, 0, ctx));
    assert_eq!(hub.guests[0].level_entered, 10);
    hub.ticks = 11;
    let confirmation = confirm(&hub, 0.0);
    push(&mut hub, vec![confirmation, turned(10.0)]);
    assert!(
        moves(&drain(&mut hub)).is_empty(),
        "the confirmation and the first tick are not heard"
    );
    assert_eq!(look(&hub), [10.0, 0.0]);
    assert!(hub.guests[0].pending_move.is_none());
    hub.ticks = 12;
    push(&mut hub, vec![turned(20.0)]);
    assert_eq!(moves(&drain(&mut hub)), [conn], "the second tick is");
    let report = hub.pending_move_report(conn).unwrap();
    assert_eq!(
        (report.from_rotation, report.to_rotation),
        ([10.0, 0.0], [20.0, 0.0])
    );
}

#[test]
fn a_teleport_in_the_level_is_not_a_first_tick_and_the_first_report_after_it_is_heard() {
    let mut hub = standing(true, vec![]);
    let conn = hub.guests[0].conn;
    hub.teleport_command(0, [10.5, 70.0, 10.5], [false; 3]);
    assert_eq!(hub.guests[0].level_entered, 0, "the same level");
    hub.ticks = 11;
    let confirmation = confirm(&hub, 0.0);
    push(&mut hub, vec![confirmation, turned(10.0)]);
    assert_eq!(
        moves(&drain(&mut hub)),
        [conn],
        "the report, not the confirmation"
    );
    let report = hub.pending_move_report(conn).unwrap();
    assert_eq!(
        report.from,
        [10.5, 70.0, 10.5],
        "from the teleport's destination"
    );
    assert_eq!(
        (report.from_rotation, report.to_rotation),
        ([0.0, 0.0], [10.0, 0.0])
    );
}

#[test]
fn a_respawn_is_a_teleport_not_a_first_tick_and_the_first_report_after_it_is_heard() {
    let mut hub = standing(true, vec![]);
    let conn = hub.guests[0].conn;
    with_input_context(|ctx| hub.respawn(0, [0.5, 64.0, 0.5], Some([0.0; 2]), ctx));
    assert_eq!(hub.guests[0].level_entered, 0, "the same level");
    hub.guests.update(0, |g| g.load_timeout = 0);
    hub.ticks = 11;
    let confirmation = confirm(&hub, 0.0);
    push(&mut hub, vec![confirmation, turned(10.0)]);
    assert_eq!(moves(&drain(&mut hub)), [conn]);
    let report = hub.pending_move_report(conn).unwrap();
    assert_eq!(
        report.from,
        [0.5, 64.0, 0.5],
        "from the respawn's destination"
    );
    assert_eq!(report.to_rotation, [10.0, 0.0]);
}

#[test]
fn a_confirmation_with_a_changed_rotation_is_validated_and_committed_but_not_heard() {
    let mut hub = standing(true, vec![]);
    hub.teleport_command(0, [10.5, 70.0, 10.5], [false; 3]);
    hub.ticks = 11;
    let confirmation = confirm(&hub, 90.0);
    push(&mut hub, vec![confirmation]);
    assert!(
        moves(&drain(&mut hub)).is_empty(),
        "a changed yaw on a confirmation"
    );
    let g = &hub.guests[0];
    assert!(g.awaiting_teleport.is_none(), "it confirmed");
    assert!(g.pending_move.is_none());
    assert_eq!(
        (g.position, g.yaw),
        ([10.5, 70.0, 10.5], 90.0),
        "and committed in place"
    );
}

#[test]
fn a_confirmation_with_a_changed_position_is_not_heard() {
    let mut hub = standing(true, vec![]);
    hub.teleport_command(0, [10.5, 70.0, 10.5], [false; 3]);
    hub.ticks = 11;
    let (id, _) = hub.guests[0].awaiting_teleport.unwrap();
    // The server's own handling: the accepted position is the teleport's, the packet's own values go
    // through the same validation as any report.
    let moved = S::AcceptTeleportation(play::AcceptTeleportation {
        id,
        x: 10.75,
        y: 70.0,
        z: 10.5,
        yaw: 0.0,
        pitch: 0.0,
    });
    push(&mut hub, vec![moved]);
    assert!(moves(&drain(&mut hub)).is_empty());
    assert!(hub.guests[0].pending_move.is_none());
    assert_eq!(hub.guests[0].position, [10.75, 70.0, 10.5]);
}

#[test]
fn the_correction_a_veto_causes_is_not_heard_when_the_client_confirms_it() {
    let mut hub = standing(true, vec![turned(10.0)]);
    let conn = hub.guests[0].conn;
    assert_eq!(moves(&drain(&mut hub)), [conn]);
    hub.resolve_move(conn, false);
    hub.ticks = 11;
    // The client confirms with the look it has now, which differs from the accepted one.
    let confirmation = confirm(&hub, 10.0);
    push(&mut hub, vec![confirmation]);
    assert!(
        moves(&drain(&mut hub)).is_empty(),
        "the correction is not heard"
    );
    assert!(hub.guests[0].pending_move.is_none());
    assert_eq!(
        look(&hub),
        [10.0, 0.0],
        "the confirmation was validated and committed"
    );
}
