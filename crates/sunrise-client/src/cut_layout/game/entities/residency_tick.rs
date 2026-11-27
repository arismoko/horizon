#![cfg(test)]
#[cfg(test)]
mod tests {
    #[cfg(feature = "client")]
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    #[cfg(feature = "client")]
    use sunrise_core::geometry::Vec3;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::GameGameExt1;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::residency_tick::ResidencyTickGameExt as _;
    #[cfg(feature = "client")]
    use sunrise_sim::mobs::Mob;

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn client_tick_drains_actual_bound_owner_events_without_a_lookup() {
        with_client_game(|game| {
            let (client, server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "residency-client").unwrap();
            game.client.remote.as_mut().unwrap().logged_in = true;
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let journal = game.resident_entities.table.borrow().producer();
            for id in 800..820 {
                game.mobs.push(Mob::new(id, kind.clone(), Vec3::ZERO, 0.0));
            }
            game.tick().unwrap();
            let mut remaining = Vec::new();
            journal.drain_into(&mut remaining);
            assert!(
                remaining.is_empty(),
                "remote tick left owner events buffered"
            );
            game.mobs.clear();
            let result = game.with_residency_tick(|_| Err("interrupted client tick".into()));
            assert!(result.is_err());
            journal.drain_into(&mut remaining);
            assert!(
                remaining.is_empty(),
                "error return left owner events buffered"
            );
            server.shutdown();
        });
    }
}
