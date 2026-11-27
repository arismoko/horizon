use sunrise_core::geometry::Direction;
use sunrise_items::items::ItemStack;
use sunrise_world::level::PlacementContext;

impl crate::game::ServerGame<'_> {
    /// `BlockItemExtension.onBlockPlacedBy`: once the block's final state,
    /// stack data and `Block.setPlacedBy` companions exist, before consumption.
    pub(in crate::game) fn native_on_block_placed_by(
        &mut self,
        call: super::fire::OnBlockPlacedByCall<'_>,
    ) {
        self.fire_on_block_placed_by(call);
    }

    /// Dispatch only for this stack's subscribers, without converting an
    /// unobserved stack or constructing a placement event.
    pub(in crate::game) fn block_item_placed(
        &mut self,
        stack: &ItemStack,
        placer: Option<mod_api::PlayerRef>,
        ctx: &PlacementContext,
    ) {
        if !mod_api::__private::item_listens(mod_api::HookId::OnBlockPlacedBy, &stack.item) {
            return;
        }
        let Some(level) = self.level(self.common.dim.id()) else {
            return;
        };
        // setPlacedBy (for example a jukebox) can change the state again.
        let pos = ctx.pos;
        let state = self.common.world.block(pos.x, pos.y, pos.z);
        let face = match ctx.face {
            Direction::Down => mod_api::BlockFace::Down,
            Direction::Up => mod_api::BlockFace::Up,
            Direction::North => mod_api::BlockFace::North,
            Direction::South => mod_api::BlockFace::South,
            Direction::West => mod_api::BlockFace::West,
            Direction::East => mod_api::BlockFace::East,
        };
        self.native_on_block_placed_by(super::fire::OnBlockPlacedByCall {
            stack,
            placer,
            level,
            pos: [pos.x, pos.y, pos.z],
            state,
            face,
            click: [ctx.hit.x, ctx.hit.y, ctx.hit.z],
        });
    }
}
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
