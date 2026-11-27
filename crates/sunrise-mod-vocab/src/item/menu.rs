//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
/// One click on the local player's open menu, submitted with `MenuRef::click`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub enum MenuClick {
    /// Pick up, place or drop the carried stack; `None` means outside.
    Pickup {
        slot: Option<usize>,
        button: MouseButton,
    },
    /// Shift-click a menu-order slot into the opposite inventory.
    QuickMove { slot: usize },
    /// Exchange a menu-order slot with a hotbar slot or the offhand.
    Swap { slot: usize, with: SwapWith },
    /// Clone a menu-order slot in creative mode.
    Clone { slot: usize },
    /// Drop one item or the whole stack from a menu-order slot.
    Throw { slot: usize, whole_stack: bool },
    /// Distribute the carried stack across menu-order slots as one complete gesture.
    Drag { kind: DragKind, slots: Vec<usize> },
    /// Collect matching stacks into the carried stack.
    PickupAll { slot: usize },
}

/// The mouse button used to pick up or place a stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the two vanilla pickup buttons
#[derive(sunrise_mod_api_macros::Wire)]
pub enum MouseButton {
    Left,
    Right,
}

/// The inventory location exchanged with a menu slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: vanilla swaps target the hotbar or the offhand
#[derive(sunrise_mod_api_macros::Wire)]
pub enum SwapWith {
    /// A hotbar index in 0..=8; other indices refuse the entire batch.
    Hotbar(u8),
    Offhand,
}

/// Whether [`PlayerRef::open_chest_menu`] or [`PlayerRef::open_mod_menu`] first sends the client a
/// close for the menu it replaces, as a Java `MenuProvider` chooses
/// (`shouldTriggerClientSideContainerClosingOnOpen`). Either way the old menu closes on the
/// server, its cursor stack going back to the player's inventory and `player_container_close`
/// firing once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: vanilla's choice is a boolean
#[derive(sunrise_mod_api_macros::Wire)]
pub enum MenuReplace {
    /// `ServerPlayer.closeContainer`, vanilla's default: the client closes the old screen first,
    /// which centres the mouse again.
    CloseFirst,
    /// `ServerPlayer.doCloseContainer`: the new screen takes the old one's place, title and rows
    /// included, and the mouse stays where it is.
    InPlace,
}

/// How a complete drag distributes the carried stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: vanilla has split, one-per-slot and creative-fill drags
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DragKind {
    Split,
    One,
    /// Fill each slot in creative mode only.
    Clone,
}

/// Identity and location of a player pressing a mod container button.
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct MenuButtonContext {
    pub player: u128,
    pub player_entity_id: i32,
    /// The block backing the menu, or None when the menu was opened without a block.
    pub pos: Option<[i32; 3]>,
    pub id: i32,
}

#[cfg(test)]
mod stacked_on_other_tests {
    use super::{ClickAction, StackedEdit, StackedEdits, StackedOnOther, StackedOnOtherDirection};
    use crate::{ItemKey, ItemStack};

    fn dirt(count: u32) -> Option<ItemStack> {
        ItemStack::new(ItemKey(3), count)
    }

    /// `take_clicked` as `Slot.safeTake(count, count, player)` on a slot of 3 dirt.
    fn take(count: u32) -> (Option<ItemStack>, Vec<StackedEdit>) {
        let mut edits = StackedEdits::new(None, dirt(3));
        let mut click = StackedOnOther::new(
            None,
            None,
            0,
            0,
            0,
            ClickAction::Primary,
            StackedOnOtherDirection::CarriedOnOther,
            &mut edits,
        );
        let taken = click.take_clicked(count);
        (taken, edits.into_edits())
    }

    #[test]
    fn a_take_of_nothing_records_nothing_and_a_large_take_takes_what_the_slot_holds() {
        assert_eq!(take(0), (None, vec![]));
        assert_eq!(take(4), (dirt(3), vec![StackedEdit::Take(3)]));
        assert_eq!(take(3), (dirt(3), vec![StackedEdit::Take(3)]));
        assert_eq!(take(2), (dirt(2), vec![StackedEdit::Take(2)]));
    }

