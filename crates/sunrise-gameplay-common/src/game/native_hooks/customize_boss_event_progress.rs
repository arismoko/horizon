#[cfg(any())]
#[cfg(any(test, feature = "test-support"))]
pub mod customize_boss_event_progress_tests {
    use mod_api::{
        BossBarColor, ClientHost, CustomizeBossEventProgressEvent, Flow, Hooks, ModEntry,
    };
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};
    use sunrise_client::hud_layout::GuiDraw;

    #[cfg(test)]
    pub(in crate::game) type Seen = Arc<Mutex<Vec<(u128, BossBarColor, i32, i32, i32)>>>;

    #[cfg(test)]
    /// Spaces the pink bar 30 pixels from the next, hides the blue one and moves the red one
    /// 10 pixels right, logging what it was told.
    pub(in crate::game) struct Rearrange(pub(in crate::game) Seen);
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl Hooks for Rearrange {
        fn customize_boss_event_progress(
            &self,
            _: &dyn ClientHost,
            event: &mut CustomizeBossEventProgressEvent<'_>,
        ) -> Flow {
            let bar = event.bar;
            self.0
                .lock()
                .unwrap()
                .push((bar.uuid, bar.color, event.x, event.y, event.increment));
            match bar.color {
                BossBarColor::Pink => event.increment = 30,
                BossBarColor::Blue => return Flow::Cancel,
                _ => event.x += 10,
            }
            Flow::Continue
        }
    }

    #[allow(unused_imports)]
pub(in crate::game) use crate::game::client_side::native_hooks_client::customize_boss_event_progress::customize_boss_event_progress_tests::add_bars;

    /// Each bar background's sprite and top-left corner.
    pub fn backgrounds(draws: &[GuiDraw]) -> Vec<(String, i32, i32)> {
        draws
            .iter()
            .filter_map(|draw| match draw {
                GuiDraw::Sprite { sprite, x, y, .. } if sprite.ends_with("_background") => {
                    Some((sprite.to_string(), *x, *y))
                }
                _ => None,
            })
            .collect()
    }
}
