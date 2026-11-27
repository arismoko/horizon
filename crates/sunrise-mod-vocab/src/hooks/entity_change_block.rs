//! The `entity_change_block` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A mob changed a block at `pos`, after the new state was applied.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_entity_change_block",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_change_block(entity: u32, kind: &str, pos: [i32; 3], from: &str, to: &str);
}
