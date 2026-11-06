//! The `clipboard` area: host methods the game implements (`game/host/clipboard.rs`).
#![allow(unused_variables)]
/// The client host's methods of the `clipboard` area, implemented by the game and reached through [`ClientHost`](crate::ClientHost).
pub trait ClipboardClientHost {
    /// Side: client.
    /// The clipboard's text (`KeyboardHandler.getClipboard`), empty when it holds none,
    /// while the game handles a key, mouse or typed-text event: a mod's key or screen hook
    /// pasting into its own field. None at any other moment (a tick, a frame, a packet),
    /// so no mod reads the clipboard without the player doing something, and None on a
    /// server.
    fn clipboard_text(&self) -> Option<String> {
        crate::phase::unprovided(
            "ClientHost::clipboard_text",
            "clipboard_text",
            crate::phase::LoadAccess::ClientSetup,
        )
    }
    /// Side: client.
    /// Puts `text` on the clipboard (`KeyboardHandler.setClipboard`) on the same terms as
    /// `native_clipboard_text` reads it. False, with nothing copied, at any other moment or
    /// on a server; empty text copies nothing, as vanilla's.
    fn set_clipboard_text(&self, text: &str) -> bool {
        crate::phase::unprovided(
            "ClientHost::set_clipboard_text",
            "set_clipboard_text",
            crate::phase::LoadAccess::ClientSetup,
        )
    }
}
