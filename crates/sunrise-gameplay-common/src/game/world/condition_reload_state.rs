use super::*;
use crate::game::native_datapack_values::DatapackVanillaValues;

pub struct StagedWorldTables {
    pub tables: Option<WorldTables>,
    pub advancements: Arc<sunrise_sim::advancements::Tree>,
    pub health: sunrise_sim::health::HealthResources,
    pub datapack_registries: Option<mod_api::DatapackRegistrySet>,
    pub datapack_vanilla_values: Option<DatapackVanillaValues>,
    pub data: Arc<sunrise_level::datapack::DataPacks>,
}

pub struct PreparedWorldReload {
    pub structure_spawns: sunrise_sim::mobs::spawn_selection::StructureSpawns,
    pub advancements: Arc<sunrise_sim::advancements::Tree>,
    pub tables: Option<WorldTables>,
    pub health: sunrise_sim::health::HealthResources,
    pub parts: PreparedParts,
    pub wire: Option<Arc<sunrise_sim::lan::data::SyncData>>,
    pub wire_tags: sunrise_sim::lan::data::WorldTags,
    pub datapack_registries: Option<mod_api::DatapackRegistrySet>,
    pub datapack_vanilla_values: Option<DatapackVanillaValues>,
    pub data: Arc<sunrise_level::datapack::DataPacks>,
}

pub struct WorldBiomeRegistry {
    pub table: Arc<Biomes>,
    pub overrides: sunrise_world::worldgen::overrides::WorldgenOverrides,
    differs_from_preload: bool,
}

