use super::*;

/// The colour resolvers block tints blend over the biomes around a block (NeoForge's
/// `RegisterColorHandlersEvent.ColorResolvers`): `register(resolver)` adds the mod's own
/// `BiomeColorResolver` and returns the `ColorResolver` its block tint sources pass to
/// `BlockAndTintGetter::block_tint`. Asked once at start on the client, before
/// `register_block_tint_sources` and before resources load.
pub fn native_register_color_resolvers(
    context: &mod_api::RegistrationContext,
    resolvers: &mut mod_api::ColorResolverRegistry,
) {
    crate::game::native_hooks::fire::fire_register_color_resolvers(context, |listen| {
        resolvers.register_listener(listen)
    });
}
