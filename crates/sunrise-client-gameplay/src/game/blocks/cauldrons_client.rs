use super::*;

impl crate::game::ClientGame<'_> {
    /// What the cauldron at `pos` answers to `held` on a client, which only
    /// asks (`CauldronInteractions` acts `if (!level.isClientSide())`). A
    /// sneak with something in hand skips `useItemOn`
    /// (`Player.isSecondaryUseActive`).
    pub fn cauldron_outcome(
        &self,
        pos: [i32; 3],
        held: Option<&ItemStack>,
    ) -> sunrise_sim::cauldron::Outcome {
        self.client_cauldron_prediction(pos, held).0
    }
}
