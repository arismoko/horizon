use super::*;

impl crate::game::ServerGame<'_> {
    /// `Player.attack`: the attack damage attribute with held and effect modifiers.
    pub fn attack_damage(&mut self) -> f32 {
        self.common
            .player
            .as_mut()
            .and_then(|p| p.attribute("minecraft:attack_damage").ok())
            .unwrap_or(1.0) as f32
    }
    /// `LivingEntity.updateFallFlying`: the glide's wear. `Util.getRandom`
    /// picks one of the gliders that will not break (from the entity's own
    /// random), and
    /// `ItemStack.hurtAndBreak` takes `points` of its durability (Unbreaking
    /// may save it). Callers skip creative.
    pub(in crate::game::server_side) fn wear_glider(&mut self, points: u32) {
        self.side_wear_glider(points);
    }
}
