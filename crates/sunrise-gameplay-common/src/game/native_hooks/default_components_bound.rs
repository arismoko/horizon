use super::*;

#[inline(always)]
pub(in crate::game) fn native_default_components_bound(
    context: &dyn mod_api::ReloadContext,
    cause: mod_api::TagsUpdateCause,
) {
    crate::game::native_hooks::fire::fire_default_components_bound(context, cause);
}

pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// Items' default components are bound on this side (NeoForge's
    /// `DefaultDataComponentsBoundEvent`), right before `native_tags_updated`.
    fn native_default_components_bound(&mut self, cause: mod_api::TagsUpdateCause) {
        native_default_components_bound(
            <Self::State as crate::game::Side>::as_reload_context(self),
            cause,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
