#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use sunrise_gameplay_common::game::native_player_death::tests::new_guest;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;

    use mod_api::PlayerRef;

    /// Opens the game to a LAN host with one placed guest (UUID 123, conn 1).
    pub fn add_guest(game: &mut crate::game::ServerGame<'_>) -> PlayerRef {
        use sunrise_sim::lan::{NetConfig, hub::HubConfig};

        let settings = super::super::lan::LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        };
        let net = NetConfig {
            announce: false,
            ..NetConfig::lan(0, "DeathTest".into(), 2)
        };
        let config = HubConfig {
            default_game_mode: 0,
            guest_commands: false,
            force_game_mode: false,
            hardcore: false,
            view_distance: 10,
            simulation_distance: 10,
            dimension: "minecraft:overworld".into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        };
        game.lan_host(settings, net, config).unwrap();
        let guest = new_guest(1, 123);
        game.server.lan.as_mut().unwrap().hub.guests.push(guest);
        game.player(123).unwrap()
    }
}
