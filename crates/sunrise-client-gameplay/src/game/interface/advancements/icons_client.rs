use super::ItemStack;
use sunrise_protocol::item::Stack;

impl crate::game::ClientGame<'_> {
    /// `DisplayInfo.fromNetwork`: convert the template once, not while drawing it.
    pub(in crate::game) fn remote_advancement_icon(&self, mut icon: Stack) -> ItemStack {
        let remote = self.client.remote.as_ref().expect("remote advancement");
        if !sunrise_protocol::item::custom::permitted_with_names(
            &icon,
            remote.registries.custom_components,
            &*remote.registries,
        )
        .unwrap_or(false)
        {
            // Strip the full patch on refused components so no unnegotiated component reaches rendering.
            log::warn!("Drawing advancement icon without its refused component patch");
            icon.added.clear();
            icon.removed.clear();
            icon.wire = Default::default();
        }
        sunrise_sim::item_sync::from_network(&icon, &remote.builtin, &remote.registries)
    }
}
