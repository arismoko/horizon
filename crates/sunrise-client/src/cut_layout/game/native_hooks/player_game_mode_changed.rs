#![cfg(test)]
#[cfg(test)]
mod player_game_mode_changed_tests {
    use mod_api::{Flow, GameMode, Hooks, ModEntry, PlayerGameModeChangedEvent, ServerHost};

    /// Sends a switch to creative to adventure, and one to spectator back
    /// to the mode the player has.
    struct Lobby;
    #[mod_api::__subscriptions]
    impl Hooks for Lobby {
        fn player_game_mode_changed(
            &self,
            _: &mut dyn ServerHost,
            event: &mut PlayerGameModeChangedEvent<'_>,
        ) -> Flow {
            match *event.to {
                GameMode::Creative => *event.to = GameMode::Adventure,
                GameMode::Spectator => *event.to = event.from,
                _ => {}
            }
            Flow::Continue
        }
    }

    /// A redirect is what the player gets; a redirect to the current mode
    /// changes nothing.
    #[test]
    #[ignore = "daily"]
    fn a_listener_redirects_the_new_game_mode() {
        let mods: mod_api::Mods = vec![ModEntry::new("lobby", Box::new(Lobby))].into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
                let mode = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                    game.server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(13)
                        .unwrap()
                        .game_mode
                };
                game.as_guest(13, |g| {
                    g.set_game_mode(GameMode::Survival);
                    assert!(g.set_game_mode(GameMode::Creative));
                    assert_eq!(g.mode, GameMode::Adventure);
                })
                .unwrap();
                assert_eq!(mode(game), GameMode::Adventure.id());
                let changed = game
                    .as_guest(13, |g| g.set_game_mode(GameMode::Spectator))
                    .unwrap();
                assert!(!changed, "a redirect to the current mode is no change");
                assert_eq!(mode(game), GameMode::Adventure.id());
            });
        });
    }

    /// Gives guest 13 a channel of its own and returns the values of the game-mode events sent
    /// to it since the last call.
    fn watch_modes(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
    ) -> impl Fn() -> Vec<f32> + use<> {
        let (tx, rx) = std::sync::mpsc::channel();
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(13, |g| {
                g.sender = sunrise_sim::lan::PlayerSender::new(tx, Default::default())
            })
            .unwrap();
        move || {
            let mut modes = Vec::new();
            while let Ok(out) = rx.try_recv() {
                let sunrise_sim::lan::Out::Packet(body) = out else {
                    continue;
                };
                let mut reader = sunrise_protocol::buf::Reader::new(&body);
                let id = reader.varint().unwrap();
                if let Ok(sunrise_protocol::packets::play::Clientbound::GameEvent(event)) =
                    sunrise_protocol::packets::play::Clientbound::decode(id, reader.rest())
                    && event.event == 3
                {
                    modes.push(event.param);
                }
            }
            modes
        }
    }

    /// The redirected mode is the one sent to the client, and a redirect to the current mode
    /// sends nothing.
    #[test]
    #[ignore = "daily"]
    fn a_redirect_sends_its_mode_and_a_redirect_to_the_current_mode_sends_none() {
        let mods: mod_api::Mods = vec![ModEntry::new("lobby", Box::new(Lobby))].into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
                let sent = watch_modes(game);
                game.as_guest(13, |g| assert!(g.set_game_mode(GameMode::Creative)))
                    .unwrap();
                // Adventure's id is 2.
                assert_eq!(sent(), [2.0]);
                let changed = game
                    .as_guest(13, |g| g.set_game_mode(GameMode::Spectator))
                    .unwrap();
                assert!(!changed);
                assert!(sent().is_empty());
            });
        });
    }

    /// Hears what the listener before it left in `to`.
    struct Second(std::sync::Arc<std::sync::Mutex<Vec<GameMode>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Second {
        fn player_game_mode_changed(
            &self,
            _: &mut dyn ServerHost,
            event: &mut PlayerGameModeChangedEvent<'_>,
        ) -> Flow {
            self.0.lock().unwrap().push(*event.to);
            Flow::Continue
        }
    }

    /// Listeners run in order: the second sees the first's write.
    #[test]
    #[ignore = "daily"]
    fn the_second_listener_sees_the_first_listeners_write() {
        let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![
            ModEntry::new("lobby", Box::new(Lobby)),
            ModEntry::new("second", Box::new(Second(heard.clone()))),
        ]
        .into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
                game.as_guest(13, |g| assert!(g.set_game_mode(GameMode::Creative)))
                    .unwrap();
                assert_eq!(*heard.lock().unwrap(), [GameMode::Adventure]);
            });
        });
    }
}
