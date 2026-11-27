//! The `player_stonecutter_recipe_select` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player selected a stonecutter recipe, after the menu accepted
    /// its button. `recipe` is the zero-based button index. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_stonecutter_recipe_select",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_stonecutter_recipe_select(player: u128, name: &str, recipe: i32);
}
