//! The `broadcast_message` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server broadcast line is about to be logged and sent. Change `line`
    /// or set `cancel` to drop it.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_broadcast_message",
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        host = "read",
        neoforge = "none (NeoForge has no such event; Bukkit's BroadcastMessageEvent)",
        cancellable = true,
        version = 2
    )]
    fn broadcast_message(line: &mut Text, cancel: &mut bool);
}
