/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// A mob or server player's death has finished, after the loot, the
    /// experience and the death message (`ServerLivingEntityEvents.AFTER_DEATH`).
    pub(in crate::game) fn native_living_death_post(
        &mut self,
        call: mod_api::__private::LivingDeathPostCall<'_>,
    ) {
        self.fire_living_death_post(call);
    }
}
