#[allow(unused_variables)]
impl SideOps for crate::game::server_side::ServerState {
    type Game<'g> = crate::game::ServerGame<'g>;

    fn side_block_entity_nbt(game: &Self::Game<'_>, pos: [i32; 3]) -> Option<Tag> {
        game.server_block_entity_nbt(pos)
    }
    fn side_send_chat_command(game: &mut Self::Game<'_>, command: &str) {
        game.server_send_chat_command(command)
    }
    fn side_apply_item_splash(game: &mut Self::Game<'_>, splash: ItemSplash) {
        game.server_apply_item_splash(splash)
    }
    fn side_level_min_y(game: &Self::Game<'_>) -> i32 {
        game.server_level_min_y()
    }
}
use sunrise_core::nbt::Tag;
use sunrise_gameplay_common::game::side_ops::ui::SideOps;
use sunrise_sim::entities::ItemSplash;
