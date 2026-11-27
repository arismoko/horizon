use crate::game::ClientGame;
use mod_api::WorldEpoch;

impl ClientGame<'_> {
    pub(in crate::game) fn client_lifecycle_open_at(&mut self, epoch: WorldEpoch) {
        self.client_reset_entity_residency();
        self.common.entity_initialization = Default::default();
        if let Some(remote) = &mut self.client.remote {
            remote.clear_attachments();
        }
        self.common.attachment_indexes.clear();
        self.common.creatures.custom_spawners.clear();
        self.common.passages.retire_world();
        self.common.native_lifecycle.begin_open_at(None, epoch);
    }

    pub(in crate::game) fn client_lifecycle_world_ready(&mut self) {
        let _ = self.common.native_lifecycle.world_open_ready();
    }

    /// Client epochs retire without dispatching server lifecycle hooks.
    pub(in crate::game) fn client_lifecycle_world_closed(&mut self) {
        self.client.instance_groups.borrow_mut().clear();
        self.client.prepared_items.borrow_mut().clear();
        if let Some(renderer) = &self.client.renderer.0 {
            renderer.clear_mod_meshes();
            renderer.clear_mod_uniforms();
        }
        self.finish_world_closed_common(|_| {}, |_| {});
    }
}

use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1 as _;
