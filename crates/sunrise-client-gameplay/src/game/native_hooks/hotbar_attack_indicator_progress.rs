#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// After the stock hotbar's items, before its attack indicator checks for full charge.
    pub(in crate::game) fn native_hotbar_attack_indicator_progress(
        &self,
        call: super::fire::HotbarAttackIndicatorProgressCall<'_>,
    ) {
        self.fire_hotbar_attack_indicator_progress(call);
    }
}
