//! Engine-stack callbacks installed by the game at its API boundary.
use mod_api::{Identifier, ItemAttributeModifiers, ItemEnchantments};
use std::sync::OnceLock;
use sunrise_items::items::ItemStack;

/// Stack-bearing hooks whose API snapshots are made by the game.
pub struct StackHooks {
    pub enchantments: fn(&ItemStack, Option<&Identifier>, &mut ItemEnchantments),
    pub attributes: fn(&ItemStack, &mut ItemAttributeModifiers),
}
static HOOKS: OnceLock<StackHooks> = OnceLock::new();
/// Install the game conversion callbacks once.
pub fn install(hooks: StackHooks) -> bool {
    HOOKS.set(hooks).is_ok()
}
pub fn enchantments(stack: &ItemStack, target: Option<&Identifier>, value: &mut ItemEnchantments) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.enchantments)(stack, target, value);
    }
}
pub fn attributes(stack: &ItemStack, value: &mut ItemAttributeModifiers) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.attributes)(stack, value);
    }
}