impl WorldBiomeRegistry {
    pub fn load(
        base: &Arc<Biomes>,
        root: &std::path::Path,
        blocks: &'static sunrise_world::registry::BlockRegistry,
        stack: &super::function_pack_stack::PackStack,
        data: &sunrise_level::datapack::DataPacks,
        overlay: &sunrise_world::loot::PackOverlay<'_>,
        read_errors: &std::collections::BTreeMap<String, String>,
    ) -> Result<Self> {
        use serde_json::Value;
        use std::{
            borrow::Cow,
            collections::{BTreeMap, BTreeSet},
        };
        use sunrise_world::worldgen::biome::{
            AdmittedBiomeGeneration, BiomeDefinition, BiomeGenerationRegistries,
        };
        const KIND: &str = "worldgen/biome";
        if let Some((file, error)) = read_errors
            .iter()
            .find(|(key, _)| key.starts_with("worldgen/biome/"))
        {
            return Err(format!("{file}: {error}").into());
        }
        // Only vanilla holders come from the boot table. Passive mod biomes
        // come from the prepared root selected for this world, not the previous mod inventory.
        let mut winners: BTreeMap<_, _> = base
            .registry_order()
            .into_iter()
            .filter(|id| id.starts_with("minecraft:"))
            .filter_map(|id| {
                base.builtin_file(id)
                    .map(|file| (id.to_owned(), Cow::Borrowed(file)))
            })
            .collect();
        let prepared_mods = sunrise_level::biome::mod_biome_files(root);
        let mut prepared_only: BTreeSet<_> =
            prepared_mods.iter().map(|(id, _)| id.clone()).collect();
        winners.extend(
            prepared_mods
                .into_iter()
                .map(|(id, file)| (id, Cow::Owned(file))),
        );
        for (id, files) in overlay.kind(KIND) {
            let value: Value = serde_json::from_slice(sunrise_world::loot::PackOverlay::top(files))
                .map_err(|error| format!("biome {id}: {error}"))?;
            prepared_only.remove(id);
            winners.insert(id.to_owned(), Cow::Owned(value));
        }
        let modified = sunrise_sim::mod_content::biomes::modified(root);
        let mut payloads = BTreeMap::new();
        let mut omitted: BTreeSet<_> = base
            .registry_order()
            .into_iter()
            .filter(|id| !winners.contains_key(*id))
            .map(str::to_owned)
            .collect();
        let mut candidates = BTreeSet::new();
        for (id, winner) in winners {
            if !overlay.conditions.allows_id(&winner, KIND, &id) {
                omitted.insert(id);
                continue;
            }
            candidates.insert(id.clone());
            let payload = match winner {
                Cow::Borrowed(winner)
                    if !modified.contains(&id) && winner.get("neoforge:conditions").is_none() =>
                {
                    // Unchanged prepared vanilla definitions are already in both base readers.
                    continue;
                }
                winner => sunrise_world::conditions::into_payload(winner.into_owned()),
            };
            payloads.insert(id, payload);
        }
        let (mut inputs, warnings) =
            sunrise_level::datapack::worldgen_overrides(data, overlay.conditions);
        for warning in warnings {
            log::warn!("Sunrise: data pack worldgen: {warning}");
        }
        // `worldgen_overrides` has already run the winning registry files' conditions.
        sunrise_sim::vanillagen::add_passive_structure_templates(
            &mut inputs,
            stack.content.source(),
        );
        let dir = root.join("worldgen");
        let mut resolved = BTreeMap::new();
        let mut overrides = inputs.clone();
        let mut generation = None;
        let mut files = BTreeMap::new();
        for (id, payload) in payloads {
            let generation = match &mut generation {
                Some(generation) => generation,
                slot @ None => slot.insert(BiomeGenerationRegistries::load(
                    &dir,
                    &inputs,
                    blocks,
                    candidates.clone(),
                )?),
            };
            let (file, decoded) = BiomeDefinition::decode(payload, generation)
                .map_err(|error| format!("biome {id}: {error}"))?
                .into_parts();
            files.insert(id.clone(), file.to_string());
            resolved.insert(id, decoded);
        }
        // Prepared-only mod files can already contain modifier output. Let the
        // modifier pass recover their original inputs from the prepared metadata.
        overrides.set_biome_winners(
            files
                .iter()
                .filter(|(id, _)| !prepared_only.contains(*id))
                .map(|(id, file)| (id.clone(), file.clone()))
                .collect(),
            omitted.clone(),
        );
        let mut overrides = sunrise_sim::vanillagen::biome_modifiers_over_packs(&dir, &overrides)?
            .unwrap_or(overrides);
        let mut final_files = files.clone();
        final_files.extend(
            overrides
                .entries("biomes")
                .map(|(id, json)| (id.to_owned(), json.to_owned())),
        );
        let mut packed = Vec::new();
        for (id, json) in &final_files {
            let file = serde_json::from_str::<Value>(json)
                .map_err(|error| format!("biome {id}: {error}"))?;
            if files.get(id).is_some_and(|original| original == json) {
                packed.push((id.to_owned(), file));
                continue;
            }
            let generation = match &mut generation {
                Some(generation) => generation,
                slot @ None => slot.insert(BiomeGenerationRegistries::load(
                    &dir,
                    &inputs,
                    blocks,
                    candidates.clone(),
                )?),
            };
            let (file, decoded) = BiomeDefinition::decode(file, generation)
                .map_err(|error| format!("biome {id}: {error}"))?
                .into_parts();
            packed.push((id.to_owned(), file));
            resolved.insert(id.to_owned(), decoded);
        }
        let differs_from_preload = !packed.is_empty() || !omitted.is_empty();
        let table = if differs_from_preload {
            overrides.set_biome_winners(
                packed
                    .iter()
                    .map(|(id, file)| (id.clone(), file.to_string()))
                    .collect(),
                omitted.clone(),
            );
            overrides.bind_biome_generation(AdmittedBiomeGeneration::new(resolved));
            Arc::new(base.with_admitted_packs(
                packed.iter().map(|(id, file)| (id.as_str(), file)),
                &omitted,
            )?)
        } else {
            base.clone()
        };
        Ok(Self {
            table,
            overrides,
            differs_from_preload,
        })
    }

    pub fn differs_from_preload(&self) -> bool {
        self.differs_from_preload
    }
}
