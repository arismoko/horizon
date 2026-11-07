//! Load-owned declarations and resolved mappings for registration-slot keys.
use crate::{
    BlockEntityTypeKey, BlockKey, BlockStateId, BlockStateRange, DimensionKey, EffectKey, FluidKey,
    ItemKey,
};

impl BlockKey {
    /// Validate an incoming slot against declarations, not a world's resolved mappings.
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        crate::parts::with_registering_keys(|registering| {
            registering
                .unwrap_or(crate::parts::requested_keys())
                .blocks
                .id(slot)
                .ok_or("unknown BlockKey slot")?;
            Ok(Self(slot))
        })
    }
}

impl FluidKey {
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        crate::parts::with_registering_keys(|registering| {
            let names = match registering {
                Some(requests) => &requests.fluids,
                None => crate::parts::resolved_fluids()
                    .get()
                    .unwrap_or(&crate::parts::requested_keys().fluids),
            };
            names.id(slot).ok_or("unknown FluidKey slot")?;
            Ok(Self(slot))
        })
    }
}

impl ItemKey {
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        crate::parts::with_registering_keys(|registering| {
            let names = match registering {
                Some(requests) => &requests.items,
                None => resolved_items()
                    .get()
                    .unwrap_or(&crate::parts::requested_keys().items),
            };
            names.id(slot).ok_or("unknown ItemKey slot")?;
            Ok(Self(slot))
        })
    }
}

fn resolved_items() -> &'static std::sync::OnceLock<horizon_mod_runtime::registry_keys::Names> {
    static EMPTY: std::sync::OnceLock<horizon_mod_runtime::registry_keys::Names> =
        std::sync::OnceLock::new();
    crate::parts::installed().map_or(&EMPTY, |installed| &installed.resolved_items)
}

/// Complete the load-owned item table with all registry entries, preserving requested slots.
/// Missing forward references are refused before publishing any keys.
pub fn resolve_items<'a>(
    ids: impl IntoIterator<Item = &'a str>,
) -> Result<(), Vec<crate::parts::ItemKeyError>> {
    let mut names = crate::parts::requested_keys().items.clone();
    let all: std::collections::HashSet<_> = ids.into_iter().collect();
    names.resolve(
        |id| all.contains(id).then_some(()),
        |mod_id, id| crate::parts::ItemKeyError {
            mod_id,
            id: id.to_owned(),
        },
    )?;
    // Registry enumeration is sorted so unrequested entries have reproducible slots.
    let mut all: Vec<_> = all.into_iter().collect();
    all.sort_unstable();
    for id in all {
        names
            .request("horizon", id)
            .expect("registry owns valid ids");
    }
    let _ = resolved_items().set(names);
    Ok(())
}

/// Find an item in the completed registry without allocating at a hook boundary.
pub fn item_key(id: &str) -> Option<ItemKey> {
    Some(ItemKey(*resolved_items().get()?.slots.get(id)?))
}

/// Resolve a typed identifier at a load boundary without formatting it as a string.
pub fn item_key_by_identifier(id: &crate::Identifier) -> Option<ItemKey> {
    Some(ItemKey(*resolved_items().get()?.identifiers.get(id)?))
}

impl EffectKey {
    /// Resolved effects also expose registry entries no mod explicitly requested.
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        crate::parts::with_registering_keys(|registering| {
            let names = match registering {
                Some(requests) => &requests.effects,
                None => crate::parts::resolved_effects()
                    .get()
                    .map_or(&crate::parts::requested_keys().effects, |effects| {
                        &effects.names
                    }),
            };
            names.id(slot).ok_or("unknown EffectKey slot")?;
            Ok(Self(slot))
        })
    }
}

impl BlockEntityTypeKey {
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        crate::parts::with_registering_keys(|registering| {
            let names = match registering {
                Some(requests) => &requests.block_entity_types,
                None => crate::parts::resolved_block_entity_types()
                    .get()
                    .unwrap_or(&crate::parts::requested_keys().block_entity_types),
            };
            names.id(slot).ok_or("unknown BlockEntityTypeKey slot")?;
            Ok(Self(slot))
        })
    }
}

impl DimensionKey {
    /// `minecraft:overworld`.
    pub const OVERWORLD: Self = Self(0);
    /// `minecraft:the_nether`.
    pub const NETHER: Self = Self(1);
    /// `minecraft:the_end`.
    pub const END: Self = Self(2);

    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        let key = Self(slot);
        dimension_id(key).ok_or("unknown DimensionKey slot")?;
        Ok(key)
    }
}

/// The key of dimension `id`, numbering it when the game meets it for the first time.
pub fn dimension_key(id: &str) -> DimensionKey {
    DimensionKey(horizon_mod_runtime::registry_keys::dimension_slot(id))
}

/// The name of a dimension's key, None for a slot the game never gave out.
pub fn dimension_id(key: DimensionKey) -> Option<&'static str> {
    horizon_mod_runtime::registry_keys::dimension_id(key.0)
}

