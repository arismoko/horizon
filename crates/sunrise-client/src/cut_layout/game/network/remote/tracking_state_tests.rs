use sunrise_client_gameplay::game::client_side::remote_client::sync::entity_sound_state;
use {
    sunrise_client_gameplay::game::client_side::remote_client::session_state::Tracked,
    sunrise_client_gameplay::game::client_side::remote_client::session_state::seat_passengers,
};

/// `ClientPacketListener.handleSetEntityPassengersPacket`: the vehicle
/// ejects its passengers, then each listed entity `startRiding`s it, so
/// `isVehicle` holds while the list is not empty, whatever the passenger's
/// type: a server's item display riding a player hides that player's name
/// tag like a text display does (#5666).
#[test]
fn any_tracked_passenger_makes_its_vehicle_a_vehicle() {
    use std::collections::{HashMap, HashSet};
    let tracked = |kind: &str| Tracked {
        kind: kind.to_owned(),
        position: [0.0, 64.0, 0.0],
        previous: [0.0, 64.0, 0.0],
        yaw: 0.0,
        pitch: 0.0,
        previous_yaw: 0.0,
        previous_pitch: 0.0,
        codec_base: [0.0, 64.0, 0.0],
        on_fire: false,
        mod_fire_immune: false,
        silent: false,
        vehicle: None,
        tick_count: 0,
        size_answer: None,
    };
    let mut entities = HashMap::from([
        (42, tracked("minecraft:player")),
        (43, tracked("minecraft:player")),
        (9, tracked("minecraft:item_display")),
        (10, tracked("minecraft:block_display")),
    ]);
    let vehicles = |e: &HashMap<i32, Tracked>| -> HashSet<u32> {
        e.values().filter_map(|t| t.vehicle).collect()
    };
    seat_passengers(&mut entities, 42, &[9, 10]);
    assert_eq!(vehicles(&entities), HashSet::from([42]));
    // The list replaces the old one: 10 got off, 9 still rides.
    seat_passengers(&mut entities, 42, &[9]);
    assert_eq!(entities[&10].vehicle, None);
    assert_eq!(entities[&9].vehicle, Some(42));
    // startRiding leaves the old vehicle first.
    seat_passengers(&mut entities, 43, &[9]);
    assert_eq!(vehicles(&entities), HashSet::from([43]));
    // An empty list: nobody rides 43 any more.
    seat_passengers(&mut entities, 43, &[]);
    assert!(vehicles(&entities).is_empty());
}

#[test]
fn a_dragon_sound_uses_live_movement_without_losing_mob_metadata() {
    use sunrise_game_data::sound_source::SoundEntityState;
    let stale = SoundEntityState {
        position: [0.0, 64.0, 0.0],
        silent: true,
        removed: false,
    };
    let tracked = std::collections::HashMap::new();
    for position in [[24.0, 80.0, -16.0], [25.5, 81.0, -18.0]] {
        let state =
            entity_sound_state(7, None, Some(stale), &tracked, Some((7, position))).unwrap();
        assert_eq!(state.position, position);
        assert!(state.silent);
        assert!(!state.removed);
    }
    let unrelated =
        entity_sound_state(7, None, Some(stale), &tracked, Some((8, [99.0; 3]))).unwrap();
    assert_eq!(unrelated.position, stale.position);
    assert!(entity_sound_state(7, None, None, &tracked, Some((7, [99.0; 3]))).is_none());
}
