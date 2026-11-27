use super::*;

impl crate::game::ClientGame<'_> {
    /// `stack`'s tooltip lines: the game's own, as `fluid_tooltip` listeners left them.
    pub fn fluid_tooltip_lines(&self, stack: &FluidStack) -> Vec<(Text, u32)> {
        let mut lines =
            sunrise_client::fluid_tooltip::lines(stack, self.options_advanced_tooltips());
        if mod_api::__private::listened(mod_api::HookId::FluidTooltip) {
            use sunrise_gameplay_common::game::api_values::{TryIntoApi, TryIntoEngine};
            if let Ok(mut snapshot) = lines.try_into_api() {
                self.native_fluid_tooltip(mod_api::__private::FluidTooltipCall {
                    stack,
                    lines: &mut snapshot,
                });
                if let Ok(replacement) = snapshot.try_into_engine(&()) {
                    lines = replacement;
                }
            }
        }
        lines
    }

    /// `stack`'s tooltip at `x`, `y`, laid out and drawn as every tooltip is.
    pub(in crate::game) fn draw_fluid_tooltip(
        &self,
        batch: &mut HudBatch,
        stack: &FluidStack,
        x: f32,
        y: f32,
    ) {
        self.draw_tooltip_lines(batch, &self.fluid_tooltip_lines(stack), x, y);
    }

    /// Fluid `fluid` (`minecraft:water`, `mymod:oil`) drawn as a square with its top left at `at`, `size`
    /// GUI pixels wide (16 in a slot): the sprite its look names (`data/<mod>/sunrise/fluid/<name>.json`)
    /// tinted with its colour, else a flat square of the colour, which vanilla water and lava have. What a
    /// recipe viewer draws for a fluid stack; the slot and the amount are the mod's.
    pub(in crate::game) fn native_fluid(
        &self,
        batch: &mut HudBatch,
        fluid: &str,
        at: [f32; 2],
        size: f32,
    ) {
        let look = sunrise_sim::mod_content::fluid_look(fluid);
        let tint = crate::game::native_panel_recipes::fluid_tint(
            fluid,
            look.as_ref().and_then(|l| l.tint),
        );
        let dst = [at[0], at[1], size, size];
        let sprite = look
            .and_then(|l| l.sprite)
            .and_then(|s| self.gui_sprite_pixels(&s).map(|size| (s, size)));
        if let Some((name, px)) = sprite {
            self.native_sprite(batch, &name, [0.0, 0.0, px[0], px[1]], dst, Some(tint));
        } else {
            let [a, r, g, b] = tint.to_be_bytes();
            batch.fill(
                &self.client.gui.atlas,
                dst[0],
                dst[1],
                dst[2],
                dst[3],
                rgba(r, g, b, a),
            );
        }
    }
}

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Sunrise does.
impl crate::game::ClientGame<'_> {
    /// Fluid `fluid`'s name as the lang names it (`fluid_type.<mod>.<name>`, vanilla's by its block),
    /// else its path in words.
    pub(in crate::game) fn fluid_name(&self, fluid: &str) -> String {
        sunrise_client::fluid_tooltip::fluid_name(fluid)
    }
}
