#[cfg(feature = "client")]
#[allow(unused_variables)]
impl SideOps for crate::game::client_side::ClientState {
    type Game<'g> = crate::game::ClientGame<'g>;

    fn side_remote_synced_timelines(&self) -> Option<sunrise_level::environment::TimelineTable> {
        self.remote.as_ref().map(|remote| remote.synced_timelines())
    }
    fn side_remote_world_clock_ids(&self) -> Option<Vec<String>> {
        let remote = self.remote.as_ref()?;
        Some(
            remote
                .registries
                .registries
                .get("minecraft:world_clock")
                .map_or_else(Vec::new, |entries| {
                    entries.iter().map(|(name, _)| name.clone()).collect()
                }),
        )
    }

    fn side_game_tag_items<'a>(&'a self, res: &'a Resources) -> Option<&'a Items> {
        self.remote.as_ref()?.tags.as_ref()
    }
    fn side_level_features<'a>(&'a self, enabled: &'a Arc<[String]>) -> &'a Arc<[String]> {
        match &self.remote {
            Some(remote) => &remote.features,
            None => sunrise_gameplay_common::game::tables::default_client_features(),
        }
    }
    fn side_remote_tags_present(&self) -> bool {
        self.remote
            .as_ref()
            .is_some_and(|r| !r.registries.tags.is_empty())
    }
    fn side_remote_limited_crafting(&self) -> bool {
        self.remote.as_ref().is_some_and(|r| r.do_limited_crafting)
    }
    fn side_publish_cauldron_registrations(&self, table: &cauldron_interactions::Table) {}
    fn side_remote_enchantments(&self) -> Option<&Arc<Enchantments>> {
        self.remote.as_ref()?.enchantments.as_ref()
    }
    fn side_remote_item_registry(&self) -> Option<Option<&Ids>> {
        Some(self.remote.as_ref()?.builtin.get("minecraft:item"))
    }
    fn side_install_lan_loop_context(&mut self, context: &Arc<LoopContext>) {}

    fn side_world_tables<'__side_g>(
        game: &'__side_g Self::Game<'_>,
    ) -> Option<&'__side_g WorldTables> {
        game.client_world_tables()
    }
    fn side_acting_player_uuid(game: &Self::Game<'_>) -> Option<u128> {
        game.client_acting_player_uuid()
    }
    fn side_tags_ready(game: &Self::Game<'_>) -> bool {
        game.client_tags_ready()
    }
    fn side_resolve_enchant_particles(game: &Self::Game<'_>) {
        {
            game.client_resolve_enchant_particles();
        }
    }
    fn side_resolve_block_paths(game: &Self::Game<'_>) {
        {}
    }
    fn side_resolve_primary_items_server(
        game: &Self::Game<'_>,
        support: sunrise_world::enchantment_answers::Answers,
    ) {
        {}
    }
    fn side_clear_loot_enchantment_answers(game: &Self::Game<'_>) {
        {}
    }
    fn side_clear_random_tick_sections(game: &mut Self::Game<'_>) {
        {}
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