/// A fluid requested by a mod but absent from the completed registry.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct UnknownFluid {
    /// Mod that requested the fluid.
    pub mod_id: &'static str,
    /// Canonical fluid identifier.
    pub id: String,
}

impl std::fmt::Display for UnknownFluid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mod {} requested unknown fluid {}", self.mod_id, self.id)
    }
}

/// Resolve fluid requests against the loaded behavior registry.
pub fn resolve_fluids(ids: &[&str]) -> Result<(), Vec<UnknownFluid>> {
    let requests = crate::parts::requested_keys();
    let mut names = requests.fluids.clone();
    let mut errors = Vec::new();
    for entry in &names.entries {
        if !ids.contains(&entry.id.as_str()) && !requests.declared_fluids.contains_key(&entry.id) {
            errors.extend(entry.mods.iter().map(|mod_id| UnknownFluid {
                mod_id,
                id: entry.id.clone(),
            }));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    for id in ids {
        names
            .request("horizon", id)
            .expect("registry owns valid ids");
    }
    for (slot, entry) in names.entries.iter().enumerate() {
        names
            .utf16_slots
            .insert(entry.id.encode_utf16().collect(), slot as u32);
        if let Some(path) = entry.id.strip_prefix("minecraft:") {
            names
                .utf16_slots
                .insert(path.encode_utf16().collect(), slot as u32);
        }
    }
    names.world_fluids = horizon_world::fluid::FluidKind::ALL
        .iter()
        .map(|kind| names.slots.get(kind.name()).copied())
        .collect();
    let _ = crate::parts::resolved_fluids().set(names);
    Ok(())
}

/// Resolve an engine fluid kind through the load-owned mapping.
pub fn world_fluid_key(kind: horizon_world::fluid::FluidKind) -> Option<FluidKey> {
    crate::parts::resolved_fluids()
        .get()?
        .world_fluids
        .get(kind as usize)
        .copied()
        .flatten()
        .map(FluidKey)
}

/// Find a fluid key at a data-loading boundary.
pub fn fluid_key(id: &str) -> Option<FluidKey> {
    let names = crate::parts::resolved_fluids().get()?;
    let slot = if id.contains(':') {
        names.slots.get(id)
    } else {
        names.slots.get(&format!("minecraft:{id}"))
    };
    Some(FluidKey(*slot?))
}

/// Resolve an NBT UTF-16 fluid name without allocating.
pub fn fluid_key_utf16(id: &[u16]) -> Option<FluidKey> {
    Some(FluidKey(
        *crate::parts::resolved_fluids().get()?.utf16_slots.get(id)?,
    ))
}

/// The canonical registry name of a resolved fluid key.
pub fn fluid_id(key: FluidKey) -> Option<&'static str> {
    crate::parts::resolved_fluids().get()?.id(key.0)
}

/// Resolve all block entity types after registration and content loading.
pub fn resolve_block_entity_types<'a, T>(
    ids: impl IntoIterator<Item = &'a str>,
    resolve: impl FnMut(&str) -> Option<T>,
) -> Result<(), Vec<String>> {
    let names = horizon_mod_runtime::registry_keys::resolve_block_entity_types(
        &crate::parts::requested_keys().block_entity_types,
        ids,
        resolve,
    )?;
    let _ = crate::parts::resolved_block_entity_types().set(names);
    Ok(())
}

/// Lookup an engine-owned name after the registry is resolved.
pub fn block_entity_type_key(id: &str) -> Option<BlockEntityTypeKey> {
    Some(BlockEntityTypeKey(
        *crate::parts::resolved_block_entity_types()
            .get()?
            .slots
            .get(id)?,
    ))
}

/// Canonical name of a load-owned block entity type slot.
pub fn block_entity_type_name(key: BlockEntityTypeKey) -> Option<&'static str> {
    crate::parts::resolved_block_entity_types()
        .get()
        .unwrap_or(&crate::parts::requested_keys().block_entity_types)
        .id(key.0)
}

pub(crate) fn block_entity_type_display_name(key: BlockEntityTypeKey) -> String {
    crate::parts::with_registering_keys(|registering| {
        registering
            .and_then(|r| r.block_entity_types.id(key.0))
            .or_else(|| block_entity_type_name(key))
            .unwrap_or("<invalid>")
            .to_owned()
    })
}

/// A block slot's mappings in one game's current tables.
#[derive(Clone, Copy, Debug)]
// exhaustive: the game builds this mapping at load; mods receive only BlockKey
pub struct ResolvedBlock {
    /// The block's default state.
    pub default: BlockStateId,
    /// Its consecutive state ids.
    pub states: BlockStateRange,
    /// Its index in the behaviour table.
    pub block: horizon_world::level::states::BlockId,
}

