//! `AbstractContainerMenu.clicked` over vanilla's slot layouts: every
//! `ContainerInput` (pickup, quick move, swap, clone, throw, the three quick
//! craft drags, pickup all) for the player inventory, the crafting table, the
//! chest family, hoppers, dispensers, shulker boxes, the furnaces, the brewing
//! stand, the crafter (its client menu and the block entity's server menu with
//! `setSlotState`), the loom (`clickMenuButton`, `slotsChanged`), the
//! stonecutter (its recipe buttons and result) and the merchant (`MerchantMenu` over a `merchant::Session`: payment slots, the
//! result slot's trade, `tryMoveItems`).
//! `tools/test_menu_click_parity.py` compares every click with vanilla's menu
//! classes.
#[path = "menu/crafting_take.rs"]
mod crafting_take;
use crate::{
    banner::{self, Banners},
    brewing::Brewing,
    crafting::Recipes,
    items::{ItemStack, Items},
};
pub use mod_api::item::menu::{LogicalSide, MenuButtonContext};
use sunrise_world::native_load_context::{LoopContext, LoopSide};
mod models;
pub use models::{
    AnvilCraftSnapshot, AnvilUpdateInputs, GrindstonePlaceEdits, GrindstoneTakeEdits, MenuGuest,
    PendingGrindstoneTake, QuickCraftDrag, SlotContext, SlotRules, SlotView, StackedClick,
    StackedEdit, StackedEditPolicy, StackedOutcome, StackedResume, StackedSlotKind,
    StackedSlotTarget,
};
mod slot_context;
use equippable::limit_slot_max_for;
pub use slot_context::{LiveSlotRules, apply_menu_slot_rules, lan_container};
use std::{collections::HashMap, sync::Arc};

mod bundle;
pub use bundle::{vanilla_carried_override, vanilla_clicked_override};
pub(crate) mod linked;
pub use linked::{LinkedSlots, linked_slot_count};
pub mod container_items;
pub use container_items::fits_inside_container_items;
mod click;
pub use click::{ClickContinuation, SlotTake, apply_stacked_outcome};
mod client_drops;
pub use client_drops::DropSpamThrottler;
mod storage;
pub use storage::{MutationStamp, Snapshot};
mod equip_movement;
mod equippable;
pub mod place;
mod remainder;

/// `ServerPlayer.getRecipeBook`: whether the player knows a recipe.
pub trait KnownRecipes {
    fn knows(&self, recipe_id: &str) -> bool;
}

impl KnownRecipes for crate::recipe_book::Settings {
    fn knows(&self, recipe_id: &str) -> bool {
        let q = crate::crafting::qualified(recipe_id);
        self.recipes.contains(&q) || self.recipes.contains(recipe_id)
    }
}

impl KnownRecipes for std::collections::BTreeSet<String> {
    fn knows(&self, recipe_id: &str) -> bool {
        let q = crate::crafting::qualified(recipe_id);
        self.contains(&q) || self.contains(recipe_id)
    }
}

impl KnownRecipes for std::collections::HashSet<String> {
    fn knows(&self, recipe_id: &str) -> bool {
        let q = crate::crafting::qualified(recipe_id);
        self.contains(&q) || self.contains(recipe_id)
    }
}

impl KnownRecipes for [String] {
    fn knows(&self, recipe_id: &str) -> bool {
        let q = crate::crafting::qualified(recipe_id);
        self.iter().any(|r| r == &q || r == recipe_id)
    }
}

impl KnownRecipes for Vec<String> {
    fn knows(&self, recipe_id: &str) -> bool {
        self.as_slice().knows(recipe_id)
    }
}

impl<const N: usize> KnownRecipes for [&str; N] {
    fn knows(&self, recipe_id: &str) -> bool {
        let q = crate::crafting::qualified(recipe_id);
        self.iter().any(|r| *r == q.as_str() || *r == recipe_id)
    }
}

impl KnownRecipes for &[&str] {
    fn knows(&self, recipe_id: &str) -> bool {
        let q = crate::crafting::qualified(recipe_id);
        self.iter().any(|r| *r == q.as_str() || *r == recipe_id)
    }
}

/// What a menu needs to know about items and recipes.
#[derive(Clone, Copy)]
pub struct Env<'a> {
    /// Authoritative stack query; absent for data-only consumers.
    pub crafting_remainder: Option<&'a dyn crate::crafting::CraftingRemainder>,
    /// UUID of the player operating this menu, when a player is available.
    pub player: Option<u128>,
    pub player_entity_id: Option<i32>,
    /// The side and settings of the resource generation this menu runs in: the
    /// one source of its side, and what a hook inside a menu loop is given.
    pub loop_context: &'a LoopContext,
    /// `IItemExtension.canEquip` for the armour slots, lent by the game (none keeps the data's).
    pub can_equip: Option<&'a dyn crate::mobs::equip::CanEquip>,
    pub items: &'a Items,
    pub recipes: &'a Recipes,
    pub brewing: &'a Brewing,
    pub banners: &'a Banners,
    /// The stonecutter's recipes.
    pub station_recipes: &'a crate::stations::StationRecipes,
    /// The grindstone's and the anvil's enchantment rules.
    pub enchantments: &'a crate::enchant::Enchantments,
    /// Item display names: the anvil compares the rename box with the
    /// input's hover name.
    pub names: &'a crate::item_names::Names,
    /// What a merchant menu needs for offers made while it is open (level-ups).
    pub trade: Option<&'a crate::merchant::TradeEnv<'a>>,
    /// The level's maps (`ContainerLevelAccess`): the cartography table reads
    /// and makes them. None is a client's menu (`ContainerLevelAccess.NULL`):
    /// it makes no result, and a taken result keeps its map id.
    pub maps: Option<&'a dyn Maps>,
    /// `GameRules.LIMITED_CRAFTING`: when true, only known or special recipes craft.
    pub limited_crafting: bool,
    /// The recipes the player knows (`ServerPlayer.getRecipeBook().contains(id)`).
    pub known_recipes: Option<&'a dyn KnownRecipes>,
    /// The mods' `menu_slot_rules`, lent by the game with the read-only `Host` behind it (none
    /// keeps every slot's own answer: a data-only consumer, or a click made without the game).
    pub slot_rules: Option<&'a dyn SlotRules>,
    /// What the mods decided about this pickup click (`item_stacked_on_other`), made by the game
    /// before the click (none: no listener, so the plain click).
    pub stacked: Option<&'a StackedOutcome>,
}

impl Env<'_> {
    /// `CraftingMenu.slotChangedCraftingGrid`'s recipe result for this player and grid.
    pub fn crafting_result(&self, grid: &[Option<ItemStack>], size: usize) -> Option<ItemStack> {
        self.recipes
            .find_with(self.loop_context, grid, size, &|id| {
                self.maps
                    .and_then(|maps| maps.state(id))
                    .map(|(_, scale)| scale)
            })
            .and_then(|(id, stack)| {
                if self.limited_crafting {
                    // `RecipeCraftingHolder.setRecipeUsed`: special recipes or known recipes craft.
                    let is_special = self.recipes.recipe(id).is_some_and(|r| r.is_special());
                    let known = self.known_recipes.is_some_and(|book| book.knows(id));
                    if !is_special && !known {
                        return None;
                    }
                }
                Some(stack)
            })
    }
}

/// The maps of the level a menu is in.
pub trait Maps {
    /// Stage derived maps and id reservations with a menu edit.
    fn with_staged(&self, edit: &mut dyn FnMut(&dyn Maps) -> bool) -> bool;
    /// `MapItem.getSavedData`: whether map `id` is locked, and its scale;
    /// None when the level has no such map.
    fn state(&self, id: i32) -> Option<(bool, u8)>;
    /// `MapItem.scaleMap` (`scale`) or `lockMap`: the copy of map `id` the
    /// level makes under a new id (`getFreeMapId`), None without the map.
    fn derive(&self, id: i32, scale: bool) -> Option<i32>;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Input {
    Pickup,
    QuickMove,
    Swap,
    Clone,
    Throw,
    QuickCraft,
    PickupAll,
}

impl Input {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "PICKUP" => Self::Pickup,
            "QUICK_MOVE" => Self::QuickMove,
            "SWAP" => Self::Swap,
            "CLONE" => Self::Clone,
            "THROW" => Self::Throw,
            "QUICK_CRAFT" => Self::QuickCraft,
            "PICKUP_ALL" => Self::PickupAll,
            _ => return None,
        })
    }
}

/// The key the player's own `InventoryMenu` answers to in `menu_slot_rules`, `menu_clicked` and
/// `recipe_book_place`. Vanilla 26.3's `InventoryMenu` has no `MenuType`: its constructor passes
/// `null` to `AbstractCraftingMenu` (`javap -c` on the 26.3 jar), so the name is Sunrise's.
pub const INVENTORY_MENU: &str = "minecraft:inventory";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Inventory,
    Crafting,
    Chest(usize),
    Hopper,
    Dispenser,
    Shulker,
    /// The cooking recipe type: smelting, blasting or smoking.
    Furnace(&'static str),
    Brewing,
    /// `CrafterMenu` over a plain crafting container (the client's menu).
    Crafter,
    /// `CrafterMenu` over the `CrafterBlockEntity` (the server's menu): the
    /// result follows the grid, and filling a disabled slot enables it.
    CrafterBlock,
    Loom,
    /// `StonecutterMenu`: the input, the result, then the inventory.
    Stonecutter,
    /// `GrindstoneMenu`: two inputs, the result, then the inventory.
    Grindstone,
    /// `AnvilMenu`: two inputs, the result, then the inventory.
    Anvil,
    /// `SmithingMenu`: template, base, addition, the result, then the inventory.
    Smithing,
    /// `EnchantmentMenu`: the item, the lapis, then the inventory; its data
    /// slots are the three costs, the seed and the clues.
    Enchanting,
    /// `BeaconMenu`: the payment slot, then the inventory; its data slots are
    /// the pyramid's levels and the two powers (`encodeEffect`: id + 1).
    Beacon,
    /// `CartographyTableMenu`: the map, the paper / map / glass pane, the
    /// result, then the inventory.
    Cartography,
    /// `LecternMenu` over the `LecternBlockEntity`: the book's one slot (no
    /// inventory slots); its data slot is the open page.
    Lectern,
    /// `MerchantMenu`: two payment slots, the result, then the inventory.
    Merchant,
    /// `HorseInventoryMenu`: the saddle and body armour slots (`ArmorSlot`s
    /// of the mount, whose entity type is `Menu::mount`), a chest of three
    /// rows of this many columns (`getInventoryColumns`), then the inventory.
    Mount(usize),
    /// `CreativeModeInventoryScreen.ItemPickerMenu`: an item tab's 45
    /// display slots, then the hotbar.
    ItemPicker,
    /// A content mod's menu type: its slots, then the inventory.
    Mod(&'static crate::containers::ModMenuType),
}

impl Kind {
    fn main_size(self) -> usize {
        match self {
            Self::Inventory | Self::Crafting => 0,
            Self::Chest(rows) => rows * 9,
            Self::Hopper | Self::Brewing => 5,
            Self::Dispenser | Self::Crafter | Self::CrafterBlock => 9,
            Self::Shulker => 27,
            Self::Mod(t) => t.slots,
            Self::Stonecutter | Self::Beacon | Self::Lectern => 1,
            Self::Grindstone
            | Self::Anvil
            | Self::Enchanting
            | Self::Cartography
            | Self::Merchant => 2,
            Self::Loom | Self::Smithing | Self::Furnace(_) => 3,
            Self::Mount(columns) => 2 + columns * 3,
            Self::ItemPicker => 45,
        }
    }

    pub(crate) fn grid_size(self) -> usize {
        match self {
            Self::Inventory => 2,
            Self::Crafting => 3,
            Self::Mod(t) => t.crafting.map_or(0, |c| c.size),
            _ => 0,
        }
    }

    /// Number of menu slots preceding the player inventory/hotbar.
    fn player_slot_start(self) -> usize {
        match self {
            Self::Inventory => 1 + self.grid_size().pow(2) + 4,
            Self::Crafting => 1 + self.grid_size().pow(2),
            Self::Loom
            | Self::Stonecutter
            | Self::Grindstone
            | Self::Anvil
            | Self::Smithing
            | Self::Cartography
            | Self::Merchant => self.main_size() + 1,
            _ => self.main_size(),
        }
    }

    /// How many slots this kind of menu has: its own, then the player's it shows.
    pub fn slot_count(self) -> usize {
        self.player_slot_start()
            + match self {
                Self::Lectern => 0,
                Self::Mod(t) if !t.player_inventory => 0,
                Self::ItemPicker => 9,
                Self::Inventory | Self::Crafter | Self::CrafterBlock => 37,
                _ => 36,
            }
    }

    fn slot(self, s: usize) -> Option<Slot> {
        if s >= self.slot_count() {
            return None;
        }
        let player = |index, role| Slot {
            container: PLAYER,
            index,
            role,
        };
        let result = |role| Slot {
            container: RESULT,
            index: 0,
            role,
        };
        let start = self.player_slot_start();
        if s >= start {
            let i = s - start;
            return Some(match (self, i) {
                (Self::Inventory, 36) => player(OFFHAND, Role::Plain),
                (Self::Crafter | Self::CrafterBlock, 36) => result(Role::Inert),
                (Self::ItemPicker, _) => player(i, Role::Plain),
                (_, 0..27) => player(i + 9, Role::Plain),
                _ => player(i - 27, Role::Plain),
            });
        }
        let main = |role| Slot {
            container: MAIN,
            index: s,
            role,
        };
        let grid = |index, role| Slot {
            container: GRID,
            index,
            role,
        };
        Some(match self {
            Self::Inventory | Self::Crafting => match s {
                0 => result(Role::CraftResult),
                5..=8 if self == Self::Inventory => player(44 - s, Role::Armor(8 - s)),
                _ => grid(s - 1, Role::Plain),
            },
            Self::Chest(_) | Self::Hopper | Self::Dispenser | Self::Lectern => main(Role::Plain),
            Self::Shulker => main(Role::Shulker),
            Self::Mod(t) => match t.crafting {
                Some(c) if c.grid_slots().contains(&s) => grid(s - c.grid, Role::Mod),
                Some(c) if c.result == s => result(Role::CraftResult),
                _ if t.results.contains(&s) => main(Role::ModResult),
                _ => main(Role::Mod),
            },
            Self::Crafter | Self::CrafterBlock => main(Role::CrafterIn),
            Self::Loom => match s {
                0 => main(Role::LoomBanner),
                1 => main(Role::LoomDye),
                2 => main(Role::LoomPattern),
                _ => result(Role::LoomResult),
            },
            Self::Stonecutter if s == 1 => result(Role::StoneResult),
            Self::Grindstone if s == 2 => result(Role::GrindResult),
            Self::Anvil if s == 2 => result(Role::CombineResult),
            Self::Smithing if s == 3 => result(Role::CombineResult),
            Self::Smithing => main(Role::SmithInput(s)),
            Self::Grindstone => main(Role::GrindInput),
            Self::Stonecutter | Self::Anvil => main(Role::Plain),
            Self::Enchanting if s == 0 => main(Role::EnchantItem),
            Self::Enchanting => main(Role::Lapis),
            Self::Beacon => main(Role::BeaconPay),
            Self::Cartography => match s {
                0 => main(Role::CartoMap),
                1 => main(Role::CartoExtra),
                _ => result(Role::CartoResult),
            },
            Self::Mount(_) => match s {
                0 => main(Role::MountEquip("saddle")),
                1 => main(Role::MountEquip("body")),
                _ => main(Role::Plain),
            },
            Self::ItemPicker => main(Role::CreativeDisplay),
            Self::Merchant if s == 2 => result(Role::MerchantResult),
            Self::Merchant => main(Role::Plain),
            Self::Brewing => match s {
                0..=2 => main(Role::Potion),
                3 => main(Role::Ingredient),
                _ => main(Role::BrewFuel),
            },
            Self::Furnace(_) => match s {
                0 => main(Role::Plain),
                1 => main(Role::Fuel),
                _ => main(Role::Output),
            },
        })
    }

    pub fn parse(name: &str) -> Option<Self> {
        if let Some(rows) = name.strip_prefix("chest") {
            return rows.parse().ok().map(Self::Chest);
        }
        if let Some(columns) = name.strip_prefix("mount") {
            return columns.parse().ok().map(Self::Mount);
        }
        Some(match name {
            "inventory" => Self::Inventory,
            "crafting" => Self::Crafting,
            "hopper" => Self::Hopper,
            "dispenser" => Self::Dispenser,
            "shulker" => Self::Shulker,
            "furnace" => Self::Furnace("smelting"),
            "blast_furnace" => Self::Furnace("blasting"),
            "smoker" => Self::Furnace("smoking"),
            "brewing" => Self::Brewing,
            "crafter" => Self::Crafter,
            "crafter_be" => Self::CrafterBlock,
            "loom" => Self::Loom,
            "stonecutter" => Self::Stonecutter,
            "grindstone" => Self::Grindstone,
            "anvil" => Self::Anvil,
            "smithing" => Self::Smithing,
            "enchanting" => Self::Enchanting,
            "beacon" => Self::Beacon,
            "cartography" => Self::Cartography,
            "lectern" => Self::Lectern,
            "merchant" => Self::Merchant,
            "item_picker" => Self::ItemPicker,
            _ => return None,
        })
    }
}

/// `Slot` subclasses that change the rules.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Plain,
    /// `ArmorSlot` for the equipment slot with this `EquipmentSlot.getIndex`.
    Armor(usize),
    Shulker,
    Fuel,
    /// `FurnaceResultSlot`.
    Output,
    /// `ResultSlot` of a crafting grid.
    CraftResult,
    /// `CrafterSlot`: refuses items while its disabled flag (data slot) is set.
    CrafterIn,
    /// `NonInteractiveResultSlot`.
    Inert,
    /// `BrewingStandMenu.PotionSlot`.
    Potion,
    /// `BrewingStandMenu.IngredientsSlot`.
    Ingredient,
    /// `BrewingStandMenu.FuelSlot`.
    BrewFuel,
    /// The loom's banner, dye and pattern slots.
    LoomBanner,
    LoomDye,
    LoomPattern,
    /// The loom's result slot (a plain slot whose `onTake` uses the inputs).
    LoomResult,
    /// `MerchantResultSlot`.
    MerchantResult,
    /// The stonecutter's result slot (`onTake` uses up one input).
    StoneResult,
    /// The grindstone's inputs: damageable or enchanted items.
    GrindInput,
    /// The grindstone's result slot (`onTake` clears both inputs).
    GrindResult,
    /// A smithing table input: its recipe property set (template, base, addition).
    SmithInput(usize),
    /// `ItemCombinerMenu`'s result slot (the anvil's and the smithing table's).
    CombineResult,
    /// The enchanting table's item slot (one item).
    EnchantItem,
    /// The enchanting table's lapis slot.
    Lapis,
    /// `BeaconMenu.PaymentSlot`: one beacon payment item.
    BeaconPay,
    /// The cartography table's map slot (anything with a map id).
    CartoMap,
    /// Its paper, empty map or glass pane slot.
    CartoExtra,
    /// Its result (`onTake`: one of each input, the map post-processed).
    CartoResult,
    /// `CustomCreativeSlot`: a creative tab's display slot, which the
    /// screen clicks itself; drags and PICKUP_ALL pass it by.
    CreativeDisplay,
    /// An `ArmorSlot` of a mount for this `EquipmentSlot` ("saddle" or
    /// "body"): `isEquippableInSlot` for `Menu::mount`, one item.
    MountEquip(&'static str),
    /// A mod menu type's slot.
    Mod,
    /// A mod menu type's result (`results`): players only take from it.
    ModResult,
    /// A slot the mod's screen leaves out: no click reaches it.
    Hidden,
}

#[derive(Clone, Copy)]
pub(crate) struct Slot {
    container: usize,
    index: usize,
    role: Role,
}

/// `BeaconBlockEntity.BEACON_EFFECTS`: the powers of each pyramid level.
pub const BEACON_EFFECTS: [&[&str]; 4] = [
    &["minecraft:speed", "minecraft:haste"],
    &["minecraft:resistance", "minecraft:jump_boost"],
    &["minecraft:strength"],
    &["minecraft:regeneration"],
];

/// `BeaconBlockEntity.getRequiredLevelsFor`: 0 for no power, `i32::MAX`
/// for one no level gives.
pub fn beacon_required_levels(effect: Option<&str>) -> i32 {
    let Some(effect) = effect else {
        return 0;
    };
    BEACON_EFFECTS
        .iter()
        .position(|tier| tier.contains(&effect))
        .map_or(i32::MAX, |i| i as i32 + 1)
}

/// The containers a menu spans.
pub(crate) const PLAYER: usize = 0;
pub(crate) const MAIN: usize = 1;
const GRID: usize = 2;
const RESULT: usize = 3;
/// `Container.getMaxStackSize`.
const CONTAINER_MAX: u32 = 99;
/// `Inventory.SLOT_OFFHAND`: the off hand's slot in the player's inventory.
pub const OFFHAND: usize = 40;

pub struct Menu {
    state: storage::State,
    stamp: storage::MutationStamp,
}

/// `AnvilMenu`'s state besides its slots and `cost`.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Anvil {
    /// `itemName`: the rename box's text (`ServerboundRenameItemPacket`).
    pub name: Option<String>,
    /// `repairItemCountCost`.
    pub repair_count: u32,
    /// `onlyRenaming`.
    pub only_renaming: bool,
    /// An `AnvilScreen` listens: a client predicting a click with the screen open, whose
    /// `slotChanged` hears each new input (`slotsChanged`'s `broadcastChanges`).
    pub screen: bool,
    /// The input the screen last heard of (`AbstractContainerMenu.lastSlots`).
    pub seen_input: Option<ItemStack>,
    /// The names the screen's `onNameChanged` sent while the click ran
    /// (`ServerboundRenameItemPacket`), in order; they go out before the click.
    pub renamed: Vec<String>,
}

/// Server recipe IDs in packet button order by item, independent of full-stack predicate checks.
#[derive(Debug, Default, PartialEq)]
pub struct StonecutterButtons(pub(crate) HashMap<Arc<str>, Vec<Arc<str>>>);

impl StonecutterButtons {
    pub fn recipe(&self, input: &str, index: usize) -> Option<&str> {
        self.0.get(input)?.get(index).map(AsRef::as_ref)
    }
}

/// What a click on an item tab's display slot does besides changing the
/// carried stack.
#[derive(Clone, PartialEq, Debug)]
pub enum DisplayClick {
    Nothing,
    /// SWAP: the hotbar slot gets a full stack of the item.
    Hotbar(usize, ItemStack),
    /// SWAP with button 40 (Key F): the off-hand gets a full stack of the item.
    Offhand(ItemStack),
    /// THROW: a copy of the item is dropped.
    Drop(ItemStack),
}

/// Whether a stack has `minecraft:creative_slot_lock`.
pub fn slot_locked(stack: &ItemStack) -> bool {
    stack
        .data()
        .is_some_and(|d| d.other("minecraft:creative_slot_lock").is_some())
}

/// `CreativeModeInventoryScreen.slotClicked` on a display slot
/// (`slot.container == CONTAINER`): `shown` is the slot's item (None past
/// the tab's end) and `max` gives an item's max stack size. The tab's items
/// are never changed: clicks make copies.
pub fn click_display(
    carried: &mut Option<ItemStack>,
    shown: Option<&ItemStack>,
    max: impl Fn(&ItemStack) -> u32,
    button: i32,
    input: Input,
) -> DisplayClick {
    // CustomCreativeSlot.mayPickup: a `creative_slot_lock` stack (the saved
    // toolbars' hint) can't be clicked at all.
    if shown.is_some_and(slot_locked) {
        return DisplayClick::Nothing;
    }
    let copy = |s: &ItemStack, count: u32| ItemStack { count, ..s.clone() };
    let quick_key = input == Input::QuickMove;
    match input {
        Input::Swap => {
            return match shown {
                Some(s) if button == 40 => DisplayClick::Offhand(copy(s, max(s))),
                Some(s) if (0..9).contains(&button) => {
                    DisplayClick::Hotbar(button as usize, copy(s, max(s)))
                }
                Some(_) | None => DisplayClick::Nothing,
            };
        }
        Input::Clone => {
            if carried.is_none()
                && let Some(s) = shown
            {
                *carried = Some(copy(s, max(s)));
            }
            return DisplayClick::Nothing;
        }
        Input::Throw => {
            return match shown {
                Some(s) => DisplayClick::Drop(copy(s, if button == 0 { 1 } else { max(s) })),
                None => DisplayClick::Nothing,
            };
        }
        _ => {}
    }
    match (carried.as_mut(), shown) {
        (Some(c), Some(s)) if c.item == s.item && c.extra == s.extra && c.damage == s.damage => {
            // isSameItemSameComponents: grow, fill or shrink what is carried.
            if button == 0 {
                if quick_key {
                    c.count = max(c);
                } else if c.count < max(c) {
                    c.count += 1;
                }
            } else {
                c.count -= 1;
            }
        }
        (None, Some(s)) => {
            *carried = Some(copy(s, if quick_key { max(s) } else { s.count }));
        }
        (Some(c), _) => {
            if button == 0 {
                *carried = None;
            } else {
                c.count -= 1;
            }
        }
        (None, None) => {}
    }
    if carried.as_ref().is_some_and(|c| c.count == 0) {
        *carried = None;
    }
    DisplayClick::Nothing
}

/// `StonecutterMenu`'s state besides its slots and `selectedRecipeIndex`.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Stonecutter {
    /// `recipesForInput`: indices into `StationRecipes.stonecutting`.
    pub recipes: Vec<usize>,
    /// The item the local or client list was made for (`input`; None for nothing).
    input: Option<Arc<str>>,
    /// The full stack checked by the projected server menu's ingredient predicates.
    server_input: Option<ItemStack>,
    /// The button map sent to the guest of this server menu, pinned until close.
    pub server_buttons: Option<Arc<StonecutterButtons>>,
    /// `ResultContainer.recipeUsed`.
    pub used: Option<usize>,
}

