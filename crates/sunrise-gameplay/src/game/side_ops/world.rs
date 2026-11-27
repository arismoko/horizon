#[allow(unused_variables)]
impl SideOps for crate::game::server_side::ServerState {
    type Game<'g> = crate::game::ServerGame<'g>;

    fn side_parked_block_ticks<'a>(game: &'a Self::Game<'_>, dimension: &str) -> Option<&'a Ticks> {
        game.server.levels.0.parked_block_ticks_of(dimension)
    }
    fn side_parked_border(&self, dim: Dim) -> Option<&sunrise_level::worldborder::WorldBorder> {
        self.levels.0.parked_border(dim)
    }
    fn side_parked_lid_openers(
        &self,
        dimension: &str,
    ) -> Option<&std::collections::HashMap<(i32, i32, i32), i32>> {
        self.levels.0.parked_lid_openers_of(dimension)
    }
    fn side_parked_world(&self, dimension: &str) -> Option<&World> {
        self.levels.0.parked_world_of(dimension)
    }
    fn side_parked_chunk_state(&self, dimension: &str) -> Option<&ChunkState> {
        self.levels
            .0
            .parked_streaming_of(dimension)
            .map(ChunkStreamer::state)
    }
    fn side_parked_block_shapes(
        &self,
        dimension: mod_api::DimensionKey,
    ) -> Option<(&World, &FxHashMap<[i32; 3], Shulker>)> {
        let (dim, parked) = self
            .levels
            .0
            .parked
            .iter()
            .find(|(dim, _)| dim.key() == dimension)?;
        Some((&parked.world, &self.levels.0.parked_anims(*dim)?.shulkers))
    }
    fn side_parked_mob(
        &self,
        dimension: mod_api::DimensionKey,
        join: JoinSeq,
    ) -> Option<(Dim, &Mob)> {
        let (dim, parked) = self
            .levels
            .0
            .parked
            .iter()
            .find(|(dim, _)| dim.key() == dimension)?;
        let mob = parked.mobs.get(parked.mobs.locate(join)?)?;
        Some((*dim, mob))
    }
    fn side_parked_target_visible(&self, dim: Dim, target: &Target) -> Option<bool> {
        self.levels.0.parked_target_visible(dim, target)
    }
    fn side_resident_target_id(game: &Self::Game<'_>, target: Target) -> Option<u32> {
        game.resident_target_id(target)
    }
    fn side_server_resident_entity(game: &Self::Game<'_>, id: u32) -> EntityLookup<Target> {
        game.server_resident_entity(id)
    }
    fn side_remote_sea_level(&self) -> Option<i32> {
        None
    }
    fn side_remote_environment(&self) -> Option<&Environment> {
        None
    }
    fn side_remote_dimension_context(&self) -> Option<(&str, Option<&DimensionFacts>)> {
        None
    }
    fn side_client_has_integrated(&self) -> bool {
        false
    }

    fn side_with_level_access<R>(
        game: &mut Self::Game<'_>,
        placement_preview: bool,
        f: impl FnOnce(&Behaviours, &mut GameLevel<'_>, &mut Ticks) -> R,
    ) -> R {
        game.server_with_level_access(placement_preview, f)
    }
    fn side_after_with_level(game: &mut Self::Game<'_>) {
        game.server_after_with_level()
    }
    fn side_set_block_live(
        game: &mut Self::Game<'_>,
        pos: [i32; 3],
        state: BlockStateId,
        flags: i32,
        create_entity: bool,
    ) -> bool {
        game.server_set_block_live(pos, state, flags, create_entity)
    }
    fn side_update_chunk_tickets(game: &mut Self::Game<'_>) {
        game.server_update_chunk_tickets()
    }
    fn side_section_mesh_pending_in(game: &Self::Game<'_>, section: (i32, i32, i32)) -> bool {
        game.server_section_mesh_pending_in(section)
    }
    fn side_landing_change_cancelled(
        game: &mut Self::Game<'_>,
        player: Option<u128>,
        position: BlockPos,
        from: BlockStateId,
        to: BlockStateId,
    ) -> bool {
        game.server_landing_change_cancelled(player, position, from, to)
    }
    fn side_tick(game: &mut Self::Game<'_>) -> Result<()> {
        game.server_tick()
    }
    fn side_drop_stack_from_eyes(
        game: &mut Self::Game<'_>,
        stack: ItemStack,
        thrown_from_hand: bool,
    ) -> Option<u32> {
        game.server_drop_stack_from_eyes(stack, thrown_from_hand)
    }
    fn side_give_experience_points(game: &mut Self::Game<'_>, points: i32) {
        game.server_give_experience_points(points)
    }
    fn side_journal_entity(game: &mut Self::Game<'_>, id: u32) {
        game.server_journal_entity(id)
    }
    fn side_active_level_ref(
        game: &Self::Game<'_>,
        epoch: mod_api::WorldEpoch,
    ) -> Option<mod_api::LevelRef> {
        game.server_active_level_ref(epoch)
    }
    fn side_recipes<'__side_g>(game: &'__side_g Self::Game<'_>) -> &'__side_g Recipes {
        game.server_recipes()
    }
    fn side_reset_entity_residency(game: &mut Self::Game<'_>) {
        game.server_reset_entity_residency()
    }
    fn side_game_side_meta(
        game: &Self::Game<'_>,
    ) -> (Option<u128>, Option<mod_api::LevelRef>, bool) {
        game.server_game_side_meta()
    }
}
use sunrise_core::position::BlockPos;
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::GameLevel;
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_gameplay_common::game::side_ops::world::SideOps;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk_streamer::ChunkState;
use sunrise_level::chunk_streamer::ChunkStreamer;
use sunrise_level::environment::Environment;
use sunrise_level::fast_hash::FxHashMap;
use sunrise_level::shulker_box::Shulker;
use sunrise_level::world::World;
use sunrise_sim::crafting::Recipes;
use sunrise_sim::join_seq::JoinSeq;
use sunrise_sim::join_seq::residency::table::EntityLookup;
use sunrise_sim::mobs::Mob;
use sunrise_world::dimension_type::DimensionFacts;
use sunrise_world::level::Behaviours;
use sunrise_world::level::Ticks;
use sunrise_world::registry::BlockStateId;
