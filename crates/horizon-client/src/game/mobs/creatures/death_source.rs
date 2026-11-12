use super::*;

/// Damage types of a projectile: its own entity is the direct one, and
/// the game keeps no id for it.
const PROJECTILE_DAMAGE: [&str; 9] = [
    "minecraft:arrow",
    "minecraft:trident",
    "minecraft:fireball",
    "minecraft:unattributed_fireball",
    "minecraft:wither_skull",
    "minecraft:mob_projectile",
    "minecraft:thrown",
    "minecraft:wind_charge",
    "minecraft:fireworks",
];

/// The `HurtSource` of a death's blow. A melee hit's direct entity is the
/// one that caused it; a projectile's isn't known, so `direct` is 0.
pub(in crate::game) fn death_hurt_source(damage_type: &str, causing: u32) -> HurtSource<'_> {
    let direct = if PROJECTILE_DAMAGE.contains(&damage_type) {
        0
    } else {
        causing as i32
    };
    HurtSource {
        damage_type,
        causing: causing as i32,
        direct,
        position: None,
    }
}

/// The damage type id of a Horizon damage kind. `caused`: the source has a
/// causing entity (only a blast asks).
pub(in crate::game) fn damage_type(
    kind: horizon_client::mobs::DamageKind,
    caused: bool,
) -> &'static str {
    use horizon_client::mobs::DamageKind as K;
    match kind {
        K::Named(named) => named.name,
        K::PlayerAttack => "minecraft:player_attack",
        K::MaceSmash => "minecraft:mace_smash",
        K::Spear => "minecraft:spear",
        K::MobAttack => "minecraft:mob_attack",
        K::Arrow => "minecraft:arrow",
        K::Thrown => "minecraft:thrown",
        K::Fireball => "minecraft:fireball",
        K::Explosion if caused => "minecraft:player_explosion",
        K::Explosion => "minecraft:explosion",
        K::Fall => "minecraft:fall",
        K::FlyIntoWall => "minecraft:fly_into_wall",
        K::FallingBlock => "minecraft:falling_block",
        K::FallingStalactite => "minecraft:falling_stalactite",
        K::OnFire => "minecraft:on_fire",
        K::InFire => "minecraft:in_fire",
        K::Campfire => "minecraft:campfire",
        K::Lava => "minecraft:lava",
        K::Drown => "minecraft:drown",
        K::DryOut => "minecraft:dry_out",
        K::Magic => "minecraft:magic",
        K::IndirectMagic => "minecraft:indirect_magic",
        K::Sting => "minecraft:sting",
        K::Cactus => "minecraft:cactus",
        K::SweetBerryBush => "minecraft:sweet_berry_bush",
        K::Generic => "minecraft:generic",
        K::InWall => "minecraft:in_wall",
        K::Thorns => "minecraft:thorns",
        K::SonicBoom => "minecraft:sonic_boom",
        K::Freeze => "minecraft:freeze",
        K::OutOfWorld => "minecraft:out_of_world",
        K::Cramming => "minecraft:cramming",
        K::FallingAnvil => "minecraft:falling_anvil",
        K::Fireworks => "minecraft:fireworks",
        K::GenericKill => "minecraft:generic_kill",
        K::LightningBolt => "minecraft:lightning_bolt",
        K::OutsideBorder => "minecraft:outside_border",
        K::Spit => "minecraft:spit",
        K::Stalagmite => "minecraft:stalagmite",
        K::Starve => "minecraft:starve",
        K::HotFloor => "minecraft:hot_floor",
        K::SulfurCubeHot => "minecraft:sulfur_cube_hot",
        K::WindCharge => "minecraft:wind_charge",
        K::Wither => "minecraft:wither",
        K::WitherSkull => "minecraft:wither_skull",
    }
}
