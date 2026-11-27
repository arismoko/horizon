/// The hook point. It does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// An entity's breathing this tick (`LivingBreatheEvent`, posted from
    /// `LivingEntity.baseTick` every tick for a living entity with the generic
    /// air logic): `entity` is the player or mob. `can_breathe`, `consume` and
    /// `refill` come in as Sunrise does it (a mod-free game changes nothing); set
    /// them to change this tick. With `can_breathe` the air rises by `refill`, up
    /// to its maximum (`entity_max_air_supply`: 300 for every entity except
    /// dolphins, which have 4800); a dolphin's vanilla `refill` is its maximum
    /// minus its current air. Without breath, air falls by `consume`, and at -20
    /// it is set to 0 and the entity takes 2 drowning damage. Negative amounts
    /// count as 0. Asked on servers for players and mobs. A water animal,
    /// axolotl or nautilus comes in with its own air logic's answer: where it
    /// breathes (water, or rain for an axolotl) `can_breathe` and a `refill` to
    /// its full air; elsewhere no breath and `consume` 1. Running out deals its
    /// own damage (drowning, or drying out).
    ///
    /// The listener has the entity's level and can read its air
    /// (`entity_air_supply`), its equipment and the block it stands in, and
    /// write items, attachments and criteria. What it writes before the air
    /// update is committed with it. The game then applies the answer to the air
    /// the entity had when the listener was called: if the entity has gone, died
    /// or had its air changed in the meantime, it refuses (never recomputes),
    /// the air is left as the listener left it, and the writes of the listener
    /// that go through the transaction (inventory edits through `ItemPlace::edit`)
    /// are rolled back. A criterion a listener granted stays granted: criteria are
    /// granted at once, not through the transaction. Mod effects that this
    /// tick's air update causes (drowning, drying out) are not part of the call.
    pub(in crate::game) fn native_living_breathe(
        &mut self,
        entity: mod_api::LivingRef,
        can_breathe: &mut bool,
        consume: &mut i32,
        refill: &mut i32,
    ) {
        self.fire_living_breathe(entity, can_breathe, consume, refill);
    }
}
