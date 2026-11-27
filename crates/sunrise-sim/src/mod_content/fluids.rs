// Included in mod_content.rs to retain its private scope.
/// The mods' mob effects in registry order: by id, as items are. Their
/// network ids follow vanilla's in this order.
pub fn mob_effects(mods: &[ContentMod]) -> Vec<&MobEffect> {
    let mut effects: Vec<&MobEffect> = mods.iter().flat_map(|m| &m.content.effects).collect();
    effects.sort_by(|a, b| a.id.cmp(&b.id));
    effects
}

/// The mods' fluids in registry order, by id as effects are. Each takes two
/// network ids after vanilla's, its source's then its flowing form's.
pub fn fluids(mods: &[ContentMod]) -> Vec<&FluidLook> {
    let mut fluids: Vec<&FluidLook> = mods.iter().flat_map(|m| &m.content.fluids).collect();
    fluids.sort_by(|a, b| a.id.cmp(&b.id));
    fluids
}

/// the mods' entity types in registry order, by id as effects are.
pub fn entity_types(mods: &[ContentMod]) -> Vec<&EntityType> {
    let mut types: Vec<&EntityType> = mods.iter().flat_map(|m| &m.content.entity_types).collect();
    types.sort_by(|a, b| a.id.cmp(&b.id));
    types
}

/// The mods' menu types and their owning package IDs, sorted by menu ID.
pub fn menu_types(mods: &[ContentMod]) -> Vec<(&MenuType, &str)> {
    let mut menus: Vec<_> = mods
        .iter()
        .flat_map(|m| {
            m.content
                .menu_types
                .iter()
                .map(|menu| (menu, m.id.as_str()))
        })
        .collect();
    menus.sort_by(|(a, _), (b, _)| a.id.cmp(&b.id));
    menus
}

static FLUID_LOOKS: std::sync::RwLock<BTreeMap<String, FluidLook>> =
    std::sync::RwLock::new(BTreeMap::new());

/// Remembers how the mods' fluids look (`data/<id>/sunrise/fluid/`) and their
/// measures, for `FluidTypeProperties` on either side.
pub fn set_fluid_looks(mods: &[ContentMod]) {
    let looks: BTreeMap<String, FluidLook> = mods
        .iter()
        .flat_map(|m| &m.content.fluids)
        .map(|f| (f.id.clone(), f.clone()))
        .collect();
    *FLUID_LOOKS.write().unwrap_or_else(|e| e.into_inner()) = looks;
}

pub fn with_fluid_looks<R>(f: impl FnOnce(&BTreeMap<String, FluidLook>) -> R) -> R {
    let looks = FLUID_LOOKS.read().unwrap_or_else(|e| e.into_inner());
    f(&looks)
}

/// Registers the world fluids the mods declare, before any block table is read from the root.
/// Returns their registry names, source and flowing, for the fluid keys.
pub fn register_world_fluids(
    mods: &[ContentMod],
) -> std::result::Result<Vec<String>, sunrise_world::fluid::ModFluidRegistrationError> {
    let mut names = Vec::new();
    for look in mods.iter().flat_map(|m| &m.content.fluids) {
        let Some(world) = &look.world else { continue };
        let flowing_id = look.flowing_id();
        names.push(look.id.clone());
        names.push(flowing_id.clone());
        sunrise_world::fluid::register_mod_fluid(sunrise_world::fluid::ModFluidType {
            id: look.id.clone(),
            flowing_id,
            block_id: look.block_id(),
            // Only a bucket the game registers pours this fluid; one the mod names keeps its own.
            bucket: look.bucket_id().filter(|_| world.bucket),
            rules: sunrise_world::fluid::ModFluidRules {
                tick_delay: world.tick_delay,
                drop_off: world.drop_off,
                slope_find_distance: world.slope_find_distance,
                converts_to_source: world.source_conversion,
            },
        })?;
    }
    Ok(names)
}