fn same_item(a: &ItemStack, b: &ItemStack) -> bool {
    a.item == b.item
}

fn split(stack: &mut ItemStack, amount: u32) -> ItemStack {
    let real = amount.min(stack.count);
    let mut out = stack.clone();
    out.count = real;
    stack.count -= real;
    out
}

/// `LecternBlockEntity.getPageCount`: a written book's pages, else a
/// writable book's.
pub fn book_pages(stack: &ItemStack) -> usize {
    stack.data().map_or(0, |d| match &d.written {
        Some(w) => w.pages.len(),
        None => d.writable_pages.len(),
    })
}

fn tidy(slot: &mut Option<ItemStack>) {
    if slot.as_ref().is_some_and(|s| s.count == 0) {
        *slot = None;
    }
}

/// Vanilla quick-craft count for one slot, based on the original carried stack.
pub(crate) fn quick_craft_count(
    kind: u8,
    source: u32,
    slots: usize,
    have: u32,
    max: u32,
    item_max: u32,
) -> u32 {
    let each = match kind {
        0 => (source as f32 / slots as f32).floor() as u32,
        1 => 1,
        2 => item_max,
        _ => source,
    };
    (each + have).min(max)
}

/// An index outside the slot list: vanilla's `IndexOutOfBoundsException`.
pub const OUT_OF_BOUNDS: &str = "IndexOutOfBoundsException";

/// `Slot.mayPlace` and the overrides of the slot's concrete role.
fn vanilla_may_place_for_role(
    role: Role,
    env: &Env,
    can_equip: Option<&dyn crate::mobs::equip::CanEquip>,
    stack: &ItemStack,
    crafter_disabled: bool,
    mount: Option<&str>,
) -> bool {
    match role {
        Role::Plain | Role::CreativeDisplay | Role::Mod | Role::EnchantItem => true,
        Role::Armor(eq) => Menu::armor_may_place_with(env, can_equip, eq, stack),
        Role::MountEquip(slot) => mount.is_some_and(|mount| {
            crate::mobs::equip::equippable_in_slot(env.items, stack, slot, mount)
        }),
        Role::Shulker => fits_inside_container_items(env.loop_context, stack),
        Role::Fuel => Menu::is_fuel(env, stack) || Menu::is_bucket(stack),
        Role::CrafterIn => !crafter_disabled,
        Role::Potion => {
            env.brewing.is_bottle(stack)
                || env
                    .items
                    .item_tags
                    .get("minecraft:brewing_potion_inputs")
                    .is_some_and(|m| m.contains(&*stack.item))
        }
        Role::Ingredient => env.brewing.is_reagent(stack),
        Role::BrewFuel => crate::brewing::brewing_fuel(env.items, stack).is_some(),
        Role::LoomBanner => Menu::is_banner(stack),
        Role::LoomDye => env.banners.loom_dye(env.items, stack).is_some(),
        Role::LoomPattern => env.banners.is_pattern_item(env.items, stack),
        Role::GrindInput => crate::stations::grindstone_accepts(env.loop_context, env.items, stack),
        Role::SmithInput(i) => env
            .station_recipes
            .smithing_accepts(env.loop_context, i, stack),
        Role::Lapis => &*stack.item == "minecraft:lapis_lazuli",
        Role::BeaconPay => env
            .items
            .item_tags
            .get("minecraft:beacon_payment_items")
            .is_some_and(|t| t.contains(&*stack.item)),
        Role::CartoMap => stack.data().is_some_and(|d| d.map_id.is_some()),
        Role::CartoExtra => Menu::is_carto_extra(stack),
        Role::Hidden
        | Role::Inert
        | Role::Output
        | Role::CraftResult
        | Role::ModResult
        | Role::LoomResult
        | Role::MerchantResult
        | Role::StoneResult
        | Role::GrindResult
        | Role::CombineResult
        | Role::CartoResult => false,
    }
}

fn vanilla_slot_max_for_role(kind: Kind, slot: Slot, stack: &ItemStack) -> u32 {
    match slot.role {
        Role::Armor(_)
        | Role::MountEquip(_)
        | Role::Potion
        | Role::EnchantItem
        | Role::BeaconPay => 1,
        // The lectern's bookAccess holds one.
        Role::Plain if kind == Kind::Lectern && slot.container == MAIN => 1,
        Role::Fuel if Menu::is_bucket(stack) => 1,
        Role::Mod | Role::ModResult => CONTAINER_MAX,
        // `Container.getMaxStackSize` is vanilla's 99, which no data pack's
        // item reaches; a native mod's item may, and its own size is the cap.
        _ => u32::MAX,
    }
}

/// `Slot.mayPickup` and its overrides. The anvil state is (player levels, cost).
fn vanilla_may_pickup_for_role(
    role: Role,
    kind: Kind,
    env: &Env,
    held: Option<&ItemStack>,
    creative: bool,
    anvil: Option<(i32, i32)>,
    merchant_satisfied: bool,
) -> bool {
    match role {
        Role::Inert | Role::Hidden => false,
        Role::CombineResult if kind == Kind::Anvil => {
            anvil.is_some_and(|(levels, cost)| (creative || levels >= cost) && cost > 0)
        }
        Role::MerchantResult => merchant_satisfied,
        Role::Armor(_) | Role::MountEquip(_) => {
            held.is_none_or(|stack| creative || !env.enchantments.prevents_armor_change(stack))
        }
        _ => true,
    }
}

impl Menu {
    pub fn new(kind: Kind) -> Self {
        let grid_size = kind.grid_size();
        let mut containers: [Vec<Option<ItemStack>>; 4] = Default::default();
        containers[PLAYER] = vec![None; 41];
        containers[MAIN] = vec![None; kind.main_size()];
        containers[GRID] = vec![None; grid_size * grid_size];
        containers[RESULT] = vec![None; 1];
        let slots = (0..kind.slot_count())
            .map(|s| kind.slot(s).unwrap())
            .collect();
        let data = match kind {
            Kind::Crafter | Kind::CrafterBlock => 10,
            Kind::Loom | Kind::Stonecutter | Kind::Anvil | Kind::Smithing | Kind::Lectern => 1,
            Kind::Enchanting => 10, // costs[3], enchantmentSeed, enchantClue[3], levelClue[3]
            Kind::Beacon => 3,      // levels, primary, secondary
            Kind::Brewing | Kind::Furnace(_) => 4,
            _ => 0,
        };
        Self {
            stamp: storage::MutationStamp::fresh(),
            state: storage::State {
                kind,
                registry_id: match kind {
                    Kind::Mod(kind) => Some(kind.id.into()),
                    Kind::Inventory => Some(INVENTORY_MENU.into()),
                    _ => None,
                },
                block_pos: None,
                slots,
                containers,
                grid_size,
                container_limits: Vec::new(),
                carried: None,
                data: vec![0; data],
                creative: false,
                on_client: false,
                drop_spam: None,
                selected: 0,
                drops: Vec::new(),
                dropped: Vec::new(),
                quick_status: 0,
                quick_type: 0,
                quick_slots: Vec::new(),
                loom_patterns: Vec::new(),
                sounds: Vec::new(),
                player_sounds: Vec::new(),
                crafted: Vec::new(),
                item_crafted: Vec::new(),
                crafted_counts: Vec::new(),
                merchant_remove_count: 0,
                brewed: Vec::new(),
                brewed_potion_hook: false,
                brewed_potion_takes: Vec::new(),
                item_smelted_hook: false,
                furnace_awarded: false,
                item_smelted: Vec::new(),
                item_smithed_hook: false,
                item_smithed: Vec::new(),
                takes: None,
                output_removed: 0,
                equipped: Vec::new(),
                merchant: None,
                stone: Stonecutter::default(),
                ground: None,
                levels: 0,
                level_change: 0,
                anvil: Anvil::default(),
                combined: false,
                anvil_hook: false,
                anvil_craft: None,
                grind_hook: false,
                grind_take: None,
                grind_xp: -1,
                anvil_made: false,
                grind_made: false,
                track_results: false,
                more_results: 0,
                enchant_changed: false,
                enchant_button: None,
                may_build: true,
                mount: None,
                sources: Vec::new(),
            },
        }
        .with_initial_data()
    }

    /// Data slots that do not start at 0: the enchanting table's clues are -1.
    fn with_initial_data(mut self) -> Self {
        if self.state.kind == Kind::Enchanting {
            self.state.data[4..].fill(-1);
        }
        self
    }

    pub fn kind(&self) -> Kind {
        self.state.kind
    }

    /// Registry identity of this menu, or the mount sentinel for a mount menu.
    /// The creative picker has no registered menu type; the player's own inventory is INVENTORY_MENU.
    pub fn registry_id(&self) -> Option<&str> {
        self.state.registry_id.as_deref()
    }

    /// Set the identity obtained from the open window's menu registry.
    pub fn set_registry_id(&mut self, id: Option<&str>) {
        self.touch();
        self.state.registry_id = id.map(Into::into);
    }

    /// The block whose menu this is (`ContainerLevelAccess`), told to `menu_slot_rules`;
    /// None for a menu with no block behind it.
    pub fn set_block_pos(&mut self, pos: Option<[i32; 3]>) {
        self.touch();
        self.state.block_pos = pos;
    }

    /// `clickMenuButton`: the loom's pattern buttons and the stonecutter's
    /// recipes. False when refused.
    pub fn button(&mut self, env: &Env, id: i32) -> bool {
        self.touch();
        if self.state.kind == Kind::Lectern {
            return self.lectern_button(env, id);
        }
        if self.state.kind == Kind::Enchanting {
            return self.enchant_click(id);
        }
        if self.state.kind == Kind::Stonecutter {
            // StonecutterMenu.clickMenuButton: an invalid index is taken too.
            if self.state.data[0] == id {
                return false;
            }
            if let Some(recipe) = self.stone_recipe(env, id) {
                self.state.data[0] = id;
                self.stone_setup_result(env, Some(recipe));
                return true;
            }
            // A projected-but-ineligible button is not a recipe selection.
            return self.state.stone.server_buttons.is_none();
        }
        if self.state.kind != Kind::Loom || id < 0 || id as usize >= self.state.loom_patterns.len()
        {
            return false;
        }
        self.state.data[0] = id;
        let pattern = self.state.loom_patterns[id as usize].clone();
        self.loom_setup_result(env, &pattern);
        true
    }

    /// `LecternMenu.clickMenuButton`: the page buttons (1 back, 2 forward,
    /// 100 + n to page n) and Take Book (3), which puts the book in the
    /// inventory or drops it.
    fn lectern_button(&mut self, env: &Env, id: i32) -> bool {
        self.touch();
        match id {
            100.. => self.set_data(0, id - 100),
            1 => self.set_data(0, self.state.data[0] - 1),
            2 => self.set_data(0, self.state.data[0] + 1),
            3 if self.state.may_build => {
                // removeItemNoUpdate: the whole book, and onBookItemRemove.
                let book = self.state.containers[MAIN][0].take();
                self.state.data[0] = 0;
                if let Some(mut book) = book
                    && !self.inventory_add(env, &mut book)
                {
                    self.drop_stack(&book, env.items);
                }
            }
            _ => return false,
        }
        true
    }

    /// `AbstractContainerMenu.setData`: the lectern's is
    /// `LecternBlockEntity.setPage`, kept within the book's pages.
    pub fn set_data(&mut self, id: usize, value: i32) {
        self.touch();
        if self.state.kind == Kind::Lectern && id == 0 {
            // Mth.clamp(page, 0, pageCount - 1).
            let count = self.state.containers[MAIN][0]
                .as_ref()
                .map_or(0, book_pages) as i32;
            self.state.data[0] = value.max(0).min(count - 1);
            return;
        }
        self.state.data[id] = value;
    }

    /// Container slot `slot` of a mod menu type is left out by its screen.
    pub fn hide_slot(&mut self, slot: usize) {
        self.touch();
        if matches!(self.state.kind, Kind::Mod(kind) if slot < kind.slots) {
            self.state.slots[slot].role = Role::Hidden;
        }
    }

    /// `CrafterBlockEntity.setSlotState` (the slot state packet): only an
    /// empty slot can be disabled or enabled.
    pub fn set_slot_state(&mut self, slot: usize, enabled: bool) {
        self.touch();
        if self.state.kind == Kind::CrafterBlock
            && slot < 9
            && self.state.containers[MAIN][slot].is_none()
        {
            self.state.data[slot] = i32::from(!enabled);
        }
    }

    /// The menu after a fixture set its slots: the block crafter's slot
    /// listener has refreshed the result (`broadcastChanges`).
    pub fn settle(&mut self, env: &Env) {
        self.touch();
        if self.state.kind == Kind::CrafterBlock {
            self.crafter_result(env);
        }
    }

    /// `CrafterMenu.refreshRecipeResult`.
    fn crafter_result(&mut self, env: &Env) {
        self.touch();
        self.state.containers[RESULT][0] = env
            .recipes
            .find(env.loop_context, &self.state.containers[MAIN], 3)
            .map(|(_, stack)| stack)
            .filter(|s| s.count > 0);
    }

    /// `LoomMenu.setupResultSlot`.
    fn loom_setup_result(&mut self, env: &Env, pattern: &str) {
        self.touch();
        let mut result = None;
        if let (Some(banner_stack), Some(dye)) = (
            self.state.containers[MAIN][0].as_ref(),
            self.state.containers[MAIN][1].as_ref(),
        ) && let Some(color) = env.banners.loom_dye(env.items, dye)
        {
            result = Some(banner::loom_result(banner_stack, &color, pattern));
        }
        self.state.containers[RESULT][0] = result;
    }

    /// `LoomMenu.slotsChanged`.
    fn loom_changed(&mut self, env: &Env) {
        self.touch();
        let has_inputs =
            self.state.containers[MAIN][0].is_some() && self.state.containers[MAIN][1].is_some();
        if !has_inputs {
            self.state.containers[RESULT][0] = None;
            self.state.loom_patterns.clear();
            self.state.data[0] = -1;
            return;
        }
        let selected = self.state.data[0];
        let valid = selected >= 0 && (selected as usize) < self.state.loom_patterns.len();
        let pattern_item = self.state.containers[MAIN][2].clone();
        let previous = std::mem::replace(
            &mut self.state.loom_patterns,
            env.banners.selectable(pattern_item.as_ref()),
        );
        let shown = if self.state.loom_patterns.len() == 1 {
            self.state.data[0] = 0;
            Some(self.state.loom_patterns[0].clone())
        } else if !valid {
            self.state.data[0] = -1;
            None
        } else {
            let value = previous[selected as usize].clone();
            match self.state.loom_patterns.iter().position(|p| *p == value) {
                Some(i) => {
                    self.state.data[0] = i as i32;
                    Some(value)
                }
                None => {
                    self.state.data[0] = -1;
                    None
                }
            }
        };
        match shown {
            Some(pattern) => {
                let full = self.state.containers[MAIN][0]
                    .as_ref()
                    .is_some_and(banner::has_max_layers);
                if full {
                    self.state.data[0] = -1;
                    self.state.containers[RESULT][0] = None;
                } else {
                    self.loom_setup_result(env, &pattern);
                }
            }
            None => self.state.containers[RESULT][0] = None,
        }
    }

    /// `StonecutterMenu.slotsChanged`: a new input item lists its recipes
    /// (`setupRecipeList`), with none selected and no result.
    fn stone_changed(&mut self, env: &Env) {
        self.touch();
        let input = self.state.containers[MAIN][0].as_ref();
        if self.state.stone.server_buttons.is_some() {
            let before = self.state.stone.server_input.as_ref();
            if input == before {
                return;
            }
            // The selected recipe's index is valid until this server generation closes.
            // A count change can refill it, but still has to pass the full predicate.
            let recipe = input.zip(before).and_then(|(input, before)| {
                self.state.stone.used.filter(|&index| {
                    input.same_item_same_components(before)
                        && env.station_recipes.stonecutting[index]
                            .input
                            .test_in(env.loop_context, input)
                })
            });
            self.state.stone.server_input = input.cloned();
            if recipe.is_none() {
                self.state.data[0] = -1;
            }
            self.stone_setup_result(env, recipe);
            return;
        }
        let input = input.map(|s| s.item.clone());
        if input == self.state.stone.input {
            return;
        }
        self.state.stone.input = input;
        self.state.data[0] = -1;
        self.state.containers[RESULT][0] = None;
        self.state.stone.recipes = match &self.state.containers[MAIN][0] {
            Some(stack) => env
                .station_recipes
                .stonecutting
                .iter()
                .enumerate()
                .filter(|(_, r)| r.input.test_in(env.loop_context, stack))
                .map(|(i, _)| i)
                .collect(),
            None => Vec::new(),
        };
    }

    fn stone_recipe(&self, env: &Env, button: i32) -> Option<usize> {
        let index = usize::try_from(button).ok()?;
        if let Some(buttons) = &self.state.stone.server_buttons {
            let stack = self.state.containers[MAIN][0].as_ref()?;
            let id = buttons.recipe(&stack.item, index)?;
            return env.station_recipes.stonecutting.iter().position(|recipe| {
                recipe.id == id && recipe.input.test_in(env.loop_context, stack)
            });
        }
        self.state.stone.recipes.get(index).copied()
    }

    /// `StonecutterMenu.setupResultSlot`.
    fn stone_setup_result(&mut self, env: &Env, recipe: Option<usize>) {
        self.touch();
        // A client's SelectableRecipe entries carry no recipe (`entry.recipe().recipe()` is empty off the
        // server): its result stays empty until the server's slot update, and a take refills nothing.
        if self.state.on_client {
            self.state.stone.used = None;
            self.state.containers[RESULT][0] = None;
            return;
        }
        self.state.stone.used = recipe;
        self.state.containers[RESULT][0] =
            recipe.map(|r| env.station_recipes.stonecutting[r].result.clone());
    }

    /// `Slot.setChanged` on slot `s`: the loom recomputes from its inputs,
    /// the stonecutter lists its recipes, the block crafter refreshes its
    /// result.
    fn slot_changed(&mut self, env: &Env, s: usize) {
        self.touch();
        // MerchantContainer.setChanged: every slot of it, the result too.
        if self.state.kind == Kind::Merchant && self.state.slots[s].container != PLAYER {
            self.merchant_update(env);
            return;
        }
        // SmithingMenu.slotsChanged runs for its result container too.
        if self.state.kind == Kind::Smithing && self.state.slots[s].container == RESULT {
            self.smith_error();
            return;
        }
        // CartographyTableMenu: its result container calls slotsChanged too.
        if self.state.kind == Kind::Cartography && self.state.slots[s].container != PLAYER {
            self.carto_changed(env);
            return;
        }
        if self.state.slots[s].container != MAIN {
            return;
        }
        match self.state.kind {
            Kind::Loom => self.loom_changed(env),
            Kind::Stonecutter => self.stone_changed(env),
            Kind::Grindstone => self.grindstone_inputs_changed(env),
            Kind::CrafterBlock => self.crafter_result(env),
            Kind::Anvil => {
                self.anvil_screen_heard(env);
                self.anvil_result(env);
            }
            Kind::Enchanting => self.enchant_changed(env),
            Kind::Smithing => {
                self.smith_result(env);
                self.smith_error();
            }
            _ => {}
        }
    }

    /// `slotsChanged(player.getInventory())`, which a bundle click calls
    /// (`BundleItem.broadcastChangesOnContainerMenu`): the menus that don't
    /// check which container changed recompute from their inputs.
    fn inventory_slots_changed(&mut self, env: &Env) {
        self.touch();
        match self.state.kind {
            Kind::Inventory | Kind::Crafting => self.recompute(env),
            Kind::Loom => self.loom_changed(env),
            Kind::Stonecutter => self.stone_changed(env),
            Kind::Cartography => self.carto_changed(env),
            Kind::Merchant => self.merchant_update(env),
            // ItemCombinerMenu.slotsChanged only for its inputs; the
            // smithing table's hasRecipeError always.
            Kind::Smithing => self.smith_error(),
            _ => {}
        }
    }

    /// `EnchantmentMenu.clickMenuButton`'s checks: the lapis, the cost and
    /// the levels (creative pays nothing). The enchanting itself needs a
    /// level and is the caller's (`enchant_button`).
    fn enchant_click(&mut self, id: i32) -> bool {
        self.touch();
        let Some(i) = usize::try_from(id).ok().filter(|&i| i < 3) else {
            return false;
        };
        let need = id + 1;
        let lapis = self.state.containers[MAIN][1]
            .as_ref()
            .map_or(0, |l| l.count as i32);
        if lapis < need && !self.state.creative {
            return false;
        }
        if self.state.data[i] <= 0
            || self.state.containers[MAIN][0].is_none()
            || (self.state.levels < need || self.state.levels < self.state.data[i])
                && !self.state.creative
        {
            return false;
        }
        self.state.enchant_button = Some(i);
        true
    }

    /// `EnchantmentMenu.slotsChanged`: no costs or clues without an
    /// enchantable item; with one, a level fills them (`enchant_changed`).
    fn enchant_changed(&mut self, env: &Env) {
        self.touch();
        let enchantable = self.state.containers[MAIN][0]
            .as_ref()
            .is_some_and(|s| crate::stations::is_enchantable(env.enchantments, s));
        if enchantable {
            self.state.enchant_changed = true;
        } else {
            self.state.data[..3].fill(0);
            self.state.data[4..].fill(-1);
        }
    }

    /// Notify the grindstone's repair input container (`SimpleContainer.setChanged`).
    pub fn grindstone_inputs_changed(&mut self, env: &Env) {
        self.touch();
        if self.result_pending() {
            self.state.more_results += 1;
            return;
        }
        self.state.containers[RESULT][0] = crate::stations::grindstone_result(
            env.loop_context,
            env.enchantments,
            env.items,
            self.state.containers[MAIN][0].as_ref(),
            self.state.containers[MAIN][1].as_ref(),
        );
        self.state.grind_made = true;
    }

    /// `AnvilMenu.createResult`.
    fn anvil_result(&mut self, env: &Env) {
        self.touch();
        if self.result_pending() {
            self.state.more_results += 1;
            return;
        }
        // createResult compares the name with the input's getHoverName().getString(), which
        // neither side's game resolves with a key only Sunrise gives.
        let hover = self.state.containers[MAIN][0]
            .as_ref()
            .map(|i| {
                crate::lang::returned_to_server(|| {
                    crate::stations::hover_name_in(env.loop_context, env.names, i).plain()
                })
            })
            .unwrap_or_default();
        let out = crate::stations::anvil_result(
            env.enchantments,
            env.items,
            self.state.containers[MAIN][0].as_ref(),
            self.state.containers[MAIN][1].as_ref(),
            self.state.anvil.name.as_deref(),
            &hover,
            self.state.creative,
        );
        // onlyRenaming is reset, repairItemCountCost only where an input is.
        self.state.anvil.only_renaming = out.only_renaming;
        if self.state.containers[MAIN][0].is_some() {
            self.state.anvil.repair_count = out.repair_item_count_cost;
        }
        self.state.data[0] = out.cost;
        self.state.containers[RESULT][0] = out.result;
        self.state.anvil_made = true;
    }

    /// The anvil's result, cost and state as `createResult` left them.
    fn anvil_state(&self) -> crate::stations::AnvilResult {
        crate::stations::AnvilResult {
            result: self.state.containers[RESULT][0].clone(),
            cost: self.state.data[0],
            repair_item_count_cost: self.state.anvil.repair_count,
            only_renaming: self.state.anvil.only_renaming,
        }
    }

    /// Whether `createResult` ran since the last call (an input or the name
    /// changed, even back to what it was, or to no visible change).
    pub fn anvil_created(&mut self) -> bool {
        self.touch();
        std::mem::take(&mut self.state.anvil_made)
    }

    /// Whether `GrindstoneMenu.createResult` ran since the last call.
    pub fn grindstone_created(&mut self) -> bool {
        self.touch();
        std::mem::take(&mut self.state.grind_made)
    }

    /// `AnvilUpdateEvent`'s question about the result `createResult` made,
    /// or `None` with no left input, where vanilla has no result.
    pub fn anvil_update(&self) -> Option<(AnvilUpdateInputs<'_>, crate::stations::AnvilResult)> {
        let inputs = AnvilUpdateInputs {
            left: self.state.containers[MAIN][0].as_ref()?,
            right: self.state.containers[MAIN][1].as_ref(),
            name: self.state.anvil.name.as_deref(),
        };
        Some((inputs, self.anvil_state()))
    }

    /// Apply the final result of [`Self::anvil_update`] after mod edits.
    pub fn anvil_updated(&mut self, state: crate::stations::AnvilResult) {
        self.touch();
        self.state.anvil.only_renaming = state.only_renaming;
        self.state.anvil.repair_count = state.repair_item_count_cost;
        self.state.data[0] = state.cost;
        self.state.containers[RESULT][0] = state.result;
    }

    /// Lend the top, bottom and vanilla result slots that `GrindstoneEvent.OnPlaceItem` reads.
    pub fn grindstone_place(&self) -> [Option<&ItemStack>; 3] {
        [
            self.state.containers[MAIN][0].as_ref(),
            self.state.containers[MAIN][1].as_ref(),
            self.state.containers[RESULT][0].as_ref(),
        ]
    }

    /// Apply the admitted output and experience edits. With no override, keep
    /// the vanilla result and use vanilla experience. Cancellation clears the
    /// result and resets experience regardless of the edits.
    pub fn grindstone_placed(&mut self, edits: GrindstonePlaceEdits, cancel: bool) {
        self.touch();
        self.state.grind_xp = -1;
        if cancel {
            self.state.containers[RESULT][0] = None;
        } else if let Some(output) = edits.output {
            self.state.grind_xp = edits.xp;
            self.state.containers[RESULT][0] = Some(output);
        }
    }

    /// Finish a deferred `onTake` outside the take listener's decided action.
    /// Stage the experience for the caller: an exact award is returned as-is,
    /// or `ground` holds the vanilla amount from before the inputs were replaced.
    pub fn grindstone_taken(
        &mut self,
        env: &Env,
        take: &PendingGrindstoneTake,
        edits: GrindstoneTakeEdits,
    ) -> Option<i32> {
        self.touch();
        if edits.xp < 0 {
            self.state.ground = Some(take.vanilla_amount(env));
        }
        let xp = (edits.xp >= 0).then_some(edits.xp);
        self.grindstone_take_inputs(env, edits);
        xp
    }

