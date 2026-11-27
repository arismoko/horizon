use super::*;

/// The hurt sound of a `ClientboundDamageEventPacket`'s damage type, by its
/// synchronized registry id: `Player.getHurtSound` picks the type's effects.
pub fn hurt_sound_for(
    registries: &Dynamic,
    damage: &sunrise_world::damage_source::Catalog,
    source_type: i32,
) -> &'static str {
    if let Some(data) = registries.data("minecraft:damage_type", source_type) {
        let effects = data.get("effects").and_then(|t| match t {
            Tag::String(effects) => effects.to_string().ok(),
            _ => None,
        });
        return sunrise_sim::health::hurt_sound(effects.as_deref().unwrap_or("hurt"));
    }
    registries
        .name("minecraft:damage_type", source_type)
        .and_then(|name| damage.id(name).and_then(|id| damage.definition(id)))
        .map_or(sunrise_sim::health::hurt_sound("hurt"), |d| {
            sunrise_sim::health::hurt_sound(&d.effects)
        })
}
