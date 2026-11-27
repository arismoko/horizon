//! The `vehicle_damage` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A vehicle is about to take damage from a player's attack. `kind` is
    /// `boat` or the minecart type; mods may change `damage` or set `cancel`
    /// to leave its damage and wobble unchanged. The attack itself still
    /// completes. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_vehicle_damage",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's VehicleDamageEvent)",
        cancellable = true
    )]
    fn vehicle_damage(kind: &str, damage: &mut f32, cancel: &mut bool);
}
