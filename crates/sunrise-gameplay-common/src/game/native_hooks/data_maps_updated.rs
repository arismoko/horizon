use super::*;

#[inline(always)]
pub(in crate::game) fn native_data_maps_updated(
    context: &dyn mod_api::ReloadContext,
    registry: &str,
    cause: mod_api::DataMapsUpdateCause,
) {
    crate::game::native_hooks::fire::fire_data_maps_updated(context, registry, cause);
}

pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// The data maps of `registry` (`minecraft:item`) changed on this side
    /// (NeoForge's `DataMapsUpdatedEvent`), after they are readable.
    fn native_data_maps_updated(&mut self, registry: &str, cause: mod_api::DataMapsUpdateCause) {
        native_data_maps_updated(
            <Self::State as crate::game::Side>::as_reload_context(self),
            registry,
            cause,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
