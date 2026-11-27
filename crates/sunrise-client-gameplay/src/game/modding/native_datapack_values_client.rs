//! Client-side datapack vanilla-value binding, remote table lookup, and obstruction queries.

use crate::game::ClientGame;
use mod_api::{
    BlockKey, BlockPredicateValue, BlockState, BlockStateProviderValue, DatapackValueError,
    HolderSetValue, IntProviderValue, LevelRef, RandomSource as ModRandomSource,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use sunrise_gameplay_common::game::native_datapack_values::{
    DatapackValuesSide, DatapackVanillaValues, GameDatapackValues, VanillaValueTableBuilder,
    datapack_active_world_entities_unobstructed, datapack_cell_and_blocker,
};
use sunrise_gameplay_common::game::tables::GameItems as _;
use sunrise_level::world::World;
use sunrise_world::worldgen::placement::PlacementQueryError;
use sunrise_world::worldgen::state_provider::StateProviderCatalog;

fn valid_resource_segment(s: &str, allow_slash: bool) -> bool {
    !s.is_empty()
        && !s.starts_with('/')
        && !s.ends_with('/')
        && !s.contains("//")
        && s.split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && s.bytes().all(|b| {
            b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || matches!(b, b'_' | b'-' | b'.')
                || (allow_slash && b == b'/')
        })
}

fn read_builtin_state_provider(
    resources: &std::path::Path,
    name: &str,
    bundle: &mut Option<Option<serde_json::Map<String, serde_json::Value>>>,
) -> Option<String> {
    let (ns, path) = name.split_once(':').unwrap_or(("minecraft", name));
    if !valid_resource_segment(ns, false) || !valid_resource_segment(path, true) {
        return None;
    }
    let file = resources
        .join("data")
        .join(ns)
        .join("worldgen/block_state_provider")
        .join(format!("{path}.json"));
    if let Ok(text) = std::fs::read_to_string(&file) {
        return Some(text);
    }
    if ns == "minecraft" {
        let map = bundle
            .get_or_insert_with(|| {
                let bytes = std::fs::read(resources.join("worldgen/state_providers.json")).ok()?;
                serde_json::from_slice::<serde_json::Value>(&bytes)
                    .ok()
                    .and_then(|v| v.as_object().cloned())
            })
            .as_ref()?;
        if let Some(value) = map.get(name).or_else(|| map.get(path)) {
            return Some(
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string()),
            );
        }
    }
    None
}

impl<'g> DatapackValuesSide<'g> for ClientGame<'g> {
    fn active_datapack_vanilla_values(&self) -> Option<&DatapackVanillaValues> {
        self.client
            .remote
            .as_ref()
            .and_then(|remote| remote.datapack_vanilla_values.as_deref())
    }

    fn datapack_block_tags(&self) -> &HashMap<String, HashSet<String>> {
        self.client
            .remote
            .as_ref()
            .and_then(|r| r.tags.as_ref())
            .map_or(&self.items().block_tags, |tags| &tags.block_tags)
    }

    fn is_datapack_block_unobstructed(
        &self,
        level: LevelRef,
        world: &World,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        let (cell, mut obstructs) = datapack_cell_and_blocker(position);
        let is_active = std::ptr::eq(world, &self.common.world);

        if is_active
            && !self.common.spectator()
            && self
                .common
                .player
                .as_ref()
                .is_some_and(|p| obstructs(p.bounds()))
        {
            return Ok(false);
        }

        if is_active {
            let remote = self.client.remote.as_ref();
            return datapack_active_world_entities_unobstructed(
                self,
                level,
                cell,
                obstructs,
                |mob| {
                    &*mob.kind.id == "minecraft:player"
                        && remote.is_some_and(|r| {
                            r.remote_player_info_game_mode(mob.id as i32) == Some(3)
                        })
                },
            );
        }

        Ok(true)
    }
}