    /// Replace the inputs and run `slotsChanged` using the admitted remainders.
    pub fn grindstone_take_inputs(&mut self, env: &Env, edits: GrindstoneTakeEdits) {
        self.put(env, 0, edits.new_top);
        self.put(env, 1, edits.new_bottom);
        self.grindstone_inputs_changed(env);
    }

    /// `AnvilMenu.setItemName` (`ServerboundRenameItemPacket`): false when
    /// the name is refused or unchanged.
    /// `BeaconMenu.updateEffects` (`ServerboundSetBeaconPacket`): with a
    /// payment, powers `BeaconBlockEntity.validateEffects` allows for the
    /// pyramid's levels (data 0) become data 1 and 2 and the payment is used.
    /// Each power is its `mob_effect` id and name. False when refused.
    pub fn beacon_update(
        &mut self,
        env: &Env,
        primary: Option<(i32, &str)>,
        secondary: Option<(i32, &str)>,
    ) -> bool {
        self.touch();
        if self.state.kind != Kind::Beacon || self.state.containers[MAIN][0].is_none() {
            return false;
        }
        let levels = self.state.data[0];
        let (p, s) = (
            beacon_required_levels(primary.map(|e| e.1)),
            beacon_required_levels(secondary.map(|e| e.1)),
        );
        let valid = !(secondary.is_some() && levels < 4)
            && p <= levels
            && s <= levels
            && p < 4
            && !(s != 0 && s < 4 && primary.map(|e| e.0) != secondary.map(|e| e.0));
        if !valid {
            return false;
        }
        self.state.data[1] = primary.map_or(0, |e| e.0 + 1);
        self.state.data[2] = secondary.map_or(0, |e| e.0 + 1);
        self.remove(env, 0, 1);
        true
    }

    /// `AnvilScreen.slotChanged` for a new input, inside the click that changed it: the name
    /// box takes the input's name, and `onNameChanged` gives the menu that name (blank for an
    /// input with no custom name of its own), which takes it when it differs (`setItemName`).
    fn anvil_screen_heard(&mut self, env: &Env) {
        if !self.state.anvil.screen || self.state.containers[MAIN][0] == self.state.anvil.seen_input
        {
            return;
        }
        self.state.anvil.seen_input = self.state.containers[MAIN][0].clone();
        let Some(input) = self.state.containers[MAIN][0].clone() else {
            return;
        };
        // The box (an EditBox of 50) keeps the first 50 UTF-16 units of what it is given.
        let name = if input.custom_name().is_some() {
            // As vanilla's client names it: the name goes to the server.
            let hover = crate::lang::returned_to_server(|| {
                crate::stations::hover_name_in(env.loop_context, env.names, &input).plain()
            });
            let mut units = 0;
            hover
                .chars()
                .take_while(|c| {
                    units += c.len_utf16();
                    units <= 50
                })
                .collect()
        } else {
            String::new()
        };
        if self.rename(env, &name) {
            self.state.anvil.renamed.push(name);
        }
    }

    pub fn rename(&mut self, env: &Env, name: &str) -> bool {
        self.touch();
        // StringUtil.filterText, then validateName's length limit.
        let name = crate::text::filter_text(name, false);
        if self.state.kind != Kind::Anvil
            || name.encode_utf16().count() > 50
            || self.state.anvil.name.as_deref() == Some(&name)
        {
            return false;
        }
        if let Some(result) = self.state.containers[RESULT][0].as_mut() {
            result.data_mut().custom_name = (!sunrise_core::java_character::is_blank(&name))
                .then(|| sunrise_core::nbt::Tag::String(name.as_str().into()));
            result.tidy();
        }
        self.state.anvil.name = Some(name);
        self.anvil_result(env);
        true
    }

    /// `SmithingMenu.createResult`: a recipe is looked up only on a `ServerLevel`, so a server's
    /// client finds none and leaves the result empty until the server's slot update.
    fn smith_result(&mut self, env: &Env) {
        self.touch();
        if self.state.on_client {
            self.state.containers[RESULT][0] = None;
            return;
        }
        let [a, b, c] = [0, 1, 2].map(|i| self.state.containers[MAIN][i].as_ref());
        self.state.containers[RESULT][0] =
            env.station_recipes
                .smithing_result(env.loop_context, env.enchantments, a, b, c);
    }

    /// `SmithingMenu.slotsChanged`'s `hasRecipeError`: every input filled and no result, set on a
    /// `ServerLevel` only (a client hears it as the menu's data).
    fn smith_error(&mut self) {
        self.touch();
        if self.state.on_client {
            return;
        }
        let error = self.state.containers[MAIN].iter().all(Option::is_some)
            && self.state.containers[RESULT][0].is_none();
        self.state.data[0] = i32::from(error);
    }

    /// `CartographyTableMenu.slotsChanged` and `setupResultSlot`: a result
    /// only where the level has the map (`access.execute`), with the post
    /// processing its take runs (`MAP_POST_PROCESSING`: 0 lock, 1 scale).
    fn carto_changed(&mut self, env: &Env) {
        self.touch();
        let [map, extra] = [0, 1].map(|i| self.state.containers[MAIN][i].clone());
        if self.state.containers[RESULT][0].is_some() && (map.is_none() || extra.is_none()) {
            self.state.containers[RESULT][0] = None;
            return;
        }
        let (Some(map), Some(extra), Some(maps)) = (map, extra, env.maps) else {
            return;
        };
        let Some((locked, scale)) = map
            .data()
            .and_then(|d| d.map_id)
            .and_then(|id| maps.state(id))
        else {
            return;
        };
        let extendable = env
            .items
            .item_tags
            .get("minecraft:extendable_maps")
            .is_some_and(|t| t.contains(&*map.item));
        let copy = |count: u32, post: Option<u8>| {
            let mut out = ItemStack {
                count,
                ..map.clone()
            };
            if post.is_some() {
                out.data_mut().map_post_processing = post;
            }
            out
        };
        self.state.containers[RESULT][0] = match &*extra.item {
            "minecraft:paper" if extendable && !locked && scale < 4 => Some(copy(1, Some(1))),
            "minecraft:glass_pane" if !locked => Some(copy(1, Some(0))),
            "minecraft:map" => Some(copy(2, None)),
            _ => None,
        };
    }

    /// `MapItem.onCraftedPostProcess`, shared by menu takes and system crafting:
    /// remove the directive and name the level's scaled or locked saved copy.
    pub fn map_crafted(maps: Option<&dyn Maps>, stack: &mut ItemStack) {
        if &*stack.item != "minecraft:filled_map" {
            return;
        }
        let Some(post) = stack.data().and_then(|d| d.map_post_processing) else {
            return;
        };
        stack.data_mut().map_post_processing = None;
        let id = stack.data().and_then(|d| d.map_id);
        if let (Some(maps), Some(id)) = (maps, id)
            && let Some(new) = maps.derive(id, post == 1)
        {
            stack.data_mut().map_id = Some(new);
        }
        stack.tidy();
    }

    pub fn slot_count(&self) -> usize {
        self.state.slots.len()
    }

    pub fn slot(&self, i: usize) -> Option<&ItemStack> {
        let s = self.state.slots[i];
        self.state.containers[s.container][s.index].as_ref()
    }

    /// The menu's own containers (`MAIN`, `GRID`, `RESULT`), in [`mod_api::MenuContainer`] order.
    pub fn slot_containers(
        &self,
    ) -> (
        &[Option<ItemStack>],
        &[Option<ItemStack>],
        &[Option<ItemStack>],
    ) {
        (
            &self.state.containers[MAIN],
            &self.state.containers[GRID],
            &self.state.containers[RESULT],
        )
    }

    /// Load an existing container slot without applying insertion limits.
    pub fn load_slot(&mut self, i: usize, stack: Option<ItemStack>) {
        self.touch();
        let slot = self.state.slots[i];
        self.state.containers[slot.container][slot.index] = stack;
    }

    /// Sets a slot as a test fixture does (`Slot.set`).
    pub fn set_slot(&mut self, env: &Env, i: usize, stack: Option<ItemStack>) {
        self.touch();
        if self.state.kind == Kind::Lectern {
            // LecternBlockEntity.setBook: the book its slot can't set.
            self.state.containers[MAIN][0] = stack;
            return;
        }
        self.put(env, i, stack);
    }

    /// A mod menu type's crafting grid, if it has one.
    pub fn mod_crafting(&self) -> Option<crate::containers::ModCrafting> {
        match self.state.kind {
            Kind::Mod(t) => t.crafting,
            _ => None,
        }
    }

    /// A mod menu's container slots from its block entity, in slot order.
    /// `shown` is the menu's authoritative result for this grid (`Some(None)`
    /// for an empty result). With no current result, prepare from the whole
    /// grid once; loading each cell is not itself a change to the grid.
    pub fn load_container(
        &mut self,
        env: &Env,
        items: &[Option<ItemStack>],
        shown: Option<Option<ItemStack>>,
    ) {
        self.touch();
        let result = self.mod_crafting().map(|c| c.result);
        for (k, stack) in items.iter().enumerate().take(self.container_slot_count()) {
            if Some(k) == result {
                continue;
            }
            if self.state.slots[k].container == GRID {
                let mut stack = stack.clone();
                tidy(&mut stack);
                self.load_slot(k, stack);
            } else {
                self.put(env, k, stack.clone());
            }
        }
        if let Some((slot, stack)) = result.zip(shown) {
            self.load_slot(slot, stack);
        } else {
            self.recompute(env);
        }
    }

    /// `load_container` back: the container slots the block entity keeps, a
    /// crafting result's left empty.
    pub fn container_items(&self) -> Vec<Option<ItemStack>> {
        let result = self.mod_crafting().map(|c| c.result);
        (0..self.container_slot_count())
            .map(|k| (Some(k) != result).then(|| self.slot(k).cloned()).flatten())
            .collect()
    }

    fn container_slot_count(&self) -> usize {
        match self.state.kind {
            Kind::Mod(kind) => kind.slots,
            _ => self.state.slots.len() - 36,
        }
    }

    pub fn player_item(&self, i: usize) -> Option<&ItemStack> {
        self.state.containers[PLAYER][i].as_ref()
    }

    pub fn set_player_item(&mut self, i: usize, stack: Option<ItemStack>) {
        self.touch();
        self.state.containers[PLAYER][i] = stack;
    }

    /// `Inventory.add`: the leftover stays in `stack`.
    pub fn give(&mut self, items: &Items, stack: &mut ItemStack) -> bool {
        self.touch();
        inventory_add(
            &mut self.state.containers[PLAYER],
            self.state.selected,
            self.state.creative,
            items,
            stack,
        )
    }

    /// `Inventory.contains(stack)`.
    pub fn player_contains(&self, stack: &ItemStack) -> bool {
        self.state.containers[PLAYER]
            .iter()
            .flatten()
            .any(|have| have.same_item_same_components(stack))
    }

    // ---- item helpers ----

    fn max_stack(env: &Env, stack: &ItemStack) -> u32 {
        item_max_stack(env.items, stack)
    }

    fn damageable(env: &Env, stack: &ItemStack) -> bool {
        env.items.get(&stack.item).is_some_and(|i| i.max_damage > 0)
    }

    /// `ItemStack.isStackable`.
    fn stackable(env: &Env, stack: &ItemStack) -> bool {
        stackable(env.items, stack)
    }

    fn is_bucket(stack: &ItemStack) -> bool {
        &*stack.item == "minecraft:bucket"
    }

    /// `AbstractFurnaceMenu.isFuel`: presence, not the evaluated burn duration.
    fn is_fuel(env: &Env, stack: &ItemStack) -> bool {
        crate::item_data::cooking_fuel::for_stack(env.items, stack).is_some()
    }

    // ---- Slot ----

    fn may_place(&self, env: &Env, s: usize, stack: &ItemStack) -> bool {
        self.may_place_with(env, env.can_equip, s, stack)
    }

    /// Placement with the equipment scope lent to a scan of unchanged-stack candidates.
    fn may_place_with(
        &self,
        env: &Env,
        can_equip: Option<&dyn crate::mobs::equip::CanEquip>,
        s: usize,
        stack: &ItemStack,
    ) -> bool {
        self.state.slots[s].role != Role::Hidden && self.slot_rules_with(env, can_equip, s, stack).0
    }

    fn vanilla_may_place_with(
        &self,
        env: &Env,
        can_equip: Option<&dyn crate::mobs::equip::CanEquip>,
        s: usize,
        stack: &ItemStack,
    ) -> bool {
        let slot = self.state.slots[s];
        vanilla_may_place_for_role(
            slot.role,
            env,
            can_equip,
            stack,
            slot.role == Role::CrafterIn && self.state.data.get(slot.index) == Some(&1),
            self.state.mount.as_deref(),
        )
    }

    /// What the cartography table's second slot takes.
    fn is_carto_extra(stack: &ItemStack) -> bool {
        matches!(
            &*stack.item,
            "minecraft:paper" | "minecraft:map" | "minecraft:glass_pane"
        )
    }

    /// `BannerItem`.
    fn is_banner(stack: &ItemStack) -> bool {
        stack.item.ends_with("_banner")
    }

    /// `Slot.mayPickup`: the slot's own answer, then, on a mod menu type and only for a stack
    /// the slot holds, the parts' (`menu_slot_rules`).
    pub fn may_pickup(&self, env: &Env, s: usize) -> bool {
        self.vanilla_may_pickup(env, s)
            && match self.slot(s) {
                Some(held) if self.asks_pickup(s) => self.slot_rules(env, s, held).1,
                _ => true,
            }
    }

    /// `canTakeItemForPickAll(carried, slot)` on a mod menu type: whether its parts
    /// (`menu_pick_all`) let a double click's gather take slot `s`'s stack into `carried`. True
    /// for an empty slot, another menu type, or a type no part answers.
    pub fn parts_allow_pick_all(&self, env: &Env, s: usize, carried: &ItemStack) -> bool {
        let (Some(menu), Some(rules), Some(stack)) =
            (self.registry_id(), env.slot_rules, self.slot(s))
        else {
            return true;
        };
        if !matches!(self.state.kind, Kind::Mod(_)) || !rules.pick_all_listens(menu) {
            return true;
        }
        let mut allowed = true;
        rules.pick_all(
            menu,
            self.state.slots[s].index,
            carried,
            stack,
            self.slot_context(env, s),
            &mut allowed,
        );
        allowed
    }

    /// The parts answer `mayPickup` on a mod menu type's slots that its screen shows, and on the
    /// player's own inventory menu ([`INVENTORY_MENU`]).
    fn asks_pickup(&self, s: usize) -> bool {
        matches!(self.state.kind, Kind::Mod(_) | Kind::Inventory)
            && self.state.slots[s].role != Role::Hidden
    }

    fn vanilla_may_pickup(&self, env: &Env, s: usize) -> bool {
        // ResultSlot inherits Slot.mayPickup: the displayed stack is authoritative.
        let role = self.state.slots[s].role;
        let anvil = (role == Role::CombineResult && self.state.kind == Kind::Anvil)
            .then(|| (self.state.levels, self.state.data[0]));
        let merchant_satisfied = role == Role::MerchantResult
            && self
                .merchant
                .as_ref()
                .and_then(|m| m.offers.get(m.active?))
                .is_some_and(|o| {
                    let (a, b) = (
                        self.state.containers[MAIN][0].as_ref(),
                        self.state.containers[MAIN][1].as_ref(),
                    );
                    o.satisfied_by(env.items, a, b) || o.satisfied_by(env.items, b, a)
                });
        vanilla_may_pickup_for_role(
            role,
            self.state.kind,
            env,
            self.slot(s),
            self.state.creative,
            anvil,
            merchant_satisfied,
        )
    }

    /// A result slot (`FurnaceResultSlot`, `ResultSlot`): nothing goes in, and
    /// what's taken from it can't be put back.
    pub fn is_result(&self, s: usize) -> bool {
        matches!(self.state.slots[s].role, Role::Output | Role::CraftResult)
    }

    /// How many of `stack` slot `s` holds: `Slot.getMaxStackSize(stack)` where `Slot.mayPlace`
    /// takes it, else none.
    pub fn slot_holds(&self, env: &Env, s: usize, stack: &ItemStack) -> u32 {
        let (place, _, max) = self.slot_rules(env, s, stack);
        if place && self.state.slots[s].role != Role::Hidden {
            self.limit_slot_max(s, max)
        } else {
            0
        }
    }

    /// `Slot.mayPickup`: whether slot `s` gives up what it holds.
    pub fn slot_gives(&self, env: &Env, s: usize) -> bool {
        self.may_pickup(env, s)
    }

    /// Slot `s` takes one more item of `stack` (`Slot.mayPlace`, and room).
    pub fn takes_one(&self, env: &Env, s: usize, stack: &ItemStack) -> bool {
        self.may_place(env, s, stack)
            && self.slot(s).is_none_or(|have| {
                have.same_item_same_components(stack) && have.count < self.slot_max(env, s, have)
            })
    }

    /// The slots `quickMoveStack` would put slot `index`'s items in: one item
    /// of a plain slot, the whole of a result slot, partial stacks first. Empty
    /// when they don't all fit. This runs the move, so call it on a menu that
    /// is thrown away afterwards.
    pub fn quick_move_landing(&mut self, env: &Env, index: usize) -> Vec<usize> {
        self.touch();
        let Some(mut stack) = self.slot(index).cloned() else {
            return Vec::new();
        };
        if !self.is_result(index) {
            stack.count = 1;
            let slot = self.state.slots[index];
            self.state.containers[slot.container][slot.index] = Some(stack.clone());
        }
        let before: Vec<Option<ItemStack>> = (0..self.state.slots.len())
            .map(|i| self.slot(i).cloned())
            .collect();
        self.quick_move(env, index);
        let (mut partial, mut empty) = (Vec::new(), Vec::new());
        let mut moved = 0;
        for i in (0..self.state.slots.len()).filter(|&i| i != index && !self.is_result(i)) {
            let had = before[i].as_ref().map_or(0, |b| b.count);
            let got = self
                .slot(i)
                .filter(|now| now.same_item_same_components(&stack))
                .map_or(0, |now| now.count.saturating_sub(had));
            if got > 0 {
                moved += got;
                if had > 0 {
                    partial.push(i);
                } else {
                    empty.push(i);
                }
            }
        }
        if moved < stack.count {
            return Vec::new();
        }
        partial.extend(empty);
        partial
    }

    /// Slot `s`'s `mayPlace`, the parts' `mayPickup` (true unless they refuse) and
    /// `getMaxStackSize` for `stack`: its own answers, then the parts' on a registered menu
    /// type, all from one ask.
    pub(crate) fn slot_rules(&self, env: &Env, s: usize, stack: &ItemStack) -> (bool, bool, u32) {
        self.slot_rules_with(env, env.can_equip, s, stack)
    }

    fn slot_rules_with(
        &self,
        env: &Env,
        can_equip: Option<&dyn crate::mobs::equip::CanEquip>,
        s: usize,
        stack: &ItemStack,
    ) -> (bool, bool, u32) {
        let place = self.vanilla_may_place_with(env, can_equip, s, stack);
        let (max, item_max) = (self.vanilla_slot_max(s, stack), Self::max_stack(env, stack));
        let Some(id) = self.registry_id() else {
            return (place, true, max.min(item_max));
        };
        slot_context::apply_menu_slot_rules(
            env.slot_rules,
            id,
            self.state.slots[s].index,
            stack,
            self.slot_context(env, s),
            (place, true, max),
            item_max,
        )
    }

    /// Capacity overrides for the backing container slots, refreshed before each server edit.
    pub fn set_container_limits(&mut self, limits: Vec<Option<u32>>) {
        self.touch();
        self.state.container_limits = limits;
    }

    /// `Slot.getMaxStackSize(stack)`.
    fn slot_max(&self, env: &Env, s: usize, stack: &ItemStack) -> u32 {
        // Capacity does not invoke the separate IItemExtension.canEquip predicate.
        let max = self.slot_rules_with(env, None, s, stack).2;
        self.limit_slot_max(s, max)
    }

    fn vanilla_slot_max(&self, s: usize, stack: &ItemStack) -> u32 {
        vanilla_slot_max_for_role(self.state.kind, self.state.slots[s], stack)
    }

    /// `Container.setItem` (a grid change recomputes the result).
    fn put(&mut self, env: &Env, s: usize, mut stack: Option<ItemStack>) {
        self.touch();
        tidy(&mut stack);
        let slot = self.state.slots[s];
        // LecternBlockEntity's bookAccess.setItem does nothing.
        if self.state.kind == Kind::Lectern && slot.container == MAIN {
            return;
        }
        // SimpleContainer / BaseContainerBlockEntity.setItem: limitSize to
        // getMaxStackSize(stack). The loom's result is a SimpleContainer too.
        if (slot.container == MAIN || slot.role == Role::LoomResult)
            && let Some(st) = stack.as_mut()
        {
            st.count = st.count.min(Self::max_stack(env, st));
            // The beacon's container holds one item (getMaxStackSize 1).
            if self.state.kind == Kind::Beacon {
                st.count = st.count.min(1);
            }
        }
        // CrafterBlockEntity.setItem: filling a disabled (empty) slot enables it.
        if self.state.kind == Kind::CrafterBlock
            && slot.container == MAIN
            && self.state.data[slot.index] == 1
            && self.state.containers[MAIN][slot.index].is_none()
        {
            self.state.data[slot.index] = 0;
        }
        self.state.containers[slot.container][slot.index] = stack;
        if slot.container == GRID {
            self.recompute(env);
        }
        self.slot_changed(env, s);
    }

    /// `Slot.setByPlayer(stack)`: the slot's item is the previous one.
    fn set_by_player(&mut self, env: &Env, s: usize, stack: Option<ItemStack>) {
        self.touch();
        let previous = self.slot(s).cloned();
        self.set_by_player_from(env, s, stack, previous.as_ref());
    }

    /// `Slot.setByPlayer(stack, previous)`: `ArmorSlot` and the inventory
    /// menu's off-hand slot first call the owner's `onEquipItem`, which acts
    /// only when the item changes (`isSameItemSameComponents`).
    fn set_by_player_from(
        &mut self,
        env: &Env,
        s: usize,
        mut stack: Option<ItemStack>,
        previous: Option<&ItemStack>,
    ) {
        self.touch();
        tidy(&mut stack);
        if let Some(slot) = self.equipment_slot(s) {
            let same = match (previous.filter(|p| p.count > 0), stack.as_ref()) {
                (None, None) => true,
                (Some(a), Some(b)) => a.same_item_same_components(b),
                _ => false,
            };
            if !same {
                self.state.equipped.push((slot, stack.clone()));
            }
        }
        self.put(env, s, stack);
        if matches!(self.kind, Kind::Anvil | Kind::Grindstone) {
            // Slot.set calls setChanged after the repair container's setItem.
            self.slot_changed(env, s);
        }
    }

