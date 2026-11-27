/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// `FurnaceResultSlot.checkTakeAchievements` ran at a furnace, blast
    /// furnace or smoker's result slot with a nonzero `removeCount` (NeoForge
    /// `PlayerEvent.ItemSmeltedEvent`): `player` is the live player handle,
    /// `stack` lends the stack vanilla passed there (`getSmelting`), and
    /// `amount_removed` is `removeCount` (`getAmountRemoved`). On the server it
    /// fires with `HostSide::Server` after the furnace awards its used recipes
    /// and pops experience (`AbstractFurnaceBlockEntity.awardUsedRecipesAndPopExperience`);
    /// on the client it fires with `HostSide::Client` during menu click
    /// prediction without awarding experience. Both logical sides.
    fn native_item_smelted(&mut self, call: mod_api::__private::ItemSmeltedCall<'_>) {
        crate::game::native_hooks::fire::FireHooks::fire_item_smelted(self, call);
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
