//! `LivingEntity.onEquipItem` for players: the equip sound and the equip
//! game event when an armour or off-hand slot takes a different item, from
//! every path that calls it (use, the inventory menu, the creative screen,
//! the swap-hands key, dispensers), for the host and for LAN guests.
use sunrise_items::items::ItemStack;

/// The equipment slots `onEquipItem` can act on, as `EquipmentSlot` names:
/// the armour (`Player.doesEmitEquipEvent`: these emit the game event) and
/// the off hand.
pub const ARMOR: [&str; 4] = ["head", "chest", "legs", "feet"];
#[allow(dead_code)]
pub(in crate::game) const OFFHAND: &str = "offhand";

/// `ItemStack.isSameItemSameComponents` over optional stacks.
pub(in crate::game) fn same(a: Option<&ItemStack>, b: Option<&ItemStack>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.same_item_same_components(b),
        _ => false,
    }
}

pub trait PlayerEquipGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `onEquipItem` for every slot that changed since `before`: what the
    /// server does for the creative screen, which sends each changed slot
    /// (`handleSetCreativeModeSlot` sets it by player).
    #[allow(dead_code)]
    fn player_equipment_changed(&mut self, before: &[Option<ItemStack>; 5]) {
        let after = self.player_equipment();
        for (i, slot) in ARMOR.into_iter().chain([OFFHAND]).enumerate() {
            self.player_on_equip(slot, before[i].as_ref(), after[i].as_ref());
        }
    }

    /// `LivingEntity.onEquipItem(slot, old, new)` for the player in place
    /// (the host, or a guest inside `as_guest`).
    fn player_on_equip(&mut self, slot: &str, old: Option<&ItemStack>, new: Option<&ItemStack>) {
        if !same(old, new) {
            if self.transactions.is_empty() {
                self.player_equipped(slot, new);
                return;
            }
            let uuid = self.native_acting_id();
            self.after_commit(super::transactions::After::PlayerEquip(
                uuid,
                slot.to_owned(),
                new.cloned(),
            ));
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> PlayerEquipGameExt1<'__sg> for G {}

pub trait PlayerEquipGameExt2<'__sg>: crate::game::SideGame<'__sg> {
    /// The body of `onEquipItem` once the item changed.
    fn player_equipped(&mut self, slot: &str, new: Option<&ItemStack>) {
        self.side_player_equipped(slot, new);
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> PlayerEquipGameExt2<'__sg> for G {}

use crate::game::side_ops::mobs_rest::MobsRestSideGameExt as _;

use crate::game::transactions::TransactionsGameExt as _;

use crate::game::native_hooks::ModGameExt1 as _;
