//! Prepare server listeners against the new packs before publishing any data.
use crate::game::ServerGame;
use mod_api::__private::game::*;
use mod_api::{
    __private::reload::{ReloadStageError, RetainedEntry, apply_listeners, prepare_listeners},
    FileToIdConverter, HookId, Identifier, ServerReloadContext, ServerReloadListener,
    ServerReloadResources,
};
use sunrise_gameplay_common::game::function_pack_stack::PackStack;
use sunrise_gameplay_common::game::packs_common::reload_files;
use sunrise_level::datapack::{DataPacks, DataStore};

pub(in crate::game) type PreparedReload = PreparedStep<Box<DataStore>>;

use mod_api::__private::reload::PreparedStep;

/// Everything one successful reload publishes: its apply steps, then the
/// retained listeners' store.
pub struct PreparedServerReload {
    pub(in crate::game) steps: Vec<PreparedReload>,
    pub(in crate::game) retained: Vec<RetainedEntry>,
    pub(in crate::game) loot: std::sync::Arc<mod_api::GlobalLootModifierManager>,
}
impl PreparedServerReload {
    pub(in crate::game::server_side) fn bind_loot(&self, world: &super::PreparedWorldReload) {
        if let Some(tables) = &world.tables {
            self.loot.bind_from(&tables.loot_modifier_manager);
        }
    }
}

/// Logs individual vanilla function load failures and refuses invalid registry data.
fn functions_store(stack: &PackStack) -> Result<DataStore, ReloadStageError> {
    let packs: Vec<_> = stack.builtin.iter().chain(stack.enabled_packs()).collect();
    let store = DataStore::load(&DataPacks::new(&packs));
    for (id, e) in &store.errors {
        log::warn!("Sunrise: failed to load function {id}: {e}");
    }
    Ok(store)
}

pub(in crate::game) trait PackStackServerExt: Sized {
    fn reload_context(
        &self,
        listener: &Identifier,
        converter: &FileToIdConverter,
    ) -> ServerReloadContext;
}
impl PackStackServerExt for PackStack {
    fn reload_context(
        &self,
        listener: &Identifier,
        converter: &FileToIdConverter,
    ) -> ServerReloadContext {
        let packs: Vec<_> = self.builtin.iter().chain(self.enabled_packs()).collect();
        ServerReloadContext::new(reload_files("data", &packs, listener, converter))
    }
}

impl ServerGame<'_> {
    pub(in crate::game::server_side) fn prepare_server_reload(
        &self,
        stack: &PackStack,
    ) -> Result<PreparedServerReload, ReloadStageError> {
        sunrise_sim::mod_content::check_jars(
            &self.common.options.resources,
            &sunrise_level::logging::game_home().join("jars"),
        )
        .map_err(|error| {
            ReloadStageError::OriginalJars(
                sunrise_gameplay_common::game::api_values::TryIntoApi::try_into_api(&error)
                    .unwrap(),
            )
        })?;
        let mut listeners = <mod_api::ServerListenerRegistry as mod_api::__private::game::ServerReloadListenersOrder>::new();
        let loot = super::global_loot_modifiers::register_anchor(&mut listeners)
            .map_err(ReloadStageError::Order)?;
        self.native_add_server_reload_listeners(&mut listeners);
        let ordered = listeners.finish().map_err(ReloadStageError::Order)?;
        let steps = prepare_listeners(
            ordered.listeners,
            HookId::AddServerReloadListeners,
            "server_reload_prepare",
            || functions_store(stack).map(Box::new),
            |id, converter| stack.reload_context(id, converter),
            ServerReloadListener::prepare,
        )?;
        Ok(PreparedServerReload {
            steps,
            retained: ordered.retained,
            loot,
        })
    }
    pub(in crate::game::server_side) fn apply_server_reload(
        &mut self,
        prepared: PreparedServerReload,
        enabled_features: &[String],
    ) {
        let skipped = apply_listeners(
            prepared.steps,
            HookId::AddServerReloadListeners,
            "server_reload_apply",
            |store| {
                self.build_native_commands(enabled_features);
                self.admit_store(*store);
                Ok::<_, std::convert::Infallible>(())
            },
        )
        .unwrap();
        if let Some(tables) = &self.server.world_tables.0 {
            prepared.loot.bind_from(&tables.loot_modifier_manager);
        }
        self.server
            .reload_resources
            .0
            .install(prepared.retained, &skipped);
        self.server.structure_templates.clear();
    }
    /// The retained server reload listeners of the last successful reload.
    pub fn reload_resources(&self) -> &dyn ServerReloadResources {
        &self.server.reload_resources.0
    }
}
