//! The `datapack_sync` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Collect recipe types once the joining player exists, after admission
    /// and before that player's recipes are sent. On reload,
    /// asked once per online player, at most once per player per reload,
    /// before their recipes are re-sent. With nobody online this
    /// hook is not asked. `player` always identifies the recipient.
    /// Configuration tags precede admission. Only this player's Sunrise
    /// client receives the selected recipes; vanilla clients receive nothing
    /// new. Recipes using a vanilla serializer carry the server's resolved
    /// item candidates, not the source's ingredient definitions or bound
    /// predicates. A mod serializer still writes its own recipe content.
    /// An empty answer retains the client's own recipes, or restores
    /// them after an earlier nonempty answer.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_datapack_sync",
        dispatch = "api_carrier",
        side = "server",
        contract = "collect",
        out = "recipe_types",
        neoforge = "OnDatapackSyncEvent",
        version = 2
    )]
    fn datapack_sync(player: PlayerRef, recipe_types: &mut Vec<String>);
}
