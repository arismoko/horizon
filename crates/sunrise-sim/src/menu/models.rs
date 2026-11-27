//! Engine menu records and slot access.
use super::{Env, ItemStack};
use mod_api::MenuContainer;
/// Operations on the clicked slot, including its placement and take rules.
pub trait SlotView {
    /// Run against staged owner state; commit all changes only when accepted.
    fn with_staged(&mut self, edit: &mut dyn FnMut(&mut dyn SlotView) -> bool) -> bool;
    fn items(&self) -> &sunrise_items::items::Items;
    fn item(&self) -> Option<ItemStack>;
    fn may_place(&self, stack: &ItemStack) -> bool;
    fn may_pickup(&self) -> bool;
    fn allow_modification(&self) -> bool;
    fn safe_insert(&mut self, stack: Option<ItemStack>, count: u32) -> Option<ItemStack>;
    fn safe_take(&mut self, count: u32, max: u32) -> Option<ItemStack>;
}
/// A pickup click as `item_stacked_on_other` is asked about it: the cursor and the clicked slot's
/// stack as the click finds them, before any listener runs.
#[derive(Clone, Debug, PartialEq)]
pub struct StackedClick {
    pub player: Option<u128>,
    pub player_entity_id: Option<i32>,
    pub slot: usize,
    pub container: usize,
    pub container_slot: usize,
    pub primary: bool,
    pub carried: Option<ItemStack>,
    pub clicked: Option<ItemStack>,
}
/// Which policy governs applying edits from [`StackedOutcome`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StackedEditPolicy {
    /// Global `Hooks::item_stacked_on_other` event: edits are allowed only when `canceled &&
    /// handled`, and result/potion slots cannot be directly modified.
    #[default]
    Global,
    /// `ItemPart::item_stacked_on_other` override phases (`CarriedOnOther` / `OtherStackedOnMe`):
    /// edits may occur on both handled ("stop") and declined ("continue with edits") outcomes, and
    /// taking from or directly editing an occupied output/result slot is allowed inside the
    /// transaction.
    Part,
}

/// Where `pickup` should resume after applying any [`StackedOutcome`] edits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StackedResume {
    /// Run vanilla carried override, then vanilla clicked override, then plain pickup.
    #[default]
    VanillaAll,
    /// Vanilla carried override won the carried phase: run it on the live menu now (skipping
    /// vanilla clicked override and plain pickup when it takes the click).
    VanillaCarried,
    /// The carried phase (carried Part + vanilla carried override) declined: run vanilla clicked
    /// override, then plain pickup.
    VanillaClicked,
    /// Global listener canceled without handling: skip both vanilla overrides and run plain pickup.
    Plain,
    /// Click was handled: stop without running vanilla overrides or plain pickup.
    Done,
}

/// Persistent quick-craft (drag) state of a [`super::Menu`], preserved across clicks by hosted
/// menus that rebuild an engine [`super::Menu`] per packet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuickCraftDrag {
    pub(super) status: i32,
    pub(super) ty: i32,
    pub(super) slots: Vec<usize>,
}

impl QuickCraftDrag {
    /// Whether a drag is currently in progress (`quickcraftStatus != 0`).
    pub fn active(&self) -> bool {
        self.status != 0
    }
}

/// One edit the mods made to a pickup click's cursor or clicked slot, applied in order.
#[derive(Clone, Debug, PartialEq)]
pub enum StackedEdit {
    /// Write the cursor (`SlotAccess.set`).
    Carried(Option<ItemStack>),
    /// Directly write the clicked slot (`Slot.set` / `setByPlayer`): `None` clears the slot
    /// without running `Slot.onTake` side effects.
    Clicked(Option<ItemStack>),
    /// Take this many items from the clicked slot via `Slot.safeTake` / `Slot.onTake`.
    Take(u32),
}