    /// The `EquipmentSlot` of an `ArmorSlot` or of `InventoryMenu`'s off-hand
    /// slot (the only menu slot over inventory slot 40).
    fn equipment_slot(&self, s: usize) -> Option<&'static str> {
        let slot = self.state.slots[s];
        match slot.role {
            Role::Armor(eq) => Some(equippable::ARMOR_SLOTS[eq]),
            _ if slot.container == PLAYER && slot.index == OFFHAND => Some("offhand"),
            _ => None,
        }
    }

    /// `Slot.remove`: the container's `removeItem`.
    fn remove(&mut self, env: &Env, s: usize, amount: u32) -> Option<ItemStack> {
        self.touch();
        let slot = self.state.slots[s];
        if slot.role == Role::MerchantResult {
            let count = amount.min(self.slot(s).map_or(0, |stack| stack.count));
            self.state.merchant_remove_count =
                self.state.merchant_remove_count.wrapping_add(count as i32);
        }
        // ResultContainer.removeItem takes the whole stack; the loom's result
        // is a SimpleContainer slot and splits.
        if slot.container == RESULT && slot.role != Role::LoomResult {
            return self.state.containers[RESULT][0].take();
        }
        let cell = &mut self.state.containers[slot.container][slot.index];
        let out = cell
            .as_mut()
            .filter(|c| c.count > 0)
            .map(|c| split(c, amount));
        tidy(cell);
        // FurnaceResultSlot.remove counts what it gave.
        if slot.role == Role::Output {
            self.state.output_removed += out.as_ref().map_or(0, |o| o.count);
        }
        // LecternBlockEntity: an emptied book slot is onBookItemRemove (page 0).
        if self.state.kind == Kind::Lectern && slot.container == MAIN && cell.is_none() {
            self.state.data[0] = 0;
        }
        if out.is_some() && slot.container == GRID {
            self.recompute(env);
        }
        // SimpleContainer.removeItem calls setChanged when it took something;
        // MerchantContainer.removeItem updates the result after a payment slot.
        if out.is_some()
            && matches!(
                self.state.kind,
                Kind::Loom
                    | Kind::Merchant
                    | Kind::Stonecutter
                    | Kind::Grindstone
                    | Kind::Anvil
                    | Kind::Smithing
                    | Kind::Enchanting
                    | Kind::Cartography
            )
        {
            self.slot_changed(env, s);
        }
        out
    }

    /// `AnvilMenu.onTake`: the levels, the material used up, then the input.
    pub fn anvil_take(&mut self, env: &Env) {
        self.record_anvil_charge();
        self.anvil_take_material(env);
        self.anvil_take_input(env);
    }

    /// Record AnvilMenu.onTake's level cost, before writing either input.
    pub fn record_anvil_charge(&mut self) {
        self.touch();
        let cost = self.state.data[0];
        if !self.state.creative {
            // Player.giveExperienceLevels(-cost).
            self.state.level_change -= cost;
            self.state.drops.push(format!(
                "levels{}{}",
                if cost <= 0 { "+" } else { "" },
                -cost
            ));
        }
    }

    /// Consume or clear the material, while the left input is still present.
    pub fn anvil_take_material(&mut self, env: &Env) {
        let repair = self.state.anvil.repair_count;
        if repair > 0 {
            let addition = self.state.containers[MAIN][1]
                .clone()
                .filter(|a| a.count > repair)
                .map(|mut a| {
                    a.count -= repair;
                    a
                });
            self.put(env, 1, addition);
        } else if !self.state.anvil.only_renaming {
            self.put(env, 1, None);
        }
    }

    /// Second half of `AnvilMenu.onTake`: zero `cost`, clear slot 0, and mark the craft combined.
    pub fn anvil_take_input(&mut self, env: &Env) {
        self.state.data[0] = 0;
        self.put(env, 0, None);
        self.state.combined = true;
    }

    /// `CraftingMenu.slotChangedCraftingGrid`.
    #[sunrise_mod_api_macros::replaceable]
    fn recompute(&mut self, env: &Env) {
        self.touch();
        if self.state.grid_size == 0 || self.state.on_client {
            return;
        }
        self.state.containers[RESULT][0] =
            env.crafting_result(&self.state.containers[GRID], self.state.grid_size);
    }

    /// `FurnaceResultSlot.checkTakeAchievements`: on the server side, always
    /// records that the furnace must award used recipes and pop experience
    /// (`AbstractFurnaceBlockEntity.awardUsedRecipesAndPopExperience`); on
    /// both logical sides, when `removeCount != 0` and a mod listens, keeps
    /// the non-empty stack and `removeCount` for `ItemSmeltedEvent` (matching
    /// NeoForge's `FurnaceResultSlot.java.patch`); `removeCount` starts again.
    fn furnace_take(&mut self, env: &Env, carried: &ItemStack) {
        self.touch();
        let removed = std::mem::take(&mut self.state.output_removed);
        if env.loop_context.side() == LoopSide::Server {
            self.state.furnace_awarded = true;
        }
        if self.state.item_smelted_hook && removed != 0 && carried.count > 0 {
            self.state.item_smelted.push((carried.clone(), removed));
        }
    }

    /// A station's take sound, gated by its menu's `lastSoundTime`: once per
    /// tick, so a shift-click that takes many results plays it once.
    fn take_sound(&mut self, sound: &'static str) {
        self.touch();
        if !self.state.sounds.contains(&sound) {
            self.state.sounds.push(sound);
        }
    }

    fn drop_stack(&mut self, stack: &ItemStack, items: &Items) {
        self.touch();
        if stack.count > 0 {
            self.state
                .drops
                .push(crate::item_data::canon_stack(Some(stack), items));
            self.state.dropped.push(stack.clone());
        }
    }

    /// `Slot.tryRemove`.
    fn try_remove(
        &mut self,
        env: &Env,
        s: usize,
        amount: u32,
        max_amount: u32,
    ) -> Option<ItemStack> {
        self.touch();
        if !self.may_pickup(env, s) {
            return None;
        }
        if !self.allow_modification(env, s) && max_amount < self.slot(s).map_or(0, |c| c.count) {
            return None;
        }
        let result = self.remove(env, s, amount.min(max_amount))?;
        if self.slot(s).is_none() {
            self.set_by_player_from(env, s, None, Some(&result));
        }
        Some(result)
    }

    /// `Slot.allowModification`: `mayPickup` and `mayPlace` of what the slot holds.
    fn allow_modification(&self, env: &Env, s: usize) -> bool {
        self.may_pickup(env, s) && self.slot(s).is_none_or(|held| self.may_place(env, s, held))
    }

    fn safe_take(
        &mut self,
        env: &Env,
        s: usize,
        amount: u32,
        max_amount: u32,
    ) -> Option<ItemStack> {
        self.touch();
        let mut result = self.try_remove(env, s, amount, max_amount)?;
        self.on_take(env, s, &mut result);
        if self.state.slots[s].role == Role::CartoResult {
            Self::map_crafted(env.maps, &mut result);
        }
        Some(result)
    }

    /// `Slot.safeInsert`: puts up to `amount` of `input` into the slot.
    fn safe_insert(&mut self, env: &Env, s: usize, input: &mut Option<ItemStack>, amount: u32) {
        self.touch();
        let Some(stack) = input.as_mut() else { return };
        if !self.may_place(env, s, stack) {
            return;
        }
        let max = self.slot_max(env, s, stack);
        let have = self.slot(s).map_or(0, |c| c.count);
        let transferable = amount.min(stack.count).min(max.saturating_sub(have));
        if transferable == 0 {
            return;
        }
        match self.slot(s).cloned() {
            None => {
                let part = split(stack, transferable);
                self.set_by_player(env, s, Some(part));
            }
            Some(mut current) if current.same_item_same_components(stack) => {
                stack.count -= transferable;
                current.count += transferable;
                self.set_by_player(env, s, Some(current));
            }
            Some(_) => {}
        }
        tidy(input);
    }

    // ---- Inventory ----

    /// `Inventory.getSlotWithRemainingSpace` (selected slot 0, offhand, rest).
    fn slot_with_space(&self, env: &Env, stack: &ItemStack) -> Option<usize> {
        slot_with_space(
            &self.state.containers[PLAYER],
            self.state.selected,
            env.items,
            stack,
        )
    }

    /// `Inventory.add(stack)`: the leftover stays in `stack`.
    fn inventory_add(&mut self, env: &Env, stack: &mut ItemStack) -> bool {
        self.give(env.items, stack)
    }
}

/// `ItemStack.getMaxStackSize`: the stack's component, else the item's; a removed
/// component (`!minecraft:max_stack_size`) uses vanilla's fallback of one.
pub use sunrise_items::items::item_max_stack;

/// Whether a stack can merge in the player's inventory.
fn stackable(items: &Items, stack: &ItemStack) -> bool {
    item_max_stack(items, stack) > 1
        && !(stack.damage > 0 && items.get(&stack.item).is_some_and(|i| i.max_damage > 0))
}

/// [`slot_with_remaining_space`] in the player's container `inventory` (the off hand at
/// [`OFFHAND`]), with `stack`'s own maximum, none for a stack that never joins another.
fn slot_with_space(
    inventory: &[Option<ItemStack>],
    selected: usize,
    items: &Items,
    stack: &ItemStack,
) -> Option<usize> {
    let max = if stackable(items, stack) {
        item_max_stack(items, stack)
    } else {
        0
    };
    slot_with_remaining_space(
        &inventory[..36],
        inventory[OFFHAND].as_ref(),
        selected,
        stack,
        max,
    )
}

/// `Inventory.getSlotWithRemainingSpace`: where `stack` joins a stack already there
/// (`hasRemainingSpaceForItem`: the same item and components, fewer than `max`, the stack's
/// `getMaxStackSize`, which is 0 for a stack that is not `isStackable`). The selected slot
/// first, then the off hand ([`OFFHAND`]), then the 36 main slots `items` in order.
/// `Inventory.add` and `placeItemBackInInventory` ask it before [`free_slot`]. NeoForge and
/// Fabric have no event that picks the slot (`ItemEntityPickupEvent.Pre` can refuse a pickup,
/// not place it), so a Java mod mixes into `Inventory.getSlotWithRemainingSpace`; a native mod
/// answers here through `[[native.mixin]]` (#4854).
#[sunrise_mod_api_macros::replaceable]
pub fn slot_with_remaining_space(
    items: &[Option<ItemStack>],
    offhand: Option<&ItemStack>,
    selected: usize,
    stack: &ItemStack,
    max: u32,
) -> Option<usize> {
    let has_space = |have: Option<&ItemStack>| {
        have.is_some_and(|have| have.same_item_same_components(stack) && have.count < max)
    };
    let slot = if has_space(items[selected].as_ref()) {
        Some(selected)
    } else if has_space(offhand) {
        Some(OFFHAND)
    } else {
        items.iter().position(|have| has_space(have.as_ref()))
    };
    #[cfg(test)]
    let slot =
        crate::rule_replacements::slot_with_remaining_space(items, offhand, selected, stack, max)
            .unwrap_or(slot);
    let _ = items;
    slot
}

/// `Inventory.getFreeSlot`: the first empty one of the 36 main slots `items`, never the off
/// hand. `Inventory.add` and `placeItemBackInInventory` put there what joins no stack
/// ([`slot_with_remaining_space`]), and pick-block's `addAndPickItem` the stack it moves out
/// of the hotbar. NeoForge and Fabric have no event that picks the slot, so a Java mod mixes
/// into `Inventory.getFreeSlot`; a native mod answers here through `[[native.mixin]]` (#4854).
#[sunrise_mod_api_macros::replaceable]
pub fn free_slot(items: &[Option<ItemStack>]) -> Option<usize> {
    let slot = items.iter().position(Option::is_none);
    #[cfg(test)]
    let slot = crate::rule_replacements::free_slot(items).unwrap_or(slot);
    let _ = items;
    slot
}

/// `Inventory.add`; leaves any remainder in `stack`.
pub(crate) fn inventory_add(
    inventory: &mut [Option<ItemStack>],
    selected: usize,
    creative: bool,
    items: &Items,
    stack: &mut ItemStack,
) -> bool {
    if stack.count == 0 {
        return false;
    }
    if items.get(&stack.item).is_some_and(|i| i.max_damage > 0) && stack.damage > 0 {
        if let Some(slot) = free_slot(&inventory[..36]) {
            inventory[slot] = Some(stack.clone());
            stack.count = 0;
            return true;
        }
        if creative {
            stack.count = 0;
            return true;
        }
        return false;
    }
    loop {
        let last = stack.count;
        let slot = slot_with_space(inventory, selected, items, stack)
            .or_else(|| free_slot(&inventory[..36]));
        if let Some(slot) = slot {
            let cell = inventory[slot].get_or_insert_with(|| {
                let mut empty = stack.clone();
                empty.count = 0;
                empty
            });
            let add = stack
                .count
                .min(item_max_stack(items, cell).saturating_sub(cell.count));
            cell.count += add;
            stack.count -= add;
        }
        if stack.count == 0 || stack.count >= last {
            if stack.count == last && creative {
                stack.count = 0;
                return true;
            }
            return stack.count < last;
        }
    }
}

/// `EquipmentSlot` of an item as the inventory menu's quick move sees it.
enum Equip {
    Armor(usize),
    Offhand,
    Other,
}

impl Menu {
    fn equip(env: &Env, stack: &ItemStack) -> Equip {
        match crate::mobs::equip::equippable(env.items, stack).map(|e| e.slot) {
            Some("feet") => Equip::Armor(0),
            Some("legs") => Equip::Armor(1),
            Some("chest") => Equip::Armor(2),
            Some("head") => Equip::Armor(3),
            Some("offhand") => Equip::Offhand,
            _ => Equip::Other,
        }
    }

    fn has_item(&self, s: usize) -> bool {
        self.slot(s).is_some()
    }

    /// `AbstractContainerMenu.moveItemStackTo`.
    fn move_to(
        &mut self,
        env: &Env,
        origin: Option<usize>,
        stack: &mut ItemStack,
        start: usize,
        end: usize,
        backwards: bool,
    ) -> bool {
        self.touch();
        self.move_item_stack_to(env, origin, stack, start, end, backwards)
    }

    /// `quickMoveStack` of the menu's kind; the moved stack's original copy
    /// (None when nothing moved).
    fn quick_move(&mut self, env: &Env, index: usize) -> Option<ItemStack> {
        self.touch();
        let mut stack = self.slot(index)?.clone();
        let clicked = stack.clone();
        let total = self.state.slots.len();
        let mut check_count = true;
        let mut after_take = false;
        match self.state.kind {
            Kind::ItemPicker => {
                // ItemPickerMenu.quickMoveStack: a hotbar stack is deleted.
                if index >= total - 9 {
                    self.put(env, index, None);
                }
                return None;
            }
            Kind::Chest(rows) => {
                check_count = false;
                let n = rows * 9;
                if index < n {
                    if !self.move_to(env, Some(index), &mut stack, n, total, true) {
                        return None;
                    }
                } else if !self.move_to(env, Some(index), &mut stack, 0, n, false) {
                    return None;
                }
            }
            Kind::Hopper => {
                check_count = false;
                if index < 5 {
                    if !self.move_to(env, Some(index), &mut stack, 5, 41, true) {
                        return None;
                    }
                } else if !self.move_to(env, Some(index), &mut stack, 0, 5, false) {
                    return None;
                }
            }
            // A crafting grid's result: CraftingMenu.quickMoveStack's.
            Kind::Mod(t) if self.state.slots[index].role == Role::CraftResult => {
                after_take = true;
                Self::map_crafted(env.maps, &mut stack);
                self.state.containers[RESULT][0] = Some(stack.clone());
                if !self.move_to(env, Some(index), &mut stack, t.slots, total, true) {
                    return None;
                }
            }
            // ChestMenu.quickMoveStack over the mod's slots.
            Kind::Mod(t) => {
                check_count = false;
                let n = t.slots;
                if index < n {
                    if !self.move_to(env, Some(index), &mut stack, n, total, true) {
                        return None;
                    }
                } else if !self.move_to(env, Some(index), &mut stack, 0, n, false) {
                    return None;
                }
            }
            Kind::Shulker => {
                check_count = false;
                if index < 27 {
                    if !self.move_to(env, Some(index), &mut stack, 27, total, true) {
                        return None;
                    }
                } else if !self.move_to(env, Some(index), &mut stack, 0, 27, false) {
                    return None;
                }
            }
            Kind::Dispenser => {
                after_take = true;
                if index < 9 {
                    if !self.move_to(env, Some(index), &mut stack, 9, 45, true) {
                        return None;
                    }
                } else if !self.move_to(env, Some(index), &mut stack, 0, 9, false) {
                    return None;
                }
            }
            Kind::Brewing => {
                after_take = true;
                let slot = self.state.slots[index];
                let commit = |m: &mut Self, stack: &ItemStack| {
                    m.containers[slot.container][slot.index] =
                        (stack.count > 0).then(|| stack.clone());
                };
                if index <= 4 {
                    if !self.move_to(env, Some(index), &mut stack, 5, 41, true) {
                        return None;
                    }
                } else if crate::brewing::brewing_fuel(env.items, &clicked).is_some() {
                    let moved = self.move_to(env, Some(index), &mut stack, 4, 5, false);
                    if moved
                        || (env.brewing.is_reagent(&stack)
                            && !self.move_to(env, Some(index), &mut stack, 3, 4, false))
                    {
                        commit(self, &stack);
                        return None;
                    }
                } else if env.brewing.is_reagent(&stack) {
                    if !self.move_to(env, Some(index), &mut stack, 3, 4, false) {
                        return None;
                    }
                } else if env.brewing.is_bottle(&stack)
                    || env
                        .items
                        .item_tags
                        .get("minecraft:brewing_potion_inputs")
                        .is_some_and(|m| m.contains(&*stack.item))
                {
                    if !self.move_to(env, Some(index), &mut stack, 0, 3, false) {
                        return None;
                    }
                } else if (5..32).contains(&index) {
                    if !self.move_to(env, Some(index), &mut stack, 32, 41, false) {
                        return None;
                    }
                } else if (32..41).contains(&index) {
                    if !self.move_to(env, Some(index), &mut stack, 5, 32, false) {
                        return None;
                    }
                } else if !self.move_to(env, Some(index), &mut stack, 5, 41, false) {
                    return None;
                }
            }
            Kind::Loom => {
                after_take = true;
                if index == 3 {
                    if !self.move_to(env, Some(index), &mut stack, 4, 40, true) {
                        return None;
                    }
                } else if index > 3 {
                    let target = if Self::is_banner(&stack) {
                        Some(0)
                    } else if env.banners.loom_dye(env.items, &stack).is_some() {
                        Some(1)
                    } else if env.banners.is_pattern_item(env.items, &stack) {
                        Some(2)
                    } else {
                        None
                    };
                    match target {
                        Some(t) => {
                            if !self.move_to(env, Some(index), &mut stack, t, t + 1, false) {
                                return None;
                            }
                        }
                        None if index < 31 => {
                            if !self.move_to(env, Some(index), &mut stack, 31, 40, false) {
                                return None;
                            }
                        }
                        None => {
                            if index < 40
                                && !self.move_to(env, Some(index), &mut stack, 4, 31, false)
                            {
                                return None;
                            }
                        }
                    }
                } else if !self.move_to(env, Some(index), &mut stack, 4, 40, false) {
                    return None;
                }
            }
            Kind::Stonecutter => {
                after_take = true;
                if index == 1 {
                    if !self.move_to(env, Some(index), &mut stack, 2, 38, true) {
                        return None;
                    }
                } else if index == 0 {
                    if !self.move_to(env, Some(index), &mut stack, 2, 38, false) {
                        return None;
                    }
                } else if env
                    .station_recipes
                    .stonecutting
                    .iter()
                    .any(|r| r.input.test_in(env.loop_context, &stack))
                {
                    if !self.move_to(env, Some(index), &mut stack, 0, 1, false) {
                        return None;
                    }
                } else if (2..29).contains(&index) {
                    if !self.move_to(env, Some(index), &mut stack, 29, 38, false) {
                        return None;
                    }
                } else if (29..38).contains(&index)
                    && !self.move_to(env, Some(index), &mut stack, 2, 29, false)
                {
                    return None;
                }
            }
            Kind::Grindstone => {
                after_take = true;
                let full = self.state.containers[MAIN].iter().all(Option::is_some);
                if index == 2 {
                    if !self.move_to(env, Some(index), &mut stack, 3, 39, true) {
                        return None;
                    }
                } else if index == 0 || index == 1 {
                    if !self.move_to(env, Some(index), &mut stack, 3, 39, false) {
                        return None;
                    }
                } else if full {
                    if (3..30).contains(&index) {
                        if !self.move_to(env, Some(index), &mut stack, 30, 39, false) {
                            return None;
                        }
                    } else if (30..39).contains(&index)
                        && !self.move_to(env, Some(index), &mut stack, 3, 30, false)
                    {
                        return None;
                    }
                } else if !self.move_to(env, Some(index), &mut stack, 0, 2, false) {
                    return None;
                }
            }
            Kind::Cartography => {
                after_take = true;
                if index == 2 {
                    // onCraftedBy edits the result in its slot first.
                    Self::map_crafted(env.maps, &mut stack);
                    self.state.containers[RESULT][0] = Some(stack.clone());
                    if !self.move_to(env, Some(index), &mut stack, 3, 39, true) {
                        return None;
                    }
                } else if index == 0 || index == 1 {
                    if !self.move_to(env, Some(index), &mut stack, 3, 39, false) {
                        return None;
                    }
                } else if stack.data().is_some_and(|d| d.map_id.is_some()) {
                    if !self.move_to(env, Some(index), &mut stack, 0, 1, false) {
                        return None;
                    }
                } else if Self::is_carto_extra(&stack) {
                    if !self.move_to(env, Some(index), &mut stack, 1, 2, false) {
                        return None;
                    }
                } else if (3..30).contains(&index) {
                    if !self.move_to(env, Some(index), &mut stack, 30, 39, false) {
                        return None;
                    }
                } else if (30..39).contains(&index)
                    && !self.move_to(env, Some(index), &mut stack, 3, 30, false)
                {
                    return None;
                }
            }
            // LecternMenu.quickMoveStack moves nothing.
            Kind::Lectern => return None,
            Kind::Mount(columns) => {
                // AbstractMountInventoryMenu.quickMoveStack: no count check
                // and no onTake.
                check_count = false;
                let start = 2 + columns * 3;
                if index < start {
                    if !self.move_to(env, Some(index), &mut stack, start, total, true) {
                        return None;
                    }
                } else if self.may_place(env, 1, &stack) && self.state.containers[MAIN][1].is_none()
                {
                    if !self.move_to(env, Some(index), &mut stack, 1, 2, false) {
                        return None;
                    }
                } else if self.may_place(env, 0, &stack) && self.state.containers[MAIN][0].is_none()
                {
                    if !self.move_to(env, Some(index), &mut stack, 0, 1, false) {
                        return None;
                    }
                } else if columns == 0
                    || !self.move_to(env, Some(index), &mut stack, 2, start, false)
                {
                    // Between the inventory and the hotbar: the stack shrinks
                    // in its slot and EMPTY is returned either way.
                    let (main_end, hotbar_end) = (start + 27, start + 36);
                    if (main_end..hotbar_end).contains(&index) {
                        self.move_to(env, Some(index), &mut stack, start, main_end, false);
                    } else if (start..main_end).contains(&index) {
                        self.move_to(env, Some(index), &mut stack, main_end, hotbar_end, false);
                    }
                    let slot = self.state.slots[index];
                    self.state.containers[slot.container][slot.index] =
                        (stack.count > 0).then(|| stack.clone());
                    return None;
                }
            }
            Kind::Beacon => {
                // BeaconMenu.quickMoveStack.
                after_take = true;
                if index == 0 {
                    if !self.move_to(env, Some(index), &mut stack, 1, 37, true) {
                        return None;
                    }
                } else if self.state.containers[MAIN][0].is_none()
                    && self.may_place(env, 0, &stack)
                    && stack.count == 1
                {
                    if !self.move_to(env, Some(index), &mut stack, 0, 1, false) {
                        return None;
                    }
                } else if (1..28).contains(&index) {
                    if !self.move_to(env, Some(index), &mut stack, 28, 37, false) {
                        return None;
                    }
                } else if (28..37).contains(&index) {
                    if !self.move_to(env, Some(index), &mut stack, 1, 28, false) {
                        return None;
                    }
                } else if !self.move_to(env, Some(index), &mut stack, 1, 37, false) {
                    return None;
                }
            }
            Kind::Enchanting => {
                after_take = true;
                if index < 2 {
                    if !self.move_to(env, Some(index), &mut stack, 2, 38, true) {
                        return None;
                    }
                } else if &*stack.item == "minecraft:lapis_lazuli" {
                    if !self.move_to(env, Some(index), &mut stack, 1, 2, true) {
                        return None;
                    }
                } else {
                    // One item into the empty item slot (setByPlayer).
                    if self.state.containers[MAIN][0].is_some() || !self.may_place(env, 0, &stack) {
                        return None;
                    }
                    let single = ItemStack {
                        count: 1,
                        ..stack.clone()
                    };
                    stack.count -= 1;
                    // The source slot keeps the rest while the item slot changes.
                    let slot = self.state.slots[index];
                    self.state.containers[slot.container][slot.index] =
                        (stack.count > 0).then(|| stack.clone());
                    self.set_by_player(env, 0, Some(single));
                }
            }
            Kind::Anvil | Kind::Smithing => {
                // ItemCombinerMenu.quickMoveStack.
                after_take = true;
                let result = if self.state.kind == Kind::Anvil { 2 } else { 3 };
                let (start, use_row) = (result + 1, result + 28);
                let end = result + 37;
                let into_inputs = self.state.kind == Kind::Anvil
                    || (0..3).any(|i| {
                        self.state.containers[MAIN][i].is_none()
                            && env
                                .station_recipes
                                .smithing_accepts(env.loop_context, i, &stack)
                    });
                if index == result {
                    if !self.move_to(env, Some(index), &mut stack, start, end, true) {
                        return None;
                    }
                } else if index < result {
                    if !self.move_to(env, Some(index), &mut stack, start, end, false) {
                        return None;
                    }
                } else if into_inputs && (start..end).contains(&index) {
                    if !self.move_to(env, Some(index), &mut stack, 0, result, false) {
                        return None;
                    }
                } else if (start..use_row).contains(&index) {
                    if !self.move_to(env, Some(index), &mut stack, use_row, end, false) {
                        return None;
                    }
                } else if (use_row..end).contains(&index)
                    && !self.move_to(env, Some(index), &mut stack, start, use_row, false)
                {
                    return None;
                }
            }
            Kind::Merchant => {
                after_take = true;
                if index == 2 {
                    if !self.move_to(env, Some(index), &mut stack, 3, 39, true) {
                        return None;
                    }
                    // Slot.onQuickCraft uses the original copy and the moved count.
                    self.state.merchant_remove_count = self
                        .merchant_remove_count
                        .wrapping_add((clicked.count - stack.count) as i32);
                    self.merchant_check_take_achievements(&clicked);
                    // MerchantMenu.playTradeSound: not for a ClientSideMerchant.
                    if let Some(m) = self.state.merchant.as_mut()
                        && !m.client_side
                    {
                        m.events.push(crate::merchant::Event::TradeSound);
                    }
                } else if index != 0 && index != 1 {
                    if (3..30).contains(&index) {
                        if !self.move_to(env, Some(index), &mut stack, 30, 39, false) {
                            return None;
                        }
                    } else if (30..39).contains(&index)
                        && !self.move_to(env, Some(index), &mut stack, 3, 30, false)
                    {
                        return None;
                    }
                } else if !self.move_to(env, Some(index), &mut stack, 3, 39, false) {
                    return None;
                }
            }
            Kind::Crafter | Kind::CrafterBlock => {
                after_take = true;
                if index < 9 {
                    if !self.move_to(env, Some(index), &mut stack, 9, 45, true) {
                        return None;
                    }
                } else if !self.move_to(env, Some(index), &mut stack, 0, 9, false) {
                    return None;
                }
            }
            Kind::Furnace(recipe) => {
                after_take = true;
                if index == 2 {
                    if !self.move_to(env, Some(index), &mut stack, 3, 39, true) {
                        return None;
                    }
                    // Slot.onQuickCraft -> FurnaceResultSlot.onQuickCraft:
                    // the moved count, with the stack as it was.
                    let moved = clicked.count - stack.count;
                    if moved > 0 {
                        self.state.output_removed += moved;
                        self.furnace_take(env, &clicked);
                    }
                } else if index != 1 && index != 0 {
                    let smelts = env.recipes.cooking_accepts(
                        env.loop_context,
                        &format!("minecraft:{recipe}"),
                        &stack,
                    );
                    if smelts {
                        if !self.move_to(env, Some(index), &mut stack, 0, 1, false) {
                            return None;
                        }
                    } else if Self::is_fuel(env, &stack) {
                        if !self.move_to(env, Some(index), &mut stack, 1, 2, false) {
                            return None;
                        }
                    } else if (3..30).contains(&index) {
                        if !self.move_to(env, Some(index), &mut stack, 30, 39, false) {
                            return None;
                        }
                    } else if (30..39).contains(&index)
                        && !self.move_to(env, Some(index), &mut stack, 3, 30, false)
                    {
                        return None;
                    }
                } else if !self.move_to(env, Some(index), &mut stack, 3, 39, false) {
                    return None;
                }
            }
            Kind::Crafting => {
                after_take = true;
                if index == 0 {
                    // CraftingMenu.quickMoveStack processes the live result
                    // before transfer; ResultSlot then processes its original copy.
                    Self::map_crafted(env.maps, &mut stack);
                    self.state.containers[RESULT][0] = Some(stack.clone());
                    if !self.move_to(env, Some(index), &mut stack, 10, 46, true) {
                        return None;
                    }
                } else if (10..46).contains(&index) {
                    if !self.move_to(env, Some(index), &mut stack, 1, 10, false) {
                        if index < 37 {
                            if !self.move_to(env, Some(index), &mut stack, 37, 46, false) {
                                return None;
                            }
                        } else if !self.move_to(env, Some(index), &mut stack, 10, 37, false) {
                            return None;
                        }
                    }
                } else if !self.move_to(env, Some(index), &mut stack, 10, 46, false) {
                    return None;
                }
            }
            Kind::Inventory => {
                after_take = true;
                let equip = Self::equip(env, &clicked);
                let armor_free = |m: &Self, eq: usize| !m.has_item(8 - eq);
                if index == 0 {
                    if !self.move_to(env, Some(index), &mut stack, 9, 45, true) {
                        return None;
                    }
                } else if (1..9).contains(&index) {
                    if !self.move_to(env, Some(index), &mut stack, 9, 45, false) {
                        return None;
                    }
                } else if matches!(equip, Equip::Armor(eq) if armor_free(self, eq)) {
                    let Equip::Armor(eq) = equip else {
                        unreachable!()
                    };
                    let pos = 8 - eq;
                    if !self.move_to(env, Some(index), &mut stack, pos, pos + 1, false) {
                        return None;
                    }
                } else if matches!(equip, Equip::Offhand) && !self.has_item(45) {
                    if !self.move_to(env, Some(index), &mut stack, 45, 46, false) {
                        return None;
                    }
                } else if (9..36).contains(&index) {
                    if !self.move_to(env, Some(index), &mut stack, 36, 45, false) {
                        return None;
                    }
                } else if (36..45).contains(&index) {
                    if !self.move_to(env, Some(index), &mut stack, 9, 36, false) {
                        return None;
                    }
                } else if !self.move_to(env, Some(index), &mut stack, 9, 45, false) {
                    return None;
                }
            }
        }
        self.finish_quick_move(env, index, clicked, stack, check_count, after_take)
    }

