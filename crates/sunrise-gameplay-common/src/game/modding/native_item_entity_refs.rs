//! Dropped-item narrowing entry point for native mods (`EntityRef -> ItemEntityRef`).
use mod_api::{EntityRef, ItemEntityRef};

pub trait NativeItemEntityRefsGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Narrows an entity to a dropped item, or None for another kind, a
    /// stale handle, a client, or an item temporarily taken out for ticking.
    fn item_entity(&self, entity: EntityRef) -> Option<ItemEntityRef> {
        self.side_item_entity(entity)
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeItemEntityRefsGameExt1<'__sg> for G {}

use crate::game::side_ops::modapi_modding::ModapiModdingSideGameExt as _;
