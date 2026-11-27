use super::*;

/// `native_living_incoming_damage`'s entity, entity type and source for a hit.
pub fn hurt_server_args<'a, 'b>(
    hurting: Hurting<'a>,
    attack: &'b Attack,
) -> (i32, &'a str, HurtSource<'b>)
where
    'a: 'b,
{
    match hurting {
        Hurting::Player(entity, source) => (entity, "minecraft:player", source),
        Hurting::Mob(mob, damage) => (
            mob.id as i32,
            mob.kind.id.as_str(),
            mob_source(damage, attack),
        ),
        Hurting::Dragon(entity, source) => (entity, "minecraft:ender_dragon", source),
        Hurting::ArmorStand(entity, source) => (entity, "minecraft:armor_stand", source),
        Hurting::Entity(entity, kind, source) => (entity, kind, source),
    }
}

pub(in crate::game) fn capture_hit(
    hurting: Hurting<'_>,
    attack: &Attack,
) -> Option<sunrise_sim::health::AcceptedHit> {
    let death = mod_api::__private::listened(mod_api::HookId::LivingDeath);
    let loot = mod_api::__private::listened(mod_api::HookId::LootDrops);
    if !death && !loot {
        return None;
    }
    let (committed, dying) = match &hurting {
        Hurting::Mob(mob, _) => (mob.last_damage, mob.health <= 0.0),
        _ => (None, true),
    };
    let (_, _, source) = hurt_server_args(hurting, attack);
    if !dying && !(loot && source.direct != 0 && attack.projectile == Some(source.direct)) {
        return None;
    }
    Some(sunrise_sim::health::AcceptedHit::capture(
        source, attack, committed,
    ))
}
