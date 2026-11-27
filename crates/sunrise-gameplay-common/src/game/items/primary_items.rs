//! Item/enchantment answers resolved when parts install, read from registry snapshots.

pub trait PrimaryItemsGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Preserve installed answers when publishing a fresh world-loot snapshot,
    /// including loads after parts install. This never asks a listener twice.
    fn copy_loot_enchantment_answers(&self, loot: &sunrise_world::loot::LootData) {
        let enchantments = &self.stations().enchantments;
        loot.primary_answers()
            .install(enchantments.primary_answers().snapshot());
        loot.support_answers()
            .install(enchantments.support_answers().snapshot());
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> PrimaryItemsGameExt1<'__sg> for G {}

use crate::game::tables::GameItems as _;
