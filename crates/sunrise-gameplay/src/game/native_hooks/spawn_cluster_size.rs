use super::*;

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// `Mob.getMaxSpawnClusterSize` as the mob's entity parts answer it. Server-side.
    pub(in crate::game) fn native_spawn_cluster_size(
        &self,
        call: mod_api::__private::SpawnClusterSizeCall<'_>,
        entity_kind: &str,
    ) {
        self.fire_spawn_cluster_size(call, entity_kind);
    }
}
