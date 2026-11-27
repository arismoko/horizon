//! Breeding completion: credit, partner cooldown and heart delivery.
use super::*;

impl crate::game::CommonGame<'_> {
    /// `Rabbit.getBreedOffspring`: a fresh biome variant
    /// (`Rabbit.getRandomRabbitVariant`, from the level's random) one time in
    /// twenty, else a parent's colour by the breeding parent's own coin toss.
    /// Called before the child exists, where vanilla draws it; None for any
    /// other species.
    pub fn bred_rabbit_variant(&mut self, parent: u32, p: &Mob, q: Option<&Mob>) -> Option<u8> {
        if p.species() != Species::Rabbit {
            return None;
        }
        let own = p.extra.variant;
        // `partner instanceof Rabbit`: a partner of another species draws no coin.
        let mate = q
            .filter(|q| q.species() == Species::Rabbit)
            .map(|q| q.extra.variant);
        let at = [p.position.x, p.position.y, p.position.z].map(|c| c.floor() as i32);
        let facts = self.biome_facts(at[0], at[1], at[2]);
        let fresh = sunrise_sim::mobs::rabbit::random_variant(
            facts.white_rabbit,
            facts.gold_rabbit,
            &mut self.world.random,
        );
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == parent) else {
            return Some(own);
        };
        if m.random.next_int_bound(20).unwrap_or(1) == 0 {
            return Some(fresh);
        }
        // The coin is drawn only for a rabbit partner.
        Some(match mate {
            Some(mate) if m.random.next_boolean() => mate,
            _ => own,
        })
    }
}
