//! The `player_login_profile` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The properties of an admitted login's profile, before `LoginFinished` sends them
    /// (`ServerLoginPacketListenerImpl.finishLoginAndWaitForClient`): the name, ban,
    /// whitelist, capacity and duplicate-player checks have passed; configuration and
    /// `player_login` come after. `properties` starts with the login's own, signed `textures`
    /// included when there are some, and none for an ordinary offline login; each mod gets
    /// the list the mods before it left, and the final list goes to login success and to the
    /// player's profile in the player list. The UUID and name are read-only, the login can't be
    /// refused here, and the hook can't wait for a profile lookup.
    ///
    /// The final list must fit the packet (`ByteBufCodecs.GAME_PROFILE_PROPERTIES`): at most 16
    /// properties, names of at most 64 UTF-16 units, values of at most 32767 and signatures of
    /// at most 1024. A list that doesn't is refused whole, with a warning, and the login keeps
    /// its own properties. On an integrated server, the owner's empty list is still filled from
    /// the owner's own profile when that fetch answers later. Once for each login, on a server
    /// opened to players.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_login_profile",
        dispatch = "flat",
        side = "server",
        contract = "collect",
        out = "properties",
        neoforge = "none (NeoForge's PlayerNegotiationEvent shows the profile but can't edit it before it is sent)"
    )]
    fn player_login_profile(
        player: u128,
        name: &str,
        properties: &mut Vec<mod_api::AccountProfileProperty>,
    );
}
