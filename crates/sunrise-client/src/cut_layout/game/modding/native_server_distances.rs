#![cfg(test)]
#[cfg(test)]
mod tests {
    use core::ops::ControlFlow;
    use mod_api::ServerHost;
    use sunrise_gameplay::game::{
        ServerGame,
        native_guest_turn_tests::{with_guests, with_guests_for_role},
        server_side::integrated::Serving,
    };
    use sunrise_gameplay_common::game::{
        execution_role::ExecutionRole, test_fixtures::fixture_root,
    };

    #[test]
    #[ignore = "daily"]
    fn a_mod_reads_and_sets_the_servers_distances() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::native_server_distances::tests::a_mod_reads_and_sets_the_servers_distances",
            || {
                let advertised = |game: &ServerGame<'_>| {
                    let config = &game.server.lan.as_ref().unwrap().hub.config;
                    (config.view_distance, config.simulation_distance)
                };
                let ticket_settings = |game: &ServerGame<'_>| {
                    (
                        game.server.settings.render_distance,
                        game.server.settings.simulation_distance,
                    )
                };
                for role in [
                    ExecutionRole::IntegratedServer,
                    ExecutionRole::DedicatedServer,
                ] {
                    with_guests_for_role(fixture_root(), role, |game| {
                        if role == ExecutionRole::IntegratedServer {
                            game.server.serving = Some(Serving::fixture(0));
                            assert_eq!(game.published_port(), None);
                        }
                        game.clock = sunrise_sim::clock::FixedStep::default();
                        for took in [40_000_000, 51_000_000, 62_000_000] {
                            game.clock.record(std::time::Duration::from_nanos(took));
                        }
                        let host: &mut dyn ServerHost = game;
                        let mut times = Vec::new();
                        assert_eq!(
                            host.visit_tick_times(&mut |took| {
                                times.push(took);
                                ControlFlow::Continue(())
                            }),
                            Ok(ControlFlow::Continue(()))
                        );
                        assert_eq!(times.len(), 100);
                        assert_eq!(times[..4], [40_000_000, 51_000_000, 62_000_000, 0]);
                        times.clear();
                        assert_eq!(
                            host.visit_tick_times(&mut |took| {
                                times.push(took);
                                ControlFlow::Break(())
                            }),
                            Ok(ControlFlow::Break(()))
                        );
                        assert_eq!(times, [40_000_000]);
                        assert_eq!(host.server_view_distance(), Some(2));
                        assert_eq!(host.server_simulation_distance(), Some(2));
                        assert!(host.server_set_view_distance(6), "{role:?}");
                        assert!(host.server_set_simulation_distance(5), "{role:?}");
                        for outside in [1, 33] {
                            assert!(!host.server_set_view_distance(outside));
                            assert!(!host.server_set_simulation_distance(outside));
                        }
                        assert_eq!(host.server_view_distance(), Some(6));
                        assert_eq!(host.server_simulation_distance(), Some(5));
                        assert_eq!(advertised(game), (6, 5));
                        assert_eq!(ticket_settings(game), (6, 5));
                        let hub = &game.server.lan.as_ref().unwrap().hub;
                        assert_eq!(hub.guest(12).unwrap().view_distance, 6);
                        assert_eq!(hub.guest(13).unwrap().view_distance, 6);

                        let player_list = game.server.lan.take();
                        let host: &mut dyn ServerHost = game;
                        assert_eq!(host.server_view_distance(), None);
                        assert_eq!(host.server_simulation_distance(), None);
                        assert!(!host.server_set_view_distance(7));
                        assert!(!host.server_set_simulation_distance(8));
                        assert_eq!(ticket_settings(game), (6, 5));

                        // Even a Game with a hub cannot act as the server from the client role.
                        game.server.lan = player_list;
                        game.options.execution_role = ExecutionRole::Client;
                        let host: &mut dyn ServerHost = game;
                        assert_eq!(host.server_view_distance(), None);
                        assert_eq!(host.server_simulation_distance(), None);
                        assert!(!host.server_set_view_distance(7));
                        assert!(!host.server_set_simulation_distance(8));
                        assert_eq!(advertised(game), (6, 5));
                        assert_eq!(ticket_settings(game), (6, 5));
                    });
                }
            },
        );
    }

    /// A single-player world's server is published while it is open to LAN, and no longer once
    /// it closes, though it runs headless (`options.dedicated` set) throughout; a dedicated
    /// server always is.
    #[test]
    #[ignore = "daily"]
    fn the_server_is_published_while_open_to_lan() {
        use sunrise_gameplay_common::game::execution_role::ExecutionRole;
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::native_server_distances::tests::the_server_is_published_while_open_to_lan",
            "SUNRISE_TEST_SERVER_PUBLISHED_CASE",
            &["published"],
        ) {
            return;
        }
        with_guests(|game| {
            assert_eq!(game.options.execution_role, ExecutionRole::IntegratedServer);
            game.options.dedicated = Some(Vec::new());
            game.server.serving =
                Some(sunrise_gameplay::game::server_side::integrated::Serving::fixture(0));
            let published = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                let host: &dyn mod_api::ServerHost = game;
                host.server_is_published()
            };
            assert!(!published(game), "a world not yet open to LAN");
            let settings = sunrise_gameplay::game::server_side::lan::LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            game.publish(settings).unwrap();
            assert!(published(game), "opened to LAN");
            assert!(game.unpublish());
            assert!(!published(game), "the LAN closed");
            game.server.serving = None;
            game.options.execution_role = ExecutionRole::DedicatedServer;
            assert!(published(game), "a dedicated server");
        });
    }
}
