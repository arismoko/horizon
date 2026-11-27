use crate::game::ServerGame;
use sunrise_gameplay_common::game::item_abilities::ability;
use sunrise_sim::mobs::happy_ghast;

impl ServerGame<'_> {
    /// `HappyGhast.mobInteract`. Returns whether the click was used.
    pub fn interact_happy_ghast(&mut self, index: usize, item: &str) -> bool {
        let m = &self.common.mobs[index];
        if !m.alive() || Some(m.id) == self.common.riding {
            return false;
        }
        if m.is_baby() {
            // Animal.mobInteract: food grows it up.
            return false;
        }
        let at = m.center();
        let sneaking = self.server_acting_sneak();
        let harnessed = m.extra.body_id().is_some();
        // Equippable.equipOnInteract: a harness goes on a bare adult.
        if item.ends_with("_harness") && item.starts_with("minecraft:") && !harnessed {
            let held = self.common.inventory[self.common.selected]
                .clone()
                .map(|mut stack| {
                    stack.count = 1;
                    stack
                });
            self.common.mobs[index].extra.set_body_stack(held);
            self.server_play_sound("minecraft:entity.happy_ghast.equip", at, 1.0, 1.0);
            self.consume_held();
            let id = self.common.mobs[index].id;
            let pos = self.common.mobs[index].position;
            self.mob_equip_event(id, [pos.x, pos.y, pos.z], true);
            return true;
        }
        // Equippable.canBeSheared: shears take the harness off again.
        if self.held_shears(ability::SHEARS_REMOVE_ARMOR)
            && harnessed
            && !sneaking
            && let Some(harness) = self.common.mobs[index].take_body_equipment()
        {
            self.server_play_sound("minecraft:entity.happy_ghast.unequip", at, 1.0, 1.0);
            self.spawn_item(harness, at, [0.0, 0.2, 0.0], 10);
            self.damage_held_tool();
            let id = self.common.mobs[index].id;
            let pos = self.common.mobs[index].position;
            self.mob_equip_event(id, [pos.x, pos.y, pos.z], false);
            return true;
        }
        if harnessed && !sneaking {
            // doPlayerRide: `player.startRiding(this)`, which keeps the
            // rider's view (`ServerPlayer.startRiding`'s `Relative.ROTATION`).
            let id = self.common.mobs[index].id;
            // `startRiding` without `force` asks `canAddPassenger`: a fifth rider is refused.
            let aboard = self.seated_passengers(id).len();
            if !sunrise_sim::mobs::riders::can_add_passenger(&self.common.mobs[index], aboard)
                || !self.native_may_board(id)
            {
                return true;
            }
            self.break_leash(id, true, false);
            let was_empty = self.common.mobs[index].rider.is_none();
            // addPassenger: the rider is not standing on top, so no still
            // timeout.
            self.common.mobs[index].extra.counter = 0;
            self.common.mount.reset_jump();
            if was_empty {
                let down = happy_ghast::goggles(true, &self.common.mobs[index]);
                let _ = self.handle_mob_events(vec![down]);
            }
            self.start_riding(id);
            return true;
        }
        false
    }
}

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;
