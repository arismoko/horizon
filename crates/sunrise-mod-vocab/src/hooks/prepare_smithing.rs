//! The `prepare_smithing` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A smithing result was prepared for a server player at `pos`. Change it before it is sent.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_prepare_smithing",
        dispatch = "flat",
        side = "server",
        contract = "replace",
        out = "result",
        neoforge = "none (NeoForge has no such event)",
        paper = "PrepareSmithingEvent",
        version = 3
    )]
    fn prepare_smithing(player: u128, name: &str, pos: [i32; 3], result: &mut Option<ItemStack>);
}
