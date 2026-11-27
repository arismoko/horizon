//! Powder snow around the player: its snowflakes, fire put out by it, and the
//! bucket that picks it up (`BucketItem.use` on a `BucketPickup` block). The
//! sinking, slowing, climbing and freezing are `PlayerMotion`'s
//! (`crate::motion`); placing it is the powder snow bucket's `BlockItem.place`.

use sunrise_core::random::RandomSource;
use sunrise_items::items::ItemStack;
use sunrise_sim::cauldron;

/// The two ordered float draws of `Entity.playEntityOnFireExtinguishedSound`.
pub fn extinguish_pitch(random: &mut impl RandomSource) -> f32 {
    1.6 + (random.next_float() - random.next_float()) * 0.4
}

impl crate::game::CommonGame<'_> {
    /// Whether the block at `pos` is powder snow.
    pub fn is_powder_snow(&self, pos: [i32; 3]) -> bool {
        &*self
            .res
            .facts()
            .get(self.world.block(pos[0], pos[1], pos[2]))
            .name
            == "minecraft:powder_snow"
    }
}

/// `Inventory.add`'s answer once `give`, its passes until one adds nothing, left `rest` of
/// `stack`: true when all of it went in. Otherwise the last pass added nothing, so it is false
/// with the rest left in `stack` for the caller to drop, except in creative, where the rest is
/// cleared and it is true.
pub(in crate::game) fn added(
    stack: &mut ItemStack,
    rest: Option<ItemStack>,
    creative: bool,
) -> bool {
    stack.count = rest.map_or(0, |rest| rest.count);
    if stack.count > 0 && creative {
        stack.count = 0;
    }
    stack.count == 0
}

/// The player acting, holding the stack in `slot` (`stack_in_mut`'s numbers), as
/// `ItemUtils.createFilledResult` and the cauldron see them.
pub struct HeldSlot<'g, 'a, S: crate::game::SideGame<'a>> {
    pub game: &'g mut S,
    pub slot: usize,
    /// The UUID of the player a mod's write acts as: what doesn't fit waits for the root
    /// commit of an open transaction. None for the game's own use.
    pub dropper: Option<u128>,
    pub resource_lifetime: std::marker::PhantomData<fn(&'a ()) -> &'a ()>,
}
impl<'__carrier, S: crate::game::SideGame<'__carrier>> cauldron::Player
    for HeldSlot<'_, '__carrier, S>
{
    fn creative(&self) -> bool {
        self.game.creative
    }
    fn hand(&mut self) -> &mut Option<ItemStack> {
        self.game.stack_in_mut(self.slot)
    }
    fn add(&mut self, stack: &mut ItemStack) -> bool {
        if stack.count == 0 {
            return false;
        }
        let rest = self.game.add_stack_to_inventory(stack.clone());
        added(stack, rest, self.game.creative)
    }
    fn contains(&self, stack: &ItemStack) -> bool {
        self.game
            .inventory
            .iter()
            .flatten()
            .any(|have| have.same_item_same_components(stack))
    }
    fn drop_stack(&mut self, stack: ItemStack) {
        match self.dropper {
            Some(uuid) => self.game.drop_after_commit(uuid, stack),
            None => {
                self.game.drop_plain(stack);
            }
        }
    }
}

use crate::game::transactions::TransactionsGameExt as _;

use crate::game::held_items::HeldItemsGameExt1 as _;

use crate::game::resident_state::ResidentStateGameExt1 as _;

pub trait FilledResultGameExt<'g>: crate::game::SideGame<'g> {
    /// `ItemUtils.createFilledResult(held, player, filled)` for the held stack.
    fn filled_result(&mut self, filled: ItemStack) {
        let slot = self.held_slot();
        let mut held = HeldSlot {
            game: self,
            slot,
            dropper: None,
            resource_lifetime: std::marker::PhantomData,
        };
        cauldron::create_filled_result(&mut held, filled, true);
    }
}
impl<'g, G: crate::game::SideGame<'g>> FilledResultGameExt<'g> for G {}

#[cfg(test)]
pub(in crate::game) mod tests {
    use sunrise_items::items::ItemStack;

    /// An inventory with room for 3 more arrows: one stack of 61 and stone everywhere else.
    fn room_for_three() -> Vec<Option<ItemStack>> {
        let mut inventory = vec![Some(ItemStack::new("minecraft:stone", 64)); 36];
        inventory[0] = Some(ItemStack::new("minecraft:arrow", 61));
        inventory
    }

    #[test]
    fn a_partial_add_leaves_the_rest_to_drop_in_survival_and_clears_it_in_creative() {
        for creative in [false, true] {
            let (mut inventory, mut offhand) = (room_for_three(), None);
            let mut arrows = ItemStack::new("minecraft:arrow", 5);
            let left = crate::game::inventory_add(&mut inventory, &mut offhand, 0, 64, &mut arrows);
            let rest = left.then(|| arrows.clone());
            assert_eq!(inventory[0].as_ref().map(|s| s.count), Some(64));
            let added = super::added(&mut arrows, rest, creative);
            // Survival: false, with the 2 that found no slot left for `createFilledResult` to
            // drop. Creative: they are cleared, and it is true.
            assert_eq!(
                (added, arrows.count),
                if creative { (true, 0) } else { (false, 2) }
            );
        }
        let mut arrows = ItemStack::new("minecraft:arrow", 2);
        assert!(super::added(&mut arrows, None, false));
        assert_eq!(arrows.count, 0);
    }
}
