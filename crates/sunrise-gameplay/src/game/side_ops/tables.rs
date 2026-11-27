#[allow(unused_variables)]
impl SideOps for crate::game::server_side::ServerState {
    type Game<'g> = crate::game::ServerGame<'g>;

    fn side_sync_test_registry_ids(game: &mut Self::Game<'_>) {
        game.sync_test_registry_ids();
    }

    fn side_game_tag_items<'a>(&'a self, res: &'a Resources) -> Option<&'a Items> {
        self.session_lock.0.is_some().then(|| res.items())
    }
    fn side_level_features<'a>(&'a self, enabled: &'a Arc<[String]>) -> &'a Arc<[String]> {
        enabled
    }
    fn side_remote_tags_present(&self) -> bool {
        false
    }
    fn side_remote_limited_crafting(&self) -> bool {
        false
    }
    fn side_publish_cauldron_registrations(&self, table: &cauldron_interactions::Table) {
        if let Some(host) = &self.lan {
            sunrise_gameplay_common::game::cauldron_prediction::publish_registrations(
                table,
                host.hub
                    .guests
                    .simulation_iter()
                    .filter(|g| g.placed || g.held),
            );
        }
    }
    fn side_remote_enchantments(&self) -> Option<&Arc<Enchantments>> {
        None
    }
    fn side_remote_item_registry(&self) -> Option<Option<&Ids>> {
        None
    }
    fn side_install_lan_loop_context(&mut self, context: &Arc<LoopContext>) {
        if let Some(host) = self.lan.as_mut() {
            host.hub.loop_context = context.clone();
        }
    }

    fn side_world_tables<'__side_g>(
        game: &'__side_g Self::Game<'_>,
    ) -> Option<&'__side_g WorldTables> {
        game.server_world_tables()
    }
    fn side_acting_player_uuid(game: &Self::Game<'_>) -> Option<u128> {
        game.server_acting_player_uuid()
    }
    fn side_tags_ready(game: &Self::Game<'_>) -> bool {
        game.server_tags_ready()
    }
    fn side_resolve_enchant_particles(game: &Self::Game<'_>) {
        {}
    }
    fn side_resolve_block_paths(game: &Self::Game<'_>) {
        {
            game.server_resolve_block_paths();
        }
    }
    fn side_resolve_primary_items_server(
        game: &Self::Game<'_>,
        support: sunrise_world::enchantment_answers::Answers,
    ) {
        {
            game.server_resolve_primary_items_server(support);
        }
    }
    fn side_clear_loot_enchantment_answers(game: &Self::Game<'_>) {
        {
            game.server_clear_loot_enchantment_answers();
        }
    }
    fn side_clear_random_tick_sections(game: &mut Self::Game<'_>) {
        {
            game.server_clear_random_tick_sections();
        }
    }
}
use std::sync::Arc;
use sunrise_gameplay_common::game::Resources;
use sunrise_gameplay_common::game::WorldTables;
use sunrise_gameplay_common::game::cauldron_interactions;
use sunrise_gameplay_common::game::side_ops::tables::SideOps;
use sunrise_items::enchant::Enchantments;
use sunrise_items::items::Items;
use sunrise_protocol::registry::Ids;
use sunrise_world::native_load_context::LoopContext;