/// What the mods decided about a pickup click (`item_stacked_on_other`), validated by the game
/// before the click.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StackedOutcome {
    pub canceled: bool,
    pub handled: bool,
    pub policy: StackedEditPolicy,
    pub resume: StackedResume,
    /// The listeners' edits to the cursor and the clicked slot, in the order they made them.
    pub edits: Vec<StackedEdit>,
    /// Set when the slot rules refuse an edit or a suspended edit's inputs changed.
    /// The click and its listener writes are then undone.
    pub refused: std::cell::Cell<bool>,
}

/// Slot category for [`super::apply_stacked_outcome`], shared by local/client [`super::Menu`] and
/// hosted LAN [`crate::lan::menu::View`] pickups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackedSlotKind {
    /// Ordinary inventory, container, or equipment slot.
    Ordinary,
    /// Non-interactive result slot (for example `CrafterMenu`'s display result).
    Inert,
    /// Furnace output slot (`Role::Output`): full take runs `safe_take`, partial same-item take
    /// runs `remove` + `on_take`, and occupied direct edits use the item's max stack size.
    Output,
    /// Brewing stand potion slot (`Role::Potion`): global policy refuses edits when held; part
    /// policy runs `safe_take` / partial take and caps occupied direct edits at 1.
    Potion,
    /// Crafting, station, merchant, or mod result slot: global policy refuses edits; part policy
    /// runs `safe_take` on full take and allows occupied direct edits up to the item's max stack.
    Result,
}

/// Slot operations required by [`super::apply_stacked_outcome`] so local/client [`super::Menu`]
/// and hosted LAN [`crate::lan::menu::View`] share one outcome-application implementation.
pub trait StackedSlotTarget {
    fn slot_kind(&self) -> StackedSlotKind;
    fn current_clicked(&self) -> Option<ItemStack>;
    fn may_pickup(&self) -> bool;
    fn may_place(&self, stack: &ItemStack) -> bool;
    fn slot_max(&self, stack: &ItemStack) -> u32;
    fn item_max_stack(&self, stack: &ItemStack) -> u32;
    fn set_carried(&mut self, carried: Option<ItemStack>);
    fn set_clicked(&mut self, clicked: Option<ItemStack>);
    /// Run a compound clicked-slot edit on private state, committing only when it succeeds.
    fn with_staged(&mut self, edit: impl FnOnce(&mut dyn StackedSlotTarget) -> bool) -> bool
    where
        Self: Sized;
    /// Full take from a slot (`Slot.safeTake`, including `onTake` side effects).
    fn safe_take(&mut self, amount: u32, max_amount: u32) -> Option<ItemStack>;
    /// Partial take from an output/potion slot (`Slot.remove` followed by `Slot.onTake`).
    fn take_partial(&mut self, amount: u32) -> Option<ItemStack>;
}
/// Who and where a `menu_slot_rules` ask is about: the player whose menu it is, the menu's
/// block, the container the asked slot indexes, and the menu's and player's live slots as they
/// stand at the time of the ask (read only, borrowed: player inventory `0..36`, armor `36..40`
/// feet-to-head, offhand `40`). None where the menu has no such thing: a client's prediction
/// knows no other player, and a menu opened with no block has no `pos`.
#[derive(Clone, Copy, Default)]
pub struct SlotContext<'a> {
    pub player: Option<u128>,
    pub pos: Option<[i32; 3]>,
    pub container: MenuContainer,
    pub(crate) slots: &'a [Option<ItemStack>],
    pub(crate) slots_second: &'a [Option<ItemStack>],
    pub(crate) linked: Option<super::LinkedSlots<'a>>,
    /// A flattened LAN container stores grid/result values in Main's unused positions.
    /// These positions are empty in the prediction's separate Main container.
    pub(crate) main_crafting: Option<crate::containers::ModCrafting>,
    pub(crate) grid: &'a [Option<ItemStack>],
    pub(crate) result: &'a [Option<ItemStack>],
    pub(crate) player_slots: &'a [Option<ItemStack>],
    pub(crate) player_armor: Option<&'a [Option<ItemStack>; 4]>,
    pub(crate) player_offhand: Option<&'a Option<ItemStack>>,
    /// A hosted rule sees the tentative slot as well as the stack being checked.
    pub(crate) staged_slot: Option<(MenuContainer, usize, Option<&'a ItemStack>)>,
}

