use crate::game::ServerGame;
use sunrise_core::random::RandomSource;
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::{Mob, Species, Who, sniffer};

impl ServerGame<'_> {
    /// A snifflet where the egg was.
    pub(in crate::game) fn hatch_sniffer(&mut self, pos: [i32; 3]) {
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        if let Some(id) = self.spawn_mob_raw("minecraft:sniffer", at, true)
            && let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id)
        {
            m.extra.age = sniffer::BABY_START_AGE;
            let yaw = self.common.world.random.next_float() * 360.0 - 180.0;
            m.yaw = yaw;
            m.body_yaw = yaw;
            m.head_yaw = yaw;
        }
    }
    /// The egg item at the parent (`Sniffer.spawnChildFromBreeding`; the
    /// brain already played the plop and dropped the experience).
    pub fn lay_sniffer_egg(&mut self, parent: &Mob) {
        let at = [parent.position.x, parent.position.y, parent.position.z];
        // new ItemEntity(level, x, y, z, stack): the little push is the item's
        // own random's, not the level's.
        self.spawn_item_own(ItemStack::new("minecraft:sniffer_egg", 1), at, 10);
    }
    /// The sniffer's server-side digging effects: `Sniffer.onDiggingStart`
    /// entity event 63 and periodic `minecraft:entity_action` vibrations.
    pub fn tick_sniffer_server(&mut self) {
        assert!(
            self.is_logical_server(),
            "tick_sniffer_server called on Client role"
        );
        let players = self.level_players();
        if players.is_empty() {
            return;
        }
        for i in 0..self.common.mobs.len() {
            let m = &self.common.mobs[i];
            if m.species() != Species::Sniffer
                || !m.alive()
                || !players
                    .iter()
                    .any(|p| m.position.distance_squared(p.position) <= 32.0 * 32.0)
            {
                continue;
            }
            if !sniffer::digging(m) {
                continue;
            }
            let id = m.id;
            let at = [m.position.x, m.position.y, m.position.z];
            let age = m.age;
            if let Some(ms) = m.anim.states[3].millis(age as f32)
                && ms == 0
            {
                // Sniffer.onDiggingStart: broadcast entity event 63.
                self.lan_entity_event(id as i32, 63, at);
            }
            // GameEvent.ENTITY_ACTION for the whole dig, outside the dust window.
            if age.is_multiple_of(10) {
                let head = sniffer::head_block(&self.common.mobs[i]);
                self.post_block_game_event(
                    "minecraft:entity_action",
                    head,
                    Some(Who::Mob(id)),
                    None,
                );
            }
        }
    }
}