    #[test]
    fn a_rollback_restores_the_stacks_handled_and_the_edit_log() {
        let mut edits = StackedEdits::new(None, dirt(3));
        let mut click = StackedOnOther::new(
            None,
            None,
            0,
            0,
            0,
            ClickAction::Primary,
            StackedOnOtherDirection::Event,
            &mut edits,
        );
        click.set_carried(dirt(1));
        let checkpoint = click.checkpoint();
        click.handled = false;
        click.set_carried(None);
        click.set_clicked(None);
        click.rollback(checkpoint);
        assert!(click.handled);
        assert_eq!(click.carried(), dirt(1).as_ref());
        assert_eq!(click.clicked(), dirt(3).as_ref());
        assert_eq!(edits.into_edits(), vec![StackedEdit::Carried(dirt(1))]);
    }
}

#[cfg(test)]
mod menu_button_context_tests {
    use super::MenuButtonContext;
    use crate::wire::{Reader, Writer};

    #[test]
    fn menu_button_context_round_trips_with_and_without_a_block_position() {
        for pos in [None, Some([12, -4, 29])] {
            let context = MenuButtonContext {
                player: 0x1234,
                player_entity_id: 57,
                pos,
                id: 8,
            };
            let mut writer = Writer::new();
            writer.put(&context);
            let decoded = Reader::decode::<MenuButtonContext>(writer.bytes()).unwrap();
            assert_eq!(decoded.player, context.player);
            assert_eq!(decoded.player_entity_id, context.player_entity_id);
            assert_eq!(decoded.pos, context.pos);
            assert_eq!(decoded.id, context.id);
        }
    }
}

