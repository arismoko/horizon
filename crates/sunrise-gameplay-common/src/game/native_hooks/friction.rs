use crate::game::native_surface::FrictionView;
use sunrise_core::position::BlockPos;
use sunrise_world::registry::BlockStateId;

impl FrictionView<'_> {
    /// `IBlockExtension.getFriction`: a subscribed state is asked with the side's
    /// borrowed level, its position, and the entity (if any) that wants the answer.
    pub(in crate::game) fn native_friction(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        entity: Option<mod_api::EntityRef>,
        friction: &mut f32,
    ) {
        self.fire_friction(pos, state, entity, friction);
    }
}
