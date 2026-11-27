//! The live slot view lent to a `menu_slot_rules` ask.
use super::{
    CONTAINER_MAX, Env, GRID, ItemStack, Kind, LinkedSlots, MAIN, Menu, PLAYER, RESULT, Role, Slot,
    SlotContext, SlotRules, item_max_stack, limit_slot_max_for, vanilla_may_pickup_for_role,
    vanilla_may_place_for_role, vanilla_slot_max_for_role,
};
use mod_api::MenuContainer;

/// The API's key for one of a menu's containers.
fn key(container: usize) -> MenuContainer {
    match container {
        MAIN => MenuContainer::Main,
        GRID => MenuContainer::Grid,
        RESULT => MenuContainer::Result,
        _ => MenuContainer::Player,
    }
}

/// The container a LAN view's slot `i` is in: the block's container's slots are `Main`, the
/// rest the player's.
pub fn lan_container(i: usize, container: Option<&[Option<ItemStack>]>) -> MenuContainer {
    match container {
        Some(c) if i < c.len() => MenuContainer::Main,
        _ => MenuContainer::Player,
    }
}

impl<'a> SlotContext<'a> {
    /// A blockless menu's live backing and player slots, borrowed for one slot-policy question.
    pub fn blockless(
        guest: &'a impl super::MenuGuest,
        pos: Option<[i32; 3]>,
        container: MenuContainer,
    ) -> Self {
        Self {
            player: Some(guest.uuid()),
            pos,
            container,
            slots: guest.native_menu(),
            slots_second: &[],
            linked: None,
            main_crafting: None,
            grid: &[],
            result: &[],
            player_slots: guest.inventory(),
            player_armor: Some(guest.armor()),
            player_offhand: Some(guest.offhand()),
            staged_slot: None,
        }
    }

    /// What slot `index` of `container` holds, keyed as `MenuSlotContext::slot`.
    pub fn slot(&self, container: MenuContainer, index: usize) -> Option<&'a ItemStack> {
        if let Some((staged_container, staged_index, stack)) = self.staged_slot
            && (container, index) == (staged_container, staged_index)
        {
            return stack;
        }
        match container {
            MenuContainer::Main => {
                if self.main_crafting.is_some_and(|crafting| {
                    crafting.grid_slots().contains(&index) || index == crafting.result
                }) {
                    None
                } else {
                    match self.linked.and_then(|linked| linked.get(index)) {
                        Some(linked) => linked,
                        None if index < self.slots.len() => self.slots.get(index),
                        None => self.slots_second.get(index - self.slots.len()),
                    }
                    .and_then(Option::as_ref)
                }
            }
            MenuContainer::Grid => self.grid.get(index).and_then(Option::as_ref),
            MenuContainer::Result => self.result.get(index).and_then(Option::as_ref),
            MenuContainer::Player => {
                if let Some(armor) = self.player_armor
                    && (36..40).contains(&index)
                {
                    armor[39 - index].as_ref()
                } else if let Some(offhand) = self.player_offhand
                    && index == 40
                {
                    offhand.as_ref()
                } else {
                    self.player_slots.get(index).and_then(Option::as_ref)
                }
            }
            _ => None,
        }
    }
}

impl Menu {
    /// Who and where this menu's `menu_slot_rules` ask about menu slot `s` is: the env's
    /// player, the menu's block (`set_block_pos`), `s`'s container and the menu's and player's
    /// containers.
    pub(super) fn slot_context<'s>(&'s self, env: &Env, s: usize) -> SlotContext<'s> {
        let container = self
            .state
            .slots
            .get(s)
            .map_or(PLAYER, |slot| slot.container);
        SlotContext {
            player: env.player,
            pos: self.state.block_pos,
            container: key(container),
            slots: &self.state.containers[MAIN],
            slots_second: &[],
            linked: None,
            main_crafting: self.mod_crafting(),
            grid: &self.state.containers[GRID],
            result: &self.state.containers[RESULT],
            player_slots: &self.state.containers[PLAYER],
            player_armor: None,
            player_offhand: None,
            staged_slot: None,
        }
    }
}

/// A resolved menu's borrowed slot state, for rule queries that do not edit the menu.
/// The caller validates the resolved menu and lends the containers
/// and the queried slot's state.
pub struct LiveSlotRules<'a> {
    pub kind: Kind,
    pub registry_id: Option<&'a str>,
    pub pos: Option<[i32; 3]>,
    pub main: &'a [Option<ItemStack>],
    pub main_second: &'a [Option<ItemStack>],
    pub linked: Option<LinkedSlots<'a>>,
    pub grid: &'a [Option<ItemStack>],
    pub result: &'a [Option<ItemStack>],
    pub player_slots: &'a [Option<ItemStack>],
    pub player_armor: &'a [Option<ItemStack>; 4],
    pub player_offhand: &'a Option<ItemStack>,
    pub container_limit: Option<u32>,
    pub crafter_disabled: bool,
    pub creative: bool,
}

