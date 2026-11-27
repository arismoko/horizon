//! The `resource_pack_status` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player's client answered a resource pack in play (vanilla's
    /// `ServerboundResourcePackPacket`): `status` is `Action`'s number, 0
    /// successfully loaded, 1 declined, 2 failed download, 3 accepted, 4
    /// downloaded, 5 invalid URL, 6 failed reload, 7 discarded. Heard when the
    /// packet arrives, after the server has handled it; it cannot be stopped.
    /// The player's state is swapped in, so the helpers and `PlayerRef::tell`
    /// work. The pack a dedicated server offers in the configuration
    /// (`resource-pack` in `server.properties`) is answered before the
    /// player has joined, and those answers are not heard. Once for each
    /// answer, on a server.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_resource_pack_status",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no resource-pack status event)"
    )]
    fn resource_pack_status(player: u128, name: &str, status: i32);
}
