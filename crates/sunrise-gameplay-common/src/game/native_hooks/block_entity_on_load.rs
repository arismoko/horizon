use super::*;

pub trait BlockEntityOnLoadGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Drain attachment notices before either logical side runs block entity tickers.
    fn fire_pending_block_entity_loads(&mut self) {
        self.side_fire_pending_block_entity_loads();
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> BlockEntityOnLoadGameExt1<'__sg> for G {}

pub fn authoritative_world_for_mods(
    resources: &crate::game::Resources,
    seed: i64,
) -> sunrise_level::world::World {
    let mut world = crate::game::world_for_mods(resources.facts().air, seed);
    world.authoritative_group_lifetime(resources.rest().behaviours.clone());
    world
}

/// Configure notice collection before this level can receive any block entities.
pub fn world_for_mods(air: BlockStateId, seed: i64) -> sunrise_level::world::World {
    let mut world = sunrise_level::world::World::new(air, seed);
    world
        .block_entities
        .set_on_load_enabled(mod_api::__private::has_part_hook(
            mod_api::HookId::BlockEntityOnLoad,
        ));
    world
        .block_entities
        .set_data_load_enabled(mod_api::__private::has_part_hook(
            mod_api::HookId::BlockEntityLoadAdditional,
        ));
    world
}

/// Notify subscribers after this instance is attached and its saved data is applied.
pub fn native_block_entity_on_load(call: mod_api::__private::BlockEntityOnLoadCall<'_>) {
    crate::game::native_hooks::fire::fire_block_entity_on_load(call);
}

use crate::game::side_ops::modapi_hooks::ModapiHooksSideGameExt as _;
