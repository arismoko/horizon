//! The `vehicle_destroy` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A boat or minecart is about to be destroyed at `pos`. TNT minecarts
    /// that prime instead of breaking are not heard. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_vehicle_destroy",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's VehicleDestroyEvent)"
    )]
    fn vehicle_destroy(kind: &str, pos: [f64; 3]);
}
