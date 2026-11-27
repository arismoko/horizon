use crate::game::native_surface::FrictionView;
use mod_api::{BlockSoundCause, BlockSoundType};
use sunrise_core::position::BlockPos;
use sunrise_world::registry::BlockStateId;

impl FrictionView<'_> {
    /// `IBlockExtension.getSoundType`: a subscribed state is asked with the side's borrowed
    /// level, as the friction query is, at the shared selection sites (`Entity.playStepSound`'s
    /// path, `BlockItem.place`, the break and fall sounds), including client prediction.
    pub(in crate::game) fn native_sound_type(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        entity: Option<(u32, &str)>,
        cause: BlockSoundCause,
        sound: &mut BlockSoundType,
    ) {
        self.fire_sound_type(pos, state, entity, cause, sound);
    }
}
