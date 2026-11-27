#![cfg(test)]
use {
    sunrise_gameplay_common::game::lan_actions::{S, play},
    sunrise_items::items::ItemStack,
};

#[allow(unused_imports)]
pub(in crate::game) use crate::game::server_side::lan_actions_server::boat_holder_movement_tests::scene;
#[derive(Debug, PartialEq)]
pub(in crate::game) struct Snapshot {
    pub(in crate::game) known: [f64; 3],
    pub(in crate::game) previous: [f64; 3],
    pub(in crate::game) position: [f64; 3],
    pub(in crate::game) moved: bool,
    pub(in crate::game) using: Option<(usize, u32)>,
    pub(in crate::game) yaw: f32,
    pub(in crate::game) pitch: f32,
    pub(in crate::game) hand: Option<ItemStack>,
}
pub(in crate::game) fn snapshot(g: &sunrise_gameplay::game::ServerGame<'_>) -> Snapshot {
    let h = g.server.lan.as_ref().unwrap();
    let p = &h.hub.guests[0];
    Snapshot {
        known: p.known_movement,
        previous: p.previous_position,
        position: p.position,
        moved: p.position_this_tick,
        using: h.play.get(&1).and_then(|p| p.using),
        yaw: p.yaw,
        pitch: p.pitch,
        hand: p.inventory[0].clone(),
    }
}
pub(in crate::game) fn group(
    g: &mut sunrise_gameplay::game::ServerGame<'_>,
    packets: Vec<S>,
    token: i32,
) {
    g.server.lan.as_mut().unwrap().hub.guests.update(0, |p| {
        for packet in packets {
            p.inbox
                .push_back(sunrise_sim::lan::guest::InboxItem::Packet(packet, None));
        }
    });
    // Real Hub queue/collision/event dispatch, before the receipt marker.
    g.tick_lan_packets();
    assert!(
        g.server.lan.as_ref().unwrap().hub.guests[0]
            .inbox
            .is_empty()
    );
    let before = snapshot(g);
    assert!(before.hand.is_none(), "original lead was consumed");
    assert!(before.using.is_none());

    let (yaw, pitch) = (
        g.server.lan.as_ref().unwrap().hub.guests[0].yaw,
        g.server.lan.as_ref().unwrap().hub.guests[0].pitch,
    );
    g.lan_guest_packet(
        1,
        S::UseItem(play::UseItem {
            hand: 0,
            sequence: token,
            yaw,
            pitch,
        }),
    );

    let after = snapshot(g);
    assert_eq!(
        after, before,
        "empty marker changed actual movement/use bookkeeping at {token}"
    );
    println!("marker {token}: before={before:?}, after={after:?}");
    // The existing network tick flushes receipts and resets its own packet
    // gate outside a level pass; no movement field is assigned by this test.
    g.tick_lan(false);
}
pub(in crate::game) fn walk_stop(assert_trajectory: bool) {
    let mut resources = None;
    let (mut g, _rx) = scene(&mut resources);
    for _ in 0..2 {
        g.tick_boats();
    }
    let id = g.boats[0].id;
    group(
        &mut g,
        vec![S::Interact(play::Interact {
            id: id as i32,
            hand: 0,
            location: [0.; 3],
            secondary: false,
        })],
        1,
    );
    assert!(g.boats[0].leash.is_some());
    assert_eq!(g.boats[0].leash_player, Some([0, 0, 0, 1]));
    g.tick_boats();
    let mut x = 0.5;
    for tick in 1..=30 {
        x += 0.4;
        group(
            &mut g,
            vec![
                S::MovePlayerPos(play::MovePlayerPos {
                    x,
                    y: 100.,
                    z: 0.5,
                    flags: 1,
                }),
                S::ClientTickEnd(sunrise_protocol::packets::Empty),
            ],
            tick + 1,
        );
        assert_eq!(
            g.server.lan.as_ref().unwrap().hub.guests[0].position,
            [x, 100.0, 0.5]
        );
        g.tick_boats();
    }
    for tick in 1..=10 {
        group(
            &mut g,
            vec![S::ClientTickEnd(sunrise_protocol::packets::Empty)],
            31 + tick,
        );
        assert_eq!(
            g.server.lan.as_ref().unwrap().hub.guests[0].known_movement,
            [0.; 3]
        );
        g.tick_boats();
    }
    assert!(g.boats[0].leash.is_some());
    assert!(g.server.lan.as_ref().unwrap().play[&1].using.is_none());
    if assert_trajectory {
        let p = g.boats[0].position;
        let rounded = [p.x, p.y, p.z].map(|v| format!("{v:.4}").parse::<f64>().unwrap());
        assert_eq!(
            rounded,
            [6.0676, 100., 1.3736],
            "original Java four-decimal still projection; full position {p:?}"
        );
    }
}
#[test]
#[ignore = "daily"]
fn empty_markers_preserve_actual_walk_and_still_movement_bookkeeping() {
    walk_stop(false);
}
#[test]
#[ignore = "daily"]
fn boat_tick_uses_stopped_players_known_movement() {
    walk_stop(true);
}
#[test]
#[ignore = "daily"]
fn walking_then_still_tick_end_preserves_the_accepted_displacement() {
    let mut resources = None;
    let (mut g, _rx) = scene(&mut resources);
    let id = g.boats[0].id;
    group(
        &mut g,
        vec![S::Interact(play::Interact {
            id: id as i32,
            hand: 0,
            location: [0.0; 3],
            secondary: false,
        })],
        1,
    );
    group(
        &mut g,
        vec![
            S::MovePlayerPos(play::MovePlayerPos {
                x: 0.9,
                y: 100.0,
                z: 0.5,
                flags: 1,
            }),
            S::ClientTickEnd(sunrise_protocol::packets::Empty),
        ],
        2,
    );
    let walking = snapshot(&g);
    assert_eq!(walking.known, [0.4, 0.0, 0.0]);
    assert_eq!(walking.previous, [0.5, 100.0, 0.5]);
    assert_eq!(walking.position, [0.9, 100.0, 0.5]);
    group(
        &mut g,
        vec![S::ClientTickEnd(sunrise_protocol::packets::Empty)],
        3,
    );
    let still = snapshot(&g);
    assert_eq!(still.known, [0.0; 3]);
    assert_eq!(still.previous, walking.previous);
    assert_eq!(still.position, walking.position);
    assert!(!still.moved);
}
