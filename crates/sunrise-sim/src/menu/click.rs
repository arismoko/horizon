//! Local click, cursor, drag and pick-all algorithms.
use super::equip_movement::MovePass;
use super::*;

/// One take from a block-backed menu: slot, stack taken and remainder in that slot.
pub type SlotTake = (usize, ItemStack, Option<ItemStack>);

/// The remainder of one click, suspended only when the server has a take to deliver.
pub struct ClickContinuation(ClickStep);

impl ClickContinuation {
    /// Only a player-side write from a completed take survives the loss of its backing menu.
    /// The mods' cursor edits that directly follow the completed take survive; the first slot
    /// edit or take that can no longer apply ends them, since a cursor write after it could
    /// hand out items the slot never gave.
    pub fn after_menu_closed(self) -> Option<Self> {
        match self.0 {
            ClickStep::SwapOverflow(_) => Some(self),
            ClickStep::StackedEdits { carried, rest, .. } => {
                let cursor: Vec<_> = rest
                    .into_iter()
                    .take_while(|edit| matches!(edit, StackedEdit::Carried(_)))
                    .collect();
                (!cursor.is_empty())
                    .then_some(Self(ClickStep::StackedCursorOnly { carried, cursor }))
            }
            _ => None,
        }
    }
}

enum ClickStep {
    QuickCraft {
        slots: Vec<usize>,
        index: usize,
        quick_type: i32,
        source: ItemStack,
        remaining: i64,
    },
    QuickMove {
        slot: usize,
        previous: Option<ItemStack>,
        checked: bool,
    },
    StationTransfer {
        slot: usize,
        original: ItemStack,
        source: ItemStack,
        pass: MovePass,
    },
    Throw {
        slot: usize,
        amount: u32,
        previous: Option<ItemStack>,
        repeat: bool,
    },
    ThrowAfterTake {
        slot: usize,
        amount: u32,
        taken: ItemStack,
        repeat: bool,
    },
    ResultNotifications,
    /// The mods' edits left after a take whose notice is due: they apply once it is delivered,
    /// if the slot and the cursor are still as the applied edits left them.
    StackedEdits {
        slot: usize,
        primary: bool,
        clicked: Option<ItemStack>,
        carried: Option<ItemStack>,
        rest: Vec<StackedEdit>,
    },
    /// The cursor edits owed after a take whose menu closed before its notice.
    StackedCursorOnly {
        carried: Option<ItemStack>,
        cursor: Vec<StackedEdit>,
    },
    PickAll {
        next: i32,
        step: i32,
        pass: u8,
    },
    SwapOverflow(ItemStack),
}

#[derive(Clone, Debug)]
pub(super) struct TakeTracker(Option<SlotTake>);

/// Validate a direct `Slot.set` write for `StackedEditPolicy::Part` against `old` and `kind`.
fn part_set_allowed(
    target: &dyn StackedSlotTarget,
    kind: StackedSlotKind,
    old: &Option<ItemStack>,
    clicked: &Option<ItemStack>,
) -> bool {
    if clicked == old {
        return true;
    }
    if kind == StackedSlotKind::Inert {
        return false;
    }
    let output_like = matches!(
        kind,
        StackedSlotKind::Output | StackedSlotKind::Result | StackedSlotKind::Potion
    );
    match (old, clicked) {
        (Some(_), None) => target.may_pickup(),
        (Some(_), Some(new_stack)) if output_like => {
            let limit = if kind == StackedSlotKind::Potion {
                1
            } else {
                target.item_max_stack(new_stack)
            };
            target.may_pickup() && new_stack.count <= limit
        }
        (None, Some(new_stack)) => {
            target.may_place(new_stack) && new_stack.count <= target.slot_max(new_stack)
        }
        (Some(_), Some(new_stack)) => {
            target.may_pickup()
                && target.may_place(new_stack)
                && new_stack.count <= target.slot_max(new_stack)
        }
        (None, None) => true,
    }
}

/// Apply one `StackedEdit::Take(amount)` on `target` for `StackedEditPolicy::Part`.
fn part_take(target: &mut dyn StackedSlotTarget, kind: StackedSlotKind, amount: u32) -> bool {
    if kind == StackedSlotKind::Inert || amount == 0 {
        return false;
    }
    let Some(orig) = target.current_clicked() else {
        return false;
    };
    if amount > orig.count {
        return false;
    }
    if amount == orig.count {
        target.safe_take(orig.count, orig.count).is_some()
    } else {
        if kind == StackedSlotKind::Result || !target.may_pickup() {
            return false;
        }
        target.take_partial(amount).is_some()
    }
}

/// Apply `outcome`'s edits to `target` in order, enforcing the clicked slot's rules and take
/// side effects (`safe_take` / `on_take`), all or nothing.
///
/// Shared by both local/client [`Menu::pickup`] and hosted LAN `lan::menu::pickup`. Returns `true`
/// when the caller should proceed according to `outcome.resume`, or `false` when the slot refused
/// an edit (`outcome.refused` is set and nothing was mutated).
pub fn apply_stacked_outcome(
    target: &mut impl StackedSlotTarget,
    outcome: &StackedOutcome,
) -> bool {
    if outcome.edits.is_empty() {
        return true;
    }
    let applied =
        target.with_staged(|target| apply_stacked_edits(target, outcome.policy, &outcome.edits));
    if !applied {
        outcome.refused.set(true);
    }
    applied
}

/// Apply `edits` to `target` in order under `policy`'s slot rules; `false` at the first one
/// the rules refuse.
fn apply_stacked_edits(
    target: &mut dyn StackedSlotTarget,
    policy: StackedEditPolicy,
    edits: &[StackedEdit],
) -> bool {
    let kind = target.slot_kind();
    for edit in edits {
        let old = target.current_clicked();
        let ok = match (edit, policy) {
            (StackedEdit::Carried(carried), _) => {
                target.set_carried(carried.clone());
                true
            }
            // A global replacement goes through the clicked slot's rules: a result slot's or filled
            // potion slot's `onTake` would not run; `mayPickup` and `mayPlace` still answer, and the
            // stack fits `Slot.getMaxStackSize`.
            (StackedEdit::Clicked(clicked), StackedEditPolicy::Global) => {
                let refused = clicked != &old
                    && (matches!(
                        kind,
                        StackedSlotKind::Output | StackedSlotKind::Result | StackedSlotKind::Inert
                    ) || (old.is_some()
                        && (kind == StackedSlotKind::Potion || !target.may_pickup()))
                        || clicked.as_ref().is_some_and(|stack| {
                            !target.may_place(stack) || stack.count > target.slot_max(stack)
                        }));
                if !refused {
                    target.set_clicked(clicked.clone());
                }
                !refused
            }
            (StackedEdit::Clicked(clicked), StackedEditPolicy::Part) => {
                let allowed = part_set_allowed(&*target, kind, &old, clicked);
                if allowed && clicked != &old {
                    target.set_clicked(clicked.clone());
                }
                allowed
            }
            (StackedEdit::Take(_), StackedEditPolicy::Global) => false,
            (StackedEdit::Take(amount), StackedEditPolicy::Part) => {
                part_take(target, kind, *amount)
            }
        };
        if !ok {
            return false;
        }
    }
    true
}

