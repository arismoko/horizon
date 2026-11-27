use crate::game::ClientGame;
use sunrise_sim::mobs::zombie_villager;

impl ClientGame<'_> {
    /// `ZombieVillager.handleEntityEvent(16)`: only the client plays the cure sound.
    pub(in crate::game::client_side) fn zombie_villager_cure_sound(&mut self, id: u32) {
        if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id)
            && let Some(sunrise_sim::mobs::MobEvent::Sound(name, at, volume, pitch)) =
                zombie_villager::cure_sound(m)
        {
            self.play_audio_unshared(&format!("minecraft:{name}"), Some(at), volume, pitch);
        }
    }
}
