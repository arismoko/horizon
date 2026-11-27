use crate::game::ClientGame;
use mod_api::{LevelReadError, LevelRef};
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

impl ClientGame<'_> {
    pub(in crate::game) fn level_tick_rate(
        &self,
        level: LevelRef,
    ) -> Result<Option<f32>, LevelReadError> {
        Ok((self.native_client_level() == Some(level)).then(|| self.common.clock.rate()))
    }
}