thread_local! {
    static BLOCKS: std::cell::RefCell<Option<Vec<ResolvedBlock>>> = const { std::cell::RefCell::new(None) };
}

/// Prepare this game's block slots without changing its current mapping.
pub fn prepare_blocks(
    mut resolve: impl FnMut(&str) -> Option<ResolvedBlock>,
) -> Result<Vec<ResolvedBlock>, Vec<String>> {
    crate::parts::requested_keys()
        .blocks
        .resolve(&mut resolve, |mod_id, id| {
            format!("mod {mod_id} requested unknown block {id}")
        })
}

/// Install a prepared mapping on its owning game thread.
pub fn install_blocks(values: Vec<ResolvedBlock>) {
    BLOCKS.with(|blocks| *blocks.borrow_mut() = Some(values));
}

/// Rebind this game's slots at the owning load boundary.
pub fn resolve_blocks(
    resolve: impl FnMut(&str) -> Option<ResolvedBlock>,
) -> Result<(), Vec<String>> {
    install_blocks(prepare_blocks(resolve)?);
    Ok(())
}

/// The current numeric mapping for a trusted key. None means this thread has no resolved
/// world tables (startup or another owning thread), not an unknown slot.
pub fn block(key: BlockKey) -> Option<ResolvedBlock> {
    BLOCKS.with(|blocks| {
        blocks
            .borrow()
            .as_ref()
            .map(|values| values[key.0 as usize])
    })
}

/// Resolve requested effects once, after every mod's content has been registered.
pub fn resolve_effects<'a>(
    ids: impl IntoIterator<Item = &'a str>,
    resolve: impl FnMut(&str) -> Option<usize>,
) -> Result<(), Vec<String>> {
    let effects = horizon_mod_runtime::registry_keys::resolve_effects(
        &crate::parts::requested_keys().effects,
        ids,
        resolve,
    )
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
    })?;
    let _ = crate::parts::resolved_effects().set(effects);
    Ok(())
}

/// Resolve an effect operation by numeric slot, never by a fresh name scan.
pub fn effect(key: EffectKey) -> Option<&'static horizon_client_core::potions::EffectDef> {
    let index = crate::parts::resolved_effects().get()?.indices[key.0 as usize];
    let vanilla = horizon_client_core::potions::EFFECTS;
    if index < vanilla.len() {
        Some(&vanilla[index])
    } else {
        Some(&horizon_client_core::potions::mod_effects()[index - vanilla.len()])
    }
}

/// Canonical diagnostic name, unavailable until the process effect registry is resolved.
pub fn effect_id(key: EffectKey) -> Option<&'static str> {
    Some(&crate::parts::resolved_effects().get()?.names.entries[key.0 as usize].id)
}

/// Canonical diagnostic name of a trusted, load-owned slot, independent of world resolution.
pub fn block_id(key: BlockKey) -> Option<&'static str> {
    Some(&crate::parts::requested_keys().blocks.entries[key.0 as usize].id)
}

/// Canonical diagnostic name retained by an item registration slot.
pub fn item_id(key: ItemKey) -> Option<&'static str> {
    resolved_items()
        .get()
        .unwrap_or(&crate::parts::requested_keys().items)
        .id(key.0)
}

/// The API key for an engine registry index, also for entries not explicitly requested by mods.
pub fn effect_key(index: usize) -> Option<EffectKey> {
    crate::parts::resolved_effects()
        .get()?
        .keys
        .get(index)
        .copied()
        .map(EffectKey)
}

impl crate::BoolGameRuleKey {
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        crate::parts::with_registering_keys(|keys| {
            keys.unwrap_or(crate::parts::requested_keys())
                .game_rules
                .id(slot)
                .ok_or("unknown bool rule slot")?;
            Ok(Self(slot))
        })
    }
}

#[cfg(test)]
mod item_tests {
    use super::*;
    use crate::{Hooks, ModEntry, Registrar, RegistrationContext};

    struct Requests;
    #[crate::__subscriptions]
    impl Hooks for Requests {
        fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
            assert_eq!(registrar.item_key("stick").0, 0);
        }
    }

    #[test]
    fn completed_items_preserve_forward_slots_and_refuse_unresolved_requests() {
        crate::install_parts(&[ModEntry::new("test", Box::new(Requests))]).unwrap();
        assert!(resolve_items(["minecraft:stone"]).is_err());
        assert!(item_key("minecraft:stone").is_none());
        resolve_items(["minecraft:stone", "minecraft:stick", "minecraft:apple"]).unwrap();
        assert_eq!(item_key("minecraft:stick"), Some(ItemKey(0)));
        let apple = item_key("minecraft:apple").unwrap();
        assert_ne!(apple, ItemKey(0));
        assert_eq!(ItemKey::from_slot(apple.0), Ok(apple));
        assert_eq!(item_id(apple), Some("minecraft:apple"));
        assert!(ItemKey::from_slot(3).is_err());
    }
}
