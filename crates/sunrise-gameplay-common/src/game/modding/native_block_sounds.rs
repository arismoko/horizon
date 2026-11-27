//! The `sound_type` Part method (`IBlockExtension.getSoundType`). The client library asks through
//! `native_client_hooks`; only a state whose block has a subscribing part is converted.
use super::api_values::block_state;
use super::native_surface::FrictionView;
use mod_api::{BlockSoundCause, BlockSoundType};
use sunrise_core::position::BlockPos;
use sunrise_sim::motion::FrictionLevel;
use sunrise_world::{level::data::BlockSound, registry::BlockStateId};

/// The game's side of `sunrise_sim::block_sounds::native_sound_type`: the sim library decides
/// (the state's own sound type), the game fires the hook with the read-only level, and the
/// answer comes back for the library to apply.
pub fn sound_type(
    level: FrictionLevel<'_>,
    pos: BlockPos,
    state: BlockStateId,
    entity: Option<(u32, &str)>,
    cause: BlockSoundCause,
    own: &BlockSoundType,
) -> Option<BlockSoundType> {
    if !block_state::block_listens(mod_api::HookId::SoundType, state) {
        return None;
    }
    let mut sound = own.clone();
    FrictionView(level, None).native_sound_type(pos, state, entity, cause, &mut sound);
    Some(sound)
}

pub trait NativeBlockSoundsGameExt1<'__sg>:
    crate::game::SideGame<'__sg> + sunrise_sim::motion::SideLevel
{
    /// The sound of `state` at `pos` for `cause`, using the block's contextual sound type.
    /// `entity` is the entity stepping, landing, placing or digging, or None for a break or a
    /// placement by no entity.
    fn selected_block_sound(
        &self,
        [x, y, z]: [i32; 3],
        state: BlockStateId,
        entity: Option<(u32, &str)>,
        cause: BlockSoundCause,
    ) -> BlockSound {
        let level = FrictionLevel {
            world: &self.world,
            registry: self.pack.assets.registry,
            context: &self.loop_context,
            side: self,
            coefficient: &|_| None,
        };
        let sound = sunrise_sim::block_sounds::sound_type(
            level,
            self.res.facts(),
            BlockPos::new(x, y, z),
            state,
            entity,
            cause,
        );
        BlockSound::from_type(&sound, cause)
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg> + sunrise_sim::motion::SideLevel>
    NativeBlockSoundsGameExt1<'__sg> for G
{
}
