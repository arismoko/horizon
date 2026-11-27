//! `LivingEntity.canFreeze`.
use super::{Mob, MobWorld};

/// `LivingEntity.canFreeze`: not a `#freeze_immune_entity_types` mob and no
/// worn piece in `#freeze_immune_wearables`.
pub fn can_freeze(m: &Mob, w: &MobWorld) -> bool {
    // Skeleton.canFreeze: a skeleton never freezes, so powder snow builds no
    // frozen ticks on it and deals it no freeze damage. Only `Skeleton`
    // overrides it: the bogged, parched and wither skeleton extend
    // `AbstractSkeleton` and freeze; the stray is immune through the
    // `#freeze_immune_entity_types` tag below.
    if m.species() == super::Species::Skeleton {
        return false;
    }
    // SulfurCube.canFreeze: a cube carrying a block in its body does not freeze
    // either; without one it takes the tags below like any other mob.
    if m.species() == super::Species::SulfurCube && crate::mobs::sulfur_cube::has_body(m) {
        return false;
    }
    if w.entity_tag_bit(&m.kind, crate::items::EntityTypeTags::FREEZE_IMMUNE) {
        return false;
    }
    let immune = w
        .items
        .and_then(|i| i.item_tags.get("minecraft:freeze_immune_wearables"));
    // A wolf's armor (its body slot) is `#freeze_immune_wearables`.
    ![
        m.extra.body_id(),
        m.extra.head_id(),
        m.extra.slot_id(sunrise_items::equipment::CHEST),
        m.extra.slot_id(sunrise_items::equipment::LEGS),
        m.extra.slot_id(sunrise_items::equipment::FEET),
    ]
    .into_iter()
    .flatten()
    .any(|item| immune.is_some_and(|t| t.contains(item)))
}

/// Whether `p`'s head item disguises their gaze from mob `observer`: vanilla's tag test,
/// then a mod's `gaze_disguise` answer (`IItemExtension.isGazeDisguise`).
pub fn gaze_disguised(observer: u32, p: &crate::mobs::PlayerView, w: &MobWorld) -> bool {
    let disguised = wears_gaze_disguise(p.head.as_deref(), w);
    w.mob_hooks.map_or(disguised, |hooks| {
        hooks.gaze_disguise(observer, p, disguised)
    })
}
/// The player's HEAD item belongs to vanilla's gaze disguise tag.
pub fn wears_gaze_disguise(head: Option<&str>, w: &MobWorld) -> bool {
    head.is_some_and(|item| {
        w.items
            .and_then(|items| items.item_tags.get("minecraft:gaze_disguise_equipment"))
            .is_some_and(|tag| tag.contains(item))
    })
}
