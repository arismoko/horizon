use super::*;

/// Gather attributed block-state definition providers at client startup, before resources load
/// (NeoForge's `RegisterBlockStateModels`).
pub fn native_register_block_state_models(
    context: &mod_api::RegistrationContext,
    definitions: &mut Vec<Answered<BlockStateDefinitionRegistration>>,
) {
    crate::game::native_hooks::fire::fire_register_block_state_models(context, definitions);
}
