#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Collect the local player's skin and arm width together, before render extraction.
    pub(in crate::game) fn native_resolve_player_skin(
        &self,
        selection: &mut Option<mod_api::PlayerSkinOverride>,
    ) {
        self.fire_resolve_player_skin(selection);
    }
}
