//! Equipment-aware movement helpers for `AbstractContainerMenu`.
use super::*;

/// Cursor of one vanilla moveItemStackTo pass. A station can run createResult
/// between destinations without restarting the pass or re-checking its route.
pub struct MovePass {
    start: usize,
    end: usize,
    backwards: bool,
    next: usize,
    filling: bool,
    pub(super) changed: bool,
}

impl MovePass {
    pub(super) fn new(start: usize, end: usize, backwards: bool) -> Self {
        Self {
            start,
            end,
            backwards,
            next: 0,
            filling: false,
            changed: false,
        }
    }

    fn next_slot(&mut self) -> Option<usize> {
        if self.next >= self.end - self.start {
            return None;
        }
        let offset = self.next;
        self.next += 1;
        Some(if self.backwards {
            self.end - 1 - offset
        } else {
            self.start + offset
        })
    }
}

impl Menu {
    pub(super) fn move_item_stack_to(
        &mut self,
        env: &Env,
        origin: Option<usize>,
        stack: &mut ItemStack,
        start: usize,
        end: usize,
        backwards: bool,
    ) -> bool {
        let mut pass = MovePass::new(start, end, backwards);
        self.move_from_source(env, origin, stack, &mut pass);
        pass.changed
    }

    pub(super) fn resume_station_transfer(
        &mut self,
        env: &Env,
        origin: usize,
        stack: &mut ItemStack,
        pass: &mut MovePass,
    ) {
        self.move_from_source(env, Some(origin), stack, pass);
    }

