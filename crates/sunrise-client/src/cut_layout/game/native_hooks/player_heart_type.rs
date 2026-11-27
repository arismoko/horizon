#![cfg(test)]
#[cfg(test)]
mod player_heart_type_tests {
    #[cfg(feature = "client")]
    use mod_api::ModEntry;
    #[cfg(feature = "client")]
    use mod_api::client::hud_layout::HeartKind;
    #[cfg(feature = "client")]
    use mod_api::{ClientHost, Hooks, PlayerHeartTypeEvent};
    #[cfg(feature = "client")]
    use mod_platform::testing::with_mods;
    #[cfg(feature = "client")]
    use std::sync::{Arc, Mutex};

    /// Draws frozen hearts for normal ones and withered for frozen ones, logging each pick.
    #[cfg(feature = "client")]
    struct Swap(Arc<Mutex<Vec<HeartKind>>>);
    #[cfg(feature = "client")]
    #[mod_api::__subscriptions]
    impl Hooks for Swap {
        fn player_heart_type(&self, _: &mut dyn ClientHost, event: &mut PlayerHeartTypeEvent) {
            self.0.lock().unwrap().push(*event.kind);
            *event.kind = match *event.kind {
                HeartKind::Normal => HeartKind::Frozen,
                HeartKind::Frozen => HeartKind::Withered,
                other => other,
            };
        }
    }

    #[cfg(feature = "client")]
    fn add_player(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
        let at = sunrise_core::geometry::Vec3::ZERO;
        game.player = Some(sunrise_sim::motion::PlayerMotion::new(game.motion, at).unwrap());
    }

    /// `PlayerHeartTypeEvent`: the mod gets vanilla's pick and its answer is drawn; without a
    /// listener the pick is vanilla's.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_client_mod_changes_the_health_bars_hearts() {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            assert_eq!(game.player_heart_kind(), None, "no player, no hearts");
            add_player(game);
            assert_eq!(game.player_heart_kind(), Some(HeartKind::Normal));
            game.player.as_mut().unwrap().ticks_frozen = 1000;
            assert_eq!(game.player_heart_kind(), Some(HeartKind::Frozen));
            game.player.as_mut().unwrap().ticks_frozen = 0;
        });
        let seen = Arc::default();
        let mods: mod_api::Mods =
            vec![ModEntry::new("swap", Box::new(Swap(Arc::clone(&seen))))].into();
        with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                add_player(game);
                assert_eq!(game.player_heart_kind(), Some(HeartKind::Frozen));
                game.player.as_mut().unwrap().ticks_frozen = 1000;
                assert_eq!(game.player_heart_kind(), Some(HeartKind::Withered));
                assert_eq!(
                    *seen.lock().unwrap(),
                    [HeartKind::Normal, HeartKind::Frozen]
                );
            });
        });
    }
}
