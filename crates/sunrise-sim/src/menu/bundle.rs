//! Vanilla `Item.overrideStackedOnOther` / `Item.overrideOtherStackedOnMe` (`BundleItem`'s click
//! overrides in `AbstractContainerMenu.tryItemClickBehaviourOverride`), shared by both local/client
//! `Menu` clicks and hosted LAN `View` clicks.
use super::container_items::{can_be_in_bundle, fits_inside_container_items};
use super::{Env, ItemStack, LoopContext, Menu, SlotView, tidy};
use crate::bundle::{Contents, INSERT, INSERT_FAIL, REMOVE_ONE, toggle_selected};
use crate::items::{ItemInfo, Items};

/// Shared vanilla `Item.overrideStackedOnOther` (`BundleItem.overrideStackedOnOther`).
///
/// Vanilla 26.3 dispatches by item class (`Item.overrideStackedOnOther` returns `false`; only
/// `BundleItem` overrides it), so a non-bundle item carrying `minecraft:bundle_contents` — even as
/// a default component — does not trigger this override. Returns `true` when the carried item's
/// vanilla override handled the click.
pub fn vanilla_carried_override(
    items: &Items,
    carried: &mut Option<ItemStack>,
    slot: &mut dyn SlotView,
    loop_context: &LoopContext,
    primary: bool,
    sounds: &mut Vec<&'static str>,
) -> bool {
    let Some(mut contents) = carried
        .as_ref()
        .filter(|c| c.count == 1 && items.get(&c.item).is_some_and(ItemInfo::is_bundle_item))
        .and_then(|c| Contents::of(items, c))
    else {
        return false;
    };
    match (slot.item(), primary) {
        (Some(other), true) => {
            // tryTransfer: what fits, taken by the slot's rules.
            let mut moved = 0;
            if can_be_in_bundle(loop_context, &other) {
                let room = contents.room_for(items, &other);
                if let Some(mut taken) = slot.safe_take(other.count, room) {
                    // `tryInsert` asks `canItemBeInBundle` again: the answer above.
                    moved = contents.try_insert(items, &mut taken, true);
                }
            }
            sounds.push(if moved > 0 { INSERT } else { INSERT_FAIL });
        }
        (None, false) => {
            if let Some(stack) = contents.remove_one(items) {
                let count = stack.count;
                let remainder = slot.safe_insert(Some(stack), count);
                match remainder {
                    Some(mut rest) => {
                        let fits = fits_inside_container_items(loop_context, &rest);
                        contents.try_insert(items, &mut rest, fits);
                    }
                    None => sounds.push(REMOVE_ONE),
                }
            }
        }
        _ => return false,
    }
    if let Some(c) = carried.as_mut() {
        contents.store(items, c);
    }
    true
}

/// Shared vanilla `Item.overrideOtherStackedOnMe` (`BundleItem.overrideOtherStackedOnMe`).
///
/// Vanilla 26.3 dispatches by item class (`ItemInfo::is_bundle_item`). Returns
/// `(handled, updated_slot_stack)`: when `updated_slot_stack` is `Some`, the caller writes it back
/// to the clicked slot in place (an unmatched click still clears `toggleSelectedItem(-1)` while
/// returning `handled = false`).
pub fn vanilla_clicked_override(
    items: &Items,
    carried: &mut Option<ItemStack>,
    slot: &dyn SlotView,
    loop_context: &LoopContext,
    primary: bool,
    sounds: &mut Vec<&'static str>,
) -> (bool, Option<ItemStack>) {
    let Some(mut bundle) = slot.item() else {
        return (false, None);
    };
    // `BundleItem` refuses a stack of several bundles in both overrides (`getCount() != 1`).
    if bundle.count != 1
        || !items
            .get(&bundle.item)
            .is_some_and(ItemInfo::is_bundle_item)
    {
        return (false, None);
    }
    let Some(mut contents) = Contents::of(items, &bundle) else {
        return (false, None);
    };
    let allowed = slot.allow_modification();
    match (carried.take(), primary) {
        (Some(mut other), true) => {
            let moved = if allowed {
                let fits = fits_inside_container_items(loop_context, &other);
                contents.try_insert(items, &mut other, fits)
            } else {
                0
            };
            sounds.push(if moved > 0 { INSERT } else { INSERT_FAIL });
            *carried = Some(other);
            tidy(carried);
        }
        (None, false) => {
            if allowed && let Some(removed) = contents.remove_one(items) {
                sounds.push(REMOVE_ONE);
                *carried = Some(removed);
            }
        }
        (prev, _) => {
            // toggleSelectedItem(-1), then the click goes on.
            *carried = prev;
            toggle_selected(items, &mut bundle, -1);
            return (false, Some(bundle));
        }
    }
    contents.store(items, &mut bundle);
    (true, Some(bundle))
}

