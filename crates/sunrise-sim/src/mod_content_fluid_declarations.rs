use super::*;

static FLUID_INTERACTIONS: std::sync::RwLock<Vec<sunrise_mod_package::content::FluidInteraction>> =
    std::sync::RwLock::new(Vec::new());

/// Remembers the fluid interactions the content mods' files declare
/// (`data/<id>/sunrise/fluid_interaction/`), in load order and file name, for each world's open.
pub fn set_fluid_interactions(mods: &[ContentMod]) {
    *FLUID_INTERACTIONS
        .write()
        .unwrap_or_else(|e| e.into_inner()) = mods
        .iter()
        .flat_map(|m| m.content.fluid_interactions.iter().cloned())
        .collect();
}

/// The interactions [`set_fluid_interactions`] remembered.
pub fn fluid_interactions() -> Vec<sunrise_mod_package::content::FluidInteraction> {
    FLUID_INTERACTIONS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}
