/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `ComputeFovModifierEvent` for the camera player `player`: the raw modifier, the effect
    /// scale, and the result, which starts as the vanilla interpolation. Not fired for
    /// non-player cameras or first-person spyglass scoping.
    pub(in crate::game) fn native_compute_fov_modifier(
        &self,
        player: mod_api::PlayerRef,
        raw: f32,
        effect_scale: f32,
        modifier: &mut f32,
    ) {
        self.fire_compute_fov_modifier(player, raw, effect_scale, modifier);
    }
}
