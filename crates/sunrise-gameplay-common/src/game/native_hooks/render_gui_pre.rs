#[cfg(any(test, feature = "test-support"))]
pub mod local_sound_tests {
    use mod_api::{ClientHost, Flow, Hooks, RenderGuiPreEvent, SoundPlay};

    pub struct Mod;
    #[mod_api::__subscriptions]
    impl Hooks for Mod {
        fn render_gui_pre(&self, host: &dyn ClientHost, _: &mut RenderGuiPreEvent) -> Flow {
            assert!(
                host.play_local_sound(&SoundPlay::ui("minecraft:ui.button.click"), 0)
                    .is_some()
            );
            Flow::Continue
        }
    }
}
