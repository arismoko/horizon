use super::*;

impl HitLevel<'_> {
    /// Refuse one eligible hand in `LivingEntity.checkTotemDeathProtection`.
    pub(in crate::game) fn native_living_use_totem(
        &mut self,
        entity: i32,
        kind: &str,
        source: &HurtSource<'_>,
        hand: Hand,
        stack: &ItemStack,
        cancel: &mut bool,
    ) {
        self.fire_living_use_totem(entity, kind, source, hand, stack, cancel);
    }
}
