/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// `BonemealEvent`: bone meal from `stack` used on the block `state` at
    /// `pos` (`BoneMealItem.growCrop`), by `player` or, when it is `None`, by
    /// a dispenser or a villager. `valid_bonemeal_target` is the block's
    /// `isValidBonemealTarget`. Cancel skips vanilla's growth, the block's
    /// own bone meal Parts and the meal's use: `growCrop` then answers
    /// `successful` (`setSuccessful`), which is `false` unless a mod sets it.
    /// Fired on both sides; changes to `stack` stay.
    fn native_bonemeal(&mut self, call: super::fire::BonemealCall<'_>) {
        crate::game::native_hooks::fire::FireHooks::fire_bonemeal(self, call);
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
