/// Per-type behaviour switches.
#[derive(Clone, Copy, Debug, Default)]
pub struct Flags {
    /// WaterAnimal / fish: not affected by fluid physics (swims with its own control).
    pub aquatic_swimmer: bool,
    pub fall_immune: bool,
    /// FlyingMob-style travel (ghast, phantom): travelFlying without gravity.
    pub flying: bool,
    /// Spiders: onClimbable while colliding horizontally.
    pub climber: bool,
    /// Striders stand on lava.
    pub lava_walker: bool,
    /// Slot in the owning level's bound entity type tags. Synthetic types have no slot.
    pub entity_tag_slot: Option<usize>,
    /// LivingEntity.omnidirectionalAirMover (bees and parrots).
    pub omnidirectional: bool,
    pub fire_immune: bool,
    /// Cleared by default, like `EntityType.Builder` before `notInPeaceful`.
    pub not_in_peaceful: bool,
    pub undead: bool,
    /// Drowns out of water (fish, squid, dolphins after their air runs out).
    pub needs_water: bool,
    /// Hurt by water (endermen, blazes, striders out of lava, snow golems).
    pub water_sensitive: bool,
    pub animal: bool,
}