    fn move_from_source(
        &mut self,
        env: &Env,
        origin: Option<usize>,
        stack: &mut ItemStack,
        pass: &mut MovePass,
    ) {
        self.touch();
        if !pass.filling {
            while stack.count > 0 && Self::stackable(env, stack) {
                let Some(dest) = pass.next_slot() else { break };
                // Empty destinations belong only to the insertion pass. Do not ask their
                // placement rule with a different top-up maximum first.
                let Some(target) = self
                    .slot(dest)
                    .filter(|held| held.same_item_same_components(stack))
                else {
                    continue;
                };
                // A mod menu's result (unless a mod's hook opens it) and a
                // hidden slot get no top-up either, as a furnace's quick move
                // never aims at its result.
                let refuses = if self.state.slots[dest].role == Role::Hidden {
                    true
                } else if let Some(id) = self.registry_id() {
                    let own = (
                        self.state.slots[dest].role != Role::ModResult,
                        true,
                        CONTAINER_MAX,
                    );
                    !slot_context::apply_menu_slot_rules(
                        env.slot_rules,
                        id,
                        self.state.slots[dest].index,
                        stack,
                        self.slot_context(env, dest),
                        own,
                        Self::max_stack(env, stack),
                    )
                    .0
                } else {
                    false
                };
                if refuses {
                    continue;
                }
                let max = self.slot_max(env, dest, target);
                let slot = self.state.slots[dest];
                let target = self.state.containers[slot.container][slot.index]
                    .as_mut()
                    .expect("checked");
                let moved = stack.count.min(max.saturating_sub(target.count));
                if moved > 0 {
                    stack.count -= moved;
                    target.count += moved;
                    self.store_transfer_remainder(origin, stack);
                    pass.changed = true;
                    self.slot_changed(env, dest);
                    if self.result_pending() {
                        return;
                    }
                }
            }
            pass.filling = true;
            pass.next = 0;
        }
        if stack.count == 0 {
            return;
        }
        let mut chosen = None;
        let mut scan = |can_equip: Option<&dyn crate::mobs::equip::CanEquip>| {
            while let Some(dest) = pass.next_slot() {
                if self.has_item(dest) || !self.may_place_with(env, can_equip, dest, stack) {
                    continue;
                }
                let max = self.slot_max(env, dest, stack);
                if max != 0 {
                    chosen = Some((dest, max));
                    break;
                }
            }
        };
        match env.can_equip {
            Some(parts) if parts.listens(&stack.item) => {
                parts.with_stack(stack, &mut |scoped| scan(Some(scoped)));
            }
            can_equip => scan(can_equip),
        }
        if let Some((dest, max)) = chosen {
            let part = split(stack, stack.count.min(max));
            self.store_transfer_remainder(origin, stack);
            self.set_by_player(env, dest, Some(part));
            if matches!(self.kind, Kind::Anvil | Kind::Grindstone) {
                // moveItemStackTo also notifies after Slot.setByPlayer.
                self.slot_changed(env, dest);
            }
            pass.changed = true;
            pass.next = pass.end - pass.start;
        }
    }
    fn store_transfer_remainder(&mut self, origin: Option<usize>, stack: &ItemStack) {
        if let Some(origin) = origin {
            let slot = self.state.slots[origin];
            // This is an in-place ItemStack.shrink. The source slot's setChanged runs
            // only when quickMoveStack finishes, after the destination pass.
            self.state.containers[slot.container][slot.index] =
                (stack.count > 0).then(|| stack.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::equip::CanEquip;
    use std::cell::Cell;

    #[derive(Default)]
    struct FeetOnly {
        asks: Cell<usize>,
        scopes: Cell<usize>,
    }
    impl CanEquip for FeetOnly {
        fn can_equip(&self, _: &ItemStack, slot: &str, _: u32, _: &str, _: bool) -> bool {
            self.asks.set(self.asks.get() + 1);
            slot == "feet"
        }
        fn with_stack(&self, _stack: &ItemStack, body: &mut dyn FnMut(&dyn CanEquip)) {
            self.scopes.set(self.scopes.get() + 1);
            body(self);
        }
    }

    #[test]
    #[ignore = "daily"]
    fn four_slot_menu_transfer_asks_each_armor_slot_until_accepted() {
        super::super::tests::with_menu(Kind::Inventory, |env, menu| {
            let parts = FeetOnly::default();
            let env = Env {
                can_equip: Some(&parts),
                player_entity_id: Some(7),
                ..*env
            };
            let mut helmet = ItemStack::new("minecraft:iron_helmet", 3);
            assert!(menu.move_to(&env, None, &mut helmet, 5, 9, false));
            assert_eq!(parts.scopes.get(), 1);
            assert_eq!(parts.asks.get(), 4);
            assert_eq!(helmet.count, 2);
            assert_eq!(menu.slot(8).unwrap().count, 1);
            assert!((5..8).all(|slot| menu.slot(slot).is_none()));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn menus_without_actor_or_equipment_candidates_do_not_ask_can_equip() {
        super::super::tests::with_menu(Kind::Inventory, |env, menu| {
            let parts = FeetOnly::default();
            let env = Env {
                can_equip: Some(&parts),
                player_entity_id: None,
                ..*env
            };
            let mut helmet = ItemStack::new("minecraft:iron_helmet", 1);
            assert!(menu.move_to(&env, None, &mut helmet, 5, 6, false));
            assert_eq!(menu.slot(5).unwrap().item.as_ref(), "minecraft:iron_helmet");
            assert_eq!(parts.asks.get(), 0);
            let env = Env {
                player_entity_id: Some(7),
                ..env
            };
            menu.carried = Some(ItemStack::new("minecraft:stone", 3));
            for (slot, button) in [(-999, 0), (9, 1), (10, 1), (-999, 2)] {
                menu.click(&env, slot, button, Input::QuickCraft).unwrap();
            }
            assert_eq!(menu.slot(9).unwrap().count, 1);
            assert_eq!(menu.slot(10).unwrap().count, 1);
            assert_eq!(menu.carried.as_ref().unwrap().count, 1);
            assert_eq!(parts.asks.get(), 0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn occupied_equipment_cursor_and_hotbar_swaps_ask_once_per_click() {
        for input in [Input::Pickup, Input::Swap] {
            super::super::tests::with_menu(Kind::Inventory, |env, menu| {
                let parts = FeetOnly::default();
                let env = Env {
                    can_equip: Some(&parts),
                    player_entity_id: Some(7),
                    ..*env
                };
                menu.set_slot(&env, 8, Some(ItemStack::new("minecraft:diamond_boots", 1)));
                if input == Input::Pickup {
                    menu.carried = Some(ItemStack::new("minecraft:iron_helmet", 1));
                } else {
                    menu.set_slot(&env, 36, Some(ItemStack::new("minecraft:iron_helmet", 1)));
                }
                menu.click(&env, 8, 0, input).unwrap();
                assert_eq!(menu.slot(8).unwrap().item.as_ref(), "minecraft:iron_helmet");
                let returned = if input == Input::Pickup {
                    menu.carried.as_ref()
                } else {
                    menu.slot(36)
                };
                assert_eq!(returned.unwrap().item.as_ref(), "minecraft:diamond_boots");
                assert_eq!(parts.asks.get(), 1);
            });
        }
    }
}
