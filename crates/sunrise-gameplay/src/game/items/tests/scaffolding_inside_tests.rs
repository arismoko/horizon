//! `ScaffoldingBlockItem.updatePlacementContext` reads the click's inside flag:
//! sneaking while the ray starts inside a scaffolding block places on the
//! opposite face, which the client's own raycast (`VoxelShape.clip`) supplies
//! and `BlockHitResult.isInside` carries.

/// The block name at `pos`.
pub fn name(game: &crate::game::ServerGame<'_>, pos: [i32; 3]) -> String {
    let state = game.common.world.block(pos[0], pos[1], pos[2]);
    game.common.res.facts().get(state).name.to_string()
}

pub fn set_sneak(game: &mut crate::game::ServerGame<'_>, held: bool) {
    game.server_set_acting_sneak(held);
}
