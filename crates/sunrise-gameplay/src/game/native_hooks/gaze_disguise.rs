use super::*;

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// `IItemExtension.isGazeDisguise`: whether `player`'s head item disguises
    /// their gaze from mob `observer` (an enderman's stare, an awake creaking's
    /// look). `disguised` comes in as vanilla's `#gaze_disguise_equipment` test.
    pub(in crate::game) fn native_gaze_disguise(
        &self,
        observer: EntityRef,
        player: PlayerRef,
        disguised: &mut bool,
    ) {
        self.fire_gaze_disguise(observer, player, disguised);
    }
}
