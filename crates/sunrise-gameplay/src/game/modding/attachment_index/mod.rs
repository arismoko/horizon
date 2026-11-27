//! Per-dimension attachment indexes, retained independently of level swaps.

use super::Dim;
use mod_api::{AttachmentHolder, ChunkRef};
use std::sync::Arc;
use sunrise_mod_runtime::attachments::{
    AttachmentLifetime, Holder, observer::IndexedAttachmentObserver,
};

mod players;

impl crate::game::ServerGame<'_> {
    /// Publish saved contents only when an authoritative entity owns their lifetime.
    pub(in crate::game::server_side) fn bind_saved_entity_attachments(
        &mut self,
        id: u32,
        slot: &mut AttachmentLifetime,
    ) {
        if !self.native_authoritative() {
            return;
        }
        let Some(level) = self.level(self.common.dim.id()) else {
            return;
        };
        let Some(map) = self
            .common
            .entity_attachments
            .get_mut(&id)
            .filter(|map| map.has_index_entries())
        else {
            return;
        };
        let index = self
            .common
            .attachment_indexes
            .entry(self.common.dim)
            .or_default();
        slot.bind(
            map,
            index,
            AttachmentHolder::Entity(mod_api::EntityRef::new(level.epoch(), id)),
        );
    }

    /// Publish the dimension's saved maps before listeners observe its load.
    pub fn bind_dimension_attachments(&mut self, dim: Dim) {
        let Some(level) = self.level(dim.id()) else {
            return;
        };
        let index = self.common.attachment_indexes.entry(dim).or_default();
        self.common
            .world
            .attachments
            .bind_index(index, AttachmentHolder::Level(level));
        self.common
            .world
            .chunks
            .observe_attachments(Arc::new(IndexedAttachmentObserver::new(
                index,
                move |holder| match holder {
                    Holder::Chunk(pos) => Some(AttachmentHolder::Chunk(ChunkRef::new(level, pos))),
                    _ => None,
                },
            )));
        self.common
            .world
            .block_entities
            .observe_attachments(Arc::new(IndexedAttachmentObserver::new(
                index,
                move |identity: sunrise_level::block_entities::BlockEntityIdentity| {
                    Some(AttachmentHolder::BlockEntity(
                        mod_api::__private::game::block_entity_ref(
                            level,
                            identity.position,
                            identity.generation,
                        ),
                    ))
                },
            )));
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
