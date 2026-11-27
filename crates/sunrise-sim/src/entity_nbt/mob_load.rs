//! Mob NBT entry points: decode the position once before construction.
use super::*;

/// The mob a saved compound describes. `registry` is what reads a field holding
/// a block state (an enderman's `carriedBlockState`); without one, such a field
/// is left out, as it was before it was modelled.
pub fn mob_from_nbt(
    tag: &Tag,
    types: &HashMap<String, Arc<MobType>>,
    id: u32,
    registry: Option<&BlockRegistry>,
) -> Option<Mob> {
    let kind = types.get(&kind(tag)?)?.clone();
    let position = loaded_mob_position(tag);
    let random = crate::mobs::entity_random(id, position);
    Some(mob_from_nbt_at(
        tag, kind, id, registry, random, position, true,
    ))
}

/// `mob_from_nbt` for a mob that is only drawn (a screen's entity): built as a client's copy is
/// (`Mob::new_remote`), so no mod part's `register_goals` or behaviour registration runs.
pub fn mob_from_nbt_remote(
    tag: &Tag,
    types: &HashMap<String, Arc<MobType>>,
    id: u32,
    registry: Option<&BlockRegistry>,
) -> Option<Mob> {
    let kind = types.get(&kind(tag)?)?.clone();
    let position = loaded_mob_position(tag);
    let random = crate::mobs::entity_random(id, position);
    Some(mob_from_nbt_at(
        tag, kind, id, registry, random, position, false,
    ))
}

/// Decode a deferred server mob with its own random and without mod dispatch.
/// The caller assigns its runtime id and attaches mod parts on insertion.
pub fn mob_from_nbt_deferred(
    tag: &Tag,
    kind: Arc<MobType>,
    registry: &BlockRegistry,
    random: sunrise_core::random::LegacyRandom,
) -> Mob {
    let position = loaded_mob_position(tag);
    mob_from_nbt_at(tag, kind, 0, Some(registry), random, position, false)
}

fn loaded_mob_position(tag: &Tag) -> Vec3 {
    // Entity.load: an absent or undecodable Pos uses Vec3.ZERO.
    position(tag).unwrap_or(Vec3::ZERO)
}

/// `mob_from_nbt` for a mob with its own random given (`Mob::with_random`).
pub fn mob_from_nbt_with_random(
    tag: &Tag,
    types: &HashMap<String, Arc<MobType>>,
    id: u32,
    registry: Option<&BlockRegistry>,
    random: sunrise_core::random::LegacyRandom,
) -> Option<Mob> {
    let kind = types.get(&kind(tag)?)?.clone();
    let position = loaded_mob_position(tag);
    Some(mob_from_nbt_at(
        tag, kind, id, registry, random, position, true,
    ))
}
