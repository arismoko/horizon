/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// A grindstone at `pos` made its result after its inputs changed, for
    /// the server player `player` (UUID) and `player_entity_id`
    /// (`GrindstoneEvent.OnPlaceItem`, in `GrindstoneMenu.createResult`):
    /// Lend `inputs` and let listeners stage only the result and experience
    /// `edits`. Set `cancel` for no result.
    /// Server-side.
    pub(in crate::game) fn native_grindstone_place(
        &mut self,
        player: u128,
        player_entity_id: i32,
        pos: [i32; 3],
        inputs: &mod_api::item::menu::GrindstonePlaceInputs<'_>,
        edits: &mut mod_api::item::menu::GrindstonePlaceEdits,
        cancel: &mut bool,
    ) {
        self.fire_grindstone_place(player, player_entity_id, pos, inputs, edits, cancel);
    }
}
