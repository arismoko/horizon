use super::*;

impl HitLevel<'_> {
    /// A protector from outside the hands in `LivingEntity.checkTotemDeathProtection`, asked
    /// once neither hand protected.
    pub fn native_living_death_protection(
        &mut self,
        entity: i32,
        kind: &str,
        source: &HurtSource<'_>,
        protector: &mut Option<ItemStack>,
    ) {
        self.fire_living_death_protection(entity, kind, source, protector);
    }
}