/// The logical side on which a menu click runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum LogicalSide {
    Client,
    Server,
}
/// Which phase of a pickup click (`AbstractContainerMenu.tryItemClickBehaviourOverride`) is
/// asking `item_stacked_on_other`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// exhaustive: NeoForge's event, then vanilla's carried-on-other and other-on-me overrides
// no wire: read only from the lent click within one call.
pub enum StackedOnOtherDirection {
    /// The global `Hooks::item_stacked_on_other` event, before either item override.
    #[default]
    Event,
    /// `Item.overrideStackedOnOther`: the carried (cursor) stack's `ItemPart`, asked first.
    CarriedOnOther,
    /// `Item.overrideOtherStackedOnMe`: the clicked slot stack's `ItemPart`, asked second.
    OtherStackedOnMe,
}
/// Vanilla's `ClickAction`: which action a pickup click is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: vanilla's two click actions
// no wire: read only from the lent click within one call.
pub enum ClickAction {
    /// `ClickAction.PRIMARY`: with the default bindings, a left click.
    Primary,
    /// `ClickAction.SECONDARY`: with the default bindings, a right click.
    Secondary,
}
/// One edit a listener made to a pickup click's cursor or clicked slot, in the order it made it.
#[derive(Clone, Debug, PartialEq)]
// hidden: the game's record of a click's edits; mods edit through `StackedOnOther`
// exhaustive: a cursor write, a slot write, or a take from the slot
// no wire: recorded and applied in-process within one click.
#[doc(hidden)]
pub enum StackedEdit {
    /// `SlotAccess.set` on the cursor.
    Carried(Option<ItemStack>),
    /// `Slot.set` on the clicked slot.
    Clicked(Option<ItemStack>),
    /// `Slot.safeTake` of this many items from the clicked slot.
    Take(u32),
}
/// The cursor and the clicked slot as one pickup click's listeners have left them so far, and
/// the edits that put them there, in order. Every phase of the click shares it.
#[derive(Debug, Default)]
// hidden: the game's working stacks for a click; mods edit through `StackedOnOther`
// exhaustive: the game builds it and reads it back
// no wire: lent to listeners within one click.
#[doc(hidden)]
pub struct StackedEdits {
    carried: Option<ItemStack>,
    clicked: Option<ItemStack>,
    edits: Vec<StackedEdit>,
}
impl StackedEdits {
    /// The click's cursor and clicked slot before any listener ran.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(carried: Option<ItemStack>, clicked: Option<ItemStack>) -> Self {
        Self {
            carried,
            clicked,
            edits: Vec::new(),
        }
    }
    /// The edits made since edit `from`.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn since(&self, from: usize) -> &[StackedEdit] {
        &self.edits[from..]
    }
    /// Every edit, in the order the listeners made them.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn into_edits(self) -> Vec<StackedEdit> {
        self.edits
    }
}
/// The working stacks of a lent click at one moment, for the generated dispatch to undo a
/// panicking listener's edits. Only [`StackedOnOther::checkpoint`] makes one.
#[derive(Debug)]
// hidden: the generated dispatch's unwind checkpoint, not part of the mod surface
// exhaustive: its fields are private and only `checkpoint` builds it.
// no wire: lives for one listener call.
#[doc(hidden)]
pub struct StackedCheckpoint {
    carried: Option<ItemStack>,
    clicked: Option<ItemStack>,
    handled: bool,
    len: usize,
}
/// A pickup click on a menu slot, as `item_stacked_on_other` is asked about it, with the cursor
/// and the clicked slot lent for this call (NeoForge's `getCarriedSlotAccess` and `getSlot`).
/// Reads see what earlier phases and this call already changed. Edits are applied to the menu in
/// the order they were made, once the click is decided.
#[derive(Debug)]
#[non_exhaustive]
// no wire: borrows the click's working stacks for the hook call.
pub struct StackedOnOther<'a> {
    /// The clicking player's UUID, when a player is clicking.
    pub player: Option<u128>,
    /// The clicking player's entity id, when a player is clicking.
    pub player_entity_id: Option<i32>,
    /// The clicked slot's menu index.
    pub slot: usize,
    /// Which container the slot is in, as the menu numbers them.
    pub container: usize,
    /// The slot's index in that container.
    pub container_slot: usize,
    /// The click action (`ItemStackedOnOtherEvent.getClickAction`).
    pub action: ClickAction,
    /// Which phase of `tryItemClickBehaviourOverride` is firing: the global event, the carried
    /// stack's `ItemPart` (`CarriedOnOther`), or the clicked stack's `ItemPart`
    /// (`OtherStackedOnMe`).
    pub direction: StackedOnOtherDirection,
    /// `ItemStackedOnOtherEvent.setCancellationResult`, for global listeners: on a cancelled
    /// click, `true` (the start value, as in NeoForge) when the listener's work is all of it,
    /// `false` when plain pickup still runs. An `ItemPart` answers with its `Flow` instead.
    pub handled: bool,
    edits: &'a mut StackedEdits,
}
impl<'a> StackedOnOther<'a> {
    /// The click `slot` (menu index) of `container`'s slot `container_slot`, lending `edits`.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        player: Option<u128>,
        player_entity_id: Option<i32>,
        slot: usize,
        container: usize,
        container_slot: usize,
        action: ClickAction,
        direction: StackedOnOtherDirection,
        edits: &'a mut StackedEdits,
    ) -> Self {
        Self {
            player,
            player_entity_id,
            slot,
            container,
            container_slot,
            action,
            direction,
            handled: true,
            edits,
        }
    }
    /// The working stacks, `handled` and the number of edits, for undoing a panicking
    /// listener's edits.
    // hidden: the generated dispatch's unwind checkpoint, not part of the mod surface
    #[doc(hidden)]
    pub fn checkpoint(&self) -> StackedCheckpoint {
        let edits = &*self.edits;
        StackedCheckpoint {
            carried: edits.carried.clone(),
            clicked: edits.clicked.clone(),
            handled: self.handled,
            len: edits.edits.len(),
        }
    }
    /// Undo the edits made since `checkpoint`. Only a checkpoint taken on this click can be
    /// given back, so it never reaches past the edits the click had then.
    // hidden: the generated dispatch's unwind restore, not part of the mod surface
    #[doc(hidden)]
    pub fn rollback(&mut self, checkpoint: StackedCheckpoint) {
        self.edits.carried = checkpoint.carried;
        self.edits.clicked = checkpoint.clicked;
        self.handled = checkpoint.handled;
        self.edits.edits.truncate(checkpoint.len);
    }
    /// The stack on the cursor, `None` when it is empty (`SlotAccess.get`).
    pub fn carried(&self) -> Option<&ItemStack> {
        self.edits.carried.as_ref()
    }
    /// Put `stack` on the cursor (`SlotAccess.set`). It must be a valid stack: a known item and
    /// a count from 1 to the item's maximum, or the click is refused.
    pub fn set_carried(&mut self, stack: Option<ItemStack>) {
        self.edits.carried = stack.clone();
        self.edits.edits.push(StackedEdit::Carried(stack));
    }
    /// The stack in the clicked slot, `None` when it is empty (`Slot.getItem`).
    pub fn clicked(&self) -> Option<&ItemStack> {
        self.edits.clicked.as_ref()
    }
    /// Write the clicked slot directly (`Slot.set`), without the slot's take side effects:
    /// `None` clears it. The slot's rules still answer: a write they refuse, or an invalid
    /// stack, refuses the click.
    pub fn set_clicked(&mut self, stack: Option<ItemStack>) {
        self.edits.clicked = stack.clone();
        self.edits.edits.push(StackedEdit::Clicked(stack));
    }
    /// Take up to `count` items from the clicked slot through `Slot.safeTake(count, count,
    /// player)`, which runs the slot's take side effects (furnace experience, `recipes_used`,
    /// crafting remainders, take hooks) when the click applies. As vanilla's `Slot.tryRemove`
    /// and `Container.removeItem`, a `count` above what the slot holds takes all of it, and
    /// nothing is taken from an empty slot or for a `count` of 0: then it returns `None` and
    /// records nothing. Only `ItemPart` overrides may take: a global listener's take refuses
    /// the click.
    pub fn take_clicked(&mut self, count: u32) -> Option<ItemStack> {
        let current = self.edits.clicked.take_if(|_| count > 0)?;
        let count = count.min(current.count());
        let split = current.split(count);
        self.edits.clicked = split.remainder;
        self.edits.edits.push(StackedEdit::Take(count));
        split.taken
    }
}
/// Whether a listener keeps the original stack or replaces it.
#[derive(Clone, Debug, Default, PartialEq)]
// exhaustive: a stack is left alone or replaced, nothing else
#[derive(sunrise_mod_api_macros::Wire)]
pub enum StackResult {
    /// Leave the original stack unchanged.
    #[default]
    Keep,
    /// Replace the stack: `None` empties it.
    Set(Option<ItemStack>),
}
/// The inputs and vanilla result of `AnvilMenu.createResult`, borrowed for
/// the `anvil_update` callback. Edits by earlier listeners do not change these values.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal, and a mod's own anvil menu builds one for
// `PlayerRef::anvil_update`
// no wire: borrows the converted stacks and rename text for the hook call.
pub struct AnvilUpdateInputs<'a> {
    /// The stack in the left input slot. The hook is not asked without one.
    pub left: &'a ItemStack,
    /// The stack in the right input slot, if present.
    pub right: Option<&'a ItemStack>,
    /// The rename box's text, `None` if the client sent none.
    pub name: Option<&'a str>,
    /// The result calculated by vanilla, if any.
    pub vanilla_output: Option<&'a ItemStack>,
    /// The level cost calculated by vanilla.
    pub vanilla_xp_cost: i32,
    /// The number of material items vanilla would charge from the right slot.
    pub vanilla_material_cost: i32,
}

