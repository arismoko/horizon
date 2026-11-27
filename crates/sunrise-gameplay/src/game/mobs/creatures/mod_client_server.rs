use crate::game::ServerGame;
use sunrise_gameplay_common::game::item_abilities::{ItemAbilitiesGameExt1 as _, ability};
use sunrise_sim::mobs::{Species, Who};

impl ServerGame<'_> {
    /// `Armadillo.mobInteract`: a brush brushes a scute off an adult
    /// (`brushOffScute`) and loses 16 durability.
    pub(in crate::game) fn interact_armadillo(&mut self, index: usize, item: &str) -> bool {
        let m = &self.common.mobs[index];
        if !self.common.held().is_some_and(|s| {
            self.stack_can_perform(s, ability::BRUSH_BRUSH, item == "minecraft:brush")
        }) || m.species() != Species::Armadillo
            || !m.alive()
            || m.is_baby()
        {
            return false;
        }
        let (id, at) = (m.id, m.position);
        let feet = [at.x, at.y, at.z];
        let tool = self.common.held().cloned();
        let table = "minecraft:brush/armadillo";
        let mut stacks = self.sequence_loot(table, table, tool.as_ref());
        let kind = crate::game::native_hooks::mob_kind(&self.common.mobs[index]);
        let player = Some(self.native_acting_id());
        self.native_use_drops(
            table,
            feet,
            None,
            Some((id, &kind)),
            player,
            tool.as_ref(),
            &mut stacks,
        );
        for d in stacks {
            self.spawn_at_location(at, 0.0, d);
        }
        self.server_play_sound("minecraft:entity.armadillo.brush", feet, 1.0, 1.0);
        let ctx = self.who_context(Some(Who::Mob(id)), None);
        self.post_game_event("minecraft:entity_interact", feet, ctx);
        // ItemStack.hurtAndBreak(16).
        if !self.common.creative
            && let Some(stack) = tool
        {
            let max = self
                .common
                .res
                .items()
                .get(&stack.item)
                .map_or(0, |i| i.max_damage);
            if max > 0 {
                self.damage_held(16);
            }
        }
        true
    }
}

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;
