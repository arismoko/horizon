use super::*;

pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// `EntityEvent.Size`: the size vanilla's `Entity.refreshDimensions` just computed for
    /// `entity` in `pose`, before its box and eyes use it (`modding/entity_sizes.rs`).
    fn native_entity_size(
        &self,
        entity: EntityRef,
        pose: mod_api::Pose,
        old_size: [f32; 3],
        size: &mut [f32; 3],
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_entity_size(
            self, entity, pose, old_size, size,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
