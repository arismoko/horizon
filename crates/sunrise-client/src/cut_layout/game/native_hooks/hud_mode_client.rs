#![cfg(test)]
pub(crate) mod tests {
    #[cfg(test)]
    use mod_api::ModEntry;
    use mod_api::{ClientHost, ClientTickPreEvent, Hooks, HudMode};
    #[cfg(test)]
    use mod_platform::testing::with_mods;

    struct ThreeState;
    #[mod_api::__subscriptions]
    impl Hooks for ThreeState {
        fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
            host.set_hud_mode(match host.hud_mode() {
                HudMode::AllVisible => HudMode::NoHud,
                HudMode::NoHud => HudMode::AllHidden,
                _ => HudMode::AllVisible,
            });
        }
    }

    #[test]
    #[ignore = "daily"]
    fn hud_mode_game_bridge_respects_client_and_guest_guards() {
        let mods: mod_api::Mods = vec![ModEntry::new("three-state", Box::new(ThreeState))].into();
        with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                for mode in [HudMode::NoHud, HudMode::AllHidden, HudMode::AllVisible] {
                    game.test_fire_client_tick_pre();
                    assert_eq!(game.hud_mode(), mode);
                }
                game.test_fire_client_tick_pre();
                assert_eq!(game.hud_mode(), HudMode::NoHud);
                assert!(
                    game.client
                        .chat_ui
                        .hands_drawn(false, false, true, true, false)
                );
                game.toggle_hide_gui();
                assert_eq!(game.hud_mode(), HudMode::AllHidden);
                game.toggle_hide_gui();
                assert_eq!(game.hud_mode(), HudMode::AllVisible);
            });
        });
    }
}
