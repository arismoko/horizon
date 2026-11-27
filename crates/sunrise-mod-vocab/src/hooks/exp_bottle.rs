//! The `exp_bottle` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An experience bottle broke and its experience was awarded.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_exp_bottle",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn exp_bottle(pos: [f64; 3], experience: u32);
}
