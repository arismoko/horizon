//! The game's side of the `player_stats` area (`horizon-mod-api/src/host/player_stats.rs`).
use crate::game::Game;
use mod_api::{prelude::*, *};
impl PlayerStatsBackend for Game<'_> {
    fn player_stat(&self, player: PlayerRef, stat_type: NativeStatType, key: &str) -> Option<i32> {
        mod_api::__private::check_access("player_stat", mod_api::__private::LoadAccess::WorldOnly);
        self.native_player_stat(player, stat_type, key)
    }
    fn player_has_discovered_recipe(&self, player: PlayerRef, recipe: &str) -> Option<bool> {
        mod_api::__private::check_access(
            "player_has_discovered_recipe",
            mod_api::__private::LoadAccess::WorldOnly,
        );
        self.native_player_has_discovered_recipe(player, recipe)
    }
    fn player_discovered_recipes(&self, player: PlayerRef) -> Option<Vec<String>> {
        mod_api::__private::check_access(
            "player_discovered_recipes",
            mod_api::__private::LoadAccess::WorldOnly,
        );
        self.native_player_discovered_recipes(player)
    }
}
impl PlayerStatsBackendMut for Game<'_> {
    fn player_award_stat(
        &mut self,
        player: PlayerRef,
        stat_type: NativeStatType,
        key: &str,
        amount: i32,
    ) -> bool {
        mod_api::__private::check_access(
            "player_award_stat",
            mod_api::__private::LoadAccess::WorldOnly,
        );
        self.native_player_award_stat(player, stat_type, key, amount)
    }
    fn player_reset_stat(
        &mut self,
        player: PlayerRef,
        stat_type: NativeStatType,
        key: &str,
    ) -> bool {
        mod_api::__private::check_access(
            "player_reset_stat",
            mod_api::__private::LoadAccess::WorldOnly,
        );
        self.native_player_reset_stat(player, stat_type, key)
    }
    fn player_award_recipes(&mut self, player: PlayerRef, recipes: &[&str]) -> Option<i32> {
        mod_api::__private::check_access(
            "player_award_recipes",
            mod_api::__private::LoadAccess::WorldOnly,
        );
        self.native_player_award_recipes(player, recipes)
    }
    fn player_reset_recipes(&mut self, player: PlayerRef, recipes: &[&str]) -> Option<i32> {
        mod_api::__private::check_access(
            "player_reset_recipes",
            mod_api::__private::LoadAccess::WorldOnly,
        );
        self.native_player_reset_recipes(player, recipes)
    }
}