    fn finish_quick_move(
        &mut self,
        env: &Env,
        index: usize,
        mut clicked: ItemStack,
        mut stack: ItemStack,
        check_count: bool,
        after_take: bool,
    ) -> Option<ItemStack> {
        // The stack was edited in place in its slot.
        let slot = self.state.slots[index];
        if stack.count == 0 {
            self.state.containers[slot.container][slot.index] = None;
            // InventoryMenu: setByPlayer(EMPTY, clicked).
            self.set_by_player_from(env, index, None, Some(&clicked));
        } else {
            self.state.containers[slot.container][slot.index] = Some(stack.clone());
            self.slot_changed(env, index);
        }
        if check_count && stack.count == clicked.count {
            return None;
        }
        if after_take || matches!(self.state.kind, Kind::Inventory | Kind::Crafting) {
            // A merchant, stonecutter or smithing result reports what its move left behind, a
            // crafting result the copy it takes (which Item.onCraftedBy may still change).
            self.take_after_quick_move(env, index, &mut clicked, &mut stack);
        }
        // What the result slot's move left over is dropped.
        if (index == 0 && matches!(self.state.kind, Kind::Inventory | Kind::Crafting))
            || (index == 1 && self.state.kind == Kind::Stonecutter)
            || (matches!(self.state.kind, Kind::Mod(_))
                && self.state.slots[index].role == Role::CraftResult)
        {
            self.drop_stack(&stack, env.items);
        }
        Some(clicked)
    }
}

/// `MerchantMenu` and `MerchantContainer` over the menu's `merchant` session.
impl Menu {
    /// `MerchantContainer.updateSellItem`.
    pub fn merchant_update(&mut self, env: &Env) {
        self.merchant_update_result(env.items);
    }

    /// Update a detached menu after an accepted edit of its live offer.
    pub fn merchant_update_result(&mut self, items: &Items) {
        self.merchant_update_with_feedback(items, crate::merchant::TradeFeedback::Current);
    }

    pub fn merchant_update_with_feedback(
        &mut self,
        items: &Items,
        feedback: crate::merchant::TradeFeedback,
    ) {
        self.touch();
        if let Some(merchant) = &mut self.state.merchant {
            let [payment, result] = self
                .state
                .containers
                .get_disjoint_mut([MAIN, RESULT])
                .expect("separate merchant inputs and result");
            if let Some((yes, pitch)) = merchant.update_sell_item(
                items,
                payment[0].as_ref(),
                payment[1].as_ref(),
                &mut result[0],
                feedback,
            ) {
                merchant
                    .events
                    .push(crate::merchant::Event::TradeUpdated(yes, pitch));
            }
        }
    }
    /// `MerchantResultSlot.checkTakeAchievements`: award the tallied count, then reset it.
    fn merchant_check_take_achievements(&mut self, stack: &ItemStack) {
        self.touch();
        let n = std::mem::take(&mut self.state.merchant_remove_count);
        self.crafted_by(stack, n);
    }

    /// `MerchantResultSlot.onTake`: the payment is taken and the merchant
    /// told (`notifyTrade`).
    fn merchant_take(&mut self, env: &Env) {
        self.touch();
        let Some(m) = self.state.merchant.as_mut() else {
            return;
        };
        let Some(i) = m.active else { return };
        let Some(offer) = m.offers.get(i).cloned() else {
            return;
        };
        let (mut a, mut b) = (
            self.state.containers[MAIN][0].take(),
            self.state.containers[MAIN][1].take(),
        );
        let took = offer.take(env.items, &mut a, &mut b) || offer.take(env.items, &mut b, &mut a);
        // The payment stacks shrink in place, before the merchant hears of it.
        self.state.containers[MAIN][0] = a;
        self.state.containers[MAIN][1] = b;
        if took {
            let prices = m.notify_trade(i, env.trade, env.items);
            if prices {
                self.merchant_update(env);
            }
            // setItem(0, buyA), setItem(1, buyB).
            self.merchant_update(env);
            self.merchant_update(env);
        }
    }
    /// `MerchantMenu.tryMoveItems` (the select trade packet, with the
    /// selection hint set first).
    pub fn merchant_select(&mut self, env: &Env, index: usize) {
        self.touch();
        let Some(m) = self.state.merchant.as_mut() else {
            return;
        };
        // MerchantContainer.setSelectionHint updates the result.
        m.selection_hint = index;
        let count = m.offers.len();
        self.merchant_update(env);
        if index >= count {
            return;
        }
        for slot in [0usize, 1] {
            if let Some(mut old) = self.state.containers[MAIN][slot].clone() {
                if !self.move_to(env, None, &mut old, 3, 39, true) {
                    return;
                }
                self.state.containers[MAIN][slot] = (old.count > 0).then_some(old);
                self.put(env, slot, self.state.containers[MAIN][slot].clone());
            }
        }
        if self.state.containers[MAIN][0].is_some() || self.state.containers[MAIN][1].is_some() {
            return;
        }
        let Some(offer) = self
            .merchant
            .as_ref()
            .and_then(|m| m.offers.get(index))
            .cloned()
        else {
            return;
        };
        self.merchant_fill(env, 0, &offer.base_cost_a);
        if let Some(b) = &offer.cost_b {
            self.merchant_fill(env, 1, b);
        }
    }
    /// `MerchantMenu.moveFromInventoryToPaymentSlot`.
    fn merchant_fill(&mut self, env: &Env, payment: usize, cost: &crate::merchant::ItemCost) {
        self.touch();
        for i in 3..39 {
            let Some(have) = self.slot(i).cloned() else {
                continue;
            };
            if !cost.test(&have) {
                continue;
            }
            let current = self.state.containers[MAIN][payment].clone();
            if current
                .as_ref()
                .is_some_and(|c| !c.same_item_same_components(&have))
            {
                continue;
            }
            let max = Self::max_stack(env, &have);
            let current_count = current.as_ref().map_or(0, |c| c.count);
            let moved = max.saturating_sub(current_count).min(have.count);
            let payment_stack = ItemStack {
                count: current_count + moved,
                ..have.clone()
            };
            let slot = self.state.slots[i];
            let mut left = have;
            left.count -= moved;
            self.state.containers[slot.container][slot.index] = (left.count > 0).then_some(left);
            self.put(env, payment, Some(payment_stack.clone()));
            if payment_stack.count >= max {
                break;
            }
        }
    }
    /// `MerchantMenu.removed` for a living player: the payment goes back to
    /// the inventory (`placeItemBackInInventory`), or is dropped.
    pub fn merchant_close(&mut self, env: &Env) {
        self.merchant_removed(env, false);
    }

    /// MerchantMenu.removed: payment drops when the player has already been removed.
    /// The caller handles the removed player's cursor through AbstractContainerMenu.removed.
    pub fn merchant_removed(&mut self, env: &Env, drop_payment: bool) {
        self.touch();
        if let Some(mut carried) = self.state.carried.take() {
            self.place_back(env, &mut carried);
        }
        for slot in [0usize, 1] {
            if let Some(mut stack) = self.state.containers[MAIN][slot].take() {
                if drop_payment {
                    self.dropped.push(stack);
                } else {
                    self.place_back(env, &mut stack);
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{banner::Banners, brewing::Brewing, crafting::Recipes};
    use sunrise_world::loot::PackOverlay;

    pub(crate) fn stack(item: &str, count: u32) -> Option<ItemStack> {
        Some(ItemStack {
            item: format!("minecraft:{item}").into(),
            count,
            damage: 0,
            extra: None,
        })
    }

    /// An item tab's ItemPickerMenu: the 45 display slots show `shown`.
    fn with_picker(shown: &str, test: impl FnOnce(&Env, &mut Menu)) {
        with_menu(Kind::ItemPicker, |env, menu| {
            assert_eq!(menu.slot_count(), 54);
            for i in 0..45 {
                menu.set_slot(env, i, stack(shown, 1));
            }
            test(env, menu);
        });
    }

    /// A menu reads a stack's own `MAX_STACK_SIZE`: a removed component is
    /// vanilla's default of one, so two cannot merge into one slot.
    #[test]
    fn a_removed_max_stack_size_leaves_one_per_slot() {
        with_menu(Kind::Inventory, |env, menu| {
            let removed = || {
                Some(sunrise_core::nbt::Tag::compound(Vec::<(
                    &str,
                    sunrise_core::nbt::Tag,
                )>::new()))
            };
            let mut stone = stack("stone", 1).unwrap();
            stone
                .data_mut()
                .set_other("!minecraft:max_stack_size", removed());
            menu.set_slot(env, 9, Some(stone.clone()));
            menu.carried = Some(stone);
            menu.click(env, 9, 0, Input::Pickup).unwrap();
            // Vanilla leaves both stacks at one; the item's default of 64 no
            // longer lets the click merge them.
            assert_eq!(menu.slot(9).map(|s| s.count), Some(1));
            assert_eq!(menu.carried.as_ref().map(|s| s.count), Some(1));
        });
    }

    /// The game's prepared data: `SUNRISE_TEST_RESOURCES`, else
    /// `.local/sunrise-client` in the checkout.
    pub(crate) fn resource_root() -> std::path::PathBuf {
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/sunrise-client")
                    .join(sunrise_core::VANILLA_VERSION)
            })
    }

    /// A menu over the game's data (`resource_root`); skipped without it.
    pub(crate) fn with_menu(kind: Kind, test: impl FnOnce(&Env, &mut Menu)) {
        let root = resource_root();
        if !root.join("items.json").exists() {
            return;
        }
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items = Items::catalog(&root, &PackOverlay::NONE, &defaults).unwrap();
        let recipes = Recipes::load(
            &root,
            &items,
            &PackOverlay::NONE,
            &crate::test_resources::server_load(),
        )
        .unwrap();
        let brewing = Brewing::load(&root, &items, &PackOverlay::NONE);
        let banners = Banners::load(&root, &defaults);
        let station_recipes = crate::stations::StationRecipes::load(
            &root,
            &items,
            &PackOverlay::NONE,
            &crate::test_resources::server_load(),
        )
        .unwrap();
        let enchantments = crate::enchant::Enchantments::default();
        let names = crate::item_names::Names::default();
        let env = Env {
            crafting_remainder: None,
            player: None,
            player_entity_id: None,
            loop_context: &crate::test_resources::loop_context(LoopSide::Server),
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            limited_crafting: false,
            can_equip: None,
            known_recipes: None,
            slot_rules: None,
            stacked: None,
        };
        let mut menu = Menu::new(kind);
        menu.creative = true;
        test(&env, &mut menu);
    }

    #[test]
    fn a_handled_replacement_goes_through_the_clicked_slots_rules() {
        bare_env(|env| {
            let mut menu = Menu::new(Kind::Grindstone);
            let handled = |clicked| StackedOutcome {
                canceled: true,
                handled: true,
                resume: StackedResume::Done,
                edits: vec![StackedEdit::Clicked(clicked)],
                ..StackedOutcome::default()
            };
            // The result slot's onTake would not run: nothing goes in, nothing is taken out.
            menu.set_slot(env, 2, stack("dirt", 1));
            for clicked in [None, stack("stone", 1)] {
                let outcome = handled(clicked);
                let env = Env {
                    stacked: Some(&outcome),
                    ..*env
                };
                menu.click(&env, 2, 0, Input::Pickup).unwrap();
                assert!(outcome.refused.get());
                assert_eq!(menu.slot(2), stack("dirt", 1).as_ref());
            }
            // An input takes a damageable item the slot's own rules admit, and only that.
            let outcome = handled(stack("dirt", 1));
            let env = Env {
                stacked: Some(&outcome),
                ..*env
            };
            menu.click(&env, 0, 0, Input::Pickup).unwrap();
            assert!(outcome.refused.get());
            assert_eq!(menu.slot(0), None);
        });
    }

    #[test]
    fn a_handled_replacement_cannot_skip_a_potion_slots_take_effects() {
        bare_env(|env| {
            let mut menu = Menu::new(Kind::Brewing);
            menu.brewed_potion_hook = true;
            let potion = stack("potion", 1);
            menu.set_slot(env, 0, potion.clone());
            let answer = StackedOutcome {
                canceled: true,
                handled: true,
                resume: StackedResume::Done,
                edits: vec![
                    StackedEdit::Clicked(None),
                    StackedEdit::Carried(potion.clone()),
                ],
                ..StackedOutcome::default()
            };
            let handled = Env {
                stacked: Some(&answer),
                ..*env
            };
            menu.click(&handled, 0, 0, Input::Pickup).unwrap();
            assert_eq!(menu.slot(0), potion.as_ref());
            assert_eq!(menu.carried, None);
            assert!(menu.brewed_potion_takes.is_empty());

            menu.click(env, 0, 0, Input::Pickup).unwrap();
            assert_eq!(menu.slot(0), None);
            assert_eq!(menu.carried, potion);
            assert_eq!(menu.brewed_potion_takes, [stack("potion", 1).unwrap()]);
        });
    }

    #[test]
    fn menu_keeps_the_open_window_registry_identity() {
        let mut menu = Menu::new(Kind::Chest(3));
        assert_eq!(menu.registry_id(), None);
        menu.set_registry_id(Some("minecraft:generic_9x3"));
        assert_eq!(menu.registry_id(), Some("minecraft:generic_9x3"));
        menu.set_registry_id(Some("example:chest"));
        assert_eq!(menu.registry_id(), Some("example:chest"));
        menu.set_registry_id(None);
        assert_eq!(menu.registry_id(), None);
    }

    #[test]
    fn inventory_add_respects_component_cap_and_remainder() {
        with_menu(Kind::Inventory, |env, menu| {
            menu.creative = false;
            let mut item = ItemStack::new("minecraft:stone", 15);
            item.data_mut().set_other(
                "minecraft:max_stack_size",
                Some(sunrise_core::nbt::Tag::Int(16)),
            );
            menu.set_player_item(0, Some(item.clone()));
            for i in 1..36 {
                menu.set_player_item(i, stack("minecraft:dirt", 64));
            }
            item.count = 8;
            menu.give(env.items, &mut item);
            assert_eq!(menu.player_item(0).unwrap().count, 16);
            assert_eq!(item.count, 7);
        });
    }

    #[test]
    fn swap_self_does_not_duplicate_and_container_slot_is_distinct() {
        with_menu(Kind::Crafting, |env, menu| {
            menu.set_player_item(0, stack("minecraft:stone", 8));
            menu.click(env, 37, 0, Input::Swap).unwrap();
            assert_eq!(menu.player_item(0).unwrap().count, 8);
            let total: u32 = (0..menu.slot_count())
                .filter_map(|s| menu.slot(s))
                .map(|s| s.count)
                .sum();
            assert_eq!(total, 8);
        });
        with_menu(Kind::Chest(3), |env, menu| {
            menu.set_slot(env, 0, stack("minecraft:dirt", 2));
            menu.set_player_item(0, stack("minecraft:stone", 8));
            menu.click(env, 0, 0, Input::Swap).unwrap();
            assert_eq!(menu.slot(0).unwrap().count, 8);
            assert_eq!(menu.player_item(0).unwrap().count, 2);
        });
    }

    /// `CrafterBlockEntity.fillStackedContents`: a crafter's 3x3 slots are
    /// part of the recipe book's craftable contents.
    #[test]
    fn a_crafters_grid_counts_for_the_recipe_book() {
        with_menu(Kind::CrafterBlock, |env, menu| {
            menu.containers[MAIN][0] = Some(ItemStack::new("minecraft:oak_planks", 4));
            let stacked = menu.stacked_contents(env);
            let planks = crate::crafting::Ingredient::of(vec!["minecraft:oak_planks".to_owned()]);
            assert!(
                stacked.can_craft(&[&planks]),
                "the crafter's own grid stacks"
            );
        });
    }

    /// `BrewingStandMenu.FuelSlot` and `quickMoveStack` use the same component.
    #[test]
    fn brewing_fuel_slot_and_quick_move_use_stack_components() {
        let items = Items::for_tests(Default::default());
        let root = std::path::Path::new("");
        let recipes = Recipes::empty();
        let brewing = Brewing::load(root, &items, &PackOverlay::NONE);
        let banners = Banners::default();
        let station_recipes = crate::stations::StationRecipes::default();
        let enchantments = crate::enchant::Enchantments::default();
        let names = crate::item_names::Names::default();
        let env = Env {
            crafting_remainder: None,
            player: None,
            player_entity_id: None,
            loop_context: &crate::test_resources::loop_context(LoopSide::Server),
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            limited_crafting: false,
            can_equip: None,
            known_recipes: None,
            slot_rules: None,
            stacked: None,
        };
        let mut fuel = ItemStack::new("minecraft:stone", 2);
        fuel.data_mut().set_loot_component(
            "minecraft:brewing_fuel",
            &Some(
                sunrise_core::json_nbt::json_to_tag(
                    &serde_json::json!({"uses": 7, "speed_multiplier": 2.5}),
                )
                .unwrap(),
            ),
        );
        let mut menu = Menu::new(Kind::Brewing);
        assert!(menu.may_place(&env, 4, &fuel));
        menu.set_slot(&env, 5, Some(fuel));
        assert_eq!(menu.quick_move_landing(&env, 5), vec![4]);
        let ordinary = ItemStack::new("minecraft:stone", 2);
        assert!(!menu.may_place(&env, 4, &ordinary));
        menu.set_slot(&env, 5, Some(ordinary));
        assert!(!menu.quick_move_landing(&env, 5).contains(&4));
    }

    /// MerchantResultSlot's removal tally and quick-craft check reset before onTake.
    #[test]
    fn merchant_takes_record_crafted_counts_once() {
        let items = Items::for_tests(Default::default());
        let recipes = Recipes::empty();
        let brewing = Brewing::load(std::path::Path::new(""), &items, &PackOverlay::NONE);
        let banners = Banners::default();
        let station_recipes = crate::stations::StationRecipes::default();
        let enchantments = crate::enchant::Enchantments::default();
        let names = crate::item_names::Names::default();
        let env = Env {
            crafting_remainder: None,
            player: None,
            player_entity_id: None,
            loop_context: &crate::test_resources::loop_context(LoopSide::Server),
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            limited_crafting: false,
            can_equip: None,
            known_recipes: None,
            slot_rules: None,
            stacked: None,
        };
        for (input, payment) in [
            (Input::Pickup, 8),
            (Input::QuickMove, 8),
            (Input::Pickup, 3),
        ] {
            let offer = crate::merchant::MerchantOffer {
                base_cost_a: crate::merchant::ItemCost {
                    stack: ItemStack::new("minecraft:emerald", 4),
                },
                cost_b: None,
                result: ItemStack::new("minecraft:glass", 3),
                uses: 0,
                max_uses: 100,
                reward_exp: false,
                special_price_diff: 0,
                demand: 0,
                price_multiplier: 0.0,
                xp: 1,
            };
            let mut menu = Menu::new(Kind::Merchant);
            menu.merchant = Some(Box::new(crate::merchant::Session::new(vec![offer], true)));
            menu.set_slot(&env, 0, stack("emerald", payment));
            assert_eq!(menu.slot(2).map(|s| s.count), (payment >= 4).then_some(3));
            assert!(menu.crafted_counts.is_empty(), "a preview is not a take");
            assert_eq!(menu.merchant.as_ref().unwrap().offers[0].uses, 0);
            menu.click(&env, 2, 0, input).unwrap();
            if payment < 4 {
                assert!(menu.crafted_counts.is_empty());
                assert_eq!(menu.slot(0).map(|s| s.count), Some(3));
                assert!(menu.carried.is_none());
                assert_eq!(menu.merchant.as_ref().unwrap().offers[0].uses, 0);
                continue;
            }
            if input == Input::Pickup {
                assert_eq!(menu.carried.as_ref().map(|s| s.count), Some(3));
                menu.click(&env, 31, 0, Input::Pickup).unwrap();
                menu.click(&env, 2, 0, Input::Pickup).unwrap();
            }
            let mut expected = Vec::new();
            for _ in 0..2 {
                expected.push((Arc::<str>::from("minecraft:glass"), 3));
                if input == Input::QuickMove {
                    expected.push((Arc::<str>::from("minecraft:air"), 0));
                }
            }
            assert_eq!(menu.crafted_counts, expected);
            assert_eq!(menu.merchant_remove_count, 0);
            assert_eq!(menu.merchant.as_ref().unwrap().offers[0].uses, 2);
            assert!(menu.slot(0).is_none());
            assert!(menu.slot(2).is_none());
        }
    }

    /// The Mouse Tweaks wheel's targets: vanilla's routing of one item.
    #[test]
    fn quick_move_landing_routes_a_furnace_like_vanilla() {
        with_menu(Kind::Furnace("smelting"), |env, menu| {
            menu.creative = false;
            // 0 input, 1 fuel, 2 output, 3..30 inventory, 30..39 hotbar.
            let mut landing = |item, count, from| {
                menu.set_slot(env, from, stack(item, count));
                let to = menu.quick_move_landing(env, from);
                menu.set_slot(env, from, None);
                for &t in &to {
                    menu.set_slot(env, t, None);
                }
                to
            };
            assert_eq!(landing("raw_iron", 5, 30), [0]);
            assert_eq!(landing("coal", 5, 30), [1]);
            // Neither: between the hotbar and the inventory, as a shift-click.
            assert_eq!(landing("dirt", 5, 30), [3]);
            assert_eq!(landing("dirt", 5, 3), [30]);
            // Out of the fuel slot: moveItemStackTo(3, 39, false).
            assert_eq!(landing("coal", 5, 1), [3]);
        });
    }

    struct AppleSets;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for AppleSets {
        fn recipe_property_sets(
            &self,
            _: &dyn mod_api::DataLoadContext,
            event: &mut mod_api::RecipePropertySetsEvent<'_>,
        ) {
            for set in ["furnace_input", "brewing_reagent", "smithing_addition"] {
                event.register(mod_api::RecipePropertyAddition::new(
                    set,
                    vec!["minecraft:apple".to_owned()],
                ));
            }
        }
    }

    /// What a mod adds to the recipe property sets is what the furnace's
    /// shift-click, the brewing stand's reagent slot and the smithing
    /// table's addition slot take, against the real recipes: with no mod
    /// an apple goes to none of them.
    #[test]
    fn mod_additions_to_the_property_sets_reach_every_reader() {
        let check = |added: bool| {
            with_menu(Kind::Furnace("smelting"), |env, menu| {
                menu.creative = false;
                menu.set_slot(env, 30, stack("apple", 5));
                let to = menu.quick_move_landing(env, 30);
                assert_eq!(to, if added { vec![0] } else { vec![3] });
                let apple = stack("apple", 1).unwrap();
                let root = resource_root();
                let defaults = sunrise_world::item_defaults::read(&root).unwrap();
                let stations = crate::stations::StationData::load(
                    &root,
                    env.items,
                    &PackOverlay::NONE,
                    &defaults,
                    &crate::test_resources::server_load(),
                )
                .unwrap();
                assert_eq!(stations.brewing.is_reagent(&apple), added);
                assert_eq!(
                    env.station_recipes
                        .smithing_accepts(env.loop_context, 2, &apple),
                    added
                );
                // Vanilla's own entries stay.
                assert!(
                    stations
                        .brewing
                        .is_reagent(&stack("nether_wart", 1).unwrap())
                );
            })
        };
        check(false);
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("apples", Box::new(AppleSets))].into(),
            || check(true),
        );
    }

