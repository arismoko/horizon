//! The `vehicle_create` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A boat or minecart was created at `pos`, after it joined the level.
    /// `kind` is the vehicle type. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_vehicle_create",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's VehicleCreateEvent)"
    )]
    fn vehicle_create(kind: &str, pos: [f64; 3]);
}