/// The shared answer to `anvil_update`. It starts at `Keep` and vanilla's costs.
/// The game applies the final edits after dispatch; cancellation leaves no output
/// and zero costs, while an invalid replacement output is logged and ignored
/// (keeping vanilla's output).
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal
// no wire: staged edits are applied in-process at the end of the hook call.
pub struct AnvilUpdateEdits {
    /// Keep vanilla's output, or set a replacement. `Set(None)` clears it.
    /// A replacement differing from vanilla must be a valid stack; an invalid
    /// stack is logged and ignored, leaving vanilla's output intact.
    pub output: StackResult,
    /// Signed level cost during listener dispatch (`AnvilUpdateEvent.setXpCost`).
    /// Negative values clamp to `0` when applied (`AnvilMenu.setCost`), and a
    /// zero cost prevents taking the result (`AnvilMenu.mayPickup`).
    pub xp_cost: i32,
    /// Units consumed from the right slot. Negative values clamp to zero;
    /// zero or a value exceeding the stack consumes all of it on take.
    pub material_cost: i32,
}

/// A mutable host that can run an anvil menu's listeners on its logical side.
pub trait AnvilUpdateHost {
    /// Borrows the active hook host for the duration of an anvil update.
    fn anvil_update_host(&mut self) -> HostSide<'_>;
}

impl<H: ServerHost> AnvilUpdateHost for H {
    fn anvil_update_host(&mut self) -> HostSide<'_> {
        HostSide::Server(self)
    }
}

