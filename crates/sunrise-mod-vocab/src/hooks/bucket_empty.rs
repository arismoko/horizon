//! The `bucket_empty` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player empties a bucket at `pos` (`fluid`: `minecraft:water`,
    /// `minecraft:lava`, `minecraft:powder_snow`; a fish or axolotl bucket
    /// empties `minecraft:water`): where `BucketItem.use` has found the block
    /// the fluid goes into and the player may build there, right before it is
    /// placed. Set `cancel` and nothing is poured and the bucket stays full.
    /// The player's state is swapped in, so `PlayerRef::tell` answers them. Only
    /// for guests of a server. Not heard: a dispenser, a cauldron, and a fluid
    /// a block places itself.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_bucket_empty",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (vanilla BucketItem.emptyContents; NeoForge has no matching event)",
        cancellable = true
    )]
    fn bucket_empty(player: u128, name: &str, pos: [i32; 3], fluid: &str, cancel: &mut bool);
}
