//! The `mob_split` and `baby_entity_spawn` listeners of the pending-children matrix as a wasm
//! guest. It declares no `[[inject]]`, so a plain engine build loads it.
pub mod pending_children;

use mod_api::Hooks;

#[derive(Default)]
pub struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn mob_split(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::MobSplitEvent<'_>,
    ) -> mod_api::Flow {
        pending_children::mob_split(host, event)
    }

    fn baby_entity_spawn(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::BabyEntitySpawnEvent<'_>,
    ) -> mod_api::Flow {
        pending_children::baby_entity_spawn(host, event)
    }
}
