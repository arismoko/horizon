//! What a server mod reads of the tick times and of whether the server is published, and sets
//! of the view and simulation distance.
use crate::game::ServerGame;
use core::ops::ControlFlow;
use mod_api::ServerReadError;

impl ServerGame<'_> {
    /// `MinecraftServer.getTickTimesNanos`.
    pub(in crate::game) fn visit_tick_times(
        &self,
        visit: &mut dyn FnMut(u64) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, ServerReadError> {
        Ok(self.common.clock.tick_times().try_for_each(visit))
    }
    /// `PlayerList.getViewDistance` on either logical server, including unpublished worlds.
    pub fn server_view_distance(&self) -> Option<i32> {
        if !self.common.is_logical_server() {
            return None;
        }
        Some(self.server.lan.as_ref()?.hub.config.view_distance)
    }
    /// `PlayerList.getSimulationDistance` on either logical server, including unpublished worlds.
    pub fn server_simulation_distance(&self) -> Option<i32> {
        if !self.common.is_logical_server() {
            return None;
        }
        Some(self.server.lan.as_ref()?.hub.config.simulation_distance)
    }
    /// `MinecraftServer.isPublished`: `DedicatedServer`'s is always true, `IntegratedServer`'s
    /// while it is open to LAN (`multiplayerScope != OFF`).
    pub(in crate::game) fn server_is_published(&self) -> bool {
        self.common.is_dedicated_server() || self.published_port().is_some()
    }
    /// `PlayerList.setViewDistance` on either server. The native range is
    /// `ChunkMap.MIN_VIEW_DISTANCE` through `MAX_VIEW_DISTANCE` in `setServerViewDistance`.
    pub fn server_set_view_distance(&mut self, distance: i32) -> bool {
        let reason = if !(2..=32).contains(&distance) {
            "expected a distance in 2..=32"
        } else if self.set_view_distance(distance) {
            return true;
        } else {
            "no logical server player list"
        };
        let mod_id = mod_api::content::crash::current_native_mod().unwrap_or("unknown");
        log::warn!("mod {mod_id}: server_set_view_distance refused {distance}: {reason}");
        false
    }
    /// `PlayerList.setSimulationDistance` on either server. The native range spans the
    /// supported endpoints of `Options.simulationDistance`'s `IntRange` (2 with
    /// `DEBUG_ALLOW_LOW_SIM_DISTANCE`, up to 32); `DistanceManager.updateSimulationDistance`
    /// applies the distance to tickets rather than validating the option.
    pub fn server_set_simulation_distance(&mut self, distance: i32) -> bool {
        let reason = if !(2..=32).contains(&distance) {
            "expected a distance in 2..=32"
        } else if self.set_simulation_distance(distance) {
            return true;
        } else {
            "no logical server player list"
        };
        let mod_id = mod_api::content::crash::current_native_mod().unwrap_or("unknown");
        log::warn!("mod {mod_id}: server_set_simulation_distance refused {distance}: {reason}");
        false
    }
}
