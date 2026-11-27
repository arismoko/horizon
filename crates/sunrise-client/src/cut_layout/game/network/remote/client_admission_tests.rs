use {sunrise_gameplay_common::game::remote::C, sunrise_gameplay_common::game::remote::play};

use crate::game::server_side::native_entity_refs::tests::with_client_game;

pub(in crate::game) fn with_isolated_client(
    name: &str,
    body: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
) {
    sunrise_gameplay_common::game::in_own_process(name, || with_client_game(body));
}

pub(in crate::game) fn registry_id(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    registry: &str,
    name: &str,
) -> i32 {
    game.client
        .remote
        .as_ref()
        .unwrap()
        .builtin
        .get(registry)
        .unwrap()
        .id(name)
        .unwrap()
}

pub(in crate::game) fn roundtrip(packet: C) -> C {
    let bytes = packet.encode();
    let mut reader = sunrise_protocol::buf::Reader::new(&bytes);
    C::decode(reader.varint().unwrap(), reader.rest()).unwrap()
}

pub(in crate::game) fn deliver(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    packet: C,
) {
    game.handle_packet(roundtrip(packet)).unwrap();
}

pub(in crate::game) fn add_entity(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    id: i32,
    uuid: u128,
    kind: &str,
    position: [f64; 3],
) -> C {
    C::AddEntity(play::AddEntity {
        id,
        uuid,
        kind: registry_id(game, "minecraft:entity_type", kind),
        position,
        velocity: [0.0; 3],
        pitch: 0,
        yaw: 0,
        head_yaw: 0,
        data: 0,
    })
}
