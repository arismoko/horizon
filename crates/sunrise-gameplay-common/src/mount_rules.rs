//! The horse inventory's common rules (`AbstractHorse`, `AbstractNautilus`, `HorseInventoryMenu`).
use sunrise_items::items::{ItemStack, Items};
use sunrise_sim::mobs::equip::equippable_in_slot;
use sunrise_sim::mobs::{Mob, Species, equine, llama, nautilus, species};

/// `AbstractChestedHorse.getInventoryColumns`.
pub const COLUMNS: usize = 5;

/// Menu slot ids: the saddle, the body armour and the chest from 710.
pub const SADDLE: usize = 700;
pub const ARMOR: usize = 701;
pub const CHEST: usize = 710;

/// `AbstractHorse.getInventoryColumns`: five for a donkey's or mule's chest,
/// a llama's strength for its chest, else none.
pub fn mount_columns(m: &Mob) -> usize {
    let species = m.species();
    if llama::is_llama(species) {
        llama::inventory_columns(m).clamp(0, 5) as usize
    } else if m.extra.chested && matches!(species, Species::Donkey | Species::Mule) {
        COLUMNS
    } else {
        0
    }
}
/// Whether the mount has a horse inventory (`AbstractHorse`, `AbstractNautilus`).
pub fn has_mount_menu(species: Species) -> bool {
    equine::is_equine(species) || llama::is_llama(species) || nautilus::is_nautilus(species)
}
/// The saddle slot is active (`canUseSlot(SADDLE)` and `CAN_EQUIP_SADDLE`):
/// not on llamas.
pub fn shows_saddle(species: Species) -> bool {
    !llama::is_llama(species)
}
/// The body slot is active: horse armour, a llama's carpet, or nautilus armour.
pub fn shows_body(species: Species) -> bool {
    can_wear_horse_armor(species) || llama::is_llama(species) || nautilus::is_nautilus(species)
}

/// `EntityTypeTags.CAN_WEAR_HORSE_ARMOR`.
pub fn can_wear_horse_armor(species: Species) -> bool {
    matches!(species, Species::Horse | Species::ZombieHorse)
}
/// Whether `stack` fits the body slot of the species: `Mob.isEquippableInSlot(stack, BODY)`, the
/// stack's `equippable` component naming the `body` slot and allowing the species' entity type.
pub fn body_armor_fits(items: &Items, species: Species, stack: &ItemStack) -> bool {
    equippable_in_slot(items, stack, "body", species::vanilla_id(species))
}