struct MenuSlotView<'a, 'e> {
    menu: &'a mut Menu,
    env: &'a Env<'e>,
    slot: usize,
}

impl SlotView for MenuSlotView<'_, '_> {
    fn with_staged(&mut self, edit: &mut dyn FnMut(&mut dyn SlotView) -> bool) -> bool {
        let saved = self.menu.clone();
        let accepted = edit(self);
        if !accepted {
            *self.menu = saved;
        }
        accepted
    }

    fn items(&self) -> &Items {
        self.env.items
    }

    fn item(&self) -> Option<ItemStack> {
        self.menu.slot(self.slot).cloned()
    }

    fn may_place(&self, stack: &ItemStack) -> bool {
        self.menu.may_place(self.env, self.slot, stack)
    }

    fn may_pickup(&self) -> bool {
        self.menu.may_pickup(self.env, self.slot)
    }

    fn allow_modification(&self) -> bool {
        self.menu.allow_modification(self.env, self.slot)
    }

    fn safe_insert(&mut self, mut stack: Option<ItemStack>, count: u32) -> Option<ItemStack> {
        self.menu
            .safe_insert(self.env, self.slot, &mut stack, count);
        tidy(&mut stack);
        stack
    }

    fn safe_take(&mut self, count: u32, max: u32) -> Option<ItemStack> {
        self.menu.safe_take(self.env, self.slot, count, max)
    }
}

impl Menu {
    /// The carried stack's `overrideStackedOnOther`, then the clicked
    /// one's `overrideOtherStackedOnMe`. True when a bundle took the click.
    pub(super) fn bundle_click(&mut self, env: &Env, s: usize, primary: bool) -> bool {
        self.touch();
        let took = self.carried_bundle_click(env, s, primary)
            || self.clicked_bundle_click(env, s, primary);
        if took {
            // broadcastChangesOnContainerMenu: the open menu's
            // slotsChanged for the player's inventory.
            self.inventory_slots_changed(env);
        }
        took
    }

    /// `BundleItem.overrideStackedOnOther`: the carried bundle on a slot.
    pub(super) fn carried_bundle_click(&mut self, env: &Env, s: usize, primary: bool) -> bool {
        self.touch();
        let mut carried = self.state.carried.clone();
        let mut sounds = Vec::new();
        let took = {
            let mut slot = MenuSlotView {
                menu: self,
                env,
                slot: s,
            };
            vanilla_carried_override(
                env.items,
                &mut carried,
                &mut slot,
                env.loop_context,
                primary,
                &mut sounds,
            )
        };
        if took {
            self.state.carried = carried;
            self.state.player_sounds.extend(sounds);
        }
        took
    }

    /// `BundleItem.overrideOtherStackedOnMe`: a click on a bundle in a slot.
    pub(super) fn clicked_bundle_click(&mut self, env: &Env, s: usize, primary: bool) -> bool {
        self.touch();
        let mut carried = self.state.carried.clone();
        let mut sounds = Vec::new();
        let (took, updated) = {
            let slot = MenuSlotView {
                menu: self,
                env,
                slot: s,
            };
            vanilla_clicked_override(
                env.items,
                &mut carried,
                &slot,
                env.loop_context,
                primary,
                &mut sounds,
            )
        };
        if let Some(bundle) = updated {
            let slot = self.state.slots[s];
            self.state.containers[slot.container][slot.index] = Some(bundle);
        }
        if took {
            self.state.carried = carried;
            self.state.player_sounds.extend(sounds);
        }
        took
    }

    /// Probe the real vanilla carried override (`vanilla_carried_override`) on a throwaway copy of
    /// `self` with `(carried, clicked)` staged into the cursor and slot `s`, returning whether the
    /// carried item's vanilla override handled the click.
    pub fn probe_carried_vanilla_override(
        &self,
        env: &Env,
        s: usize,
        primary: bool,
        carried: Option<ItemStack>,
        clicked: Option<ItemStack>,
    ) -> bool {
        if s >= self.state.slots.len() {
            return false;
        }
        let mut copy = self.clone();
        copy.state.carried = carried;
        let slot = copy.state.slots[s];
        copy.state.containers[slot.container][slot.index] = clicked;
        copy.carried_bundle_click(env, s, primary)
    }
}
