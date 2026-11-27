//! Happy ghasts on the game side: putting a harness on (and shearing it
//! off), riding, getting off on top, and the ghastling a dried ghast block
//! hatches.
use crate::game::ServerGame;
use mod_api::SoundSource;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::level_sounds_common::LevelSound;
use sunrise_sim::mobs::{MobEvent, happy_ghast};
use sunrise_world::registry::BlockStateId;

impl ServerGame<'_> {
    /// Getting off lands on top of it.
    pub(in crate::game) fn dismount_happy_ghast(&mut self, id: u32) {
        // removePassenger: held still for half a second, and with no one
        // aboard the ghast forgets its home.
        if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
            m.extra.counter = 10;
            m.ai.clear_home();
        }
        let Some(m) = self.common.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let top = crate::game::mounts::on_top(m);
        let up: MobEvent = happy_ghast::goggles(false, m);
        if let Some(p) = self.common.player.as_mut() {
            p.position = top;
            p.velocity = Vec3::ZERO;
        }
        let _ = self.handle_mob_events(vec![up]);
    }
    /// `DriedGhastBlock.spawnGhastling`: a ghastling facing the way the block did.
    pub(in crate::game) fn spawn_ghastling(&mut self, pos: [i32; 3], state: BlockStateId) {
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]),
            f64::from(pos[2]) + 0.5,
        ];
        let facing = self
            .common
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("facing").cloned());
        let yaw = match facing.as_deref() {
            Some("north") => 180.0,
            Some("east") => 270.0,
            Some("west") => 90.0,
            _ => 0.0,
        };
        if let Some(id) = self.spawn_mob_raw("minecraft:happy_ghast", at, false)
            && let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id)
        {
            // DriedGhastBlock.spawnGhastling sets the baby age before
            // addFreshEntity, so ageBoundaryReached runs the ghastling setup
            // (`babyGhastSetup`) while the mob is still being hatched: its
            // goals, navigation and brain are a ghastling's before its first
            // tick, not an adult's until the next aiStep.
            sunrise_sim::mobs::finalize::set_age(m, -24000, None);
            m.yaw = yaw;
            m.body_yaw = yaw;
            m.head_yaw = yaw;
            // DriedGhastBlock.spawnGhastling: level.playSound(null, ghastling,
            // GHASTLING_SPAWN, BLOCKS, 1, 1).
            let (id, at) = (m.id, [m.position.x, m.position.y, m.position.z]);
            let sound = LevelSound::new("minecraft:entity.ghastling.spawn", at, 1.0, 1.0)
                .in_source(SoundSource::Blocks);
            self.play_bound_sound(sound, id, None);
        }
    }
}

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;
