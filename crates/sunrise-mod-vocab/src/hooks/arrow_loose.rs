//! The `arrow_loose` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `ArrowLooseEvent`: an armed bow is released or a player fires
    /// a loaded crossbow. Cancel or set `charge` negative to prevent the shot;
    /// a crossbow starts at `1` and other non-negative values do not change its
    /// shot. A bow uses the edited held-tick charge. `has_ammo` is always true.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_arrow_loose",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "ArrowLooseEvent",
        version = 2,
        cancellable = true
    )]
    fn arrow_loose(stack: &ItemStack, has_ammo: bool, charge: &mut i32, canceled: &mut bool);
}
