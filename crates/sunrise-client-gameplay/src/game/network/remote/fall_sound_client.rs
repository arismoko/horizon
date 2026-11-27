#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn block_fall(
        &mut self,
        event: sunrise_sim::landing::FallEvent,
    ) {
        let (position, state) = match event {
            sunrise_sim::landing::FallEvent::Sound {
                sound,
                volume,
                pitch,
                ..
            } => {
                if let Some(name) = sound
                    .player_fall_sound()
                    .or_else(|| sound.honey_landing_sound())
                {
                    self.play_client_player_sound(name, volume, pitch);
                }
                return;
            }
            sunrise_sim::landing::FallEvent::BlockFallSound { position, state } => {
                (position, state)
            }
            _ => return,
        };
        // `LivingEntity.playBlockFallSound` returns early for a silent entity; the
        // server syncs the local player's Silent flag (entity data index 4).
        if self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.local_entity_silent)
        {
            return;
        }
        let sound = self.own_block_sound(
            [position.x, position.y, position.z],
            state,
            mod_api::BlockSoundCause::Fall,
        );
        self.play_client_player_sound(sound.event.as_str(), sound.volume * 0.5, sound.pitch * 0.75);
    }
}
