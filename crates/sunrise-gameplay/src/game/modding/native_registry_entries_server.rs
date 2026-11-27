use crate::game::ServerGame;
use std::sync::Arc;
use sunrise_sim::lan::data::SyncData;

impl ServerGame<'_> {
    pub(in crate::game) fn server_registry_sync(&self) -> Option<Arc<SyncData>> {
        self.common.native_lifecycle.epoch()?;
        self.server
            .lan
            .as_ref()
            .map(|host| host.hub.lan.shared.sync())
            .or_else(|| {
                // The same lazy, frozen/current table the server's next connection receives. Never
                // reconstruct a dynamic registry from the prepared files at a mod call boundary.
                sunrise_gameplay_common::game::lan_common::sync::sync_data(
                    &self.common.options.resources,
                    &self.common.res.core.item_data,
                    self.items(),
                    &self.pack_files(),
                )
                .ok()
            })
    }

    pub(in crate::game) fn registry_generator_data(
        &self,
    ) -> Option<Arc<sunrise_world::worldgen::generator::GeneratorData>> {
        self.common
            .end
            .locator()
            .map(|terrain| Arc::clone(terrain.generator_data()))
            .or_else(|| {
                self.common
                    .res
                    .behaviours()
                    .live_features()
                    .map(|features| Arc::clone(features.data()))
            })
    }
}

#[cfg(test)]
pub(in crate::game) use sunrise_gameplay_common::game::native_registry_entries::load_registry_tags;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;