impl ClientGame<'_> {
    /// Re-decode every vanilla value in `received` against the client's current registries and
    /// tags, returning the bound set and its live value table without mutating `self.client.remote`.
    fn bind_client_datapack_set(
        &self,
        received: &mod_api::DatapackRegistrySet,
    ) -> sunrise_game_data::Result<(
        Arc<mod_api::DatapackRegistrySet>,
        Option<Arc<DatapackVanillaValues>>,
    )> {
        let declarations = mod_api::__private::datapack_registry_declarations();
        if !mod_api::__private::declarations_use_vanilla_values(declarations) {
            return Ok((Arc::new(received.clone()), None));
        }
        let (block_tags, fluid_tags, biome_tags) = self.client_datapack_tags();
        let catalog = self.client_state_provider_catalog()?;
        let biomes = self
            .common
            .world_biomes
            .as_deref()
            .unwrap_or_else(|| self.common.res.biomes());
        let biome_names = biomes.registry_order();
        let mut decoder = VanillaValueTableBuilder::new(
            self.common.pack.assets.registry,
            self.common.res.behaviours(),
            &block_tags,
            &fluid_tags,
            &biome_names,
            &biome_tags,
            catalog.as_ref(),
        )
        .map_err(|e| format!("client datapack vanilla values: {e}"))?;
        let bound = mod_api::__private::rebind_datapack_vanilla_values(
            received,
            declarations,
            &mut decoder,
        )
        .map_err(|error| {
            format!("The server's mod registries contain an invalid vanilla value: {error:?}")
        })?;
        Ok((Arc::new(bound), Some(Arc::new(decoder.finish()))))
    }

    /// Atomically bind `set` and swap both `remote.datapacks` and `remote.datapack_vanilla_values`
    /// only on success.
    pub fn install_client_datapack_set(
        &mut self,
        set: &mod_api::DatapackRegistrySet,
    ) -> sunrise_game_data::Result<()> {
        let (bound, values) = self.bind_client_datapack_set(set)?;
        if let Some(remote) = self.client.remote.as_mut() {
            remote.datapacks.replace_received(bound);
            remote.datapack_vanilla_values = values;
        }
        Ok(())
    }

    /// Atomically install a configuration-phase `datapacks` receiver: if it carries a set, bind
    /// its vanilla values before swapping `remote.datapacks` and `remote.datapack_vanilla_values`.
    pub fn install_client_datapack_receiver(
        &mut self,
        mut datapacks: sunrise_sim::datapack_net::Receiver,
    ) -> sunrise_game_data::Result<()> {
        let bound = match datapacks.received() {
            Some(received) => Some(self.bind_client_datapack_set(received)?),
            None => None,
        };
        if let Some((set, values)) = bound {
            datapacks.replace_received(set);
            if let Some(remote) = self.client.remote.as_mut() {
                remote.datapacks = datapacks;
                remote.datapack_vanilla_values = values;
            }
        } else if let Some(remote) = self.client.remote.as_mut() {
            remote.datapacks = datapacks;
            remote.datapack_vanilla_values = None;
        }
        Ok(())
    }

    /// Build a `StateProviderCatalog` from the remote server's synced
    /// `minecraft:worldgen/block_state_provider` registry (converting `Some(tag)` via `tag_to_json`
    /// and falling back to the client's built-in `worldgen/block_state_provider` JSON when `data`
    /// is `None` from the known-pack optimization). Never falls back to a local world's
    /// `live_features()`.
    fn client_state_provider_catalog(
        &self,
    ) -> sunrise_game_data::Result<Option<StateProviderCatalog>> {
        let Some(entries) = self
            .client
            .remote
            .as_ref()
            .and_then(|r| {
                r.registries
                    .registries
                    .get("minecraft:worldgen/block_state_provider")
            })
            .filter(|entries| !entries.is_empty())
        else {
            return Ok(None);
        };
        let mut resources = BTreeMap::new();
        let mut builtin_bundle: Option<Option<serde_json::Map<String, serde_json::Value>>> = None;
        for (name, data) in entries {
            if let Some(tag) = data {
                resources.insert(
                    name.clone(),
                    sunrise_game_data::scoreboard::tag_to_json(tag).to_string(),
                );
                continue;
            }
            if let Some(json) = read_builtin_state_provider(
                &self.common.options.resources,
                name,
                &mut builtin_bundle,
            ) {
                resources.insert(name.clone(), json);
            }
        }
        if resources.is_empty() {
            return Ok(None);
        }
        StateProviderCatalog::from_resources(&resources)
            .map(Some)
            .map_err(|error| {
                format!("client worldgen/block_state_provider catalog: {error}").into()
            })
    }

    fn client_datapack_tags(
        &self,
    ) -> (
        HashMap<String, HashSet<String>>,
        HashMap<String, HashSet<String>>,
        HashMap<String, HashSet<String>>,
    ) {
        let fallback_blocks = &self.items().block_tags;
        let fallback_fluids = &self.items().fluid_tags;
        let Some(remote) = self.client.remote.as_ref() else {
            return (
                fallback_blocks.clone(),
                fallback_fluids.clone(),
                HashMap::new(),
            );
        };
        let names = sunrise_protocol::registry::SyncedNames {
            builtin: &remote.builtin,
            dynamic: &remote.registries,
        };
        let decode_tags = |registry: &str| -> Option<HashMap<String, HashSet<String>>> {
            let tags = remote.registries.tags.get(registry)?;
            Some(
                tags.iter()
                    .map(|(tag, ids)| {
                        let members = ids
                            .iter()
                            .filter_map(|&id| {
                                sunrise_protocol::item::Names::name(&names, registry, id)
                            })
                            .collect();
                        (tag.clone(), members)
                    })
                    .collect(),
            )
        };
        let block_tags = decode_tags("minecraft:block")
            .or_else(|| remote.tags.as_ref().map(|t| t.block_tags.clone()))
            .unwrap_or_else(|| fallback_blocks.clone());
        let fluid_tags = decode_tags("minecraft:fluid")
            .or_else(|| remote.tags.as_ref().map(|t| t.fluid_tags.clone()))
            .unwrap_or_else(|| fallback_fluids.clone());
        let biome_tags = decode_tags("minecraft:worldgen/biome").unwrap_or_default();
        (block_tags, fluid_tags, biome_tags)
    }

    pub(in crate::game) fn datapack_int_provider_sample(
        &self,
        value: IntProviderValue,
        random: &mut dyn ModRandomSource,
    ) -> Result<i32, DatapackValueError> {
        GameDatapackValues::datapack_int_provider_sample(self, value, random)
    }

    pub(in crate::game) fn datapack_block_state_provider_state(
        &self,
        value: BlockStateProviderValue,
        level: LevelRef,
        random: &mut dyn ModRandomSource,
        pos: [i32; 3],
    ) -> Result<BlockState, DatapackValueError> {
        GameDatapackValues::datapack_block_state_provider_state(self, value, level, random, pos)
    }

    pub(in crate::game) fn datapack_block_predicate_test(
        &self,
        value: BlockPredicateValue,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Result<bool, DatapackValueError> {
        GameDatapackValues::datapack_block_predicate_test(self, value, level, pos)
    }

    pub(in crate::game) fn datapack_holder_set_contains(
        &self,
        value: HolderSetValue,
        block: BlockKey,
    ) -> Result<bool, DatapackValueError> {
        GameDatapackValues::datapack_holder_set_contains(self, value, block)
    }
}
