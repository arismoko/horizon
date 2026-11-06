//! The `player_stats` area: host methods the game implements (`game/host/player_stats.rs`).
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `player_stats` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
pub trait PlayerStatsBackend {
    /// Side: server.
    /// The value of statistic `stat_type` for `key` (`minecraft:stone`,
    /// `minecraft:jump`...), as `ServerStatsCounter.getValue`: 0 when never
    /// counted.
    fn player_stat(&self, player: PlayerRef, stat_type: NativeStatType, key: &str) -> Option<i32> {
        crate::phase::unprovided(
            "Backend::player_stat",
            "player_stat",
            crate::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: server.
    /// Whether the player's recipe book knows `recipe`
    /// (`ServerRecipeBook.contains`).
    fn player_has_discovered_recipe(&self, player: PlayerRef, recipe: &str) -> Option<bool> {
        crate::phase::unprovided(
            "Backend::player_has_discovered_recipe",
            "player_has_discovered_recipe",
            crate::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: both.
    /// The recipes in the player's recipe book (`ServerRecipeBook.recipes`), as qualified ids in
    /// order (`minecraft:oak_planks`). None for a player it can't reach. Server-side.
    fn player_discovered_recipes(&self, player: PlayerRef) -> Option<Vec<String>> {
        crate::phase::unprovided(
            "Backend::player_discovered_recipes",
            "player_discovered_recipes",
            crate::phase::LoadAccess::WorldOnly,
        )
    }
}
/// The handles' server writes of the `player_stats` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
pub trait PlayerStatsBackendMut {
    /// Side: server.
    /// `Player.awardStat(stat, amount)`: adds `amount`, as the game does
    /// for its own counts, so `player_stat_award` hears it and may cancel
    /// it, and scoreboard criteria follow. False for a player it can't
    /// reach.
    fn player_award_stat(
        &mut self,
        player: PlayerRef,
        stat_type: NativeStatType,
        key: &str,
        amount: i32,
    ) -> bool {
        crate::phase::unprovided(
            "BackendMut::player_award_stat",
            "player_award_stat",
            crate::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: server.
    /// `Player.resetStat`: sets the statistic back to 0, which scoreboard
    /// criteria follow. False for a player it can't reach.
    fn player_reset_stat(
        &mut self,
        player: PlayerRef,
        stat_type: NativeStatType,
        key: &str,
    ) -> bool {
        crate::phase::unprovided(
            "BackendMut::player_reset_stat",
            "player_reset_stat",
            crate::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: server.
    /// `ServerPlayer.awardRecipes`, as `/recipe give` does: the recipes
    /// unlock in the player's book, with their toasts and the
    /// `player_recipe_discovered` hook, and the client hears of them.
    /// Unknown ids are skipped. The number of recipe displays added, None
    /// for a player it can't reach.
    fn player_award_recipes(&mut self, player: PlayerRef, recipes: &[&str]) -> Option<i32> {
        crate::phase::unprovided(
            "BackendMut::player_award_recipes",
            "player_award_recipes",
            crate::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: server.
    /// `ServerPlayer.resetRecipes`, as `/recipe take` does. The number of
    /// recipe displays taken out, None for a player it can't reach.
    fn player_reset_recipes(&mut self, player: PlayerRef, recipes: &[&str]) -> Option<i32> {
        crate::phase::unprovided(
            "BackendMut::player_reset_recipes",
            "player_reset_recipes",
            crate::phase::LoadAccess::WorldOnly,
        )
    }
}
