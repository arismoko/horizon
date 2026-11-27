use sunrise_client::level_loading::Reason;
use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
use sunrise_protocol::{connection::Pipe, packets::play::SpawnInfo};
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_client_gameplay::game::client_side::native_hooks_client::particle_count_tests::fill,
};

#[test]
#[ignore = "daily"]
fn connection_and_dimension_replacement_clear_every_particle_group() {
    with_client_game(|game| {
        fill(game);
        // Include pending particles as well as live groups before replacing the connection.
        game.client.particles.0.elder_guardian();
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "ParticleReset").unwrap();
        assert_eq!(game.client_particle_count(), 0);
        assert!(game.client.pickups.0.live.is_empty());
        game.tick_particles();
        assert_eq!(game.client_particle_count(), 0);
        let spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 1,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        fill(game);
        game.client.particles.0.elder_guardian();
        game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
        assert_eq!(game.client_particle_count(), 0);
        assert!(game.client.pickups.0.live.is_empty());
        game.tick_particles();
        assert_eq!(
            game.client_particle_count(),
            0,
            "pending groups cannot return in the new level"
        );
    });
}