impl AnvilUpdateHost for dyn ServerHost + '_ {
    fn anvil_update_host(&mut self) -> HostSide<'_> {
        HostSide::Server(self)
    }
}

impl AnvilUpdateHost for dyn ClientHost + '_ {
    fn anvil_update_host(&mut self) -> HostSide<'_> {
        HostSide::Client(self)
    }
}

impl AnvilUpdateHost for HostSide<'_> {
    fn anvil_update_host(&mut self) -> HostSide<'_> {
        self.reborrow()
    }
}

impl PlayerRef {
    /// Side: both.
    /// Runs every mod's `anvil_update` listeners for a mod's own anvil menu, as NeoForge's
    /// `CommonHooks.onAnvilUpdate` does for a menu that is not vanilla's `AnvilMenu`: an
    /// upgraded or portable anvil computes its own result, then asks this so the other mods'
    /// anvil recipes, cost changes and repair rules work in it too. `pos` is the anvil's block
    /// (any position for one with no block) and `inputs` lends the stacks, the rename text and
    /// the menu's own result and costs, which the listeners see as vanilla's. The listeners run
    /// in their usual order, on this side, for `player`, starting from `Keep` and those costs.
    /// Returns the edits they left (apply the output, the level cost, clamped at 0 as
    /// `AnvilMenu.setCost` does, and the material cost to your menu), or None when a listener
    /// canceled, which clears your result and both costs as it does vanilla's. Also None, with
    /// no listener asked, for a stale player, a client's player other than its local one, and a
    /// call from inside an `anvil_update` listener, which would run them again without bound.
    /// Pass the active server or client hook host, or borrow its `HostSide`.
    pub fn anvil_update(
        self,
        host: &mut (impl AnvilUpdateHost + ?Sized),
        pos: [i32; 3],
        inputs: &AnvilUpdateInputs<'_>,
    ) -> Option<AnvilUpdateEdits> {
        match host.anvil_update_host() {
            HostSide::Server(host) => backend_mut_of(host)?.player_anvil_update(self, pos, inputs),
            HostSide::Client(host) => {
                client_backend_mut_of(host)?.player_anvil_update(self, pos, inputs)
            }
        }
    }
}

#[cfg(test)]
#[test]
fn anvil_update_uses_the_active_sides_backend() {
    struct Stub;
    impl Host for Stub {}
    crate::impl_host_areas!(Stub);
    impl ServerHost for Stub {}
    crate::impl_server_host_areas!(Stub);
    impl ClientHost for Stub {
        fn client_backend_mut(&mut self) -> Option<ClientBackendMutRef<'_>> {
            Some(ClientBackendMutRef::of(self))
        }
    }
    crate::impl_client_host_areas!(Stub);
    impl ClientBackendMut for Stub {}
    crate::impl_client_backend_mut_areas!(Stub, except(player_menus));
    impl prelude::PlayerMenusClientBackendMut for Stub {
        fn player_anvil_update(
            &mut self,
            player: PlayerRef,
            pos: [i32; 3],
            inputs: &AnvilUpdateInputs<'_>,
        ) -> Option<AnvilUpdateEdits> {
            assert_eq!(player.uuid(), 42);
            assert_eq!(pos, [7, 64, 7]);
            Some(AnvilUpdateEdits {
                output: StackResult::Keep,
                xp_cost: inputs.vanilla_xp_cost + 1,
                material_cost: inputs.vanilla_material_cost,
            })
        }
    }

    let player = __private::game::player_ref(EntityRef::new(WorldEpoch::new(1), 7), 42);
    let inputs = AnvilUpdateInputs {
        left: &ItemStack::of(ItemKey(3)),
        right: None,
        name: None,
        vanilla_output: None,
        vanilla_xp_cost: 3,
        vanilla_material_cost: 1,
    };
    let stub = &mut Stub;
    let pos = [7, 64, 7];
    assert_eq!(player.anvil_update(stub, pos, &inputs), None);
    assert_eq!(
        player.anvil_update(stub as &mut dyn ServerHost, pos, &inputs),
        None
    );
    let client: &mut dyn ClientHost = stub;
    assert_eq!(
        player.anvil_update(client, pos, &inputs).unwrap().xp_cost,
        4
    );
    let mut side = HostSide::Client(client);
    assert_eq!(
        player
            .anvil_update(&mut side, pos, &inputs)
            .unwrap()
            .xp_cost,
        4
    );
}