struct MenuSlotTarget<'a, 'e> {
    menu: &'a mut Menu,
    env: &'a Env<'e>,
    slot: usize,
}

impl StackedSlotTarget for MenuSlotTarget<'_, '_> {
    fn slot_kind(&self) -> StackedSlotKind {
        match self.menu.state.slots[self.slot].role {
            Role::Inert | Role::Hidden => StackedSlotKind::Inert,
            Role::Output => StackedSlotKind::Output,
            Role::Potion => StackedSlotKind::Potion,
            Role::CraftResult
            | Role::LoomResult
            | Role::MerchantResult
            | Role::StoneResult
            | Role::GrindResult
            | Role::CombineResult
            | Role::CartoResult
            | Role::ModResult => StackedSlotKind::Result,
            _ => StackedSlotKind::Ordinary,
        }
    }

    fn current_clicked(&self) -> Option<ItemStack> {
        self.menu.slot(self.slot).cloned()
    }

    fn may_pickup(&self) -> bool {
        self.menu.may_pickup(self.env, self.slot)
    }

    fn may_place(&self, stack: &ItemStack) -> bool {
        self.menu.may_place(self.env, self.slot, stack)
    }

    fn slot_max(&self, stack: &ItemStack) -> u32 {
        self.menu.slot_max(self.env, self.slot, stack)
    }

    fn item_max_stack(&self, stack: &ItemStack) -> u32 {
        Menu::max_stack(self.env, stack)
    }

    fn set_carried(&mut self, carried: Option<ItemStack>) {
        self.menu.state.carried = carried;
    }

    fn set_clicked(&mut self, clicked: Option<ItemStack>) {
        self.menu.set_by_player(self.env, self.slot, clicked);
    }

    fn with_staged(&mut self, edit: impl FnOnce(&mut dyn StackedSlotTarget) -> bool) -> bool {
        let mut staged = self.menu.clone();
        if !edit(&mut MenuSlotTarget {
            menu: &mut staged,
            env: self.env,
            slot: self.slot,
        }) {
            return false;
        }
        self.menu.install_state(staged);
        true
    }

    fn safe_take(&mut self, amount: u32, max_amount: u32) -> Option<ItemStack> {
        self.menu.safe_take(self.env, self.slot, amount, max_amount)
    }

    fn take_partial(&mut self, amount: u32) -> Option<ItemStack> {
        let mut taken = self.menu.remove(self.env, self.slot, amount)?;
        self.menu.on_take(self.env, self.slot, &mut taken);
        Some(taken)
    }
}

impl Menu {
    pub(super) fn quick_replace(&self, s: usize, stack: &ItemStack, env: &Env) -> bool {
        match self.slot(s) {
            None => true,
            Some(have) if have.same_item_same_components(stack) => {
                have.count <= Self::max_stack(env, stack)
            }
            Some(_) => false,
        }
    }

    /// Whether a drag (quick craft) is in progress (`quickcraftStatus != 0`):
    /// the next click other than a quick craft only ends it.
    pub fn quick_crafting(&self) -> bool {
        self.state.quick_status != 0
    }

    /// Snapshot the menu's quick-craft drag state so a hosted menu rebuilt per packet can preserve
    /// it across clicks.
    pub fn quick_craft_state(&self) -> QuickCraftDrag {
        QuickCraftDrag {
            status: self.state.quick_status,
            ty: self.state.quick_type,
            slots: self.state.quick_slots.clone(),
        }
    }

    /// Restore a previously captured quick-craft drag state onto this menu.
    pub fn set_quick_craft_state(&mut self, drag: QuickCraftDrag) {
        self.touch();
        self.state.quick_status = drag.status;
        self.state.quick_type = drag.ty;
        self.state.quick_slots = drag.slots;
    }

    pub(super) fn reset_quick_craft(&mut self) {
        self.touch();
        self.state.quick_status = 0;
        self.state.quick_slots.clear();
    }

    /// The resolved pickup click `click` would run for `(slot_index, button, input)`, as
    /// `item_stacked_on_other` is asked about it: the stacks it finds.
    ///
    /// Resolves both a direct left/right `Input::Pickup` when no drag is active (`quick_status ==
    /// 0`) and a one-slot left/right `Input::QuickCraft` drag release (`header == 2`,
    /// `quick_status == 1`, `quick_slots.len() == 1`, `quick_type` in `0..=1`), matching vanilla
    /// 26.3 `AbstractContainerMenu.doClick`. Returns `None` for active-drag resets, multi-slot
    /// drags, middle-button (`quick_type == 2`) drags, out-of-range or hidden slots, and other
    /// inputs.
    pub fn stacked_click(
        &self,
        env: &Env,
        slot_index: i32,
        button: i32,
        input: Input,
    ) -> Option<StackedClick> {
        let (s, primary) = match input {
            Input::Pickup => {
                let s = usize::try_from(slot_index).ok()?;
                if !(0..=1).contains(&button) || self.state.quick_status != 0 {
                    return None;
                }
                (s, button == 0)
            }
            Input::QuickCraft => {
                let header = button & 3;
                if header != 2
                    || self.state.quick_status != 1
                    || self.state.carried.is_none()
                    || self.state.quick_slots.len() != 1
                    || !(0..=1).contains(&self.state.quick_type)
                {
                    return None;
                }
                (self.state.quick_slots[0], self.state.quick_type == 0)
            }
            _ => return None,
        };
        if s >= self.state.slots.len() || self.state.slots[s].role == Role::Hidden {
            return None;
        }
        let slot = self.state.slots[s];
        Some(StackedClick {
            player: env.player,
            player_entity_id: env.player_entity_id,
            slot: s,
            container: slot.container,
            container_slot: slot.index,
            primary,
            carried: self.state.carried.clone(),
            clicked: self.slot(s).cloned(),
        })
    }

