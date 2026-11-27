use super::*;

/// Gather attributed item model node providers at client startup, before resources load
/// (NeoForge's `RegisterItemModelsEvent`).
pub fn native_register_item_models(
    context: &mod_api::RegistrationContext,
    models: &mut Vec<Answered<ItemModelRegistration>>,
) {
    crate::game::native_hooks::fire::fire_register_item_models(context, models);
}
