use sunrise_items::items::ItemStack;
use sunrise_sim::first_person_sim::Hand;

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// A player is about to nock `bow` with `hand` (`BowItem.use`, NeoForge's
    /// `ArrowNockEvent`): `has_ammo` is whether `getProjectile` found ammunition
    /// or the player is in creative. Set `cancel` to fail the use.
    /// 375.
    pub(in crate::game) fn native_arrow_nock(
        &mut self,
        player: u128,
        name: &str,
        bow: &ItemStack,
        hand: Hand,
        has_ammo: bool,
        cancel: &mut bool,
    ) {
        self.fire_arrow_nock(player, name, bow, hand, has_ammo, cancel);
    }
}
