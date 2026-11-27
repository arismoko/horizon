//! The `block_dispense_armor` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A dispenser equipped `item` on the first eligible `target` in front
    /// of its block at `pos`. `target` is `player`, `mob` or `armor_stand`.
    /// The equip, item consumption, sound and game event have completed.
    /// Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_block_dispense_armor",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn block_dispense_armor(dimension: &str, pos: [i32; 3], item: &str, target: &str);
}
