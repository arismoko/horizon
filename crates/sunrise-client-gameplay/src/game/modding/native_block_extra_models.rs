#[cfg(feature = "client")]
use mod_api::{BlockExtraModelsError, ModelRef};

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `models` (resolved standalone models) replace those drawn as part of the block at `pos`,
    /// its section meshed again when they change. A stale or unknown handle refuses the list.
    pub(in crate::game) fn set_block_extra_models(
        &mut self,
        pos: [i32; 3],
        models: &[ModelRef],
    ) -> Result<(), BlockExtraModelsError> {
        for &model in models {
            self.client
                .client_pack
                .render
                .validate_standalone_model(model)
                .map_err(BlockExtraModelsError::Model)?;
        }
        let [x, y, z] = pos;
        self.common.world.set_extra_models(x, y, z, models)
    }
}
