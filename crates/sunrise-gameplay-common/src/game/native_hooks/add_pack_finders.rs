use super::*;

#[doc(hidden)]
#[macro_export]
macro_rules! impl_pack_discovery_context {
    ($game:ty) => {
        impl mod_api::PackDiscoveryContext for $game {
            fn live_host(&mut self) -> Option<mod_api::HostSide<'_>> {
                Some($crate::game::execution_role::host_side(self))
            }

            fn client_text(&self) -> Option<&dyn mod_api::prelude::ClientTextClientHost> {
                <<Self as $crate::game::SideGame<'_>>::State as $crate::game::Side>::as_client_text_host(self)
            }
        }
    };
}

pub use crate::impl_pack_discovery_context;

#[inline(always)]
pub(in crate::game) fn native_add_pack_finders(
    context: &mut dyn mod_api::PackDiscoveryContext,
    register: impl FnMut(&mut dyn FnMut(&mut dyn mod_api::PackFinders) -> bool),
    enabled: impl FnMut(&str) -> bool,
) {
    crate::game::native_hooks::fire::fire_add_pack_finders(context, register, enabled);
}

pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// Register sources when a resource or data pack repository is discovered.
    fn native_add_pack_finders(&mut self, packs: &mut crate::game::pack_discovery::Registry) {
        let mods = self.mods.clone();
        native_add_pack_finders(
            <Self::State as crate::game::Side>::as_pack_discovery_context_mut(self),
            |listen| packs.register_listener(listen),
            |id| mods.code_owner_enabled(id),
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
