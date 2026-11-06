//! The `player_client_brand` helper: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Helpers {
    /// The brand the player's client announced when it connected (`vanilla`, `fabric`,
    /// `horizon`), as the connection stored it; empty when the client sent none. Includes
    /// displaced connected players. Empty for an acting player with no known connection; None for
    /// a stale or unavailable player. Server-side.
    #[notes = r#"## Client brand

`PlayerRef::client_brand(host)` answers which client a player joined with, from the brand string
the connection recorded (`ServerPlayer.clientBrand`). Horizon's own clients say `horizon`, which
is what the game itself checks before it sends them Horizon-only packets, so a mod that does the
same asks here rather than guessing from the name."#]
    #[helper(
        side = "server",
        receiver = "player",
        target = "horizon_client::game::native_server::Game<'_>::native_player_client_brand"
    )]
    fn player_client_brand();
}