/// The inputs and vanilla result of `GrindstoneMenu.createResult`, borrowed for
/// the `grindstone_place` callback. Edits by earlier listeners do not change these values.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
// no wire: borrows the converted stacks for the hook call.
pub struct GrindstonePlaceInputs<'a> {
    /// The stack in the top input slot, if present.
    pub top: Option<&'a ItemStack>,
    /// The stack in the bottom input slot, if present.
    pub bottom: Option<&'a ItemStack>,
    /// The result calculated by vanilla, if any.
    pub vanilla_output: Option<&'a ItemStack>,
}

/// The shared answer to `grindstone_place`, initially no override and `-1` XP.
/// Cancellation is checked before converting `output`; an invalid replacement is logged
/// and ignored so vanilla's result stays intact.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal
// no wire: staged edits are applied in-process at the end of the hook call.
pub struct GrindstonePlaceEdits {
    /// Leave vanilla's result and experience with `None`, or replace the result with a
    /// valid stack. To clear the result, cancel the hook instead.
    pub output: Option<ItemStack>,
    /// Experience to award when the replacement is taken. Any negative value falls
    /// through to vanilla's award; nonnegative values set the award.
    pub xp: i32,
}
/// The input stacks before the grindstone result was taken, borrowed for the
/// `grindstone_take` callback. Edits by earlier listeners do not change this snapshot.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
// no wire: borrows the converted pre-take stacks for the hook call.
pub struct GrindstoneTakeInputs<'a> {
    /// The stack in the top input slot before the take, if present.
    pub top: Option<&'a ItemStack>,
    /// The stack in the bottom input slot before the take, if present.
    pub bottom: Option<&'a ItemStack>,
}

/// The shared answer to `grindstone_take`. Both remainders start at `None`;
/// cancellation is checked before converting remainders, and an invalid remainder is
/// logged and ignored as an empty slot.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal
// no wire: staged edits are applied in-process at the end of the hook call.
pub struct GrindstoneTakeEdits {
    /// Experience to award, initially the resolved award (`>= 0`, from `grindstone_place`
    /// or vanilla's pre-rolled award). Non-positive values (`<= 0`) spawn no experience orbs.
    pub xp: i32,
    /// A valid replacement for the top input, or `None` to empty the slot.
    pub new_top: Option<ItemStack>,
    /// A valid replacement for the bottom input, or `None` to empty the slot.
    pub new_bottom: Option<ItemStack>,
}
/// The anvil inputs and result before the take. Both `anvil_craft_pre` and
/// `anvil_craft_post` borrow this same snapshot for their callback.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
// no wire: borrows the pre-take snapshot for this hook call.
pub struct AnvilCraftInputs<'a> {
    /// The stack in the left slot before the take, if present.
    pub left: Option<&'a ItemStack>,
    /// The stack in the right slot before the take, if present.
    pub right: Option<&'a ItemStack>,
    /// The result stack that was taken.
    pub output: &'a ItemStack,
}

/// One of a menu's containers: the key a `menu_slot_rules` ask names its slot's container by
/// (`event.container`, with `event.slot` indexing inside it) and [`MenuSlotContext::slot`] reads
/// by. A plain menu has `Main` and `Player`; a mod menu type with a crafting grid keeps its grid
/// slots in `Grid` and its crafted result in `Result` (slot 0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum MenuContainer {
    /// The menu's own slots (a chest's, a machine's).
    #[default]
    Main,
    /// A crafting grid's slots, row by row.
    Grid,
    /// A crafting grid's result slot.
    Result,
    /// The player's inventory, using vanilla `Inventory` indices: `0..36` inventory, `36..40`
    /// armor (feet, legs, chest, head) and `40` offhand.
    Player,
}
/// The menu's player, its block entity and a read-only view of its live slots for one
/// `menu_slot_rules` ask, on either the server or the predicting client. Read only the slots
/// needed to answer the rule; the view cannot outlive the ask.
// no wire: this view borrows the menu's live slots during a click.
#[non_exhaustive]
pub struct MenuSlotContext<'a> {
    /// The UUID of the player whose menu it is; None where no player is known.
    pub player: Option<u128>,
    /// The block entity the menu belongs to; None when the menu is not a block's
    /// (opened by a mod with no block, a vehicle's, the player's own inventory).
    pub pos: Option<[i32; 3]>,
    read: &'a dyn Fn(MenuContainer, usize) -> Option<ItemStack>,
}
impl<'a> MenuSlotContext<'a> {
    /// Lend a read of the menu's live slots, with the player and block position of this ask.
    /// The reader returns `None` for empty, absent or unrepresentable slots.
    pub fn new(
        player: Option<u128>,
        pos: Option<[i32; 3]>,
        read: &'a dyn Fn(MenuContainer, usize) -> Option<ItemStack>,
    ) -> Self {
        Self { player, pos, read }
    }

