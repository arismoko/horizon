#[cfg(feature = "client")]
#[allow(unused_variables)]
impl SideOps for crate::game::client_side::ClientState {
    type Game<'g> = crate::game::ClientGame<'g>;

    fn side_process_level_events(game: &mut Self::Game<'_>) -> Result<()> {
        game.client_process_level_events()
    }
    fn side_level_event_at(game: &mut Self::Game<'_>, id: i32, pos: [i32; 3], data: i32) {
        {
            game.client_level_event_at(id, pos, data);
        }
    }
    fn side_update_world(game: &mut Self::Game<'_>) {
        {
            game.client_update_world();
        }
    }
    fn side_post_block_game_event(
        game: &mut Self::Game<'_>,
        event: &'static str,
        pos: [i32; 3],
        actor: Option<Who>,
        state: Option<BlockStateId>,
    ) {
        {
            game.client_post_block_game_event(event, pos, actor, state);
        }
    }
    fn side_post_game_event(
        game: &mut Self::Game<'_>,
        event: &'static str,
        pos: [f64; 3],
        ctx: SculkContext,
    ) {
        {
            game.client_post_game_event(event, pos, ctx);
        }
    }
    fn side_sculk_actor(game: &Self::Game<'_>) -> Option<Who> {
        game.client_sculk_actor()
    }
    fn side_tick_bell(
        game: &mut Self::Game<'_>,
        pos: [i32; 3],
        tags: &sunrise_items::items::EntityTypeTags,
    ) {
        {
            game.client_tick_bell(pos, tags);
        }
    }
    fn side_award_stat(game: &mut Self::Game<'_>, category: &str, item: &str, amount: i32) {
        {
            game.client_award_stat(category, item, amount);
        }
    }
    fn side_award_custom_stat(game: &mut Self::Game<'_>, key: &str, amount: i32) {
        {
            game.client_award_custom_stat(key, amount);
        }
    }
    fn side_container_changed(game: &mut Self::Game<'_>, key: (i32, i32, i32)) {
        {
            game.client_container_changed(key);
        }
    }
    fn side_tx_close_storage_transaction(
        game: &mut Self::Game<'_>,
        commit: bool,
    ) -> Option<transactions::StorageSettlement> {
        game.client_tx_close_storage_transaction(commit)
    }
    fn side_tx_run_after(game: &mut Self::Game<'_>, after: transactions::After) {
        {
            game.client_tx_run_after(after);
        }
    }
    fn side_tx_container_change_effect(
        game: &Self::Game<'_>,
        pos: (i32, i32, i32),
        holder: Option<BlockEntityIdentity>,
    ) -> transactions::After {
        game.client_tx_container_change_effect(pos, holder)
    }
    fn side_tx_journal_player(game: &mut Self::Game<'_>, uuid: u128) {
        {
            game.client_tx_journal_player(uuid);
        }
    }
    fn side_tx_journal_block_entity(game: &mut Self::Game<'_>, pos: (i32, i32, i32)) {
        {
            game.client_tx_journal_block_entity(pos);
        }
    }
    fn side_tx_journal_menus(game: &mut Self::Game<'_>, player: mod_api::PlayerRef) {
        {
            game.client_tx_journal_menus(player);
        }
    }
}
use sunrise_gameplay_common::game::Result;
use sunrise_gameplay_common::game::side_ops::events::SideOps;
use sunrise_gameplay_common::game::transactions;
use sunrise_level::block_entities::BlockEntityIdentity;
use sunrise_level::sculk::Context as SculkContext;
use sunrise_sim::mobs::Who;
use sunrise_world::registry::BlockStateId;
