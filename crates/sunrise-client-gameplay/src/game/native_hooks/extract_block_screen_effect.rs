use sunrise_core::position::BlockPos;

/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `LevelExtractor.extractPlayerState`: let client listeners choose the overlay renderer
    /// using the overlay's state and position and the world and player interpolation values.
    pub(in crate::game) fn native_extract_block_screen_effect(
        &self,
        effect: mod_api::BlockScreenEffect,
        state: mod_api::BlockState,
        pos: BlockPos,
        partial_tick: f32,
        player_partial_tick: f32,
        canceled: &mut bool,
        renderer: &mut Option<mod_api::__private::SelectedBlockScreenEffectRenderer>,
    ) {
        self.fire_extract_block_screen_effect(
            effect,
            state,
            pos,
            partial_tick,
            player_partial_tick,
            canceled,
            renderer,
        );
    }
}
