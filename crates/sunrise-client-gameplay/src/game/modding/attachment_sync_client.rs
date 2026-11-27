//! Client-side synced attachment reception and index binding.
use crate::game::ClientGame;
use sunrise_mod_runtime::attachments::{Attachments, Holder};
use sunrise_protocol::attachment_sync::Message;

impl ClientGame<'_> {
    /// Apply an already decoded sync in play packet order, when its holder should exist.
    pub fn remote_attachment_sync(&mut self, message: Message) {
        let Some(remote) = self.client.remote.as_ref() else {
            return;
        };
        let foreign_player = match message.holder {
            Holder::Entity(id) => {
                let id_i32 = id as i32;
                let known = id_i32 == remote.entity_id
                    || remote.tracked(id_i32).is_some()
                    || remote.tracked_entity(id_i32).is_some()
                    || self.common.dragon.as_ref().is_some_and(|d| {
                        sunrise_gameplay_common::game::dragon_common::dragon_part_index(d.id, id)
                            .is_some()
                    });
                if !known {
                    log::warn!("Sunrise: received attachment sync for unknown entity {id}");
                    return;
                }
                None
            }
            Holder::Player(uuid) if uuid != remote.profile() => {
                let Some(id) = remote.player_entity(uuid) else {
                    log::warn!("Sunrise: received attachment sync for unknown player {uuid:032x}");
                    return;
                };
                Some(id)
            }
            // The recipient's own initial sync does not come from an entity pairing.
            _ => None,
        };
        let remote = self.client.remote.as_mut().unwrap();
        let map = remote.attachments.entry(message.holder).or_default();
        if message.replace {
            map.replace_contents(Attachments::default());
        }
        for change in message.changes {
            map.apply_sync(change);
        }
        if let Holder::Entity(id) = message.holder {
            remote.general.attachments_arrived(id as i32, map);
        }
        self.bind_remote_attachment_with_player(message.holder, foreign_player);
    }

    /// Publish a synced copy once its scoped client holder can be named.
    pub(in crate::game) fn bind_remote_attachment(&mut self, holder: Holder) {
        self.bind_remote_attachment_with_player(holder, None);
    }

    fn bind_remote_attachment_with_player(&mut self, holder: Holder, player: Option<u32>) {
        use mod_api::{AttachmentHolder as H, ChunkRef, EntityRef};
        let Some(remote) = &self.client.remote else {
            return;
        };
        let Some(level) = self.level(&remote.level) else {
            return;
        };
        let scoped = match holder {
            Holder::Entity(id) => H::Entity(EntityRef::new(level.epoch(), id)),
            Holder::Player(uuid) => {
                let id = if uuid == remote.profile() {
                    u32::try_from(remote.entity_id).ok()
                } else {
                    player.or_else(|| remote.player_entity(uuid))
                };
                let Some(id) = id else { return };
                H::Player(mod_api::__private::game::player_ref(
                    EntityRef::new(level.epoch(), id),
                    uuid,
                ))
            }
            Holder::Chunk(pos) => H::Chunk(ChunkRef::new(level, pos)),
            Holder::BlockEntity(pos) => {
                let Some(world) = self.client_view_world(level) else {
                    return;
                };
                let Some(entity) =
                    sunrise_gameplay_common::game::native_block_entity_read::world_block_entity(
                        world, level, pos,
                    )
                else {
                    return;
                };
                H::BlockEntity(entity)
            }
            Holder::Level => H::Level(level),
            _ => return,
        };
        if let Some(map) = self
            .client
            .remote
            .as_mut()
            .and_then(|r| r.attachments.get_mut(&holder))
            && !map.has_index_binding()
        {
            map.bind_index(
                self.common
                    .attachment_indexes
                    .entry(self.common.dim)
                    .or_default(),
                scoped,
            );
        }
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
