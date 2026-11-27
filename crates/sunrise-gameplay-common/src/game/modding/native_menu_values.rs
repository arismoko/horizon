use mod_api::MenuSlotPlace;
use sunrise_items::items::ItemStack;

/// The places backing one guest's open mod menu.
#[derive(Clone)]
pub struct MenuSlotPlaces {
    /// The menu's window id: the places end with that window.
    pub window: i32,
    /// What container slot `n` shows; slots past these stay the mods' own.
    pub places: Vec<MenuSlotPlace>,
    /// The hotbar slot holding a main-hand `HeldItem` place's stack, which takes no click.
    pub locked: Option<usize>,
    /// The stacks the `HeldItem` places were bound to when the slots were backed, by hand (main,
    /// off): the inventory slot holding it and the stack without its contents
    /// ([`held_identity`]). Another stack there, or another hotbar slot selected, closes the menu.
    pub held: [Option<(usize, ItemStack)>; 2],
    /// A change notice is queued; further writes queue none until it is taken.
    pub noticed: bool,
}
