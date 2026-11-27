//! Sunrise's extras in the game (`docs/extras.md`).
//! Each hook does nothing while its extra is off and no mod uses it.
#[path = "extras/mouse_tweaks.rs"]
pub mod mouse_tweaks;
#[path = "extras/sorting.rs"]
mod sorting;

use super::Screen;
use std::time::Duration;
use sunrise_client::{
    extras::Corner,
    extras_hud::{self, Corners},
    hud::HudBatch,
};

#[derive(Default)]
pub(in crate::game) struct ExtrasState {
    /// Captured before pre listeners; shared by the draw and post listeners.
    pub(in crate::game) frame_partial_ticks: Option<(f32, f32)>,
}

impl crate::game::ClientGame<'_> {
    /// Once per frame: update frame timing and the extra HUD state.
    pub fn extras_frame(&mut self, elapsed: Duration) {
        let (world_partial_tick, partial_tick) = self.frame_partial_ticks();
        self.client.extras.frame_partial_ticks = Some((world_partial_tick, partial_tick));
        self.native_render_frame_pre(elapsed, world_partial_tick, partial_tick);
        // A clear a mod asked for (in this frame's `RenderFrameEvent.Pre`, or after the last
        // frame was drawn) lands before this frame's particles are drawn, paused or not.
        self.apply_mod_particle_clear();
        self.pvp_frame();
        self.recipes_received_now();
    }
    /// The PvP HUD widgets and Sunrise hook layers (hidden with F1 by the caller).
    pub(in crate::game::client_side) fn draw_extras(&mut self, batch: &mut HudBatch) {
        let mut corners = Corners::new(batch.width, batch.height, self.effect_rows_height());
        self.draw_pvp_hud(batch, &mut corners);
        // The layers crates registered (`sunrise_hooks::hud`): none, no canvas.
        let layers = sunrise_hooks::hud::engine::all();
        if !layers.is_empty() {
            let shown = Corner::ALL.map(|corner| !(self.client.debug && corner == Corner::TopLeft));
            extras_hud::draw_layers(layers, batch, &self.client.gui.atlas, &mut corners, shown);
        }
    }
    /// The height the status effect icons take at the top right
    /// (`Hud.extractEffects`: beneficial at y 1, harmful at y 26).
    pub(in crate::game::client_side) fn effect_rows_height(&self) -> f32 {
        let effects = &self.common.potions.effects.list;
        if effects.is_empty() || self.client.screen != Screen::Playing || self.pvp_effects_shown() {
            return 0.0;
        }
        let harmful = effects.iter().any(|e| {
            sunrise_level::brewing::effect(e.id)
                .is_some_and(|d| d.category != sunrise_level::brewing::Category::Beneficial)
        });
        if harmful { 50.0 } else { 25.0 }
    }
}
