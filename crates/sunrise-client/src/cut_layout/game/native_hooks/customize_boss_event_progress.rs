#![cfg(test)]
#[cfg(test)]
#[cfg(any())]
pub(crate) mod customize_boss_event_progress_tests {

    use sunrise_client_gameplay::game::client_side::native_hooks_client::customize_boss_event_progress::customize_boss_event_progress_tests::add_bars;

    use sunrise_gameplay_common::game::native_hooks::customize_boss_event_progress::customize_boss_event_progress_tests::backgrounds;

    use sunrise_gameplay_common::game::native_hooks::customize_boss_event_progress::customize_boss_event_progress_tests::{Rearrange, Seen};

    use mod_api::{
        BossBarColor, ClientHost, CustomizeBossEventProgressEvent, Flow, Hooks, ModEntry,
    };

    use mod_platform::testing::with_mods;

    use std::sync::{Arc, Mutex};

    pub(crate) use crate::game::client_side::native_hooks_client::customize_boss_event_progress::customize_boss_event_progress_tests::add_bars;

    /// `CustomizeGuiOverlayEvent.BossEventProgress`: with no mod the bars sit 19 pixels apart from
    /// y = 12; a mod hears each bar at vanilla's place, and hides one, spaces the next and moves
    /// another without touching the rest.
    #[test]
    #[ignore = "daily"]
    fn a_client_mod_hides_and_spaces_boss_bars() {
        let (w, h) = (480, 270);
        let left = w / 2 - 91;
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            add_bars(game);
            assert_eq!(
                backgrounds(&game.boss_bar_draws(w, h)),
                [
                    ("boss_bar/pink_background".to_owned(), left, 12),
                    ("boss_bar/blue_background".to_owned(), left, 31),
                    ("boss_bar/red_background".to_owned(), left, 50),
                ]
            );
        });
        let seen = Seen::default();
        let mods: mod_api::Mods = vec![ModEntry::new(
            "rearrange",
            Box::new(Rearrange(Arc::clone(&seen))),
        )]
        .into();
        with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                add_bars(game);
                assert_eq!(
                    backgrounds(&game.boss_bar_draws(w, h)),
                    [
                        ("boss_bar/pink_background".to_owned(), left, 12),
                        (
                            "boss_bar/red_background".to_owned(),
                            left + 10,
                            12 + 30 + 19
                        ),
                    ]
                );
                assert_eq!(
                    *seen.lock().unwrap(),
                    [
                        (1, BossBarColor::Pink, left, 12, 19),
                        (2, BossBarColor::Blue, left, 42, 19),
                        (3, BossBarColor::Red, left, 61, 19),
                    ]
                );
            });
        });
    }
}