/// What a blockless menu reads of a guest's live slots when it asks the slot
/// rules: the identity the parts see and the containers the menu shows.
pub trait MenuGuest {
    fn uuid(&self) -> u128;
    fn native_menu(&self) -> &[Option<ItemStack>];
    fn inventory(&self) -> &[Option<ItemStack>];
    fn armor(&self) -> &[Option<ItemStack>; 4];
    fn offhand(&self) -> &Option<ItemStack>;
}

/// `menu_slot_rules` inside a menu click, which cannot reach the game: the game lends the menu
/// one for the click.
pub trait SlotRules {
    /// Whether a part answers the menu type `menu`; a menu type none answers is never asked.
    fn listens(&self, menu: &str) -> bool;
    /// The slot's own answers arrive in `may_place`, `may_pickup` and `max_stack`; the parts may
    /// change them.
    /// `context` is who and where the ask is about (see [`SlotContext`]).
    fn rule(
        &self,
        menu: &str,
        slot: usize,
        stack: &ItemStack,
        context: SlotContext<'_>,
        may_place: &mut bool,
        may_pickup: &mut bool,
        max_stack: &mut u32,
    );
    /// Whether a part answers `menu_pick_all` for the menu type `menu`.
    fn pick_all_listens(&self, menu: &str) -> bool {
        let _ = menu;
        false
    }
    /// `canTakeItemForPickAll(carried, slot)` of a mod menu type: `allowed` arrives true, and the
    /// parts may refuse the gather the slot's `stack`.
    fn pick_all(
        &self,
        menu: &str,
        slot: usize,
        carried: &ItemStack,
        stack: &ItemStack,
        context: SlotContext<'_>,
        allowed: &mut bool,
    ) {
        let _ = (menu, slot, carried, stack, context, allowed);
    }
}

/// The anvil's inputs while asking about the result vanilla just calculated.
pub struct AnvilUpdateInputs<'a> {
    pub left: &'a ItemStack,
    pub right: Option<&'a ItemStack>,
    pub name: Option<&'a str>,
}
/// An admitted answer to `GrindstoneEvent.OnPlaceItem`: a replacement result
/// and its experience award, or no replacement to keep the vanilla calculation.
pub struct GrindstonePlaceEdits {
    pub output: Option<ItemStack>,
    /// `-1` for the vanilla award, or a nonnegative experience amount.
    pub xp: i32,
}

impl Default for GrindstonePlaceEdits {
    fn default() -> Self {
        Self {
            output: None,
            xp: -1,
        }
    }
}

/// The pre-take inputs and initial experience of a deferred grindstone result take.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingGrindstoneTake {
    pub top: Option<ItemStack>,
    pub bottom: Option<ItemStack>,
    /// `-1` for vanilla's award, or the amount set when the result was placed.
    pub xp: i32,
}

impl PendingGrindstoneTake {
    /// `GrindstoneMenu.getExperienceAmount`, before the server rolls Level.random.
    pub fn vanilla_amount(&self, env: &Env<'_>) -> i32 {
        crate::stations::grindstone_xp_base(
            env.enchantments,
            self.top.as_ref(),
            self.bottom.as_ref(),
        )
    }
}

/// The admitted experience and replacements for a taken grindstone result.
pub struct GrindstoneTakeEdits {
    /// `-1` for the award from the pre-take inputs, or a nonnegative amount.
    pub xp: i32,
    pub new_top: Option<ItemStack>,
    pub new_bottom: Option<ItemStack>,
}

impl GrindstoneTakeEdits {
    /// Finish the take without listener edits: keep the initial award and empty both slots.
    pub fn initial(xp: i32) -> Self {
        Self {
            xp,
            new_top: None,
            new_bottom: None,
        }
    }
}

/// An anvil craft as `AnvilCraftEvent` reports it: the inputs and the result
/// taken, before the crafting changed anything.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct AnvilCraftSnapshot {
    pub left: Option<ItemStack>,
    pub right: Option<ItemStack>,
    pub output: ItemStack,
}
