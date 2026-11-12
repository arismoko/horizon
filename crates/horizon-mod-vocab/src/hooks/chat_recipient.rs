//! The `chat_recipient` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A public chat line from `sender` (`sender_name`) is about to reach the
    /// placed player `recipient`: a player's chat after `server_chat` let it
    /// through, or a `/say` or `/me`. `message` is the text players see (what
    /// `server_chat` left). Cancel and that one receiver is not sent the line
    /// at all: the others still get it, signed as it was sent, so its
    /// signature still verifies for them. Asked once for each receiver, the
    /// sender included, before any of them is sent the line; whispers
    /// (`/msg`) are not public and not heard here. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "hidden",
        neoforge = "none (NeoForge sends a chat line to every player; it has no per-receiver event)",
        paper = "AsyncChatEvent.viewers()",
        cancellable = true
    )]
    fn chat_recipient();
}
