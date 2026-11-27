impl crate::game::ClientGame<'_> {
    /// The columns this update's block edits touch, summarized again by far terrain when
    /// it is on.
    pub(in crate::game::client_side) fn observe_light_edits(&mut self) {
        if self.client.lod.0.is_some() {
            for &((x, _, z), _, _) in &self.common.world.light_edits {
                self.client.lod_observe.insert((x >> 4, z >> 4));
            }
        }
    }
}
