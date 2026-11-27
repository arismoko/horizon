#[allow(unused_variables)]
impl SideOps for crate::game::server_side::ServerState {
    type Game<'g> = crate::game::ServerGame<'g>;

    fn side_has_remote(&self) -> bool {
        false
    }

    fn side_fire_pending_block_entity_loads(game: &mut Self::Game<'_>) {
        {
            game.server_fire_pending_block_entity_loads();
        }
    }
    fn side_creative_holders(game: &Self::Game<'_>) -> Option<Holders> {
        game.server_creative_holders()
    }
    fn side_creative_catalog<'__side_g>(
        game: &'__side_g Self::Game<'_>,
    ) -> &'__side_g Arc<Catalog> {
        game.server_creative_catalog()
    }
    fn side_reset_creative_screen(game: &mut Self::Game<'_>) {
        {}
    }
    fn side_publish_creative_order(_game: &Self::Game<'_>, _lists: &[Arc<TabContents>]) {}

    fn side_sync_native_setting_ui_row(
        _game: &mut Self::Game<'_>,
        _owner: &str,
        _spec: &mod_api::SettingSpec,
        _value: mod_api::SettingValue,
    ) {
    }

    fn side_ask_swim_eligibility(game: &Self::Game<'_>, check: SwimmingCheck) -> Option<bool> {
        game.server_ask_swim_eligibility(check)
    }
    fn side_acting_id(game: &mut Self::Game<'_>) -> u128 {
        game.server_acting_id()
    }
    fn side_clicker(game: &mut Self::Game<'_>) -> (u128, i32) {
        game.server_clicker()
    }
    fn side_acting_or_local_player(game: &Self::Game<'_>) -> Option<PlayerRef> {
        game.server_acting_or_local_player()
    }
    fn side_chunks_loaded(game: &mut Self::Game<'_>) {
        {
            game.server_chunks_loaded();
        }
    }
    fn side_chunks_unloaded(game: &mut Self::Game<'_>, unloaded: &[Arc<Chunk>]) {
        {
            game.server_chunks_unloaded(unloaded);
        }
    }
    fn side_apply_after_use_components(game: &mut Self::Game<'_>, stack: &ItemStack) {
        {
            game.server_apply_after_use_components(stack);
        }
    }
}
use mod_api::PlayerRef;
use mod_api::SwimmingCheck;
use std::sync::Arc;
use sunrise_gameplay_common::game::side_ops::modapi_hooks::SideOps;
use sunrise_items::creative_catalog::Catalog;
use sunrise_items::creative_catalog::Holders;
use sunrise_items::creative_catalog::TabContents;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