    /// `AbstractContainerMenu.clicked`: Err carries a vanilla exception name.
    pub fn click(
        &mut self,
        env: &Env,
        slot_index: i32,
        button: i32,
        input: Input,
    ) -> Result<(), &'static str> {
        self.start_click(env, slot_index, button, input).map(|_| ())
    }

    /// Like [`Self::click`], resolving each creative-screen `Input::Throw` drop
    /// through `on_creative_drop` before charging the throttler and deciding the
    /// next take in a repeated throw loop.
    pub fn click_with_creative_drop(
        &mut self,
        env: &Env,
        slot_index: i32,
        button: i32,
        input: Input,
        mut on_creative_drop: impl FnMut(&ItemStack) -> bool,
    ) -> Result<(), &'static str> {
        let rest = self.click_planned(env, slot_index, button, input, &mut on_creative_drop)?;
        let _ = self.with_result_notifications(rest);
        Ok(())
    }

    /// Start a click that may suspend at the first take or `createResult`.
    pub fn start_click(
        &mut self,
        env: &Env,
        slot_index: i32,
        button: i32,
        input: Input,
    ) -> Result<Option<ClickContinuation>, &'static str> {
        let rest = self.click_planned(env, slot_index, button, input, &mut |_| false)?;
        Ok(self.with_result_notifications(rest))
    }

    /// Start a `QuickMove` whose initial `Slot.mayPickup` check already succeeded.
    pub fn start_checked_quick_move(
        &mut self,
        env: &Env,
        slot_index: i32,
        button: i32,
    ) -> Result<Option<ClickContinuation>, &'static str> {
        self.touch();
        let size = self.state.slots.len() as i32;
        if self.state.quick_status != 0 {
            self.reset_quick_craft();
            return Ok(None);
        }
        if (0..size).contains(&slot_index)
            && self.state.slots[slot_index as usize].role == Role::Hidden
        {
            return Ok(None);
        }
        if button != 0 && button != 1 {
            return Ok(None);
        }
        if slot_index < 0 {
            return Ok(None);
        }
        if !(0..size).contains(&slot_index) {
            return Err(OUT_OF_BOUNDS);
        }
        Ok(self.run_click_step(
            env,
            ClickStep::QuickMove {
                slot: slot_index as usize,
                previous: None,
                checked: true,
            },
            &mut |_| false,
        ))
    }

    pub(super) fn click_planned(
        &mut self,
        env: &Env,
        slot_index: i32,
        button: i32,
        input: Input,
        on_creative_drop: &mut dyn FnMut(&ItemStack) -> bool,
    ) -> Result<Option<ClickContinuation>, &'static str> {
        self.touch();
        let size = self.state.slots.len() as i32;
        let in_range = |i: i32| (0..size).contains(&i);
        if input == Input::QuickCraft {
            let expected = self.state.quick_status;
            self.state.quick_status = button & 3;
            if ((expected != 1 || self.state.quick_status != 2)
                && expected != self.state.quick_status)
                || self.state.carried.is_none()
            {
                self.reset_quick_craft();
            } else if self.state.quick_status == 0 {
                self.state.quick_type = (button >> 2) & 3;
                if self.state.quick_type == 0
                    || self.state.quick_type == 1
                    || (self.state.quick_type == 2 && self.state.creative)
                {
                    self.state.quick_status = 1;
                    self.state.quick_slots.clear();
                } else {
                    self.reset_quick_craft();
                }
            } else if self.state.quick_status == 1 {
                if !in_range(slot_index) {
                    return Err(OUT_OF_BOUNDS);
                }
                let s = slot_index as usize;
                let carried = self.state.carried.clone().expect("checked");
                if self.quick_replace(s, &carried, env)
                    && self.may_place(env, s, &carried)
                    && (self.state.quick_type == 2
                        || carried.count as usize > self.state.quick_slots.len())
                    && self.can_drag_to(s)
                    && !self.state.quick_slots.contains(&s)
                {
                    self.state.quick_slots.push(s);
                }
            } else if self.state.quick_status == 2 {
                if !self.state.quick_slots.is_empty() {
                    if self.state.quick_slots.len() == 1 {
                        let s = self.state.quick_slots[0] as i32;
                        let ty = self.state.quick_type;
                        self.reset_quick_craft();
                        return self.start_click(env, s, ty, Input::Pickup);
                    }
                    let mut source = self.state.carried.clone().expect("checked");
                    let mut remaining = source.count as i64;
                    let observed = env.player_entity_id.is_some()
                        && env
                            .can_equip
                            .is_some_and(|parts| parts.listens(&source.item))
                        && self
                            .state
                            .quick_slots
                            .iter()
                            .any(|slot| matches!(self.state.slots[*slot].role, Role::Armor(_)));
                    if !observed && self.state.track_results {
                        let slots = self.state.quick_slots.clone();
                        let quick_type = self.state.quick_type;
                        return Ok(self.run_click_step(
                            env,
                            ClickStep::QuickCraft {
                                slots,
                                index: 0,
                                quick_type,
                                source,
                                remaining,
                            },
                            on_creative_drop,
                        ));
                    }
                    let quick_slots = self.state.quick_slots.clone();
                    let mut distribute = |can_equip: Option<&dyn crate::mobs::equip::CanEquip>| {
                        for &s in &quick_slots {
                            if self.quick_replace(s, &source, env)
                                && self.may_place_with(env, can_equip, s, &source)
                                && (self.state.quick_type == 2
                                    || source.count as usize >= quick_slots.len())
                                && self.can_drag_to(s)
                            {
                                let have = self.slot(s).map_or(0, |c| c.count);
                                let max = self.slot_max(env, s, &source);
                                let new_count = quick_craft_count(
                                    self.state.quick_type as u8,
                                    source.count,
                                    quick_slots.len(),
                                    have,
                                    max,
                                    Self::max_stack(env, &source),
                                );
                                remaining -= i64::from(new_count) - i64::from(have);
                                let mut placed = source.clone();
                                placed.count = new_count;
                                self.set_by_player(env, s, Some(placed));
                            }
                        }
                    };
                    match env.can_equip {
                        Some(parts) if parts.listens(&source.item) => {
                            parts.with_stack(&source, &mut |scoped| distribute(Some(scoped)));
                        }
                        can_equip => distribute(can_equip),
                    }
                    source.count = remaining.max(0) as u32;
                    self.state.carried = Some(source);
                    tidy(&mut self.state.carried);
                }
                self.reset_quick_craft();
            } else {
                self.reset_quick_craft();
            }
            return Ok(None);
        }
        if self.state.quick_status != 0 {
            self.reset_quick_craft();
            return Ok(None);
        }
        // A slot the mod's screen leaves out takes no click.
        if in_range(slot_index) && self.state.slots[slot_index as usize].role == Role::Hidden {
            return Ok(None);
        }
        if matches!(input, Input::Pickup | Input::QuickMove) && (button == 0 || button == 1) {
            let primary = button == 0;
            if slot_index == -999 {
                if let Some(carried) = self.state.carried.as_mut() {
                    if primary {
                        let dropped = self.state.carried.take().expect("some");
                        self.drop_stack(&dropped, env.items);
                    } else {
                        let one = split(carried, 1);
                        self.drop_stack(&one, env.items);
                        tidy(&mut self.state.carried);
                    }
                }
            } else if input == Input::QuickMove {
                if slot_index < 0 {
                    return Ok(None);
                }
                if !in_range(slot_index) {
                    return Err(OUT_OF_BOUNDS);
                }
                let s = slot_index as usize;
                return Ok(self.run_click_step(
                    env,
                    ClickStep::QuickMove {
                        slot: s,
                        previous: None,
                        checked: false,
                    },
                    on_creative_drop,
                ));
            } else {
                if slot_index < 0 {
                    return Ok(None);
                }
                if !in_range(slot_index) {
                    return Err(OUT_OF_BOUNDS);
                }
                return Ok(self.pickup(env, slot_index as usize, primary));
            }
        } else if input == Input::Swap && ((0..9).contains(&button) || button == 40) {
            if !in_range(slot_index) {
                return Err(OUT_OF_BOUNDS);
            }
            return Ok(self.swap(env, slot_index as usize, button as usize));
        } else if input == Input::Clone
            && self.state.creative
            && self.state.carried.is_none()
            && slot_index >= 0
        {
            if !in_range(slot_index) {
                return Err(OUT_OF_BOUNDS);
            }
            if let Some(stack) = self.slot(slot_index as usize) {
                let mut copy = stack.clone();
                copy.count = Self::max_stack(env, stack);
                // The cartography result's safeClone runs onCraftedBy.
                if self.state.slots[slot_index as usize].role == Role::CartoResult {
                    Self::map_crafted(env.maps, &mut copy);
                }
                self.state.carried = Some(copy);
            }
        } else if input == Input::Throw && self.state.carried.is_none() && slot_index >= 0 {
            if !in_range(slot_index) {
                return Err(OUT_OF_BOUNDS);
            }
            let s = slot_index as usize;
            let amount = if button == 0 {
                1
            } else {
                self.slot(s).map_or(0, |c| c.count)
            };
            return Ok(self.run_click_step(
                env,
                ClickStep::Throw {
                    slot: s,
                    amount,
                    previous: None,
                    repeat: button == 1,
                },
                on_creative_drop,
            ));
        } else if input == Input::PickupAll && slot_index >= 0 {
            if !in_range(slot_index) {
                return Err(OUT_OF_BOUNDS);
            }
            let s = slot_index as usize;
            if self.carried.is_some() && !(self.has_item(s) && self.may_pickup(env, s)) {
                let (next, step) = if button == 0 { (0, 1) } else { (size - 1, -1) };
                return Ok(self.run_click_step(
                    env,
                    ClickStep::PickAll {
                        next,
                        step,
                        pass: 0,
                    },
                    on_creative_drop,
                ));
            }
        }
        Ok(None)
    }

    /// Starts take recording for an authoritative block-backed mod menu click.
    pub fn track_takes(&mut self, track: bool) {
        self.takes = track.then_some(TakeTracker(None));
    }

    /// The take performed before the click yielded or finished.
    pub fn take_notice(&mut self) -> Option<SlotTake> {
        self.takes.as_mut()?.0.take()
    }

    fn take_pending(&self) -> bool {
        self.takes
            .as_ref()
            .is_some_and(|tracker| tracker.0.is_some())
    }

    /// Suspend multi-slot clicks whenever `AnvilMenu.createResult` or `GrindstoneMenu.createResult`
    /// runs so the caller can dispatch live result hooks between slot writes.
    pub fn track_results(&mut self, track: bool) {
        self.touch();
        self.state.track_results = track;
    }

    pub(super) fn result_pending(&self) -> bool {
        self.state.track_results
            && match self.state.kind {
                Kind::Anvil => self.state.anvil_made,
                Kind::Grindstone => self.state.grind_made,
                _ => false,
            }
    }

    fn step_pending(&self) -> bool {
        self.take_pending() || self.result_pending() || self.station_take_pending()
    }

    fn station_take_pending(&self) -> bool {
        self.state.track_results
            && (self.state.anvil_craft.is_some() || self.state.grind_take.is_some())
    }

    fn with_result_notifications(
        &self,
        rest: Option<ClickContinuation>,
    ) -> Option<ClickContinuation> {
        rest.or_else(|| {
            (self.state.more_results > 0)
                .then_some(ClickContinuation(ClickStep::ResultNotifications))
        })
    }

    fn tracks_take(&self, s: usize) -> bool {
        self.takes.is_some() && matches!(self.kind, Kind::Mod(t) if t.owns_slot(s))
    }

    /// `Slot.onTake`, with the slot's remainder before a crafting result recomputes.
    pub(super) fn note_take(&mut self, s: usize, taken: &ItemStack) {
        if taken.count > 0 && self.tracks_take(s) {
            let left = self.slot(s).cloned();
            self.takes.as_mut().unwrap().0 = Some((s, taken.clone(), left));
        }
    }

    /// The crafting result's `onTake` records itself. Record other positive quick moves only
    /// while this source has a listener.
    fn quick_move_noted(&mut self, env: &Env, s: usize) -> Option<ItemStack> {
        if !self.tracks_take(s) || self.state.slots[s].role == Role::CraftResult {
            return self.quick_move(env, s);
        }
        let before = self.slot(s).map_or(0, |held| held.count);
        let moved = self.quick_move(env, s)?;
        let left = self.slot(s).map_or(0, |now| now.count);
        let count = before.saturating_sub(left);
        if count > 0 {
            let mut taken = moved.clone();
            taken.count = count;
            self.note_take(s, &taken);
        }
        Some(moved)
    }

    /// The PICKUP branch of `doClick`: a bundle's override, else the
    /// plain click, ending with `slot.setChanged()`.
    fn pickup(&mut self, env: &Env, s: usize, primary: bool) -> Option<ClickContinuation> {
        self.touch();
        let mut resume = StackedResume::VanillaAll;
        if let Some(outcome) = env.stacked {
            resume = outcome.resume;
            if self.tracks_take(s)
                && outcome.policy == StackedEditPolicy::Part
                && outcome
                    .edits
                    .iter()
                    .any(|edit| matches!(edit, StackedEdit::Take(_)))
            {
                return self.apply_tracked_edits(env, s, primary, &outcome.edits);
            }
            let mut target = MenuSlotTarget {
                menu: self,
                env,
                slot: s,
            };
            if !apply_stacked_outcome(&mut target, outcome) {
                return None;
            }
        }
        self.finish_pickup(env, s, primary, resume);
        None
    }

    /// Apply the mods' edits on a slot whose takes are delivered one at a time: up to and
    /// including each take, then suspend until that take's notice has gone out.
    fn apply_tracked_edits(
        &mut self,
        env: &Env,
        slot: usize,
        primary: bool,
        edits: &[StackedEdit],
    ) -> Option<ClickContinuation> {
        let outcome = env
            .stacked
            .expect("a tracked Part click keeps its decision");
        let split = edits
            .iter()
            .position(|edit| matches!(edit, StackedEdit::Take(_)))
            .map_or(edits.len(), |take| take + 1);
        let (now, rest) = edits.split_at(split);
        let mut target = MenuSlotTarget {
            menu: self,
            env,
            slot,
        };
        if !target.with_staged(|target| apply_stacked_edits(target, StackedEditPolicy::Part, now)) {
            outcome.refused.set(true);
            return None;
        }
        if !rest.is_empty() && self.take_pending() {
            return Some(ClickContinuation(ClickStep::StackedEdits {
                slot,
                primary,
                clicked: self.slot(slot).cloned(),
                carried: self.state.carried.clone(),
                rest: rest.to_vec(),
            }));
        }
        if !rest.is_empty() {
            return self.apply_tracked_edits(env, slot, primary, rest);
        }
        if self.take_pending() {
            return Some(ClickContinuation(ClickStep::StackedEdits {
                slot,
                primary,
                clicked: self.slot(slot).cloned(),
                carried: self.state.carried.clone(),
                rest: Vec::new(),
            }));
        }
        self.finish_pickup(env, slot, primary, outcome.resume);
        None
    }

    fn finish_pickup(&mut self, env: &Env, s: usize, primary: bool, resume: StackedResume) {
        let handled = match resume {
            StackedResume::Done => true,
            StackedResume::Plain => false,
            StackedResume::VanillaAll => self.bundle_click(env, s, primary),
            StackedResume::VanillaCarried => {
                let took = self.carried_bundle_click(env, s, primary);
                if took {
                    self.inventory_slots_changed(env);
                }
                took
            }
            StackedResume::VanillaClicked => {
                let took = self.clicked_bundle_click(env, s, primary);
                if took {
                    self.inventory_slots_changed(env);
                }
                took
            }
        };
        if !handled {
            self.pickup_inner(env, s, primary);
        }
        self.slot_changed(env, s);
    }

    pub(super) fn pickup_inner(&mut self, env: &Env, s: usize, primary: bool) {
        self.touch();
        let clicked = self.slot(s).cloned();
        match clicked {
            None => {
                if let Some(carried) = self.state.carried.take() {
                    let amount = if primary { carried.count } else { 1 };
                    let mut input = Some(carried);
                    self.safe_insert(env, s, &mut input, amount);
                    self.state.carried = input;
                    tidy(&mut self.state.carried);
                }
            }
            Some(clicked) if self.may_pickup(env, s) => match self.state.carried.take() {
                None => {
                    let amount = if primary {
                        clicked.count
                    } else {
                        clicked.count.div_ceil(2)
                    };
                    if let Some(mut taken) = self.try_remove(env, s, amount, u32::MAX) {
                        self.state.carried = Some(taken.clone());
                        self.on_take(env, s, &mut taken);
                        if self.state.slots[s].role == Role::CartoResult {
                            Self::map_crafted(env.maps, &mut taken);
                        }
                        if matches!(
                            self.state.slots[s].role,
                            Role::CartoResult | Role::CraftResult
                        ) {
                            self.state.carried = Some(taken);
                        }
                    }
                }
                Some(carried) => {
                    if self.may_place(env, s, &carried) {
                        if clicked.same_item_same_components(&carried) {
                            let amount = if primary { carried.count } else { 1 };
                            let mut input = Some(carried);
                            self.safe_insert(env, s, &mut input, amount);
                            self.state.carried = input;
                            tidy(&mut self.state.carried);
                        } else if carried.count <= self.slot_max(env, s, &carried) {
                            self.state.carried = Some(clicked);
                            self.set_by_player(env, s, Some(carried));
                        } else {
                            self.state.carried = Some(carried);
                        }
                    } else if clicked.same_item_same_components(&carried) {
                        let room = Self::max_stack(env, &carried).saturating_sub(carried.count);
                        self.state.carried = Some(carried);
                        if let Some(mut taken) = self.try_remove(env, s, clicked.count, room) {
                            if let Some(c) = self.state.carried.as_mut() {
                                c.count += taken.count;
                            }
                            self.on_take(env, s, &mut taken);
                            if self.state.slots[s].role == Role::CartoResult {
                                // onCraftedBy edits the taken stack, not the carried one.
                                Self::map_crafted(env.maps, &mut taken);
                            }
                        }
                    } else {
                        self.state.carried = Some(carried);
                    }
                }
            },
            Some(_) => {}
        }
    }

    /// The SWAP branch of `doClick`.
    fn swap(&mut self, env: &Env, target: usize, button: usize) -> Option<ClickContinuation> {
        self.touch();
        let inv_slot = button;
        let target_slot = self.state.slots[target];
        if target_slot.container == PLAYER && target_slot.index == inv_slot {
            return None;
        }
        let source = self.state.containers[PLAYER][inv_slot].clone();
        let target_stack = self.slot(target).cloned();
        if source.is_none() && target_stack.is_none() {
            return None;
        }
        match (source, target_stack) {
            (None, Some(mut taken)) => {
                if self.may_pickup(env, target) {
                    self.state.containers[PLAYER][inv_slot] = Some(taken.clone());
                    self.set_by_player(env, target, None);
                    self.on_take(env, target, &mut taken);
                    if self.state.slots[target].role == Role::CraftResult {
                        self.state.containers[PLAYER][inv_slot] = Some(taken);
                    } else if self.state.slots[target].role == Role::CartoResult
                        && let Some(stack) = self.state.containers[PLAYER][inv_slot].as_mut()
                    {
                        Self::map_crafted(env.maps, stack);
                    }
                }
            }
            (Some(mut source), None) => {
                if self.may_place(env, target, &source) {
                    let max = self.slot_max(env, target, &source);
                    if source.count > max {
                        let part = split(&mut source, max);
                        self.state.containers[PLAYER][inv_slot] = Some(source);
                        self.set_by_player(env, target, Some(part));
                    } else {
                        self.state.containers[PLAYER][inv_slot] = None;
                        self.set_by_player(env, target, Some(source));
                    }
                }
            }
            (Some(mut source), Some(mut taken)) => {
                if self.may_pickup(env, target) && self.may_place(env, target, &source) {
                    let max = self.slot_max(env, target, &source);
                    if source.count > max {
                        let part = split(&mut source, max);
                        self.state.containers[PLAYER][inv_slot] = Some(source);
                        self.set_by_player(env, target, Some(part));
                        self.on_take(env, target, &mut taken);
                        let rest = ClickStep::SwapOverflow(taken);
                        return if self.take_pending() {
                            Some(ClickContinuation(rest))
                        } else {
                            self.run_click_step(env, rest, &mut |_| false)
                        };
                    }
                    self.state.containers[PLAYER][inv_slot] = Some(taken.clone());
                    self.set_by_player(env, target, Some(source));
                    self.on_take(env, target, &mut taken);
                }
            }
            (None, None) => {}
        }
        None
    }

    fn quick_move_step(
        &mut self,
        env: &Env,
        slot: usize,
        previous: Option<&ItemStack>,
        checked: bool,
    ) -> Option<ItemStack> {
        let allowed = match previous {
            None => checked || self.may_pickup(env, slot),
            Some(previous) => self.slot(slot).is_some_and(|now| same_item(now, previous)),
        };
        allowed.then(|| self.quick_move_noted(env, slot)).flatten()
    }

    fn throw_step(
        &mut self,
        env: &Env,
        slot: usize,
        amount: u32,
        previous: Option<&ItemStack>,
    ) -> Option<ItemStack> {
        if previous
            .is_some_and(|previous| !self.slot(slot).is_some_and(|now| same_item(now, previous)))
        {
            return None;
        }
        let taken = self.safe_take(env, slot, amount, u32::MAX)?;
        self.drop_stack(&taken, env.items);
        Some(taken)
    }

    /// One iteration of a merchant result click, so the authority can finish each actual
    /// trade and publish its offer back to this menu before deciding the next iteration.
    /// Only quick-move and throw-all may repeat; each continuation checks the result item.
    pub fn merchant_result_step(
        &mut self,
        env: &Env,
        button: i32,
        input: Input,
        previous: Option<&ItemStack>,
        throw_amount: u32,
    ) -> Option<ItemStack> {
        if self.quick_crafting() {
            self.reset_quick_craft();
            return None;
        }
        match input {
            Input::QuickMove if (0..=1).contains(&button) => {
                self.quick_move_step(env, 2, previous, false)
            }
            Input::Throw if self.carried.is_none() => {
                let taken = self.throw_step(env, 2, throw_amount, previous);
                taken.filter(|_| button == 1)
            }
            _ => {
                let _ = self.click(env, 2, button, input);
                None
            }
        }
    }

    /// The PICKUP_ALL branch (double click).
    /// `canTakeItemForPickAll`: never the result slots of the menus that
    /// override it (crafting, smithing, stonecutter, cartography; a mod menu
    /// type's crafting result is one), MerchantMenu's or an item tab's
    /// display slots.
    /// The screen asks it before a double click's PICKUP_ALL
    /// (`AbstractContainerScreen.mouseReleased`): a refused slot's second
    /// click stays a PICKUP, another craft.
    pub fn can_take_for_pick_all(&self, t: usize) -> bool {
        self.state.kind != Kind::Merchant
            && self.state.slots[t].role != Role::CreativeDisplay
            && (self.state.slots[t].container != RESULT
                || matches!(self.state.kind, Kind::Loom | Kind::Grindstone | Kind::Anvil))
    }

    /// `AbstractContainerScreen.mouseReleased` on a Shift double click:
    /// QUICK_MOVE on every slot of `clicked`'s container that holds
    /// `moved`'s item (`lastQuickMoved`, the stack the first click moved).
    pub fn quick_move_all(
        &mut self,
        env: &Env,
        clicked: usize,
        moved: &ItemStack,
    ) -> Result<(), &'static str> {
        self.touch();
        self.quick_move_all_each(env, clicked, moved, &mut |_, _| {})
    }

    /// Whether slot `t` participates in a Shift double click started on `clicked` for `moved`.
    pub fn quick_move_all_accepts(
        &self,
        env: &Env,
        clicked: usize,
        t: usize,
        moved: &ItemStack,
    ) -> bool {
        clicked < self.state.slots.len()
            && t < self.state.slots.len()
            && self.has_item(t)
            && self.state.slots[t].container == self.state.slots[clicked].container
            && self.quick_replace(t, moved, env)
            && self.may_pickup(env, t)
    }

    /// `quick_move_all`, calling `each` with the menu and the slot after each
    /// QUICK_MOVE (a server's client sends every one of them).
    pub fn quick_move_all_each(
        &mut self,
        env: &Env,
        clicked: usize,
        moved: &ItemStack,
        each: &mut dyn FnMut(&Self, usize),
    ) -> Result<(), &'static str> {
        self.touch();
        if clicked >= self.state.slots.len() || !self.can_take_for_pick_all(clicked) {
            return Ok(());
        }
        for t in 0..self.state.slots.len() {
            if self.quick_move_all_accepts(env, clicked, t, moved) {
                self.click(env, t as i32, 0, Input::QuickMove)?;
                each(self, t);
            }
        }
        Ok(())
    }

    /// `AbstractContainerScreen.shouldAddSlotToQuickCraft`: whether a drag
    /// of type `ty` over `dragged` slots so far takes slot `s` too.
    pub fn quick_craft_accepts(&self, env: &Env, s: usize, ty: i32, dragged: usize) -> bool {
        let Some(carried) = self.state.carried.as_ref() else {
            return false;
        };
        s < self.state.slots.len()
            && (carried.count as usize > dragged || ty == 2)
            && self.quick_replace(s, carried, env)
            && self.may_place(env, s, carried)
            && self.can_drag_to(s)
    }

    /// What a quick craft drag of type `ty` over `slots` shows
    /// (`AbstractContainerScreen.extractSlot` and
    /// `recalculateQuickCraftRemaining`): each slot's would-be count and
    /// whether the slot caps it (a yellow count), then the count left in
    /// hand. Nothing while the drag spans a single slot.
    pub fn quick_craft_preview(
        &self,
        env: &Env,
        ty: i32,
        slots: &[usize],
    ) -> Option<(Vec<Option<(u32, bool)>>, u32)> {
        let carried = self.state.carried.as_ref()?;
        if slots.len() < 2 {
            return None;
        }
        let carried_max = Self::max_stack(env, carried);
        // AbstractContainerMenu.getQuickCraftPlaceCount.
        let place = match ty {
            0 => carried.count / slots.len() as u32,
            1 => 1,
            2 => carried_max,
            _ => carried.count,
        };
        let mut left = i64::from(if ty == 2 { carried_max } else { carried.count });
        let mut shown = Vec::with_capacity(slots.len());
        for &s in slots {
            let carry = self.slot(s).map_or(0, |i| i.count);
            let max = carried_max.min(self.slot_max(env, s, carried));
            if ty != 2 {
                left -= i64::from((place + carry).min(max)) - i64::from(carry);
            }
            shown.push(
                (s < self.state.slots.len()
                    && self.quick_replace(s, carried, env)
                    && self.can_drag_to(s))
                .then(|| ((place + carry).min(max), place + carry > max)),
            );
        }
        Some((shown, left.max(0) as u32))
    }

    /// `canDragTo`: ItemPickerMenu keeps drags off its display slots.
    pub(super) fn can_drag_to(&self, s: usize) -> bool {
        self.state.slots[s].role != Role::CreativeDisplay
    }

    /// Run until the next take or the end of this click. The menu and its rules are borrowed
    /// anew after dispatch, so a listener can change the next step's inputs.
    fn run_click_step(
        &mut self,
        env: &Env,
        step: ClickStep,
        on_creative_drop: &mut dyn FnMut(&ItemStack) -> bool,
    ) -> Option<ClickContinuation> {
        match step {
            ClickStep::QuickCraft {
                slots,
                mut index,
                quick_type,
                mut source,
                mut remaining,
            } => {
                while index < slots.len() {
                    let s = slots[index];
                    index += 1;
                    if self.quick_replace(s, &source, env)
                        && self.may_place(env, s, &source)
                        && (quick_type == 2 || source.count as usize >= slots.len())
                        && self.can_drag_to(s)
                    {
                        let have = self.slot(s).map_or(0, |c| c.count);
                        let max = self.slot_max(env, s, &source);
                        let new_count = quick_craft_count(
                            quick_type as u8,
                            source.count,
                            slots.len(),
                            have,
                            max,
                            Self::max_stack(env, &source),
                        );
                        remaining -= i64::from(new_count) - i64::from(have);
                        let mut placed = source.clone();
                        placed.count = new_count;
                        self.set_by_player(env, s, Some(placed));
                        if self.result_pending() {
                            return Some(ClickContinuation(ClickStep::QuickCraft {
                                slots,
                                index,
                                quick_type,
                                source,
                                remaining,
                            }));
                        }
                    }
                }
                source.count = remaining.max(0) as u32;
                self.state.carried = Some(source);
                tidy(&mut self.state.carried);
                self.reset_quick_craft();
            }
            ClickStep::QuickMove {
                slot,
                mut previous,
                mut checked,
            } => {
                loop {
                    let allowed = match &previous {
                        None => checked || self.may_pickup(env, slot),
                        Some(before) => self.slot(slot).is_some_and(|now| same_item(now, before)),
                    };
                    if !allowed {
                        break;
                    }
                    // Only station inputs run createResult inside moveItemStackTo. Retain the
                    // cursor of this pass, distinct from doClick's repeat of quickMoveStack.
                    if self.state.track_results
                        && slot > 2
                        && (self.kind == Kind::Anvil
                            || self.kind == Kind::Grindstone
                                && self.state.containers[MAIN].iter().any(Option::is_none))
                    {
                        let original = self.slot(slot)?.clone();
                        return self.run_click_step(
                            env,
                            ClickStep::StationTransfer {
                                slot,
                                source: original.clone(),
                                original,
                                pass: MovePass::new(0, 2, false),
                            },
                            on_creative_drop,
                        );
                    }
                    let Some(now) = self.quick_move_noted(env, slot) else {
                        break;
                    };
                    checked = false;
                    previous = Some(now);
                    if self.step_pending() {
                        return Some(ClickContinuation(ClickStep::QuickMove {
                            slot,
                            previous,
                            checked,
                        }));
                    }
                }
            }
            ClickStep::StationTransfer {
                slot,
                original,
                mut source,
                mut pass,
            } => {
                self.resume_station_transfer(env, slot, &mut source, &mut pass);
                if self.result_pending() {
                    return Some(ClickContinuation(ClickStep::StationTransfer {
                        slot,
                        original,
                        source,
                        pass,
                    }));
                }
                if pass.changed {
                    let previous =
                        self.finish_quick_move(env, slot, original, source, true, true)?;
                    return self.run_click_step(
                        env,
                        ClickStep::QuickMove {
                            slot,
                            previous: Some(previous),
                            checked: false,
                        },
                        on_creative_drop,
                    );
                }
            }
            ClickStep::Throw {
                slot,
                amount,
                mut previous,
                repeat,
            } => loop {
                if previous
                    .as_ref()
                    .is_some_and(|p| !self.slot(slot).is_some_and(|now| same_item(now, p)))
                {
                    break;
                }
                if !self.can_throw() {
                    break;
                }
                let Some(taken) = self.safe_take(env, slot, amount, u32::MAX) else {
                    break;
                };
                if self.station_take_pending() {
                    return Some(ClickContinuation(ClickStep::ThrowAfterTake {
                        slot,
                        amount,
                        taken,
                        repeat,
                    }));
                }
                self.throw_stack(&taken, env.items, on_creative_drop);
                if !repeat || taken.is_empty() {
                    break;
                }
                previous = Some(taken);
                if self.step_pending() {
                    return Some(ClickContinuation(ClickStep::Throw {
                        slot,
                        amount,
                        previous,
                        repeat,
                    }));
                }
            },
            ClickStep::ThrowAfterTake {
                slot,
                amount,
                taken,
                repeat,
            } => {
                self.throw_stack(&taken, env.items, on_creative_drop);
                if repeat && !taken.is_empty() {
                    return self.run_click_step(
                        env,
                        ClickStep::Throw {
                            slot,
                            amount,
                            previous: Some(taken),
                            repeat,
                        },
                        on_creative_drop,
                    );
                }
            }
            ClickStep::ResultNotifications => {}
            ClickStep::PickAll {
                mut next,
                step,
                mut pass,
            } => {
                let max = Self::max_stack(env, self.carried.as_ref()?);
                let total = self.state.slots.len() as i32;
                let start = if step > 0 { 0 } else { total - 1 };
                while pass < 2 {
                    while (0..total).contains(&next) {
                        let have = self.carried.clone().filter(|c| c.count < max)?;
                        let t = next as usize;
                        next += step;
                        if self.has_item(t)
                            && self.quick_replace(t, &have, env)
                            && self.may_pickup(env, t)
                            && self.can_take_for_pick_all(t)
                            && self.parts_allow_pick_all(env, t, &have)
                        {
                            let item = self.slot(t).expect("has item").clone();
                            if (pass != 0 || item.count != Self::max_stack(env, &item))
                                && let Some(removed) =
                                    self.safe_take(env, t, item.count, max - have.count)
                                && let Some(c) = self.carried.as_mut()
                            {
                                c.count += removed.count;
                                if self.step_pending() {
                                    return Some(ClickContinuation(ClickStep::PickAll {
                                        next,
                                        step,
                                        pass,
                                    }));
                                }
                            }
                        }
                    }
                    pass += 1;
                    next = start;
                }
            }
            ClickStep::StackedEdits {
                slot,
                primary,
                clicked,
                carried,
                rest,
            } => {
                let outcome = env
                    .stacked
                    .expect("a suspended Part click keeps its decision");
                // The take's notice may not change what the remaining edits apply to.
                if self.slot(slot) != clicked.as_ref() || self.state.carried != carried {
                    outcome.refused.set(true);
                    return None;
                }
                if rest.is_empty() {
                    self.finish_pickup(env, slot, primary, outcome.resume);
                } else {
                    return self.apply_tracked_edits(env, slot, primary, &rest);
                }
            }
            ClickStep::StackedCursorOnly { carried, cursor } => {
                let outcome = env.stacked.expect("an owed Part cursor keeps its decision");
                if self.state.carried != carried {
                    outcome.refused.set(true);
                    return None;
                }
                for edit in cursor {
                    if let StackedEdit::Carried(stack) = edit {
                        self.carried = stack;
                    }
                }
            }
            ClickStep::SwapOverflow(mut taken) => {
                if !self.inventory_add(env, &mut taken) {
                    self.drop_stack(&taken, env.items);
                }
            }
        }
        None
    }

    /// Run any next result calculation from the same slot write, after delivering the prior one.
    pub fn advance_result_notification(&mut self, env: &Env) -> bool {
        if self.state.more_results == 0 {
            return false;
        }
        self.state.more_results -= 1;
        match self.kind {
            Kind::Anvil => self.anvil_result(env),
            Kind::Grindstone => self.grindstone_inputs_changed(env),
            _ => unreachable!("only repair input containers queue results"),
        }
        true
    }

    /// Continue the suspended click.
    pub fn resume_click(
        &mut self,
        env: &Env,
        rest: ClickContinuation,
    ) -> Option<ClickContinuation> {
        if self.advance_result_notification(env) {
            return Some(rest);
        }
        let next = self.run_click_step(env, rest.0, &mut |_| false);
        self.with_result_notifications(next)
    }
}