    /// `FurnaceResultSlot.checkTakeAchievements` (vanilla + NeoForge
    /// `FurnaceResultSlot.java.patch`): on the server side, every take marks
    /// `furnace_awarded` (including hotbar swap where `removeCount == 0` and
    /// when no mod listens); on both logical sides, `item_smelted` records
    /// nonzero `removeCount` with a non-empty stack when a mod listens (so
    /// shift-click fires once from `onQuickCraft`, never a second time with 0
    /// from `onTake`, and hotbar swap never fires `item_smelted`). Client-side
    /// prediction never marks `furnace_awarded`.
    #[test]
    fn a_furnace_take_keeps_what_check_take_achievements_passes() {
        bare_env(|env| {
            let furnace = |hook: bool| {
                let mut menu = Menu::new(Kind::Furnace("smelting"));
                menu.item_smelted_hook = hook;
                menu.set_slot(env, 2, stack("iron_ingot", 64));
                menu
            };

            // Full left-click pickup.
            let mut menu = furnace(true);
            menu.click(env, 2, 0, Input::Pickup).unwrap();
            assert!(menu.furnace_awarded);
            assert_eq!(menu.item_smelted, [(stack("iron_ingot", 64).unwrap(), 64)]);

            // Right-click pickup takes half (32 of 64).
            let mut menu = furnace(true);
            menu.click(env, 2, 1, Input::Pickup).unwrap();
            assert!(menu.furnace_awarded);
            assert_eq!(menu.item_smelted, [(stack("iron_ingot", 32).unwrap(), 32)]);

            // Partial shift-click: inventory full of dirt except one stack with room for 2.
            // Fires once with the pre-move stack and amount_removed == 2 (not a second 0 call).
            let mut menu = furnace(true);
            for s in 3..39 {
                menu.set_slot(env, s, stack("dirt", 64));
            }
            menu.set_slot(env, 3, stack("iron_ingot", 62));
            menu.click(env, 2, 0, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(2), stack("iron_ingot", 62).as_ref());
            assert!(menu.furnace_awarded);
            assert_eq!(menu.item_smelted, [(stack("iron_ingot", 64).unwrap(), 2)]);

            // Full shift-click: fires once with all 64 removed.
            let mut menu = furnace(true);
            menu.click(env, 2, 0, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(2), None);
            assert!(menu.furnace_awarded);
            assert_eq!(menu.item_smelted, [(stack("iron_ingot", 64).unwrap(), 64)]);

            // Hotbar swap: removeCount == 0 awards XP/recipes without firing item_smelted.
            let mut menu = furnace(true);
            menu.click(env, 2, 0, Input::Swap).unwrap();
            assert_eq!(menu.slot(2), None);
            assert!(menu.furnace_awarded);
            assert!(menu.item_smelted.is_empty());

            // Throw (Q key, button 0 throws 1 item).
            let mut menu = furnace(true);
            menu.click(env, 2, 0, Input::Throw).unwrap();
            assert!(menu.furnace_awarded);
            assert_eq!(menu.item_smelted, [(stack("iron_ingot", 1).unwrap(), 1)]);

            // No mod listener: XP/recipes still awarded, no event recorded.
            let mut menu = furnace(false);
            menu.click(env, 2, 0, Input::Pickup).unwrap();
            assert!(menu.furnace_awarded);
            assert!(menu.item_smelted.is_empty());

            // Empty result slot click: neither award nor event.
            let mut menu = Menu::new(Kind::Furnace("smelting"));
            menu.item_smelted_hook = true;
            menu.click(env, 2, 0, Input::Pickup).unwrap();
            assert!(!menu.furnace_awarded);
            assert!(menu.item_smelted.is_empty());
        });

        // Client-side prediction records nonzero takes for item_smelted, never furnace_awarded.
        bare_env_in(
            &crate::test_resources::loop_context(LoopSide::Client),
            |env| {
                let furnace = |hook: bool| {
                    let mut menu = Menu::new(Kind::Furnace("smelting"));
                    menu.item_smelted_hook = hook;
                    menu.set_slot(env, 2, stack("iron_ingot", 64));
                    menu
                };

                let mut menu = furnace(true);
                menu.click(env, 2, 0, Input::Pickup).unwrap();
                assert!(!menu.furnace_awarded);
                assert_eq!(menu.item_smelted, [(stack("iron_ingot", 64).unwrap(), 64)]);

                let mut menu = furnace(true);
                for s in 3..39 {
                    menu.set_slot(env, s, stack("dirt", 64));
                }
                menu.set_slot(env, 3, stack("iron_ingot", 62));
                menu.click(env, 2, 0, Input::QuickMove).unwrap();
                assert!(!menu.furnace_awarded);
                assert_eq!(menu.item_smelted, [(stack("iron_ingot", 64).unwrap(), 2)]);

                let mut menu = furnace(true);
                menu.click(env, 2, 0, Input::Swap).unwrap();
                assert!(!menu.furnace_awarded);
                assert!(menu.item_smelted.is_empty());

                let mut menu = furnace(false);
                menu.click(env, 2, 0, Input::Pickup).unwrap();
                assert!(!menu.furnace_awarded);
                assert!(menu.item_smelted.is_empty());
            },
        );
    }

    #[path = "remainder_tests.rs"]
    mod remainder;

    /// A test `Env` with no vanilla data: the anvil's and grindstone's own
    /// results don't matter where the mods answer.
    pub(crate) fn bare_env(test: impl FnOnce(&Env)) {
        bare_env_in(&crate::test_resources::loop_context(LoopSide::Server), test)
    }

