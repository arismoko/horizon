//! Datapack registry reads through a level handle.
use mod_api::{DatapackReadError, LevelRef};
use std::ops::ControlFlow;

pub trait NativeDatapackRefsGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// The datapack registry set of this level's world session: on the logical server, the
    /// server's own set for both the active dimension and any parked loaded dimension
    /// (matching vanilla `Level.registryAccess()` on `ServerLevel`); on a client, the
    /// synchronized set its connection received (also available for the constructing level
    /// during `client_level_load` before installation).
    fn level_datapack_set(
        &self,
        level: LevelRef,
    ) -> Option<std::sync::Arc<mod_api::DatapackRegistrySet>> {
        self.side_level_datapack_set(level)
    }

    /// Visit the IDs of a mod's datapack registry as the world last loaded them.
    fn level_visit_datapack_entries(
        &self,
        level: LevelRef,
        registry: &str,
        visit: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, DatapackReadError> {
        let set = self
            .level_datapack_set(level)
            .ok_or(DatapackReadError::UnknownRegistry)?;
        Ok(set.entries(registry)?.try_for_each(visit))
    }

    /// Visit the published entry `entry` of `registry`.
    fn level_datapack_read(
        &self,
        level: LevelRef,
        registry: &str,
        entry: &str,
        visit: &mut dyn FnMut(&mod_api::DatapackEntry),
    ) -> Result<(), DatapackReadError> {
        let set = self
            .level_datapack_set(level)
            .ok_or(DatapackReadError::UnknownRegistry)?;
        if !set.registry_available(registry) {
            return Err(DatapackReadError::UnknownRegistry);
        }
        visit(set.get(registry, entry)?);
        Ok(())
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeDatapackRefsGameExt1<'__sg> for G {}

use crate::game::side_ops::modapi_modding::ModapiModdingSideGameExt as _;
