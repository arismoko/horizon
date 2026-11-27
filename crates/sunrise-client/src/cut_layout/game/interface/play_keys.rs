#![cfg(test)]
#[cfg(test)]
pub(crate) mod pause_close_tests {
    use sunrise_protocol::packets::play::Serverbound as S;
    use winit::keyboard::KeyCode;
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_gameplay_common::game::Screen,
    };

    /// On a server, Escape out of the inventory closes its menu with a
    /// ServerboundContainerClosePacket; Escape out of the pause menu sends
    /// nothing (PauseScreen.onClose only swaps the screen).
    #[test]
    #[ignore = "daily"]
    fn escape_out_of_the_pause_menu_closes_no_container() {
        with_client_game(|game| {
            let (mut peer, _writer) = crate::game::remote::tests::connect_menu_test(game);
            peer.set_read_timeout(Some(std::time::Duration::from_millis(50)))
                .unwrap();
            while peer.recv().is_ok() {}
            for (screen, closes) in [(Screen::Paused, 0), (Screen::Inventory, 1)] {
                game.set_screen(screen);
                game.key_pressed(KeyCode::Escape, KeyCode::Escape).unwrap();
                assert_eq!(game.client.screen, Screen::Playing);
                let mut sent = 0;
                while let Ok((id, bytes)) = peer.recv() {
                    if matches!(S::decode(id, &bytes), Ok(S::ContainerClose(_))) {
                        sent += 1;
                    }
                }
                assert_eq!(sent, closes, "Escape out of {screen:?}");
            }
        });
    }
}

#[cfg(test)]
pub(crate) mod creative_close_tests {
    use mod_api::{ClientHost, Hooks, ModEntry, ScreenOpening, ScreenOpeningEvent};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use sunrise_protocol::packets::play::Serverbound as S;
    use winit::keyboard::KeyCode;
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_gameplay_common::game::Screen,
    };

    /// Keeps the open screen while `veto` is set.
    struct Veto(Arc<AtomicBool>);
    #[mod_api::__subscriptions]
    impl Hooks for Veto {
        fn screen_opening(
            &self,
            _host: &dyn ClientHost,
            _event: &mut ScreenOpeningEvent<'_>,
        ) -> Option<ScreenOpening> {
            self.0
                .load(Ordering::SeqCst)
                .then_some(ScreenOpening::Cancel)
        }
    }

    /// On a server, leaving the creative inventory closes its menu, window 0, with one
    /// ServerboundContainerClosePacket (LocalPlayer.closeContainer), by Escape and by the
    /// inventory key; a close a mod vetoes keeps the screen and tells the server nothing.
    #[test]
    #[ignore = "daily"]
    fn leaving_the_creative_inventory_tells_the_server() {
        let veto = Arc::new(AtomicBool::new(true));
        let mods: mod_api::Mods = vec![ModEntry::new("veto", Box::new(Veto(veto.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                let (mut peer, _writer) = crate::game::remote::tests::connect_menu_test(game);
                peer.set_read_timeout(Some(std::time::Duration::from_millis(50)))
                    .unwrap();
                let closes = |peer: &mut sunrise_protocol::connection::ConnReader| {
                    let mut closed = Vec::new();
                    while let Ok((id, bytes)) = peer.recv() {
                        if let Ok(S::ContainerClose(p)) = S::decode(id, &bytes) {
                            closed.push(p.container);
                        }
                    }
                    closed
                };
                veto.store(false, Ordering::SeqCst);
                game.set_screen(Screen::Creative);
                closes(&mut peer);
                veto.store(true, Ordering::SeqCst);
                game.key_pressed(KeyCode::Escape, KeyCode::Escape).unwrap();
                assert_eq!(
                    game.client.screen,
                    Screen::Creative,
                    "a vetoed close keeps the screen"
                );
                assert!(closes(&mut peer).is_empty());
                veto.store(false, Ordering::SeqCst);
                for key in [KeyCode::Escape, KeyCode::KeyE] {
                    game.set_screen(Screen::Creative);
                    closes(&mut peer);
                    game.key_pressed(key, key).unwrap();
                    assert_eq!(game.client.screen, Screen::Playing, "{key:?}");
                    assert_eq!(closes(&mut peer), [0], "{key:?}");
                }
            });
        });
    }
}
