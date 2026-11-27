use crate::game::ClientGame;
use sunrise_gameplay_common::game::spear::OWN;
use sunrise_sim::spear_sim::Kinetic;

impl ClientGame<'_> {
    /// A sound at the player's feet.
    pub(in crate::game) fn client_play_at_feet(&mut self, sound: &str) {
        if let Some(p) = &self.common.player {
            let at = [p.position.x, p.position.y, p.position.z];
            self.play_audio(sound, Some(at), 1.0, 1.0);
        }
    }
    /// `getTicksSinceLastKineticHitFeedback` of this window's player.
    pub(in crate::game) fn kinetic_hit_ticks(&self, partial: f32) -> f32 {
        self.common
            .kinetic_feedback
            .get(&OWN)
            .map_or(0.0, |&t| (self.common.world.game_time - t) as f32 + partial)
    }
    /// `LivingEntity.handleEntityEvent(2)` on a server's client: the
    /// player `id` (this one, or another) was hit by the spear it uses.
    pub(in crate::game) fn remote_kinetic_hit(&mut self, id: i32) {
        let own = self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.entity_id == id);
        let (item, at) = if own {
            let item = self
                .common
                .using
                .and_then(|(slot, _)| self.common.stack_in(slot))
                .map(|s| s.item.clone());
            let at = self.common.player.as_ref().map(|p| p.position);
            (item, at)
        } else {
            let mob = self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id as u32 && m.extra.using_item > 0);
            let item = mob.and_then(|m| {
                if m.extra.using_off_hand {
                    sunrise_sim::mobs::finalize::item(m, sunrise_sim::mobs::finalize::OFFHAND)
                } else {
                    sunrise_sim::mobs::finalize::item(m, sunrise_sim::mobs::finalize::MAINHAND)
                }
            });
            (item, mob.map(|m| m.position))
        };
        if let (Some(kinetic), Some(at)) = (item.as_deref().and_then(Kinetic::of), at)
            && self.common.kinetic_hit_feedback(if own { OWN } else { id })
        {
            self.play_audio_unshared(kinetic.hit_sound, Some(at.components()), 1.0, 1.0);
        }
    }
}
