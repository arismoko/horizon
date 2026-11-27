//! The `fill_bucket` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player fills an empty bucket (`stack`) at `pos`: `BucketItem.use`'s
    /// pickup branch, once `Level.mayInteract` allows it and before
    /// `BucketPickup.pickupBlock`. Set `canceled` and the bucket does nothing.
    /// Emptying a bucket is `bucket_empty`'s moment, not this one. Server-side,
    /// for every player of the world (`host.players().acting_player()` says who).
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_fill_bucket",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "canceled",
        version = 2,
        neoforge = "none (NeoForge 26.3 has no bucket-fill event)"
    )]
    fn fill_bucket(stack: &ItemStack, pos: [i32; 3], canceled: &mut bool);
}