    /// Read one slot at its backing index, converting just that stack to an API value.
    /// Returns `None` if it is empty, absent or cannot be represented. For `Player`, indices follow
    /// vanilla `Inventory`: `0..36` inventory, `36..40` armor (feet-to-head) and `40` offhand.
    pub fn slot(&self, container: MenuContainer, index: usize) -> Option<ItemStack> {
        (self.read)(container, index)
    }
}

/// One non-result container slot that changed during a menu action, reported to
/// `menu_slot_changed`.
#[derive(Clone, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct MenuSlotChange {
    /// Which container the slot belongs to ([`MenuContainer`]).
    pub container: MenuContainer,
    /// The slot's index inside `container`.
    pub slot: usize,
    /// The stack held before the change (`None` when empty).
    pub before: Option<ItemStack>,
    /// The stack held after the change (`None` when empty).
    pub after: Option<ItemStack>,
}

impl MenuSlotChange {
    /// Creates a slot-change record for `container`'s slot `slot`.
    pub fn new(
        container: MenuContainer,
        slot: usize,
        before: Option<ItemStack>,
        after: Option<ItemStack>,
    ) -> Self {
        Self {
            container,
            slot,
            before,
            after,
        }
    }
}

/// An authoritative container slot write requested by a `menu_slot_changed` listener.
#[derive(Clone, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct MenuSlotWrite {
    /// Which container the slot belongs to ([`MenuContainer`]).
    pub container: MenuContainer,
    /// The slot's index inside `container`.
    pub slot: usize,
    /// The stack to place in the slot (`None` clears it).
    pub stack: Option<ItemStack>,
}

impl MenuSlotWrite {
    /// Creates an authoritative slot write for `container`'s slot `slot`.
    pub fn new(container: MenuContainer, slot: usize, stack: Option<ItemStack>) -> Self {
        Self {
            container,
            slot,
            stack,
        }
    }
}

#[cfg(test)]
mod menu_slot_change_wire_tests {
    use super::{MenuContainer, MenuSlotChange, MenuSlotWrite};
    use crate::wire::{Reader, Writer};
    use crate::{ItemKey, ItemStack};

    #[test]
    fn menu_slot_change_and_write_round_trip_on_wire() {
        let mut requests = sunrise_mod_runtime::registry_keys::Requests::default();
        requests.items.request("example", "example:gear").unwrap();
        let previous = crate::parts::REGISTERING_KEYS
            .with(|keys| keys.replace(Some(std::rc::Rc::new(std::cell::RefCell::new(requests)))));
        let key = ItemKey::from_slot(0).unwrap();
        let before = Some(ItemStack::new(key, 3).unwrap());
        let after = Some(ItemStack::new(key, 1).unwrap());
        let change = MenuSlotChange::new(MenuContainer::Main, 1, before.clone(), after.clone());
        let mut writer = Writer::new();
        writer.put(&change);
        let decoded = Reader::decode::<MenuSlotChange>(writer.bytes()).unwrap();
        assert_eq!(decoded, change);

        for write in [
            MenuSlotWrite::new(MenuContainer::Main, 2, after),
            MenuSlotWrite::new(MenuContainer::Main, 2, None),
        ] {
            let mut writer = Writer::new();
            writer.put(&write);
            let decoded = Reader::decode::<MenuSlotWrite>(writer.bytes()).unwrap();
            assert_eq!(decoded, write);
        }
        crate::parts::REGISTERING_KEYS.with(|keys| keys.replace(previous));
    }
}
