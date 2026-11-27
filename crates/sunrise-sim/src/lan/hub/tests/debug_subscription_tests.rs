//! `ServerPlayer.requestDebugSubscriptions` and `ServerDebugSubscribers.hasRequiredPermissions`:
//! anyone's request is stored, only an operator's is honoured.
use super::pending_move_tests::{drain, standing};
use super::*;

fn request(subscriptions: Vec<i32>) -> S {
    S::DebugSubscriptionRequest(play::DebugSubscriptionRequest { subscriptions })
}

#[test]
fn a_request_is_stored_and_only_an_operator_is_a_tick_time_subscriber() {
    let mut hub = standing(false, vec![request(vec![0])]);
    let (uuid, conn) = (hub.guests[0].uuid, hub.guests[0].conn);
    drain(&mut hub);
    assert_eq!(hub.guests[0].debug_subscriptions, [0]);
    assert!(hub.tick_time_subscribers().is_empty(), "not an operator");
    hub.ops.insert(uuid, 0);
    assert_eq!(hub.tick_time_subscribers(), [conn], "any op level passes");
    hub.ops.remove(&uuid);
    assert!(
        hub.tick_time_subscribers().is_empty(),
        "deop ends it, the request stays"
    );
    assert_eq!(hub.guests[0].debug_subscriptions, [0]);
}

#[test]
fn the_tick_time_snapshot_counts_joined_live_network_players_including_respawns() {
    let mut hub = standing(false, vec![request(vec![0])]);
    let conn = hub.guests[0].conn;
    hub.ops.insert(hub.guests[0].uuid, 4);
    drain(&mut hub);
    assert_eq!(hub.tick_time_subscribers(), [conn]);

    hub.guests.update(0, |g| g.placed = false);
    assert!(hub.tick_time_subscribers().is_empty());
    hub.guests.update(0, |g| g.respawning = true);
    assert_eq!(hub.tick_time_subscribers(), [conn]);

    hub.guests.update(0, |g| g.disconnect_pending = true);
    assert!(hub.tick_time_subscribers().is_empty());
}

#[test]
fn a_closed_outbox_stops_counting_before_the_hub_sees_the_disconnect() {
    use crate::lan::PlayerSender;
    use crate::raw_payloads::Transport;

    let mut hub = standing(false, vec![request(vec![0])]);
    let (sender, _) = PlayerSender::payload_executor_test_sender(Transport::empty());
    hub.guests.update(0, |g| g.sender = sender);
    let (uuid, conn) = (hub.guests[0].uuid, hub.guests[0].conn);
    hub.ops.insert(uuid, 4);
    drain(&mut hub);
    assert_eq!(hub.tick_time_subscribers(), [conn]);

    hub.guests[0].sender.close();
    assert!(!hub.guests[0].disconnect_pending);
    assert!(hub.tick_time_subscribers().is_empty());
}

#[test]
fn an_empty_request_unsubscribes_and_other_ids_do_not_subscribe() {
    let mut hub = standing(false, vec![request(vec![0]), request(vec![])]);
    hub.ops.insert(hub.guests[0].uuid, 4);
    drain(&mut hub);
    assert!(hub.guests[0].debug_subscriptions.is_empty());
    assert!(hub.tick_time_subscribers().is_empty());
    hub.guests.update(0, |g| g.debug_subscriptions = vec![1, 2]);
    assert!(hub.tick_time_subscribers().is_empty());
}

#[test]
fn a_cleared_request_survives_the_frozen_inbox() {
    let mut hub = standing(false, vec![request(vec![])]);
    hub.ops.insert(hub.guests[0].uuid, 4);
    hub.guests.update(0, |g| g.debug_subscriptions = vec![0]);

    hub.keep_acknowledgements(0, &mut Vec::new());
    assert_eq!(
        hub.guests[0].debug_subscriptions,
        [0],
        "not applied while frozen"
    );
    drain(&mut hub);
    assert!(hub.tick_time_subscribers().is_empty(), "applied on thaw");
}

#[test]
fn a_request_does_not_reset_the_idle_timer() {
    let mut hub = standing(false, vec![request(vec![0])]);
    hub.guests.update(0, |g| g.last_action_tick = 7);
    drain(&mut hub);
    assert_eq!(hub.guests[0].debug_subscriptions, [0]);
    assert_eq!(hub.guests[0].last_action_tick, 7);
}
