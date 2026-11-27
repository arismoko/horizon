#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn native_play_hit_sound(
        &mut self,
        call: super::fire::PlayHitSoundCall<'_>,
    ) {
        self.fire_play_hit_sound(call);
    }
}
