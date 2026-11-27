use super::*;

/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// `PlayLevelSoundEvent.AtPosition` before server broadcast or client playback.
    fn native_play_level_sound_at_position(
        &mut self,
        call: mod_api::__private::PlayLevelSoundAtPositionCall<'_>,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_play_level_sound_at_position(self, call);
    }
    /// `PlayLevelSoundEvent.AtEntity` for a sound following an entity on either side.
    fn native_play_level_sound_at_entity(
        &mut self,
        call: mod_api::__private::PlayLevelSoundAtEntityCall<'_>,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_play_level_sound_at_entity(self, call);
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