    pub(crate) fn bare_env_in(loop_context: &LoopContext, test: impl FnOnce(&Env)) {
        static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let run = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("sunrise-bare-{}-{run}", std::process::id()));
        std::fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        let items = Items::for_tests(Default::default());
        let recipes = Recipes::load(
            &root,
            &items,
            &PackOverlay::NONE,
            &crate::test_resources::server_load(),
        )
        .unwrap();
        let brewing = Brewing::load(&root, &items, &PackOverlay::NONE);
        let _ = std::fs::remove_dir_all(&root);
        let (banners, station_recipes) = (Default::default(), Default::default());
        let (enchantments, names) = (Default::default(), Default::default());
        let rules = ContextRules(loop_context);
        test(&Env {
            crafting_remainder: None,
            player: Some(7),
            player_entity_id: Some(41),
            loop_context,
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            limited_crafting: false,
            can_equip: None,
            known_recipes: None,
            slot_rules: Some(&rules),
            stacked: None,
        });
    }

    /// The `menu_slot_rules` answers a mod menu `test:rules` gives in the tests: slot 0 takes
    /// a stack on a server only, and holds what the `rules_mod` setting says (7 without it).
    struct ContextRules<'a>(&'a LoopContext);
    impl SlotRules for ContextRules<'_> {
        fn listens(&self, menu: &str) -> bool {
            matches!(menu, "test:rules" | "test:context")
        }
        fn rule(
            &self,
            menu: &str,
            slot: usize,
            _: &ItemStack,
            context: SlotContext<'_>,
            may_place: &mut bool,
            _: &mut bool,
            max_stack: &mut u32,
        ) {
            // `test:context` slot 1 takes a stack only for player 7 at its block, beside an
            // empty slot 0.
            if menu == "test:context" && slot == 1 {
                *may_place = context.player == Some(7)
                    && context.pos == Some([4, 5, 6])
                    && context.slot(mod_api::MenuContainer::Main, 0).is_none();
            }
            if menu == "test:rules" && slot == 0 {
                *may_place = self.0.side() == LoopSide::Server;
                *max_stack = self
                    .0
                    .setting("rules_mod", "stack")
                    .map_or(7, |value| value.parse().unwrap());
            }
        }
    }

    /// `AnvilUpdateEvent` is not asked without a left input. A replacement
    /// result uses the supplied level and material costs when taken.
    #[test]
    fn an_anvil_update_sets_the_result_cost_and_material() {
        bare_env(|env| {
            let mut menu = Menu::new(Kind::Anvil);
            menu.set_slot(env, 1, stack("diamond", 5));
            assert!(menu.anvil_created());
            assert!(menu.anvil_update().is_none());
            menu.set_slot(env, 0, stack("iron_sword", 1));
            assert!(menu.anvil_created());
            assert!(!menu.anvil_created());
            let (inputs, mut result) = menu.anvil_update().unwrap();
            assert_eq!(Some(inputs.left), stack("iron_sword", 1).as_ref());
            assert_eq!(inputs.right, stack("diamond", 5).as_ref());
            assert!(result.result.is_none());
            result.result = stack("netherite_sword", 1);
            result.cost = 7;
            result.repair_item_count_cost = 2;
            menu.anvil_updated(result);
            assert_eq!(
                (menu.slot(2), menu.data[0]),
                (stack("netherite_sword", 1).as_ref(), 7)
            );
            menu.levels = 30;
            menu.click(env, 2, 0, Input::Pickup).unwrap();
            assert_eq!(menu.carried, stack("netherite_sword", 1));
            assert_eq!(
                (menu.slot(0), menu.slot(1)),
                (None, stack("diamond", 3).as_ref())
            );
        });
    }

    #[test]
    fn an_anvil_update_preserves_signed_xp_cost_without_allowing_a_refund() {
        bare_env(|env| {
            for creative in [false, true] {
                for cost in [i32::MIN, -7, 0] {
                    let mut menu = Menu::new(Kind::Anvil);
                    menu.creative = creative;
                    menu.levels = 30;
                    menu.set_slot(env, 0, stack("iron_sword", 1));
                    menu.set_slot(env, 1, stack("diamond", 5));
                    let (_, mut result) = menu.anvil_update().unwrap();
                    result.result = stack("netherite_sword", 1);
                    result.cost = cost;
                    result.repair_item_count_cost = 0;
                    menu.anvil_updated(result);
                    assert_eq!(menu.data[0], cost);
                    assert_eq!(menu.anvil.repair_count, 0);
                    assert_eq!(menu.slot(2), stack("netherite_sword", 1).as_ref());
                    menu.click(env, 2, 0, Input::Pickup).unwrap();
                    assert_eq!(menu.carried, None);
                    assert_eq!(menu.slot(2), stack("netherite_sword", 1).as_ref());
                    assert_eq!(menu.slot(0), stack("iron_sword", 1).as_ref());
                    assert_eq!(menu.slot(1), stack("diamond", 5).as_ref());
                    assert_eq!(menu.level_change, 0);
                }
            }
        });
    }

    /// `AnvilUpdateEvent` on a rename with no right input: the question
    /// borrows the text and carries the result that vanilla calculated.
    #[test]
    fn an_anvil_rename_alone_starts_at_vanillas_result() {
        bare_env(|env| {
            let renamed = || {
                let mut menu = Menu::new(Kind::Anvil);
                menu.set_slot(env, 0, stack("iron_sword", 1));
                menu.anvil_created();
                assert!(menu.rename(env, "Edge"));
                menu
            };
            let mut menu = renamed();
            let vanilla = (menu.slot(2).cloned(), menu.data[0]);
            assert!(vanilla.0.is_some());
            let (inputs, mut result) = menu.anvil_update().unwrap();
            assert_eq!((inputs.right, inputs.name), (None, Some("Edge")));
            assert_eq!((&result.result, result.cost), (&vanilla.0, vanilla.1));
            assert!(result.only_renaming);
            result.cost = 5;
            menu.anvil_updated(result);
            assert_eq!((menu.slot(2).cloned(), menu.data[0]), (vanilla.0, 5));
            assert!(menu.anvil.only_renaming);
        });
    }

    #[test]
    fn an_anvil_rename_uses_vanillas_definition_of_blank() {
        bare_env(|env| {
            let renamed = |name| {
                let mut menu = Menu::new(Kind::Anvil);
                menu.set_slot(env, 0, stack("iron_sword", 1));
                menu.anvil_created();
                assert!(menu.rename(env, name));
                menu
            };
            let menu = renamed("\u{85}");
            assert_eq!(
                menu.slot(2).unwrap().custom_name().as_deref(),
                Some("\u{85}")
            );
            assert_eq!(menu.data[0], 1);

            let menu = renamed("\u{a0}\u{2007}\u{202f}");
            assert!(menu.slot(2).is_none());
            assert_eq!(menu.data[0], 0);
        });
    }

    /// A rename beside a right input the anvil can't use changes neither
    /// the (empty) result nor the cost, but `createResult` ran: the mods
    /// are asked, and their output is the result.
    #[test]
    fn an_anvil_rename_that_changes_no_result_is_asked() {
        bare_env(|env| {
            let mut menu = Menu::new(Kind::Anvil);
            menu.set_slot(env, 0, stack("iron_sword", 1));
            menu.set_slot(env, 1, stack("dirt", 1));
            menu.anvil_created();
            let before = (menu.slot(2).cloned(), menu.data[0]);
            assert!(menu.rename(env, "Edge"));
            assert_eq!((menu.slot(2).cloned(), menu.data[0]), before);
            assert!(menu.anvil_created(), "createResult ran");
            let (inputs, mut result) = menu.anvil_update().unwrap();
            assert_eq!(inputs.name, Some("Edge"));
            result.result = stack("golden_sword", 1);
            menu.anvil_updated(result);
            assert_eq!(menu.slot(2), stack("golden_sword", 1).as_ref());
        });
    }

    /// `GrindstoneEvent`: a placed output with its experience is what the
    /// take reports; the admitted remainder edits set the inputs after the take.
    #[test]
    fn grindstone_events_set_the_result_the_inputs_and_the_experience() {
        bare_env(|env| {
            let mut menu = Menu::new(Kind::Grindstone);
            menu.grind_hook = true;
            menu.set_slot(env, 0, stack("iron_sword", 1));
            menu.set_slot(env, 1, stack("iron_sword", 1));
            menu.grindstone_placed(
                GrindstonePlaceEdits {
                    output: stack("stick", 1),
                    xp: 5,
                },
                false,
            );
            menu.click(env, 2, 0, Input::Pickup).unwrap();
            assert_eq!(menu.carried, stack("stick", 1));
            let take = menu.grind_take.take().unwrap();
            assert_eq!(
                (&take.top, &take.bottom, take.xp),
                (&stack("iron_sword", 1), &stack("iron_sword", 1), 5)
            );
            // The inputs wait for the mods.
            assert_eq!(menu.slot(0), stack("iron_sword", 1).as_ref());
            let edits = GrindstoneTakeEdits {
                new_top: stack("dirt", 1),
                ..GrindstoneTakeEdits::initial(take.xp)
            };
            assert_eq!(menu.grindstone_taken(env, &take, edits), Some(5));
            assert_eq!(
                (menu.slot(0), menu.slot(1)),
                (stack("dirt", 1).as_ref(), None)
            );
            assert_eq!(menu.ground, None);

            let mut menu = Menu::new(Kind::Grindstone);
            menu.set_slot(env, 0, stack("iron_sword", 1));
            menu.load_slot(2, stack("golden_sword", 1));
            assert_eq!(
                menu.grindstone_place(),
                [
                    stack("iron_sword", 1).as_ref(),
                    None,
                    stack("golden_sword", 1).as_ref(),
                ]
            );
            menu.grindstone_placed(
                GrindstonePlaceEdits {
                    output: None,
                    xp: 5,
                },
                false,
            );
            assert_eq!(menu.slot(2), stack("golden_sword", 1).as_ref());
            assert_eq!(menu.state.grind_xp, -1);

            menu.grindstone_placed(
                GrindstonePlaceEdits {
                    output: stack("stick", 1),
                    xp: 5,
                },
                false,
            );
            assert_eq!(menu.slot(2), stack("stick", 1).as_ref());
            assert_eq!(menu.state.grind_xp, 5);

            // Cancellation wins even when a mod supplies an override.
            menu.grindstone_placed(
                GrindstonePlaceEdits {
                    output: stack("stick", 1),
                    xp: 5,
                },
                true,
            );
            assert_eq!(menu.slot(2), None);
            assert_eq!(menu.state.grind_xp, -1);
        });
    }

    #[test]
    fn anvil_and_grindstone_track_results_yields_per_slot_write() {
        bare_env(|env| {
            let enchanted = |count| {
                let mut s = ItemStack::new("minecraft:stick", count);
                s.set_crafting_enchantments(vec![("minecraft:sharpness".into(), 1)]);
                s
            };

            // QuickCraft drag across slots 0 and 1 yields once per slot write on both menus.
            for kind in [Kind::Anvil, Kind::Grindstone] {
                let mut menu = Menu::new(kind);
                menu.track_results(true);
                menu.carried = Some(enchanted(2));
                assert!(
                    menu.start_click(env, -999, 0, Input::QuickCraft)
                        .unwrap()
                        .is_none()
                );
                assert!(
                    menu.start_click(env, 0, 1, Input::QuickCraft)
                        .unwrap()
                        .is_none()
                );
                assert!(
                    menu.start_click(env, 1, 1, Input::QuickCraft)
                        .unwrap()
                        .is_none()
                );

                let mut seen = Vec::new();
                let mut rest = menu.start_click(env, -999, 2, Input::QuickCraft).unwrap();
                loop {
                    let made = match kind {
                        Kind::Anvil => menu.anvil_created(),
                        Kind::Grindstone => menu.grindstone_created(),
                        _ => false,
                    };
                    if made {
                        seen.push((menu.slot(0).map(|s| s.count), menu.slot(1).map(|s| s.count)));
                    }
                    let Some(step) = rest else {
                        break;
                    };
                    rest = menu.resume_click(env, step);
                }
                assert_eq!(
                    seen,
                    vec![
                        (Some(1), None),
                        (Some(1), None),
                        (Some(1), Some(1)),
                        (Some(1), Some(1)),
                    ]
                );
                assert_eq!(menu.carried, None);

                // PickAll uses Slot.safeTake: each emptied slot notifies through
                // removeItem, twice through setByPlayer, and again through onTake.
                menu.carried = Some(enchanted(1));
                let mut pick_seen = Vec::new();
                let mut rest = menu.start_click(env, 3, 0, Input::PickupAll).unwrap();
                loop {
                    let made = match kind {
                        Kind::Anvil => menu.anvil_created(),
                        Kind::Grindstone => menu.grindstone_created(),
                        _ => false,
                    };
                    if made {
                        pick_seen
                            .push((menu.slot(0).map(|s| s.count), menu.slot(1).map(|s| s.count)));
                    }
                    let Some(step) = rest else {
                        break;
                    };
                    rest = menu.resume_click(env, step);
                }
                assert_eq!(
                    pick_seen,
                    vec![
                        (None, Some(1)),
                        (None, Some(1)),
                        (None, Some(1)),
                        (None, Some(1)),
                        (None, None),
                        (None, None),
                        (None, None),
                        (None, None),
                    ]
                );
                assert_eq!(menu.carried.as_ref().map(|s| s.count), Some(3));
            }

            // A full matching pickup still calls Slot.setChanged even when nothing moves.
            let mut grind = Menu::new(Kind::Grindstone);
            grind.set_slot(env, 0, Some(enchanted(64)));
            let _ = grind.grindstone_created();
            grind.carried = Some(enchanted(64));
            grind.click(env, 0, 0, Input::Pickup).unwrap();
            assert!(grind.grindstone_created());
            assert_eq!(grind.slot(0).map(|s| s.count), Some(64));

            // QuickMove completes the original destination pass after the top-up,
            // then delivers the three notifications for setting the empty slot.
            let mut anvil = Menu::new(Kind::Anvil);
            anvil.set_slot(env, 0, Some(ItemStack::new("minecraft:stick", 63)));
            anvil.set_slot(env, 3, Some(ItemStack::new("minecraft:stick", 2)));
            let _ = anvil.anvil_created();
            anvil.track_results(true);
            let mut qm_seen = Vec::new();
            let mut rest = anvil.start_click(env, 3, 0, Input::QuickMove).unwrap();
            loop {
                if anvil.anvil_created() {
                    qm_seen.push((
                        anvil.slot(0).map(|s| s.count),
                        anvil.slot(1).map(|s| s.count),
                    ));
                }
                let Some(step) = rest else {
                    break;
                };
                rest = anvil.resume_click(env, step);
            }
            assert_eq!(
                qm_seen,
                vec![
                    (Some(64), None),
                    (Some(64), Some(1)),
                    (Some(64), Some(1)),
                    (Some(64), Some(1)),
                ]
            );
        });
    }

    #[test]
    fn quick_move_landing_never_fills_a_result() {
        with_menu(Kind::Furnace("smelting"), |env, menu| {
            menu.creative = false;
            menu.set_slot(env, 2, stack("iron_ingot", 5));
            menu.set_slot(env, 30, stack("iron_ingot", 5));
            // An ingot with room in the output still goes elsewhere.
            assert_eq!(menu.quick_move_landing(env, 30), [3]);
            assert_eq!(menu.slot(2).map(|s| s.count), Some(5));
        });
        with_menu(Kind::Furnace("smelting"), |env, menu| {
            menu.creative = false;
            // The whole output moves: into the partial stack, then an empty slot.
            menu.set_slot(env, 2, stack("iron_ingot", 5));
            menu.set_slot(env, 20, stack("iron_ingot", 62));
            assert_eq!(menu.quick_move_landing(env, 2), [20, 38]);
        });
        with_menu(Kind::Furnace("smelting"), |env, menu| {
            menu.creative = false;
            // A full inventory: the output stays, nothing to land on.
            menu.set_slot(env, 2, stack("iron_ingot", 5));
            for i in 3..39 {
                menu.set_slot(env, i, stack("stone", 64));
            }
            assert!(menu.quick_move_landing(env, 2).is_empty());
        });
    }

    #[test]
    fn quick_move_landing_routes_a_crafting_table_like_vanilla() {
        with_menu(Kind::Crafting, |env, menu| {
            menu.creative = false;
            // 0 result, 1..10 grid, 10..37 inventory, 37..46 hotbar.
            menu.set_slot(env, 37, stack("oak_log", 2));
            assert_eq!(menu.quick_move_landing(env, 37), [1]);
        });
        with_menu(Kind::Crafting, |env, menu| {
            menu.creative = false;
            menu.set_slot(env, 1, stack("oak_log", 1));
            menu.settle(env);
            assert_eq!(menu.slot(0).map(|s| s.count), Some(4));
            assert_eq!(menu.quick_move_landing(env, 0), [45]);
        });
    }

    /// A smithing result taken is recorded for `ItemSmithedEvent` while a mod
    /// listens: the inputs before the take used them and the stack taken.
    /// A shift-click records it too (`onQuickCraft`).
    #[test]
    fn taking_a_smithing_result_records_its_inputs() {
        for input in [Input::Pickup, Input::QuickMove] {
            with_menu(Kind::Smithing, |env, menu| {
                menu.creative = false;
                menu.item_smithed_hook = true;
                let inputs = [
                    stack("netherite_upgrade_smithing_template", 1),
                    stack("diamond_sword", 1),
                    stack("netherite_ingot", 2),
                ];
                for (i, input) in inputs.iter().enumerate() {
                    menu.set_slot(env, i, input.clone());
                }
                menu.settle(env);
                menu.click(env, 3, 0, input).unwrap();
                assert_eq!(
                    menu.item_smithed,
                    vec![(inputs, stack("netherite_sword", 1).unwrap())],
                    "{input:?}"
                );
                // One ingot is left, and nothing more is recorded.
                assert_eq!(menu.slot(2).map(|s| s.count), Some(1));
            });
        }
        // With no mod listening nothing is kept.
        with_menu(Kind::Smithing, |env, menu| {
            menu.creative = false;
            menu.set_slot(env, 0, stack("netherite_upgrade_smithing_template", 1));
            menu.set_slot(env, 1, stack("diamond_sword", 1));
            menu.set_slot(env, 2, stack("netherite_ingot", 1));
            menu.settle(env);
            menu.click(env, 3, 0, Input::Pickup).unwrap();
            assert!(menu.item_smithed.is_empty());
            assert!(menu.carried.is_some());
        });
    }

    /// A crafting grid's result taken is recorded for `native_item_crafted`:
    /// the recipe the grid matched and the stack the player took.
    #[test]
    fn taking_a_crafting_result_records_the_crafted_stack() {
        with_menu(Kind::Crafting, |env, menu| {
            menu.creative = false;
            menu.set_slot(env, 1, stack("oak_log", 1));
            menu.settle(env);
            menu.click(env, 0, 0, Input::Pickup).unwrap();
            let mut grid = vec![None; 9];
            grid[0] = stack("oak_log", 1);
            assert_eq!(
                menu.item_crafted,
                vec![(
                    "minecraft:oak_planks".to_owned(),
                    stack("oak_planks", 4).unwrap(),
                    grid
                )]
            );
        });
    }

    /// The inventory's own 2x2 grid is recorded as its 4 cells, the log in
    /// the cell it was in.
    #[test]
    fn an_inventory_craft_records_its_four_grid_cells() {
        with_menu(Kind::Inventory, |env, menu| {
            menu.creative = false;
            menu.set_slot(env, 2, stack("oak_log", 1));
            menu.settle(env);
            menu.click(env, 0, 0, Input::Pickup).unwrap();
            assert_eq!(
                menu.item_crafted,
                vec![(
                    "minecraft:oak_planks".to_owned(),
                    stack("oak_planks", 4).unwrap(),
                    vec![None, stack("oak_log", 1), None, None]
                )]
            );
        });
    }

    /// A shift-click that crafts three times records three takes, each with
    /// the grid as it was before that craft.
    #[test]
    fn a_shift_click_craft_records_each_batch_with_its_grid() {
        with_menu(Kind::Crafting, |env, menu| {
            menu.creative = false;
            menu.set_slot(env, 1, stack("oak_log", 3));
            menu.settle(env);
            menu.click(env, 0, 0, Input::QuickMove).unwrap();
            let logs: Vec<_> = menu
                .item_crafted
                .iter()
                .map(|(_, _, grid)| (grid.len(), grid[0].as_ref().map(|s| s.count)))
                .collect();
            assert_eq!(logs, [(9, Some(3)), (9, Some(2)), (9, Some(1))]);
        });
    }

    /// `StonecutterMenu`'s `lastSoundTime`: a shift-click crafting a whole
    /// stack plays the take sound once, not once per item.
    #[test]
    fn a_stonecutter_shift_click_plays_the_take_sound_once() {
        with_menu(Kind::Stonecutter, |env, menu| {
            menu.creative = false;
            menu.set_slot(env, 0, stack("granite", 60));
            let polished = menu
                .stone
                .recipes
                .iter()
                .position(|&r| {
                    &*env.station_recipes.stonecutting[r].result.item
                        == "minecraft:polished_granite"
                })
                .expect("granite cuts into polished granite");
            assert!(menu.button(env, polished as i32));
            menu.click(env, 1, 0, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(0), None);
            let made: u32 = (2..menu.slot_count())
                .filter_map(|i| menu.slot(i))
                .map(|s| s.count)
                .sum();
            assert_eq!(made, 60);
            assert_eq!(menu.sounds, ["minecraft:ui.stonecutter.take_result"]);
        });
    }

    #[test]
    fn quick_move_landing_routes_plain_containers() {
        for (kind, size) in [
            (Kind::Hopper, 5),
            (Kind::Dispenser, 9),
            (Kind::Chest(3), 27),
        ] {
            with_menu(kind, |env, menu| {
                menu.creative = false;
                let hotbar = size + 27;
                menu.set_slot(env, hotbar, stack("stone", 3));
                assert_eq!(menu.quick_move_landing(env, hotbar), [0]);
            });
            with_menu(kind, |env, menu| {
                menu.creative = false;
                menu.set_slot(env, 0, stack("stone", 3));
                assert_eq!(menu.quick_move_landing(env, 0), [size + 35]);
            });
        }
    }

    #[test]
    fn takes_one_follows_the_slots_rules() {
        with_menu(Kind::Furnace("smelting"), |env, menu| {
            menu.creative = false;
            let (coal, cobble) = (stack("coal", 1).unwrap(), stack("cobblestone", 1).unwrap());
            let bucket = stack("bucket", 1).unwrap();
            assert!(menu.takes_one(env, 0, &cobble), "the input takes anything");
            assert!(menu.takes_one(env, 1, &coal));
            assert!(!menu.takes_one(env, 1, &cobble), "not fuel");
            assert!(!menu.takes_one(env, 2, &coal), "the output");
            menu.set_slot(env, 1, Some(bucket.clone()));
            assert!(
                !menu.takes_one(env, 1, &bucket),
                "one bucket in the fuel slot"
            );
            menu.set_slot(env, 1, stack("coal", 1));
            assert!(!menu.takes_one(env, 1, &bucket), "another item");
            assert!(menu.takes_one(env, 1, &coal));
        });
    }

    /// `HorseInventoryMenu`: a shift-click wears horse armour, then saddles,
    /// then fills the chest; the body slot takes only what the mount may wear.
    #[test]
    fn mount_menu_slots_take_what_the_mount_wears() {
        with_menu(Kind::Mount(5), |env, menu| {
            menu.creative = false;
            menu.mount = Some("minecraft:donkey".into());
            // 2 saddle and body, 15 chest, then the inventory from 17.
            assert_eq!(menu.slot_count(), 2 + 15 + 36);
            let hotbar = 17 + 27;
            menu.set_slot(env, hotbar, stack("iron_horse_armor", 1));
            menu.click(env, hotbar as i32, 0, Input::QuickMove).unwrap();
            // A donkey wears no horse armour: it goes in the chest.
            assert_eq!(menu.slot(1), None);
            assert_eq!(
                menu.slot(2).map(|s| &*s.item),
                Some("minecraft:iron_horse_armor")
            );
            menu.set_slot(env, hotbar, stack("saddle", 1));
            menu.click(env, hotbar as i32, 0, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(0).map(|s| &*s.item), Some("minecraft:saddle"));
            // Back out of the mount: to the inventory, from the hotbar's end.
            menu.click(env, 0, 0, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(0), None);
            assert_eq!(
                menu.slot(17 + 35).map(|s| &*s.item),
                Some("minecraft:saddle")
            );
        });
        with_menu(Kind::Mount(0), |env, menu| {
            menu.creative = false;
            menu.mount = Some("minecraft:horse".into());
            let hotbar = 2 + 27;
            menu.set_slot(env, hotbar, stack("diamond_horse_armor", 1));
            menu.click(env, hotbar as i32, 0, Input::QuickMove).unwrap();
            assert_eq!(
                menu.slot(1).map(|s| &*s.item),
                Some("minecraft:diamond_horse_armor")
            );
            // No chest: dirt goes between the hotbar and the inventory.
            menu.set_slot(env, hotbar, stack("dirt", 3));
            menu.click(env, hotbar as i32, 0, Input::QuickMove).unwrap();
            assert_eq!(
                menu.slot(2).map(|s| (&*s.item, s.count)),
                Some(("minecraft:dirt", 3))
            );
            assert_eq!(menu.slot(hotbar), None);
            // Placing by hand: a saddle doesn't go in the body slot.
            menu.carried = stack("saddle", 1);
            menu.click(env, 1, 0, Input::Pickup).unwrap();
            assert_eq!(
                menu.slot(1).map(|s| &*s.item),
                Some("minecraft:diamond_horse_armor")
            );
            menu.click(env, 0, 0, Input::Pickup).unwrap();
            assert_eq!(menu.slot(0).map(|s| &*s.item), Some("minecraft:saddle"));
        });
    }

    /// `onEquipItem` runs from the armour and off-hand slots' `setByPlayer`
    /// when the item changes: never from `Inventory.setItem` (a swap's source).
    #[test]
    fn armour_and_off_hand_slots_report_what_is_equipped() {
        with_menu(Kind::Inventory, |env, menu| {
            menu.creative = false;
            let name = |e: &[(&str, Option<ItemStack>)]| {
                e.iter()
                    .map(|(slot, s)| format!("{slot}:{}", s.as_ref().map_or("-", |s| &*s.item)))
                    .collect::<Vec<_>>()
            };
            // A shift-click puts the helmet on (InventoryMenu.quickMoveStack).
            menu.set_slot(env, 9, stack("iron_helmet", 1));
            menu.click(env, 9, 0, Input::QuickMove).unwrap();
            assert_eq!(name(&menu.equipped), ["head:minecraft:iron_helmet"]);
            // Taking it off; the same helmet back on is a change both ways.
            menu.equipped.clear();
            menu.click(env, 5, 0, Input::Pickup).unwrap();
            menu.click(env, 5, 0, Input::Pickup).unwrap();
            assert_eq!(
                name(&menu.equipped),
                ["head:-", "head:minecraft:iron_helmet"]
            );
            // F over a hotbar slot: the shield reaches the off hand by
            // Inventory.setItem, so nothing; a number key onto the off-hand
            // slot sets it by player.
            menu.equipped.clear();
            menu.set_slot(env, 36, stack("shield", 1));
            menu.click(env, 36, 40, Input::Swap).unwrap();
            assert_eq!(menu.slot(45).map(|s| &*s.item), Some("minecraft:shield"));
            assert!(menu.equipped.is_empty());
            menu.click(env, 45, 1, Input::Swap).unwrap();
            assert_eq!(name(&menu.equipped), ["offhand:-"]);
            // Other menus have no equipment slots.
            let mut chest = Menu::new(Kind::Chest(1));
            chest.set_slot(env, 0, stack("iron_helmet", 1));
            chest.click(env, 0, 0, Input::QuickMove).unwrap();
            assert!(chest.equipped.is_empty());
        });
    }

    #[test]
    fn slot_capacity_uses_one_answer_for_placement_and_maximum() {
        #[derive(Default)]
        struct Capacity(std::cell::Cell<u32>);
        impl SlotRules for Capacity {
            fn listens(&self, menu: &str) -> bool {
                menu == "minecraft:generic_9x1"
            }

            fn rule(
                &self,
                _: &str,
                _: usize,
                _: &ItemStack,
                _: SlotContext<'_>,
                _: &mut bool,
                _: &mut bool,
                max_stack: &mut u32,
            ) {
                self.0.set(self.0.get() + 1);
                *max_stack = 7;
            }
        }

        bare_env(|env| {
            let capacity = Capacity::default();
            let env = Env {
                slot_rules: Some(&capacity),
                ..*env
            };
            let mut menu = Menu::new(Kind::Chest(1));
            menu.set_registry_id(Some("minecraft:generic_9x1"));
            let stone = ItemStack::new("minecraft:stone", 1);
            assert_eq!(menu.slot_holds(&env, 0, &stone), 7);
            assert_eq!(capacity.0.get(), 1);
        });
    }

    #[test]
    fn slot_rules_are_told_the_player_the_block_and_the_other_slots() {
        let kind = Kind::Mod(Box::leak(Box::new(crate::containers::ModMenuType {
            id: "test:context",
            owner: "test",
            slots: 2,
            player_inventory: true,
            data_slots: 0,
            owned_slots: None,
            results: vec![],
            inputs: Vec::new(),
            links: None,
            crafting: None,
        })));
        let stack = ItemStack::new("minecraft:stone", 1);
        bare_env(|env| {
            let mut menu = Menu::new(kind);
            menu.set_registry_id(Some("test:context"));
            // No block: the rule refuses.
            assert!(!menu.slot_rules(env, 1, &stack).0);
            menu.set_block_pos(Some([4, 5, 6]));
            assert!(menu.slot_rules(env, 1, &stack).0);
            // What slot 0 holds is seen by the ask about slot 1.
            menu.set_slot(env, 0, Some(ItemStack::new("minecraft:dirt", 1)));
            assert!(!menu.slot_rules(env, 1, &stack).0);
            // Another player's menu (player 8) is refused even at the block.
            let other = Env {
                player: Some(8),
                ..*env
            };
            menu.set_slot(env, 0, None);
            assert!(!menu.slot_rules(&other, 1, &stack).0);
        });
    }

    /// A deferred anvil take (`anvil_hook = true`): taking slot 2 captures the
    /// pre-take snapshot in `anvil_craft` and moves the output to the cursor or
    /// inventory without running `anvil_take`. On a cancelled
    /// `AnvilCraftEvent.Pre`, skipping `anvil_take` leaves the output with the
    /// player while inputs, cost and levels remain untouched (matching NeoForge
    /// `AnvilMenu.onTake` returning early).
    #[test]
    fn a_deferred_anvil_craft_skips_on_take_without_undoing_the_move() {
        let root =
            std::env::temp_dir().join(format!("sunrise-anvil-cancel-{}", std::process::id()));
        std::fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        let items = Items::for_tests(Default::default());
        let recipes = Recipes::load(
            &root,
            &items,
            &PackOverlay::NONE,
            &crate::test_resources::server_load(),
        )
        .unwrap();
        let brewing = Brewing::load(&root, &items, &PackOverlay::NONE);
        let _ = std::fs::remove_dir_all(&root);
        let (banners, station_recipes) = (Banners::default(), Default::default());
        let (enchantments, names) = (Default::default(), Default::default());
        let env = &Env {
            crafting_remainder: None,
            player: None,
            player_entity_id: None,
            loop_context: &crate::test_resources::loop_context(LoopSide::Server),
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            limited_crafting: false,
            can_equip: None,
            known_recipes: None,
            slot_rules: None,
            stacked: None,
        };
        // Both a pick-up onto the cursor and a shift-click into the inventory.
        for input in [Input::Pickup, Input::QuickMove] {
            let mut menu = Menu::new(Kind::Anvil);
            menu.creative = false;
            menu.levels = 10;
            menu.anvil_hook = true;
            menu.set_slot(env, 0, stack("iron_sword", 1));
            menu.set_slot(env, 1, stack("iron_ingot", 2));
            menu.set_slot(env, 2, stack("iron_sword", 1));
            menu.data[0] = 3;
            let before = (
                menu.slot(0).cloned(),
                menu.slot(1).cloned(),
                menu.slot(2).cloned(),
            );
            menu.click(env, 2, 0, input).unwrap();
            // The take waits for the mods, and the click moved the result.
            let craft = menu.anvil_craft.take().expect("the take is deferred");
            assert_eq!(craft.left, before.0);
            assert_eq!(craft.right, before.1);
            assert_eq!(craft.output, before.2.clone().unwrap());
            assert_eq!(menu.slot(2), None);
            // Skipping menu.anvil_take (cancelled AnvilCraftEvent.Pre) keeps the
            // moved result with the player and leaves inputs, cost and levels intact.
            assert_eq!(menu.slot(0), before.0.as_ref());
            assert_eq!(menu.slot(1), before.1.as_ref());
            assert_eq!(menu.data[0], 3);
            assert_eq!(menu.level_change, 0);
            assert!(!menu.combined);
            if input == Input::Pickup {
                assert_eq!(menu.carried, before.2);
                assert!((0..36).all(|i| menu.player_item(i).is_none()));
            } else {
                assert_eq!(menu.carried, None);
                assert_eq!(
                    (0..36)
                        .filter(|&i| menu.player_item(i) == before.2.as_ref())
                        .count(),
                    1
                );
            }
            assert!(menu.anvil_craft.is_none());
        }
    }

    /// A mod menu type: three slots, then a result (slot 3).
    fn mod_menu() -> Kind {
        Kind::Mod(Box::leak(Box::new(crate::containers::ModMenuType {
            id: "test:press",
            owner: "test",
            slots: 4,
            player_inventory: true,
            data_slots: 0,
            owned_slots: None,
            results: vec![3],
            inputs: Vec::new(),
            links: None,
            crafting: None,
        })))
    }

    /// A mod menu type's result takes nothing in by a click, a drag, a swap
    /// or a shift-click (`Slot.mayPlace`), not even a top-up of its own item,
    /// and gives what it holds. A slot its screen leaves out takes no click.
    #[test]
    fn a_mod_menu_result_is_take_only() {
        // No game data: an item table of no items, each stacking to 64.
        let root = std::env::temp_dir().join(format!("sunrise-mod-menu-{}", std::process::id()));
        std::fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        let items = Items::for_tests(Default::default());
        let recipes = Recipes::load(
            &root,
            &items,
            &PackOverlay::NONE,
            &crate::test_resources::server_load(),
        )
        .unwrap();
        let brewing = Brewing::load(&root, &items, &PackOverlay::NONE);
        let _ = std::fs::remove_dir_all(&root);
        let (banners, station_recipes) = (Banners::default(), Default::default());
        let (enchantments, names) = (Default::default(), Default::default());
        let env = &Env {
            crafting_remainder: None,
            player: None,
            player_entity_id: None,
            loop_context: &crate::test_resources::loop_context(LoopSide::Server),
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            limited_crafting: false,
            can_equip: None,
            known_recipes: None,
            slot_rules: None,
            stacked: None,
        };
        let menu = &mut Menu::new(mod_menu());
        {
            menu.creative = false;
            let n = |s: Option<&ItemStack>| s.map(|s| s.count);
            // 0..4 the mod's, 4..31 inventory, 31..40 hotbar.
            menu.carried = stack("stick", 4);
            menu.click(env, 3, 0, Input::Pickup).unwrap();
            menu.click(env, 0, 0, Input::Pickup).unwrap();
            assert_eq!((n(menu.slot(3)), n(menu.slot(0))), (None, Some(4)));
            // The carried ingots take the result's one instead of going in.
            menu.put(env, 3, stack("iron_ingot", 1));
            menu.carried = stack("iron_ingot", 2);
            menu.click(env, 3, 0, Input::Pickup).unwrap();
            assert_eq!((n(menu.slot(3)), n(menu.carried.as_ref())), (None, Some(3)));
            // A drag over 1 to 3 passes the result by.
            menu.carried = stack("dirt", 4);
            for (slot, button) in [(-999, 0), (1, 1), (2, 1), (3, 1), (-999, 2)] {
                menu.click(env, slot, button, Input::QuickCraft).unwrap();
            }
            let held = |m: &Menu| [0, 1, 2, 3].map(|s| n(m.slot(s)));
            assert_eq!(held(menu), [Some(4), Some(2), Some(2), None]);
            // A shift-click from the hotbar tops up no result, and a hotbar
            // key puts nothing in it but takes from it into an empty slot.
            menu.put(env, 3, stack("iron_ingot", 1));
            menu.set_player_item(0, stack("iron_ingot", 3));
            menu.click(env, 31, 0, Input::QuickMove).unwrap();
            menu.click(env, 3, 0, Input::Swap).unwrap();
            assert_eq!(
                (n(menu.slot(3)), n(menu.player_item(0))),
                (Some(1), Some(3))
            );
            menu.click(env, 3, 1, Input::Swap).unwrap();
            assert_eq!((n(menu.slot(3)), n(menu.player_item(1))), (None, Some(1)));
            // F takes it into an empty off hand, and a double click collects from it.
            menu.put(env, 3, stack("iron_ingot", 2));
            menu.click(env, 3, 40, Input::Swap).unwrap();
            assert_eq!((n(menu.slot(3)), n(menu.player_item(40))), (None, Some(2)));
            menu.put(env, 3, stack("iron_ingot", 2));
            menu.set_player_item(0, None);
            menu.set_player_item(1, None);
            menu.carried = stack("iron_ingot", 1);
            menu.click(env, 4, 0, Input::PickupAll).unwrap();
            assert_eq!((n(menu.slot(3)), n(menu.carried.as_ref())), (None, Some(3)));
            // A hidden slot takes neither a click nor a shift-click.
            menu.set_player_item(0, stack("iron_ingot", 3));
            menu.put(env, 0, None);
            menu.hide_slot(0);
            menu.carried = stack("stone", 1);
            menu.click(env, 0, 0, Input::Pickup).unwrap();
            menu.click(env, 31, 0, Input::QuickMove).unwrap();
            assert_eq!((n(menu.slot(0)), n(menu.player_item(0))), (None, Some(3)));
        }
    }

    /// A mod menu type laid out as `CraftingMenu`: the result in slot 0, the
    /// grid in 1 to 9, as many container slots as the table's.
    pub(crate) fn mod_crafting_table() -> Kind {
        Kind::Mod(Box::leak(Box::new(crate::containers::ModMenuType {
            id: "test:station",
            owner: "test",
            slots: 10,
            player_inventory: true,
            data_slots: 0,
            owned_slots: None,
            results: vec![],
            inputs: Vec::new(),
            links: None,
            crafting: Some(crate::containers::ModCrafting {
                grid: 1,
                size: 3,
                result: 0,
            }),
        })))
    }

    /// Everything a click leaves: the slots, the cursor, the drops and the
    /// recipes crafted.
    type Seen = (
        Vec<Option<ItemStack>>,
        Option<ItemStack>,
        Vec<ItemStack>,
        Vec<(String, Vec<Option<ItemStack>>)>,
    );
    fn seen(menu: &mut Menu) -> Seen {
        (
            (0..menu.slot_count())
                .map(|s| menu.slot(s).cloned())
                .collect(),
            menu.carried.clone(),
            std::mem::take(&mut menu.dropped),
            std::mem::take(&mut menu.crafted),
        )
    }

    /// A mod menu type's crafting grid crafts as the crafting table does
    /// (`CraftingMenu`, which `tools/test_menu_click_parity.py` pins to
    /// vanilla): the same clicks on both leave the same slots, cursor, drops
    /// and crafted recipes, remainders and shift-crafting included.
    #[test]
    fn a_mod_crafting_grid_crafts_as_the_crafting_table() {
        type Step = (&'static [(usize, &'static str, u32)], i32, i32, Input);
        let steps: [Step; 7] = [
            // Planks over planks: sticks, taken one craft at a time.
            (
                &[(1, "oak_planks", 5), (4, "oak_planks", 3)],
                0,
                0,
                Input::Pickup,
            ),
            (&[], 0, 1, Input::Pickup),
            // A cursor of sticks takes one more craft; then shift-crafting
            // makes as many as the grid allows.
            (&[], 0, 0, Input::QuickMove),
            // Cake: the milk buckets leave their buckets behind.
            (
                &[
                    (1, "milk_bucket", 1),
                    (2, "milk_bucket", 1),
                    (3, "milk_bucket", 1),
                    (4, "sugar", 1),
                    (5, "egg", 1),
                    (6, "sugar", 1),
                    (7, "wheat", 1),
                    (8, "wheat", 1),
                    (9, "wheat", 1),
                ],
                0,
                0,
                Input::QuickMove,
            ),
            // Shift-clicks from the inventory fill the grid first.
            (&[(9 + 10, "oak_log", 3)], 19, 0, Input::QuickMove),
            (&[], 0, 0, Input::QuickMove),
            // A number key takes the result into the hotbar.
            (&[(1, "stone", 3), (4, "stick", 2)], 0, 5, Input::Swap),
        ];
        with_menu(Kind::Crafting, |env, table| {
            let station = &mut Menu::new(mod_crafting_table());
            assert_eq!(station.slot_count(), table.slot_count());
            for (k, (put, slot, button, input)) in steps.into_iter().enumerate() {
                for menu in [&mut *table, &mut *station] {
                    for &(s, item, n) in put {
                        menu.set_slot(env, s, stack(item, n));
                    }
                    menu.click(env, slot, button, input).unwrap();
                }
                let (expected, got) = (seen(table), seen(station));
                assert_eq!(got, expected, "step {k}");
                if k == 0 {
                    assert!(got.1.is_some(), "the sticks were crafted");
                }
                if k == 3 {
                    assert!(got.3.iter().any(|(id, _)| id.ends_with("cake")));
                    assert!(
                        got.0[1]
                            .as_ref()
                            .is_some_and(|b| &*b.item == "minecraft:bucket")
                    );
                }
            }
        });
    }

    mod crafted_stat_tests;
    mod mod_grid_tests;
    mod server_stonecutter_tests;

    /// Extra sources (`native_recipe_sources`) count as more inventory: the
    /// grid fills as the table's does when the player holds what the sources
    /// hold, the inventory gives first, then the sources in order, and a
    /// refused transfer leaves them as they were.
    #[test]
    fn recipe_placement_takes_from_the_sources_after_the_inventory() {
        with_menu(Kind::Crafting, |env, table| {
            let station = &mut Menu::new(mod_crafting_table());
            table.set_player_item(0, stack("oak_planks", 10));
            station.set_player_item(0, stack("oak_planks", 1));
            station.sources = vec![
                vec![None, stack("dirt", 5)],
                vec![stack("oak_planks", 2), stack("oak_planks", 7)],
            ];
            for menu in [&mut *table, &mut *station] {
                menu.place_recipe(env, "minecraft:crafting_table", false, false, None);
            }
            let grid = |m: &Menu| (0..10).map(|s| m.slot(s).cloned()).collect::<Vec<_>>();
            assert_eq!(grid(station), grid(table));
            assert_eq!(station.slot(0), stack("crafting_table", 1).as_ref());
            // 1 from the inventory, then 2 and 1 from the second source.
            assert_eq!(station.player_item(0), None);
            assert_eq!(
                station.sources,
                vec![
                    vec![None, stack("dirt", 5)],
                    vec![None, stack("oak_planks", 6)]
                ]
            );
            // A furnace needs 8 cobblestone, which no one has: nothing moves.
            let before = station.sources.clone();
            let refused = station.transfer_recipe(env, "minecraft:furnace", false);
            assert_eq!(refused, Err(place::TransferError::Missing));
            assert_eq!(station.sources, before);
        });
    }

    /// The block entity holds a crafting grid but never its result: loading
    /// its slots shows what they craft, and the slots it keeps leave the
    /// result out.
    #[test]
    fn a_mod_crafting_result_is_the_menus_own() {
        with_menu(mod_crafting_table(), |env, menu| {
            let mut items = vec![None; 10];
            items[0] = stack("diamond", 1);
            items[1] = stack("oak_log", 1);
            menu.load_container(env, &items, None);
            assert_eq!(menu.slot(0), stack("oak_planks", 4).as_ref());
            let kept = menu.container_items();
            assert_eq!(kept[0], None);
            assert_eq!(kept[1], stack("oak_log", 1));
        });
    }

    #[test]
    fn a_quick_craft_drag_previews_each_slot_and_the_rest_in_hand() {
        with_menu(Kind::Inventory, |env, menu| {
            // 9 + 36 player slots: 9 is the first main inventory slot.
            menu.set_slot(env, 10, stack("stone", 60));
            menu.set_slot(env, 11, stack("dirt", 1));
            menu.carried = stack("stone", 20);
            // shouldAddSlotToQuickCraft: same item or empty, placeable, not the result.
            assert!(menu.quick_craft_accepts(env, 9, 0, 0));
            assert!(menu.quick_craft_accepts(env, 10, 0, 1));
            assert!(!menu.quick_craft_accepts(env, 11, 0, 1));
            assert!(!menu.quick_craft_accepts(env, 0, 0, 1));
            // Stone has no place in an armor slot (5).
            assert!(!menu.quick_craft_accepts(env, 5, 0, 1));
            // A right drag stops once every carried item has a slot.
            assert!(!menu.quick_craft_accepts(env, 12, 1, 20));
            // A left drag over three slots: 6 each, the 60 capped at 64 (yellow), 2 + 4 left.
            assert_eq!(menu.quick_craft_preview(env, 0, &[9]), None);
            let (shown, left) = menu.quick_craft_preview(env, 0, &[9, 10, 12]).unwrap();
            assert_eq!(
                shown,
                vec![Some((6, false)), Some((64, true)), Some((6, false))]
            );
            assert_eq!(left, 20 - 6 - 4 - 6);
            // A right drag: one each.
            let (shown, left) = menu.quick_craft_preview(env, 1, &[9, 10]).unwrap();
            assert_eq!(shown, vec![Some((1, false)), Some((61, false))]);
            assert_eq!(left, 18);
        });
    }

    #[test]
    fn item_picker_quick_move_deletes_a_hotbar_stack() {
        with_picker("stone", |env, menu| {
            menu.set_slot(env, 45, stack("dirt", 12));
            menu.set_slot(env, 46, stack("dirt", 5));
            menu.click(env, 45, 0, Input::QuickMove).unwrap();
            assert_eq!(menu.player_item(0), None);
            // Nothing moves anywhere: the other hotbar stack and the tab stay.
            assert_eq!(menu.player_item(1), stack("dirt", 5).as_ref());
            assert_eq!(menu.slot(0), stack("stone", 1).as_ref());
            assert_eq!(menu.carried, None);
        });
    }

    #[test]
    fn item_picker_drags_and_pickup_all_skip_the_display_slots() {
        with_picker("dirt", |env, menu| {
            // A right-drag over a display slot and two hotbar slots.
            menu.carried = stack("dirt", 10);
            for (slot, header) in [(-999, 0), (3, 1), (45, 1), (47, 1), (-999, 2)] {
                menu.click(env, slot, header | (1 << 2), Input::QuickCraft)
                    .unwrap();
            }
            assert_eq!(menu.player_item(0), stack("dirt", 1).as_ref());
            assert_eq!(menu.player_item(2), stack("dirt", 1).as_ref());
            assert_eq!(menu.slot(3), stack("dirt", 1).as_ref());
            assert_eq!(menu.carried, stack("dirt", 8));
            // A double click gathers from the hotbar only.
            menu.click(env, 48, 0, Input::PickupAll).unwrap();
            assert_eq!(menu.carried, stack("dirt", 10));
            assert_eq!(menu.player_item(0), None);
            assert_eq!(menu.slot(0), stack("dirt", 1).as_ref());
        });
    }

    /// #1737: a Shift+Right-Click quick move in a chest (a double chest's
    /// 54 slots) moves the stack and leaves the menu's state clean, so the
    /// next quick move still moves its stack and the inventory still
    /// answers.
    #[test]
    fn a_chest_quick_move_keeps_the_next_one_working() {
        with_menu(Kind::Chest(6), |env, menu| {
            menu.creative = false;
            // The hotbar holds two stacks; the chest is empty.
            menu.set_slot(env, 81, stack("cobblestone", 64));
            menu.set_slot(env, 82, stack("dirt", 32));
            // Shift+Right-Click on the hotbar: the stack goes into the chest.
            menu.click(env, 81, 1, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(81), None);
            assert_eq!(menu.slot(0).map(|s| s.count), Some(64));
            // Nothing is left on the cursor and no quick craft is running.
            assert_eq!(menu.carried, None);
            assert_eq!(menu.quick_status, 0);
            // The next quick move still works.
            menu.click(env, 82, 1, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(82), None);
            assert_eq!(menu.slot(1).map(|s| s.count), Some(32));
            assert_eq!(menu.carried, None);
            assert_eq!(menu.quick_status, 0);
        });
    }

    /// #6907: a drag in progress is seen from outside the menu, and a
    /// quick move during it only ends the drag (`doClick`'s resetQuickCraft).
    #[test]
    fn a_quick_move_during_a_drag_only_ends_it() {
        with_menu(Kind::Chest(3), |env, menu| {
            menu.creative = false;
            menu.set_slot(env, 27, stack("dirt", 10));
            menu.carried = stack("cobblestone", 64);
            assert!(!menu.quick_crafting());
            // Start a left drag, then drag across chest slot 0.
            menu.click(env, -999, 0, Input::QuickCraft).unwrap();
            menu.click(env, 0, 1, Input::QuickCraft).unwrap();
            assert!(menu.quick_crafting());
            // A shift-click now moves nothing and ends the drag.
            menu.click(env, 27, 0, Input::QuickMove).unwrap();
            assert!(!menu.quick_crafting());
            assert_eq!(menu.slot(27).map(|s| s.count), Some(10));
            assert_eq!(menu.slot(0), None);
        });
    }

    /// #1832: a potion taken from the creative inventory and put in the
    /// hotbar keeps its `PotionContents` (the brew), as vanilla's
    /// `PotionContents.createItemStack` builds it: the item tab's stack is
    /// the real one, and the hotbar slot holds it as it is.
    #[test]
    fn a_creative_potion_keeps_its_contents_in_the_hotbar() {
        let snbt = r#"{components:{"minecraft:potion_contents":{potion:"minecraft:swiftness"}},count:1,id:"minecraft:potion"}"#;
        let potion = crate::item_data::parse_stack(snbt).unwrap().unwrap();
        assert_eq!(potion.potion(), Some("minecraft:swiftness"));
        // The creative screen hands the stack over...
        let mut carried = None;
        click_display(
            &mut carried,
            Some(&potion),
            |_: &ItemStack| 1,
            0,
            Input::Pickup,
        );
        assert_eq!(
            carried.as_ref().and_then(|s| s.potion()),
            Some("minecraft:swiftness")
        );
        // ... and the hotbar slot keeps it: it is not the uncraftable
        // potion, which is a `minecraft:potion` with no contents at all.
        with_menu(Kind::ItemPicker, |env, menu| {
            menu.carried = carried;
            menu.click(env, 45, 0, Input::Pickup).unwrap();
            let in_hand = menu.slot(45).expect("the hotbar slot");
            assert_eq!(in_hand.potion(), Some("minecraft:swiftness"));
            assert_eq!(&*in_hand.item, "minecraft:potion");
        });
    }

    /// `RecipeCraftingHolder.setRecipeUsed`: when `limited_crafting` is false,
    /// crafting works even if the recipe is not known by the player.
    #[test]
    fn limited_crafting_disabled_allows_crafting_unknown_recipe() {
        use crate::crafting::{Ingredient, Recipe, Template};
        let mut recipes = Recipes::empty();
        recipes.crafting.push((
            "oak_planks".into(),
            Recipe::Shapeless {
                ingredients: vec![Ingredient::of(vec!["minecraft:oak_log".into()])],
                result: Template {
                    item: Some("minecraft:oak_planks".into()),
                    count: 4,
                    damage: 0,
                    extra: None,
                    components: None,
                },
            },
        ));
        let items = Items::for_tests(Default::default());
        let brewing = Brewing::load(&std::path::PathBuf::new(), &items, &PackOverlay::NONE);
        let banners = Banners::default();
        let station_recipes = crate::stations::StationRecipes::default();
        let enchantments = crate::enchant::Enchantments::default();
        let names = crate::item_names::Names::default();

        let env = Env {
            crafting_remainder: None,
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            player: None,
            player_entity_id: None,
            loop_context: &crate::test_resources::loop_context(LoopSide::Client),
            limited_crafting: false,
            can_equip: None,
            known_recipes: None,
            slot_rules: None,
            stacked: None,
        };

        let mut menu = Menu::new(Kind::Inventory);
        menu.set_slot(&env, 1, stack("oak_log", 1));
        assert_eq!(
            menu.slot(0).map(|s| (&*s.item, s.count)),
            Some(("minecraft:oak_planks", 4)),
            "when limited_crafting is false, unlearned recipe should craft"
        );
    }

    /// `RecipeCraftingHolder.setRecipeUsed`: when `limited_crafting` is true,
    /// an unlearned recipe is blocked (result slot is None).
    #[test]
    fn limited_crafting_enabled_blocks_unknown_recipe() {
        use crate::crafting::{Ingredient, Recipe, Template};
        let mut recipes = Recipes::empty();
        recipes.crafting.push((
            "oak_planks".into(),
            Recipe::Shapeless {
                ingredients: vec![Ingredient::of(vec!["minecraft:oak_log".into()])],
                result: Template {
                    item: Some("minecraft:oak_planks".into()),
                    count: 4,
                    damage: 0,
                    extra: None,
                    components: None,
                },
            },
        ));
        let items = Items::for_tests(Default::default());
        let brewing = Brewing::load(&std::path::PathBuf::new(), &items, &PackOverlay::NONE);
        let banners = Banners::default();
        let station_recipes = crate::stations::StationRecipes::default();
        let enchantments = crate::enchant::Enchantments::default();
        let names = crate::item_names::Names::default();

        let known = ["minecraft:stick"];
        let env = Env {
            crafting_remainder: None,
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            player: None,
            player_entity_id: None,
            loop_context: &crate::test_resources::loop_context(LoopSide::Client),
            limited_crafting: true,
            can_equip: None,
            known_recipes: Some(&known),
            slot_rules: None,
            stacked: None,
        };

        let mut menu = Menu::new(Kind::Inventory);
        menu.set_slot(&env, 1, stack("oak_log", 1));
        assert_eq!(
            menu.slot(0),
            None,
            "when limited_crafting is true, unknown recipe must produce None in result slot"
        );
    }

    /// `RecipeCraftingHolder.setRecipeUsed`: when `limited_crafting` is true,
    /// a learned recipe (present in `known_recipes`) crafts successfully.
    #[test]
    fn limited_crafting_enabled_allows_known_recipe() {
        use crate::crafting::{Ingredient, Recipe, Template};
        let mut recipes = Recipes::empty();
        recipes.crafting.push((
            "oak_planks".into(),
            Recipe::Shapeless {
                ingredients: vec![Ingredient::of(vec!["minecraft:oak_log".into()])],
                result: Template {
                    item: Some("minecraft:oak_planks".into()),
                    count: 4,
                    damage: 0,
                    extra: None,
                    components: None,
                },
            },
        ));
        let items = Items::for_tests(Default::default());
        let brewing = Brewing::load(&std::path::PathBuf::new(), &items, &PackOverlay::NONE);
        let banners = Banners::default();
        let station_recipes = crate::stations::StationRecipes::default();
        let enchantments = crate::enchant::Enchantments::default();
        let names = crate::item_names::Names::default();

        let known = ["minecraft:oak_planks"];
        let env = Env {
            crafting_remainder: None,
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            player: None,
            player_entity_id: None,
            loop_context: &crate::test_resources::loop_context(LoopSide::Client),
            limited_crafting: true,
            can_equip: None,
            known_recipes: Some(&known),
            slot_rules: None,
            stacked: None,
        };

        let mut menu = Menu::new(Kind::Inventory);
        menu.set_slot(&env, 1, stack("oak_log", 1));
        assert_eq!(
            menu.slot(0).map(|s| (&*s.item, s.count)),
            Some(("minecraft:oak_planks", 4)),
            "when limited_crafting is true, learned recipe in known_recipes must craft"
        );
    }

    /// `RecipeCraftingHolder.setRecipeUsed`: special recipes bypass the `known_recipes` check
    /// (`recipe.value().isSpecial()`) and craft even when limited crafting is enabled.
    #[test]
    fn limited_crafting_enabled_allows_special_recipe() {
        use crate::crafting::{Ingredient, Recipe, Special, Template};
        let mut recipes = Recipes::empty();
        recipes.crafting.push((
            "dye_shulker_box".into(),
            Recipe::Special(Special::Transmute {
                input: Ingredient::of(vec!["minecraft:shulker_box".into()]),
                material: Ingredient::of(vec!["minecraft:red_dye".into()]),
                min: 1,
                max: 1,
                add_count: false,
                result: Template {
                    item: Some("minecraft:red_shulker_box".into()),
                    count: 1,
                    damage: 0,
                    extra: None,
                    components: None,
                },
            }),
        ));
        let items = Items::for_tests(Default::default());
        let brewing = Brewing::load(&std::path::PathBuf::new(), &items, &PackOverlay::NONE);
        let banners = Banners::default();
        let station_recipes = crate::stations::StationRecipes::default();
        let enchantments = crate::enchant::Enchantments::default();
        let names = crate::item_names::Names::default();

        let known: [&str; 0] = [];
        let env = Env {
            crafting_remainder: None,
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            player: None,
            player_entity_id: None,
            loop_context: &crate::test_resources::loop_context(LoopSide::Client),
            limited_crafting: true,
            can_equip: None,
            known_recipes: Some(&known),
            slot_rules: None,
            stacked: None,
        };

        let mut menu = Menu::new(Kind::Inventory);
        menu.set_slot(&env, 1, stack("shulker_box", 1));
        menu.set_slot(&env, 2, stack("red_dye", 1));
        assert_eq!(
            menu.slot(0).map(|s| (&*s.item, s.count)),
            Some(("minecraft:red_shulker_box", 1)),
            "special recipe must craft with limited_crafting: true without being in known_recipes"
        );
    }

    /// `BrewingStandMenu.PotionSlot.onTake`: a pickup and a shift-click on a
    /// bottle slot each queue the taken stack once for the game's hook adapter;
    /// with no listener nothing is kept.
    #[test]
    fn a_bottle_slot_take_records_once_when_a_hook_listens() {
        let root = std::env::temp_dir().join(format!("sunrise-brewed-{}", std::process::id()));
        std::fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        let items = sunrise_items::items::Items::for_tests(Default::default());
        let overlay = &sunrise_world::loot::PackOverlay::NONE;
        let recipes = Recipes::load(
            &root,
            &items,
            overlay,
            &crate::test_resources::server_load(),
        )
        .unwrap();
        let brewing = Brewing::load(&root, &items, overlay);
        let _ = std::fs::remove_dir_all(&root);
        let (banners, station_recipes) = (Default::default(), Default::default());
        let (enchantments, names) = (Default::default(), Default::default());
        let env = &Env {
            crafting_remainder: None,
            player: Some(7),
            player_entity_id: Some(41),
            loop_context: &crate::test_resources::loop_context(LoopSide::Server),
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
            limited_crafting: false,
            can_equip: None,
            known_recipes: None,
            slot_rules: None,
            stacked: None,
        };
        let potion = ItemStack::new("minecraft:potion", 1);
        let mut seen = Vec::new();
        for input in [Input::Pickup, Input::QuickMove] {
            let mut menu = Menu::new(Kind::Brewing);
            menu.set_slot(env, 0, Some(potion.clone()));
            menu.click(env, 0, 0, input).unwrap();
            // Without a listener the take is only the advancement's.
            assert!(menu.brewed_potion_takes.is_empty(), "{input:?}");
            assert_eq!(menu.brewed, std::slice::from_ref(&potion), "{input:?}");

            let mut menu = Menu::new(Kind::Brewing);
            menu.brewed_potion_hook = true;
            menu.set_slot(env, 0, Some(potion.clone()));
            menu.click(env, 0, 0, input).unwrap();
            assert_eq!(
                menu.brewed_potion_takes,
                std::slice::from_ref(&potion),
                "{input:?}"
            );
            for stack in &menu.brewed_potion_takes {
                seen.push(stack.clone());
            }
        }
        assert_eq!(seen, [potion.clone(), potion]);
    }

    /// `Inventory.add` takes its slots from `getSlotWithRemainingSpace` and `getFreeSlot`
    /// ([`slot_with_remaining_space`], [`free_slot`]), and pick-block from
    /// `getSuitableHotbarSlot` and `getFreeSlot`: one dirt at a time lands where the pickers
    /// answer and nowhere else, in vanilla's order (the selected slot, the off hand, the main
    /// slots, then the first empty one).
    #[test]
    fn inventory_add_and_pick_block_land_where_the_slot_pickers_answer() {
        use crate::pick_item::{
            Picked, apply, plan, suitable_hotbar_slot, vanilla_not_replaceable,
        };
        let items = Items::default();
        let mut inventory = vec![stack("stone", 64); 41];
        inventory[36..].fill(None);
        for (slot, have) in [
            (0, stack("stone", 5)),
            (1, None),
            (2, None),
            (3, stack("dirt", 63)),
            (7, stack("dirt", 63)),
            (20, None),
            (OFFHAND, stack("dirt", 62)),
        ] {
            inventory[slot] = have;
        }
        let selected = 3;
        let dirt = stack("dirt", 1).unwrap();
        let mut landed = Vec::new();
        for _ in 0..6 {
            let answer = slot_with_remaining_space(
                &inventory[..36],
                inventory[OFFHAND].as_ref(),
                selected,
                &dirt,
                64,
            )
            .or_else(|| free_slot(&inventory[..36]))
            .unwrap();
            let mut expected = inventory.clone();
            match &mut expected[answer] {
                Some(have) => have.count += 1,
                empty => *empty = Some(dirt.clone()),
            }
            let mut one = dirt.clone();
            assert!(inventory_add(
                &mut inventory,
                selected,
                false,
                &items,
                &mut one
            ));
            assert_eq!((one.count, &inventory), (0, &expected), "into {answer}");
            landed.push(answer);
        }
        assert_eq!(landed, [3, OFFHAND, OFFHAND, 7, 1, 1]);

        // Pick-block with infinite materials: the hotbar slot `getSuitableHotbarSlot` answers
        // takes the item, and what it held moves to the slot `getFreeSlot` answers.
        let mut main: Vec<_> = (0..36)
            .map(|i| if i == 20 { None } else { stack("stone", 5) })
            .collect();
        let mut sword = stack("diamond_sword", 1).unwrap();
        sword.enchant("minecraft:sharpness", 1);
        main[3] = Some(sword);
        let wood = stack("oak_planks", 1).unwrap();
        let mut vanilla = |s: &ItemStack, _: usize| vanilla_not_replaceable(s);
        let hotbar = suitable_hotbar_slot(&main, selected, &mut vanilla);
        let free = free_slot(&main).unwrap();
        assert_eq!(
            (hotbar, free),
            (4, 20),
            "past the enchanted sword; the first empty slot"
        );
        let picked = plan(&main, selected, &wood, true, &mut vanilla);
        assert_eq!(picked, Some(Picked::Add { hotbar }));
        let mut chosen = selected;
        apply(&mut main, &mut chosen, &wood, picked.unwrap());
        assert_eq!(
            (chosen, &main[hotbar], &main[free]),
            (hotbar, &Some(wood), &stack("stone", 5))
        );
    }
    /// A mod's rows that keep hotbar slots 6 to 8 free (`getFreeSlot`,
    /// `getSlotWithRemainingSpace` and `getSuitableHotbarSlot` answering past them, replaced on
    /// this thread through `rule_replacements`) keep a stack out of them wherever a slot is
    /// chosen: the menus' `Inventory.add` passes a stack in slot 6 with room and the selected
    /// slot 7, and a creative pick takes slot 0 and moves its stack to slot 1.
    #[test]
    fn replaced_slot_pickers_keep_added_and_picked_stacks_out_of_locked_slots() {
        use crate::pick_item::{apply, plan};
        use crate::rule_replacements::{FREE_SLOT, HOTBAR_SLOT, REMAINING_SPACE, replaced};
        fn open(i: usize) -> bool {
            !(6..=8).contains(&i)
        }
        fn free(items: &[Option<ItemStack>]) -> Option<usize> {
            (0..items.len()).find(|&i| open(i) && items[i].is_none())
        }
        fn space(
            items: &[Option<ItemStack>],
            offhand: Option<&ItemStack>,
            selected: usize,
            stack: &ItemStack,
            max: u32,
        ) -> Option<usize> {
            let has = |have: Option<&ItemStack>| {
                have.is_some_and(|have| have.same_item_same_components(stack) && have.count < max)
            };
            if open(selected) && has(items[selected].as_ref()) {
                Some(selected)
            } else if has(offhand) {
                Some(OFFHAND)
            } else {
                (0..items.len()).find(|&i| open(i) && has(items[i].as_ref()))
            }
        }
        fn hotbar(_: &[Option<ItemStack>], _: usize) -> usize {
            0
        }
        let locked = |test: &mut dyn FnMut()| {
            replaced(&FREE_SLOT, free, || {
                replaced(&REMAINING_SPACE, space, || {
                    replaced(&HOTBAR_SLOT, hotbar, test)
                })
            })
        };

        let items = Items::default();
        let mut inventory = vec![None; 41];
        inventory[6] = stack("dirt", 10);
        let mut vanilla = inventory.clone();
        let selected = 7;
        let mut one = stack("dirt", 1).unwrap();
        locked(&mut || {
            assert!(inventory_add(
                &mut inventory,
                selected,
                false,
                &items,
                &mut one
            ));
        });
        assert_eq!(
            (&inventory[0], &inventory[6], &inventory[7]),
            (&stack("dirt", 1), &stack("dirt", 10), &None)
        );
        // Without the rows, vanilla's slot: the stack in slot 6 with room.
        let mut one = stack("dirt", 1).unwrap();
        assert!(inventory_add(
            &mut vanilla,
            selected,
            false,
            &items,
            &mut one
        ));
        assert_eq!(vanilla[6], stack("dirt", 11));

        let mut main = vec![None; 36];
        main[0] = stack("stone", 5);
        let wood = stack("oak_planks", 1).unwrap();
        let mut chosen = selected;
        locked(&mut || {
            let picked = plan(&main, selected, &wood, true, &mut |_, _| false).unwrap();
            apply(&mut main, &mut chosen, &wood, picked);
        });
        assert_eq!(
            (chosen, &main[0], &main[1], &main[7]),
            (0, &Some(wood.clone()), &stack("stone", 5), &None)
        );
    }
    #[path = "../dfa_tests.rs"]
    mod dfa;
    #[test]
    fn smithing_transform_overlay_matches_optional_inputs_and_takes_template_result() {
        use crate::enchant::tests::sample;
        use crate::item_names::Names;
        use crate::stations::StationRecipes;
        use serde_json::Value;
        use std::fs;
        let root = std::env::temp_dir().join(format!("sunrise-smithing-{}", std::process::id()));
        fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        fs::write(
            root.join("items.json"),
            r#"{
            "minecraft:stick":{"minecraft:max_stack_size":64},
            "minecraft:diamond":{"minecraft:max_stack_size":64},
            "minecraft:emerald":{"minecraft:max_stack_size":64},
            "minecraft:diamond_sword":{"minecraft:max_stack_size":1,"minecraft:max_damage":1561}
        }"#,
        )
        .unwrap();
        let context = crate::test_resources::server_load();
        let ctx = crate::test_resources::server_loop_context();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items = Items::catalog(&root, &PackOverlay::NONE, &defaults).unwrap();
        let recipe = serde_json::json!({"type":"minecraft:smithing_transform",
            "base":"minecraft:stick", "addition":"minecraft:diamond",
            "result":{"id":"minecraft:emerald","count":3,
                "components":{"minecraft:custom_name":"Upgraded","minecraft:repair_cost":0}}});
        let files = vec![(
            "recipe/test:upgrade".to_owned(),
            vec![serde_json::to_vec(&recipe).unwrap()],
        )];
        let overlay = PackOverlay::new(&files);
        let stations = StationRecipes::load(&root, &items, &overlay, &context).unwrap();
        let e = sample();
        let mut base = ItemStack::new("minecraft:stick", 2);
        base.data_mut()
            .set_loot_name("minecraft:custom_name", &serde_json::json!("Original"));
        base.data_mut().repair_cost = 7;
        base.data_mut().set_loot_component(
            "minecraft:custom_data",
            &Some(sunrise_core::json_nbt::json_to_tag(&serde_json::json!({"kept":1})).unwrap()),
        );
        let addition = ItemStack::new("minecraft:diamond", 2);
        let result = stations
            .smithing_result(ctx, &e, None, Some(&base), Some(&addition))
            .expect("omitted template accepts an empty slot");
        assert_eq!(
            (&*result.item, result.count, result.repair_cost()),
            ("minecraft:emerald", 3, 0)
        );
        assert_eq!(
            result.data().unwrap().custom_name,
            Some(sunrise_core::nbt::Tag::String("Upgraded".into()))
        );
        assert!(
            result
                .data()
                .unwrap()
                .other("minecraft:custom_data")
                .is_some()
        );
        assert!(
            stations
                .smithing_result(ctx, &e, Some(&addition), Some(&base), Some(&addition))
                .is_none()
        );
        assert!(
            stations
                .smithing_result(ctx, &e, None, Some(&base), None)
                .is_none()
        );
        let recipes = Recipes::load(&root, &items, &PackOverlay::NONE, &context).unwrap();
        let brewing = Brewing::load(&root, &items, &PackOverlay::NONE);
        let banners = Banners::load(&root, &defaults);
        let names = Names::default();
        let env = Env {
            crafting_remainder: None,
            player: None,
            player_entity_id: None,
            loop_context: ctx,
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &stations,
            enchantments: &e,
            names: &names,
            trade: None,
            maps: None,
            limited_crafting: false,
            can_equip: None,
            known_recipes: None,
            slot_rules: None,
            stacked: None,
        };
        let mut menu = Menu::new(Kind::Smithing);
        menu.set_slot(&env, 1, Some(base));
        menu.set_slot(&env, 2, Some(addition));
        assert_eq!(menu.slot(3), Some(&result));
        menu.click(&env, 3, 0, Input::Pickup).unwrap();
        assert_eq!(menu.carried.as_ref(), Some(&result));
        assert!(menu.slot(0).is_none());
        assert_eq!(menu.slot(1).unwrap().count, 1);
        assert_eq!(menu.slot(2).unwrap().count, 1);
        assert_eq!(menu.crafted[0].0, "test:upgrade");
        for (result, malformed) in [
            (
                serde_json::json!({"id":"minecraft:emerald","count":0}),
                true,
            ),
            (
                serde_json::json!({"id":"minecraft:emerald","count":100}),
                true,
            ),
            (
                serde_json::json!({"id":"minecraft:diamond_sword","count":2}),
                false,
            ),
            (
                serde_json::json!({"id":"minecraft:emerald","components":{"minecraft:max_damage":10}}),
                false,
            ),
        ] {
            let mut invalid = recipe.clone();
            invalid["result"] = result;
            let files = vec![(
                "recipe/test:invalid".to_owned(),
                vec![serde_json::to_vec(&invalid).unwrap()],
            )];
            let candidate =
                StationRecipes::load(&root, &items, &PackOverlay::new(&files), &context);
            if malformed {
                let error = candidate.err().unwrap().to_string();
                assert!(error.contains("data/test/recipe/invalid.json"), "{error}");
                assert!(
                    error.contains("invalid ingredients or required fields"),
                    "{error}"
                );
            } else {
                assert!(
                    candidate
                        .unwrap()
                        .smithing_result(ctx, &e, None, menu.slot(1), menu.slot(2))
                        .is_none(),
                    "{invalid}"
                );
            }
        }
        for key in ["template", "base", "addition"] {
            let mut invalid = recipe.clone();
            invalid[key] = Value::Null;
            let files = vec![(
                "recipe/test:invalid".to_owned(),
                vec![serde_json::to_vec(&invalid).unwrap()],
            )];
            let error = StationRecipes::load(&root, &items, &PackOverlay::new(&files), &context)
                .err()
                .unwrap()
                .to_string();
            assert!(error.contains("data/test/recipe/invalid.json"), "{error}");
            assert!(error.contains("invalid ingredient: null"), "{error}");
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod storage_tests;
