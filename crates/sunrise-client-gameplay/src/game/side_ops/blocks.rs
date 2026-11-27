#[cfg(feature = "client")]
#[allow(unused_variables)]
impl SideOps for crate::game::client_side::ClientState {
    type Game<'g> = crate::game::ClientGame<'g>;

    fn side_encode_live_block_entity(
        game: &Self::Game<'_>,
        key: (i32, i32, i32),
        entity: &BlockEntity,
        block: &str,
    ) -> Tag {
        game.client_encode_live_block_entity(key, entity, block)
    }
    fn side_station_changed(game: &mut Self::Game<'_>) {
        game.client_station_changed()
    }
    fn side_combiner_take_effects(
        game: &mut Self::Game<'_>,
        kind: stations_common::Kind,
        pos: [i32; 3],
        level_change: i32,
    ) -> bool {
        game.client_combiner_take_effects(kind, pos, level_change)
    }
    fn side_move_local_player(
        game: &mut Self::Game<'_>,
        mover: MoverType,
        movement: Vec3,
        auto_jump: bool,
    ) -> Result<Option<Vec3>> {
        game.client_move_local_player(mover, movement, auto_jump)
    }
    fn side_push_piston_entities(game: &mut Self::Game<'_>, pushes: Vec<piston_push::Push>) {
        game.client_push_piston_entities(pushes)
    }
    fn side_carry_pass(game: &mut Self::Game<'_>, start: &Prepared) {
        game.client_carry_pass(start)
    }
}
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::piston_push;
use sunrise_gameplay_common::game::side_ops::blocks::SideOps;
use sunrise_gameplay_common::game::stations_common;
use sunrise_level::containers::BlockEntity;
use sunrise_world::entity_move::MoverType;
use sunrise_world::moving_colliders::Prepared;
