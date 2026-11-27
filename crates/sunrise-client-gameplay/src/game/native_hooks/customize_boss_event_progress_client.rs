#[cfg(any())]
#[cfg(any(test, feature = "test-support"))]
pub mod customize_boss_event_progress_tests {
    use sunrise_gameplay_common::game::native_hooks::customize_boss_event_progress::customize_boss_event_progress_tests::backgrounds;

    use crate::game::native_entity_refs::tests::run_fresh_cases;
use mod_api::{
             ModEntry,
        text::TextFormError};
use mod_platform::testing::with_mods;
use std::sync::{Arc};
use sunrise_core::nbt::Tag;
use sunrise_protocol::packets::play::{BossAction, BossEvent};
use sunrise_gameplay_common::game::native_hooks::customize_boss_event_progress::customize_boss_event_progress_tests::Seen;
use sunrise_gameplay_common::game::native_hooks::customize_boss_event_progress::customize_boss_event_progress_tests::Rearrange;
use sunrise_gameplay_common::game::native_hooks::customize_boss_event_progress::customize_boss_event_progress_tests::backgrounds;

    /// Pink, blue and red bars, in that order.
    pub fn add_bars(game: &mut crate::game::ClientGame<'_>) {
        for (id, color) in [(1, 0), (2, 1), (3, 2)] {
            let packet = BossEvent {
                id,
                action: BossAction::Add {
                    name: Tag::String(format!("bar {id}").as_str().into()),
                    progress: 1.0,
                    color,
                    overlay: 0,
                    darken: false,
                    music: false,
                    fog: false,
                },
            };
            game.client.boss_overlay.apply(&packet, 0);
        }
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn an_unconvertible_boss_bar_keeps_vanilla_draws_without_dispatch() {
        if run_fresh_cases(
            "game::native_hooks::customize_boss_event_progress::customize_boss_event_progress_tests::an_unconvertible_boss_bar_keeps_vanilla_draws_without_dispatch",
            "SUNRISE_BOSS_BAR_CONVERSION_CHILD",
            &["malformed-profile"],
        ) {
            return;
        }
        super::super::super::native_entity_refs::tests::with_client_game(|game| {
            // Inject a malformed local component without the packet codec. Its UUID has
            // three words instead of the four required by vanilla's UUIDUtil.CODEC.
            let name = Tag::compound([
                ("object", Tag::String("player".into())),
                (
                    "player",
                    Tag::compound([
                        ("name", Tag::String("Steve".into())),
                        ("id", Tag::IntArray(vec![1, 2, 3])),
                    ]),
                ),
            ]);
            game.side.boss_overlay.apply(
                &BossEvent {
                    id: 1,
                    action: BossAction::Add {
                        name,
                        progress: 1.0,
                        color: 0,
                        overlay: 0,
                        darken: false,
                        music: false,
                        fog: false,
                    },
                },
                0,
            );
            assert!(matches!(
                crate::game::client_side::api_values_client::mod_bar_value(
                    &game.side.boss_overlay.mod_bar(0, 0).unwrap()
                ),
                Err(TextFormError::Malformed)
            ));
            let vanilla = game.boss_bar_draws(480, 270);
            assert!(!backgrounds(&vanilla).is_empty());
            let seen = Seen::default();
            let mods = vec![ModEntry::new(
                "rearrange",
                Box::new(Rearrange(Arc::clone(&seen))),
            )]
            .into();
            with_mods(mods, || {
                assert_eq!(game.boss_bar_draws(480, 270), vanilla);
                assert!(seen.lock().unwrap().is_empty());
            });
        });
    }
}
