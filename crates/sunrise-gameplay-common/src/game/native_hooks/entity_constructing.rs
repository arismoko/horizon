use super::*;

pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// `EntityEvent.EntityConstructing`: a new entity of type `kind`, whose attachments are
    /// `holder`'s, is being made on this side (see `modding/entity_initialization.rs`).
    fn native_entity_constructing(&mut self, holder: mod_api::Holder, kind: &str) {
        crate::game::native_hooks::fire::FireHooks::fire_entity_constructing(self, holder, kind);
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
