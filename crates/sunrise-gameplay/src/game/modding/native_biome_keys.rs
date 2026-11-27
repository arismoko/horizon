//! Typed biome identities belong to the same installed snapshot as data maps.
use crate::game::ServerGame;
impl ServerGame<'_> {
    /// The installed snapshot's keys, empty before world data is installed.
    pub(in crate::game::server_side) fn biome_key_snapshot(
        &self,
    ) -> std::sync::Arc<mod_api::__private::BiomeKeys> {
        self.server
            .world_tables
            .0
            .as_ref()
            .map(|tables| tables.data_maps.biome_keys.clone())
            .unwrap_or_default()
    }
    /// Mod features answer `FeatureContext::biome` with the keys `biome_key` gives: publish the
    /// installed snapshot to every level's generation workers and to the live level's
    /// placements, as each data load installs it.
    pub(in crate::game::server_side) fn share_biome_keys(&self) {
        if !self.common.options.execution_role.is_logical_server() {
            return;
        }
        let keys = self.biome_key_snapshot();
        self.common
            .res
            .behaviours()
            .mod_biome_keys()
            .publish(keys.clone());
        for terrain in self
            .common
            .end
            .locator()
            .iter()
            .chain(self.server.levels.0.parked_terrains())
        {
            terrain.set_biome_keys(&keys);
        }
    }
}
