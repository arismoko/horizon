//! Loaded-mod query retained for generated `LevelsHost::is_loaded` routes.

/// Whether a mod with the `sunrise-mod.toml` id `id` is installed: a
/// native or content package (Fabric's `FabricLoader.isModLoaded`,
/// NeoForge's `ModList.isLoaded`). The list is fixed at startup.
pub fn is_loaded(id: &str) -> bool {
    mod_api::__private::loaded_mods().get(id).is_some()
}
