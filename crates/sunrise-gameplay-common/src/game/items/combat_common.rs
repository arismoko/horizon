use sunrise_items::{
    enchant::{EffectContext, Enchantments},
    items::ItemStack,
};

/// `EntityEvent.DROWN_PARTICLES`: broadcast with the drowning damage.
pub const DROWN_EVENT: i8 = 67;

/// `LivingEntity.lastHurtByPlayer`: a reference that survives a disconnect.
#[derive(Clone, Copy)]
pub struct PlayerCredit(pub u128);

impl From<PlayerCredit> for u128 {
    fn from(credit: PlayerCredit) -> Self {
        credit.0
    }
}

/// Shared weapon damage effects; callers choose live RNG for an attack or a snapshot for a read.
pub fn stack_enchanted_damage(
    enchantments: &Enchantments,
    stack: &ItemStack,
    ctx: &EffectContext<'_>,
    random: &mut sunrise_core::random::LegacyRandom,
    base: f32,
) -> f32 {
    enchantments.modify_value(Some(stack), "minecraft:damage", ctx, random, base)
}
