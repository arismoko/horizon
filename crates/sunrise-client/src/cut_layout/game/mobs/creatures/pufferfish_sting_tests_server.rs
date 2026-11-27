use sunrise_core::geometry::Vec3;
use sunrise_protocol::packets::play::Clientbound;
use sunrise_sim::mobs::Mob;
use sunrise_sim::motion::PlayerMotion;

/// The game events the stung player's client was sent, as `(event, parameter)`.
pub(crate) fn stings(game: &sunrise_gameplay::game::ServerGame<'_>) -> Vec<(u8, f32)> {
    game.server
        .guest_packets
        .iter()
        .filter_map(|packet| match packet {
            Clientbound::GameEvent(e) => Some((e.event, e.param)),
            _ => None,
        })
        .collect()
}

/// A game whose window player touches one puffed (state 2) pufferfish. The
/// server acts for the guest `lan_acting`: `playerTouch` runs there, and the
/// event it sends reaches that player's own client.
pub(crate) fn with_fish(
    silent: bool,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        game.creative = false;
        // The acting body is the guest's, not a second local player with the same ID.
        let mut body = PlayerMotion::new(game.motion, Vec3::new(8.5, 66.0, 8.5)).unwrap();
        body.entity_id = 1;
        game.player = Some(body);
        game.server.lan_acting_entity = Some(1);
        game.server.lan_acting_uuid = Some(1);
        game.server.lan_acting = Some(7);
        game.server.lan_acting_name = Some("PufferfishTest".into());
        sunrise_gameplay::game::native_entity_refs::tests::bind_acting_guest(game);
        let kind = game
            .res
            .mob_types()
            .get("minecraft:pufferfish")
            .unwrap()
            .clone();
        let mut fish = Mob::new(9001, kind, Vec3::new(8.5, 66.0, 8.5), 0.0);
        fish.extra.size = 2;
        fish.silent = silent;
        game.mobs.push(fish);
        test(game);
    });
}
