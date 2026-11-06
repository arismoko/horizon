//! The `translate` area: host methods the game implements (`game/host/translate.rs`).
#![allow(unused_variables)]
/// The client host's methods of the `translate` area, implemented by the game and reached through [`ClientHost`](crate::ClientHost).
pub trait TranslateClientHost {
    /// Side: client.
    /// The selected language's text for a translation key (`Component.translatable`'s lookup), `None`
    /// when no language layer defines it.
    fn translate(&self, key: &str) -> Option<String> {
        crate::phase::unprovided(
            "ClientHost::translate",
            "translate",
            crate::phase::LoadAccess::ClientSetup,
        )
    }
}
