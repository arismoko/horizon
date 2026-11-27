use crate::game::ServerGame;
#[allow(unused_imports)]
use mod_api::__private::game::*;
#[allow(unused_imports)]
use std::sync::Arc;
use sunrise_gameplay_common::game::mods::ModsReload;

impl ServerGame<'_> {
    pub(in crate::game) fn begin_condition_reload(
        &mut self,
        stack: &sunrise_gameplay_common::game::function_pack_stack::PackStack,
        resources: &std::path::Path,
    ) -> Result<
        crate::game::server_side::server_reload::PreparedServerReload,
        mod_api::__private::reload::ReloadStageError,
    > {
        use mod_api::__private::reload::ReloadStageError;
        if self.common.mods.load_condition_candidate.is_none() {
            let inventory = sunrise_mod_package::enabled::EnabledMods::read(resources)
                .map_err(|error| ReloadStageError::DataSource(error.to_string()))?;
            self.common.mods.load_condition_candidate = Some(
                sunrise_gameplay_common::game::load_conditions::conditions_for_mods(
                    &self.common.mods,
                    &inventory,
                )
                .map_err(ReloadStageError::DataSource)?,
            );
        }
        self.prepare_server_reload(stack)
    }

    pub fn prepare_pack_condition_reload(
        &mut self,
        stack: &sunrise_gameplay_common::game::function_pack_stack::PackStack,
    ) -> Result<
        crate::game::server_side::server_reload::PreparedServerReload,
        mod_api::__private::reload::ReloadStageError,
    > {
        self.common.mods.load_condition_candidate = None;
        self.begin_condition_reload(stack, &self.common.options.resources.clone())
    }

    pub(in crate::game) fn server_set_audio_loop_context(
        &mut self,
        audio: Option<sunrise_world::native_load_context::LoopContext>,
    ) {
        debug_assert!(audio.is_none());
    }

    pub(in crate::game) fn server_retire_disabled_instance_groups(
        &self,
        _effective: &std::collections::BTreeSet<String>,
    ) {
    }

    /// The server's side of a live switch: the data packs reload from the new root.
    pub(in crate::game) fn apply_mods_reload(&mut self, reload: ModsReload) {
        let at = std::time::Instant::now();
        self.common.options.resources = reload.root;
        self.common.pack.data_source = reload.data_source;
        self.common.pack.data_changes = reload.data_changes;
        self.common.pack.additional_data = reload.additional_data;
        self.server.settings.mods.enabled = reload.enabled.clone();
        self.common.mods.applied = Some(reload.enabled);
        self.common.mods.selected = reload.selected;
        self.common.mods.refused = reload.refused;
        self.common.mods.data_switched = true;
        if self.server.world_root.as_os_str().is_empty() {
            return;
        }
        if let Err(error) = self.reload_datapacks(true, &self.common.level.enabled_features.clone())
        {
            log::warn!(
                "Sunrise: content mods data reload failed: {}",
                error.diagnostic()
            );
            return;
        }
        log::info!(
            "Sunrise: content mods switched live: server data reloaded in {} ms",
            at.elapsed().as_millis()
        );
    }
}

#[cfg(any(test, feature = "test-support"))]
impl crate::game::ServerGame<'_> {
    pub fn test_save_root(&self) -> std::path::PathBuf {
        self.server.save.0.root.clone()
    }

    pub fn test_acquire_session_lock(&mut self, root: &std::path::Path) {
        self.server.session_lock.0 = Some(sunrise_sim::save::SessionLock::acquire(root).unwrap());
    }

    pub fn test_clear_session_lock(&mut self) {
        self.server.session_lock.0 = None;
    }

    pub fn test_set_world_recipes(&mut self, recipes: sunrise_sim::crafting::Recipes) {
        self.server.world_tables.0.as_mut().unwrap().recipes = recipes;
    }

    pub fn test_update_tickets(&mut self, sources: Option<sunrise_sim::ticket_levels::Sources>) {
        self.server.tickets.0.update(sources);
    }

    pub fn test_block_ticking(&self, cx: i32, cz: i32) -> bool {
        self.server.tickets.0.levels().block_ticking(cx, cz)
    }

    pub fn test_last_other_id(&self) -> u32 {
        self.server.others.0.last().unwrap().id
    }
}
