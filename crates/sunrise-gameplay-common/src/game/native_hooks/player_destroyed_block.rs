use sunrise_core::nbt::Compound;
use sunrise_world::registry::BlockStateId;

/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// A player's break removed the block `state` from `pos`:
    /// `ServerPlayerGameMode.destroyBlock`'s `Block.destroy` call, once
    /// `Level.removeBlock` succeeded. `state` and `block_entity` (the saved
    /// data of the block entity there) are read just before the removal.
    /// The breaking client fires it too (`MultiPlayerGameMode.destroyBlock`),
    /// with the state it read before the removal and no block entity.
    fn native_player_destroyed_block(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        block_entity: Option<Compound>,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_player_destroyed_block(
            self,
            pos,
            state,
            block_entity,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
