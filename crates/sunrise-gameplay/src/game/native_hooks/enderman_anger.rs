use mod_api::{EntityRef, PlayerRef};

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// `EnderManAngerEvent`: `player` stares at enderman `enderman`
    /// (`Enderman.isBeingStaredBy` passed). Set `cancel` and the stare
    /// doesn't count: no anger, no freezing in place, no teleport closer.
    pub(in crate::game) fn native_enderman_anger(
        &self,
        enderman: EntityRef,
        player: PlayerRef,
        cancel: &mut bool,
    ) {
        self.fire_enderman_anger(enderman, player, cancel);
    }
}
