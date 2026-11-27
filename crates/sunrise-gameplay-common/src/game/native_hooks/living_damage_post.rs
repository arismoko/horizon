use super::*;

impl HitLevel<'_> {
    /// NeoForge's `LivingDamageEvent.Post`: entity `entity` of type `kind` has been hurt by the
    /// hit `container` describes, with its final values (health damage, 0 when none was taken,
    /// the reductions, the post-attack invulnerability ticks, and `should_cause_side_effects`,
    /// `hurtServer`'s fresh-hit decision). It only listens, and fires once per hit at the end of
    /// `hurtServer`, after Pre and any death or totem protection. It can't write the
    /// entity or the world's blocks: the level host gives the hit's random and level attachments.
    pub fn native_living_damage_post(
        &mut self,
        entity: i32,
        kind: &str,
        source: &HurtSource<'_>,
        container: mod_api::DamageContainerView<'_>,
    ) {
        self.fire_living_damage_post(entity, kind, source, container);
    }
}
