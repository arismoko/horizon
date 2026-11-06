//! The game's side of the `player_client_brand` area (`horizon-mod-api/src/host/player_client_brand.rs`).
use crate::game::Game;
use mod_api::__private::game::*;
use mod_api::{prelude::*, *};
impl PlayerClientBrandBackend for Game<'_> {
    fn player_client_brand(&self, player: PlayerRef) -> Option<String> {
        mod_api::__private::check_access(
            "player_client_brand",
            mod_api::__private::LoadAccess::WorldOnly,
        );
        self.native_player_client_brand(player)
    }
}
