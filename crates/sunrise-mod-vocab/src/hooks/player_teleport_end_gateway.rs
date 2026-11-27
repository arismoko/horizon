//! The `player_teleport_end_gateway` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player is about to travel through an End gateway at `gateway`. Set `cancel` to leave
    /// them at the entrance; the gateway is not consumed and its cooldown does not start. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_teleport_end_gateway",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true
    )]
    fn player_teleport_end_gateway(player: u128, name: &str, gateway: [i32; 3], cancel: &mut bool);
}
