//! Pack-layered `data/neoforge/recipe_priorities.json` (NeoForge's public
//! recipe-priorities format). Order the shared sources before any table is built.
use super::{ResolvedSources, registration_key};
use crate::Result;
use serde::Deserialize as _;
use serde_json::Value;
use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashMap, hash_map::Entry},
    path::Path,
};
use sunrise_core::identifier::Identifier;
use sunrise_world::{
    conditions::LoadConditions,
    loot::{PackOverlay, RecipePriorityLayer},
};

/// One `recipe_priorities.json`, validated whole before any of it applies.
#[derive(serde::Deserialize)]
struct Layer {
    #[serde(default)]
    replace: bool,
    #[serde(default)]
    entries: BTreeMap<String, i32>,
}

#[derive(Default)]
struct Priorities(HashMap<String, i32>);

impl Priorities {
    /// Apply one layer, or refuse it as a whole naming `source`: a root that is not an object,
    /// a `replace` that is not a boolean, an `entries` that is not an object of signed 32-bit
    /// integers, or invalid load conditions. A layer with a valid false condition changes nothing.
    fn layer(&mut self, layer: &RecipePriorityLayer, conditions: &LoadConditions) -> Result<()> {
        let RecipePriorityLayer {
            pack,
            path,
            contents,
        } = layer;
        let refused =
            |reason: String| format!("recipe priorities of {pack}: {path} refused: {reason}");
        let bytes = match contents {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("{pack}: {path} not readable: {error}; recipe priorities skipped");
                return Ok(());
            }
        };
        let file: Value = serde_json::from_slice(bytes).map_err(|e| refused(e.to_string()))?;
        if !file.is_object() {
            return Err(refused("the root must be an object".into()).into());
        }
        if file
            .get("neoforge:conditions")
            .is_some_and(|value| !value.is_array())
        {
            return Err(refused("neoforge:conditions must be an array".into()).into());
        }
        if !conditions
            .allows_file_isolated(&file, path)
            .map_err(refused)?
        {
            return Ok(());
        }
        let layer = Layer::deserialize(&file).map_err(|e| refused(e.to_string()))?;
        let mut entries = HashMap::with_capacity(layer.entries.len());
        for (name, priority) in layer.entries {
            let id = Identifier::parse(&name)
                .map_err(|error| refused(format!("invalid recipe ID {name:?}: {error}")))?;
            match entries.entry(id.to_string()) {
                Entry::Vacant(slot) => {
                    slot.insert(priority);
                }
                Entry::Occupied(slot) => {
                    return Err(refused(format!("duplicate recipe ID {}", slot.key())).into());
                }
            }
        }
        if layer.replace {
            self.0.clear();
        }
        // Unknown ids are intentionally retained, including conditioned-out recipes.
        self.0.extend(entries);
        Ok(())
    }

    fn load(root: &Path, overlay: &PackOverlay) -> Result<Self> {
        let inventory = sunrise_mod_package::enabled::EnabledMods::read(root)?;
        let conditions = overlay
            .conditions
            .with_mod_inventory(inventory.server_ids());
        let mut priorities = Self::default();
        let base = sunrise_assets::resource_source::ResourceSource::directory(root.to_owned());
        if let Some(layer) = crate::datapack::read_recipe_priority_layer("vanilla", &base) {
            priorities.layer(&layer, &conditions)?;
        }
        for layer in overlay.priority_layers {
            priorities.layer(layer, &conditions)?;
        }
        Ok(priorities)
    }

    fn get(&self, name: &str) -> i32 {
        if name.contains(':') {
            self.0.get(name).copied().unwrap_or(0)
        } else {
            self.0
                .get(&format!("minecraft:{name}"))
                .copied()
                .unwrap_or(0)
        }
    }
}

impl ResolvedSources {
    /// Order `sources` once for a load: descending priority, then vanilla's registration order.
    pub(crate) fn order_by_priority(
        sources: &mut [(String, Value)],
        root: &Path,
        overlay: &PackOverlay,
    ) -> Result<()> {
        let priorities = Priorities::load(root, overlay)?;
        sources.sort_by_cached_key(|(name, _)| {
            (Reverse(priorities.get(name)), registration_key(name))
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests;