impl<'a> LiveSlotRules<'a> {
    pub(super) fn slot_context(&self, env: &Env, slot: Slot) -> SlotContext<'a> {
        SlotContext {
            player: env.player,
            pos: self.pos,
            container: key(slot.container),
            slots: self.main,
            slots_second: self.main_second,
            linked: self.linked,
            main_crafting: match self.kind {
                Kind::Mod(t) => t.crafting,
                _ => None,
            },
            grid: self.grid,
            result: self.result,
            player_slots: self.player_slots,
            player_armor: Some(self.player_armor),
            player_offhand: Some(self.player_offhand),
            staged_slot: None,
        }
    }

    pub(super) fn vanilla_answers(
        &self,
        env: &Env,
        slot: Slot,
        stack: &ItemStack,
    ) -> (bool, bool, u32) {
        (
            vanilla_may_place_for_role(
                slot.role,
                env,
                env.can_equip,
                stack,
                self.crafter_disabled,
                None,
            ),
            vanilla_may_pickup_for_role(
                slot.role,
                self.kind,
                env,
                Some(stack),
                self.creative,
                None,
                false,
            ),
            vanilla_slot_max_for_role(self.kind, slot, stack),
        )
    }

    pub(super) fn excludes(&self, s: usize) -> bool {
        matches!(self.linked.and_then(|linked| linked.get(s)), Some(None))
    }

    /// What this menu's resolved container slot currently holds, in menu order.
    pub fn held_container_slot(&self, env: &Env, s: usize) -> Option<&'a ItemStack> {
        let slot = self.kind.slot(s)?;
        self.slot_context(env, slot)
            .slot(key(slot.container), slot.index)
    }

    /// The maximum count allowed by the live slot, or zero if it refuses placement.
    /// `None` means the index is not part of this menu layout.
    pub fn slot_holds(&self, env: &Env, s: usize, stack: &ItemStack) -> Option<u32> {
        let slot = self.kind.slot(s)?;
        if slot.role == Role::Hidden || self.excludes(s) {
            return Some(0);
        }
        let own = self.vanilla_answers(env, slot, stack);
        let item_max = item_max_stack(env.items, stack);
        let (place, _, max) = if let Some(id) = self.registry_id {
            apply_menu_slot_rules(
                env.slot_rules,
                id,
                slot.index,
                stack,
                self.slot_context(env, slot),
                own,
                item_max,
            )
        } else {
            (own.0, true, own.2.min(item_max))
        };
        Some(if place {
            limit_slot_max_for(self.kind, s, slot, self.container_limit, max)
        } else {
            0
        })
    }

    /// Whether the live slot gives up its stack, or `None` for an index outside the layout.
    pub fn slot_gives(&self, env: &Env, s: usize, held: Option<&ItemStack>) -> Option<bool> {
        let slot = self.kind.slot(s)?;
        let own = vanilla_may_pickup_for_role(
            slot.role,
            self.kind,
            env,
            held,
            self.creative,
            None,
            false,
        );
        if !own || self.excludes(s) {
            return Some(false);
        }
        let (Some(stack), Some(id)) = (held, self.registry_id) else {
            return Some(true);
        };
        Some(
            apply_menu_slot_rules(
                env.slot_rules,
                id,
                slot.index,
                stack,
                self.slot_context(env, slot),
                self.vanilla_answers(env, slot, stack),
                item_max_stack(env.items, stack),
            )
            .1,
        )
    }
}

/// Apply the scoped rule to a slot's own answers (`may_place`, `may_pickup` and the maximum, in
/// that order), then enforce its item cap. The rules are asked directly at each live slot check,
/// only for a menu type a part answers; with none lent the slot's answers stand.
pub fn apply_menu_slot_rules(
    rules: Option<&dyn SlotRules>,
    menu: &str,
    slot: usize,
    stack: &ItemStack,
    context: SlotContext<'_>,
    own: (bool, bool, u32),
    item_max: u32,
) -> (bool, bool, u32) {
    let (mut may_place, mut may_pickup, mut max_stack) = own;
    if let Some(rules) = rules.filter(|rules| rules.listens(menu)) {
        rules.rule(
            menu,
            slot,
            stack,
            context,
            &mut may_place,
            &mut may_pickup,
            &mut max_stack,
        );
    }
    let max_stack = max_stack.min(CONTAINER_MAX).min(item_max);
    (may_place, may_pickup, max_stack)
}
