use crate::game::server_side::commands::knot_identity_tests::game;
use crate::game::server_side::commands::knot_identity_tests::knot_uuid;
use crate::game::server_side::commands::knot_identity_tests::knots;
use crate::game::server_side::commands::knot_identity_tests::step;
use crate::game::server_side::commands::knot_identity_tests::summon_pig;
use std::sync::{Arc, atomic::AtomicI32, mpsc};
use sunrise_gameplay_common::game::Dim;
use sunrise_protocol::packets::configuration::ClientInformation;
use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};

#[test]
#[ignore = "daily"]
fn a_server_that_starts_tracking_later_shows_the_same_knot() {
    let mut resources = None;
    let mut game = game(&mut resources);
    summon_pig(&mut game, 1, 16.5);
    step(&mut game);
    assert!(game.server.lan.is_none());
    let [id] = knots(&game)[..] else {
        panic!("{:?}", knots(&game));
    };
    let uuid = knot_uuid(&mut game);
    assert_eq!(game.persist.uuids.get(&id), Some(&uuid));
    // A later save prunes the memory of dead entities, not of this knot.
    game.prune_entity_memory();
    assert_eq!(knot_uuid(&mut game), uuid);
    // The world opens to the network; the tracker shows the same entity.
    host(&mut game);
    step(&mut game);
    step(&mut game);
    assert!(game.server.lan.is_some());
    game.lan_assign_ids();
    assert_eq!(
        game.server.knot_ids.get(&[8, 201, 0]),
        Some(&id),
        "the tracker's id"
    );
    assert_eq!(game.uuid_for(id), uuid, "the tracker's UUID");
    assert_eq!(knots(&game), [id]);
    assert_eq!(knot_uuid(&mut game), uuid);
    assert_eq!(
        game.network_entity_id(sunrise_gameplay_common::game::commands_common::Target::Knot(id)),
        Some(id as i32)
    );
}

pub(crate) fn host(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    let net = NetConfig {
        announce: false,
        ..NetConfig::lan(0, "knot-identity-test".into(), 2)
    };
    game.lan_host(
        sunrise_gameplay::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        net,
        HubConfig {
            default_game_mode: 1,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: Dim::Overworld.id().into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        },
    )
    .unwrap();
    // Existing mobs and knots already own runtime IDs when hosting starts.
    game.server.next_entity += 1;
    let guest_id = game.server.next_entity;
    let (tx, _rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: 2,
            uuid: 2,
            name: "KnotGuest".into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        guest_id as i32,
        [12.5, 201.0, 4.5],
        1,
    );
    guest.placed = true;
    guest.loaded = true;
    game.server.lan.as_mut().unwrap().hub.guests.push(guest);
}