#[cfg(test)]
mod after_menu_closed_tests {
    use super::*;

    fn owed(rest: Vec<StackedEdit>) -> Option<Vec<StackedEdit>> {
        let step = ClickContinuation(ClickStep::StackedEdits {
            slot: 0,
            primary: true,
            clicked: None,
            carried: None,
            rest,
        });
        match step.after_menu_closed()?.0 {
            ClickStep::StackedCursorOnly { cursor, .. } => Some(cursor),
            _ => panic!("only cursor edits survive a closed menu"),
        }
    }

    #[test]
    fn a_closed_menu_keeps_only_the_cursor_edits_before_the_next_slot_edit() {
        let dirt = |count| Some(ItemStack::new("minecraft:dirt", count));
        // The take that suspended the click completed: its cursor write is owed.
        assert_eq!(
            owed(vec![StackedEdit::Carried(dirt(1))]),
            Some(vec![StackedEdit::Carried(dirt(1))])
        );
        // A later take can no longer run, so neither it nor the cursor write after it applies.
        assert_eq!(
            owed(vec![
                StackedEdit::Carried(dirt(1)),
                StackedEdit::Take(1),
                StackedEdit::Carried(dirt(2))
            ]),
            Some(vec![StackedEdit::Carried(dirt(1))])
        );
        // A second take that can no longer run would leave the cursor holding items the slot
        // never gave: nothing applies.
        assert_eq!(
            owed(vec![StackedEdit::Take(1), StackedEdit::Carried(dirt(2))]),
            None
        );
        assert_eq!(
            owed(vec![
                StackedEdit::Clicked(None),
                StackedEdit::Carried(dirt(2))
            ]),
            None
        );
        assert_eq!(owed(vec![]), None);
    }
}
