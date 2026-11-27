//! The `recipes_received` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The server's recipes of `recipe_types` arrived (NeoForge's
    /// `RecipesReceivedEvent`): `host.recipes_of_type` reads the server's recipes
    /// of those types in place of the window's own, so rebuild what you made
    /// of them. Vanilla serializers expose resolved candidates, not native
    /// ingredient predicates; matching on the server remains authoritative.
    /// The window retains the resolved recipes of the types not sent.
    /// It comes after a join, once the window has its player, and
    /// after every `/reload`, on a server whose mods asked for them
    /// (`native_datapack_sync`), single player included; with no types when
    /// the server stopped sending them and the window is back on its own
    /// recipes. A recipe's recipe book ids are not part of what arrives:
    /// `recipe_book_ids` answers them. Client-side.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_recipes_received",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "RecipesReceivedEvent",
        version = 2
    )]
    fn recipes_received(recipe_types: &[String]);
}
