use sunrise_items::enchant::EffectContext;

/// `MaceItem.SMASH_ATTACK_FALL_THRESHOLD`.
pub(in crate::game) const FALL_THRESHOLD: f64 = 1.5;

/// `MaceItem.SMASH_ATTACK_KNOCKBACK_RADIUS`.
pub const KNOCKBACK_RADIUS: f64 = 3.5;

/// `MaceItem.canSmashAttack` with explicit actor state.
pub fn can_smash(fall: f64, fall_flying: bool) -> bool {
    fall > FALL_THRESHOLD && !fall_flying
}

/// `MaceItem.getAttackDamageBonus`, with an explicit stack, actor fall and RNG.
pub fn stack_smash_damage(
    enchantments: &sunrise_items::enchant::Enchantments,
    stack: &sunrise_items::items::ItemStack,
    fall: f64,
    ctx: &EffectContext<'_>,
    random: &mut sunrise_core::random::LegacyRandom,
) -> f32 {
    let per_block = enchantments.modify_value(
        Some(stack),
        "minecraft:smash_damage_per_fallen_block",
        ctx,
        random,
        0.0,
    );
    smash_damage(fall, per_block)
}

pub fn smash_damage(fall: f64, per_block: f32) -> f32 {
    let damage = if fall <= 3.0 {
        4.0 * fall
    } else if fall <= 8.0 {
        12.0 + 2.0 * (fall - 3.0)
    } else {
        22.0 + fall - 8.0
    };
    (damage + f64::from(per_block) * fall) as f32
}
