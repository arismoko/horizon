use super::*;

#[inline(always)]
pub(in crate::game) fn native_tags_updated(
    context: &dyn mod_api::ReloadContext,
    cause: mod_api::TagsUpdateCause,
) {
    crate::game::native_hooks::fire::fire_tags_updated(context, cause);
}

pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// Tags changed on this side (NeoForge's `TagsUpdatedEvent`), after they are readable.
    fn native_tags_updated(&mut self, cause: mod_api::TagsUpdateCause) {
        native_tags_updated(
            <Self::State as crate::game::Side>::as_reload_context(self),
            cause,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
