//! Common mod pack-finder discovery hook shared across server and client roles.

use crate::game::pack_discovery::{DiscoveredPack, Registry};
use mod_api::{HookId, PackType};

pub trait CommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    fn discover_mod_packs(&mut self, kind: PackType) -> Vec<DiscoveredPack> {
        if !mod_api::__private::enabled_listened(HookId::AddPackFinders) {
            return Vec::new();
        }
        let mut registry = Registry::new(kind, self.mods.clone());
        self.native_add_pack_finders(&mut registry);
        registry.packs
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> CommonGameExt1<'__sg> for G {}

use crate::game::native_hooks::add_pack_finders::GameHooks as _;
