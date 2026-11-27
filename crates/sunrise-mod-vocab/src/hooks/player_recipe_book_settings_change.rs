//! The `player_recipe_book_settings_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player's authoritative recipe book pair changed. `book` is a
    /// typed RecipeBookIdentity, never a custom vanilla ordinal. Rejected and
    /// unchanged custom requests do not notify. Delivery happens at the outermost
    /// commit of a transaction, once per final changed pair, so a set followed by
    /// a restore inside one transaction notifies nothing. The guest is not swapped in.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_recipe_book_settings_change",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerRecipeBookSettingsChange)",
        paper = "PlayerRecipeBookSettingsChangeEvent",
        version = 2
    )]
    fn player_recipe_book_settings_change(
        player: u128,
        name: &str,
        book: mod_api::RecipeBookIdentity,
        open: bool,
        filtering: bool,
    );
}
