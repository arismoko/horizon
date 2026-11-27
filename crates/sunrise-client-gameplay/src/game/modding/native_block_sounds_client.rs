use crate::game::ClientGame;
use mod_api::{BlockSoundCause, SoundSource};
use sunrise_world::{level::data::BlockSound, registry::BlockStateId};

impl ClientGame<'_> {
    /// [`CommonGame::selected_block_sound`] for the sounds this window's player plays itself.
    pub(in crate::game) fn own_block_sound(
        &self,
        pos: [i32; 3],
        state: BlockStateId,
        cause: BlockSoundCause,
    ) -> BlockSound {
        let player = (self.own_entity_id() as u32, "minecraft:player");
        self.selected_block_sound(pos, state, Some(player), cause)
    }
    /// `LevelEventHandler`'s break sound (level event 2001, which a client's predicted break
    /// also plays): the block's break sound at its centre, at `(volume + 1) / 2` and pitch × 0.8.
    /// None for air, or when a part on the block cancels it (`play_break_sound`).
    pub fn play_break_sound(&mut self, pos: [i32; 3], state: BlockStateId) {
        if self.common.res.facts().is_air(state) || self.mod_play_break_sound(state, pos) {
            return;
        }
        let sound = self.selected_block_sound(pos, state, None, BlockSoundCause::Break);
        self.play_audio_on(
            SoundSource::Blocks.index(),
            sound.event.as_str(),
            Some(pos.map(|v| f64::from(v) + 0.5)),
            (sound.volume + 1.0) / 2.0,
            sound.pitch * 0.8,
        );
    }
}

use sunrise_gameplay_common::game::native_block_sounds::NativeBlockSoundsGameExt1 as _;
use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
