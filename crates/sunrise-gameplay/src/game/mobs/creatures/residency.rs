//! Cleanup of constructor state that never joined a resident owner.
impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn discard_unjoined_mob<T>(&mut self, id: u32) -> Option<T> {
        self.forget_entity(id);
        None
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn clear_pending_mob_actions(&mut self) {
        self.common.creatures.pending.clear();
    }
}
