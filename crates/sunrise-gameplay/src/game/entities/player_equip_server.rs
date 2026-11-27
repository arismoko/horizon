use crate::game::ServerGame;

impl ServerGame<'_> {
    pub fn put_equipment_modifiers(&mut self) {
        let mut applied = std::mem::take(&mut self.server.applied_modifiers);
        let ran = self.apply_equipment_modifiers_with(&mut applied);
        self.server.applied_modifiers = applied;
        if !ran {
            return;
        }
        if let Some(player) = self.common.player.as_mut() {
            let _ = player.clamp_health_to_max();
        }
        self.apply_enchantment_attributes();
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
