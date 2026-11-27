/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// The server player `player` (UUID) and `player_entity_id` took a
    /// grindstone's result at `pos` (`GrindstoneEvent.OnTakeItem`, in its
    /// result slot's `onTake`): lend the pre-take `inputs` and let listeners
    /// stage experience and remainder `edits`. Set `cancel` to keep the click
    /// but skip experience, sound and input replacement. Server-side.
    pub(in crate::game) fn native_grindstone_take(
        &mut self,
        player: u128,
        player_entity_id: i32,
        pos: [i32; 3],
        inputs: &mod_api::item::menu::GrindstoneTakeInputs<'_>,
        edits: &mut mod_api::item::menu::GrindstoneTakeEdits,
        cancel: &mut bool,
    ) {
        self.fire_grindstone_take(player, player_entity_id, pos, inputs, edits, cancel);
    }
}
