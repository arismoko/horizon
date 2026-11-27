//! Active item ownership retained across a mutable listener dispatch.
use super::Items;
use std::{ops::Deref, sync::Arc};

/// Retain the selected owner without copying its item registry.
pub enum ItemSnapshot {
    Shared(Arc<Items>),
    Resources(Arc<super::Resources>),
}

impl ItemSnapshot {
    pub(in crate::game) fn capture(
        server: Option<&Arc<Items>>,
        world: Option<&Arc<Items>>,
        base: impl FnOnce() -> Self,
    ) -> Self {
        match server.or(world) {
            Some(items) => Self::Shared(Arc::clone(items)),
            None => base(),
        }
    }
}

impl Deref for ItemSnapshot {
    type Target = Items;
    fn deref(&self) -> &Items {
        match self {
            Self::Shared(items) => items,
            Self::Resources(resources) => resources.items(),
        }
    }
}

pub trait ItemSnapshotGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    fn stack_boundary_items(&self) -> Option<ItemSnapshot> {
        Some(ItemSnapshot::capture(
            self.server_items.as_ref(),
            self.side_world_table_items(),
            || ItemSnapshot::Resources(Arc::clone(&self.res)),
        ))
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> ItemSnapshotGameExt1<'__sg> for G {}

impl<'a> super::GameLevel<'a> {
    pub(in crate::game) fn stack_boundary_items(&self) -> Option<&'a Items> {
        Some(self.items)
    }
}

use crate::game::side_ops::modapi_modding::ModapiModdingSideGameExt as _;
