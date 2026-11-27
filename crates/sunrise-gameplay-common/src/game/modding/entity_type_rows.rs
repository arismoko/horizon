//! The entity types of a resource root's `entity-types.json`: vanilla's and the content mods' (each
//! mod type has its template's row). A living entity's row lists its default attributes, so a row
//! with `attributes` is a living type (`LivingEntity`), and one without is not (a boat, an arrow, a
//! display). Read once per root: the file doesn't change while the game runs.
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

/// The entity types under one root, each list sorted.
#[derive(Debug, Default, PartialEq)]
pub struct EntityTypeRows {
    /// Every entity type (`minecraft:boat`), namespaced as commands name it.
    pub ids: Vec<String>,
    /// Every entity type except `minecraft:player`, sorted: what `EntityLayers::entity_types`
    /// borrows directly without allocating on every call.
    pub non_player_ids: Vec<String>,
    /// The living ones.
    pub(in crate::game) living: Vec<String>,
    /// Each type's `EntityType.getDimensions()` (width, height, eye height), by id.
    pub(in crate::game) dimensions: Vec<(String, [f32; 3])>,
}

impl EntityTypeRows {
    /// The rows of `entity-types.json`'s text; none when it isn't an object.
    pub fn parse(bytes: &[u8]) -> Self {
        let rows = serde_json::from_slice::<serde_json::Value>(bytes).unwrap_or_default();
        let mut out = Self::default();
        for (id, row) in rows.as_object().into_iter().flatten() {
            let id = sunrise_command::args::normalize(id);
            if row.get("attributes").is_some() {
                out.living.push(id.clone());
            }
            let size = |key| row.get(key).and_then(serde_json::Value::as_f64);
            if let (Some(width), Some(height), Some(eye)) =
                (size("width"), size("height"), size("eye_height"))
            {
                // The exporter writes the float fields as their shortest decimal.
                out.dimensions
                    .push((id.clone(), [width as f32, height as f32, eye as f32]));
            }
            out.ids.push(id);
        }
        out.ids.sort();
        out.living.sort();
        out.dimensions.sort_by(|a, b| a.0.cmp(&b.0));
        out.non_player_ids = out
            .ids
            .iter()
            .filter(|id| id.as_str() != "minecraft:player")
            .cloned()
            .collect();
        out
    }
}

/// Each root's rows, read on first use.
pub(in crate::game) static ROWS: Mutex<Vec<(PathBuf, Arc<EntityTypeRows>)>> =
    Mutex::new(Vec::new());

/// The entity types under `root` (none when its `entity-types.json` is missing or unreadable).
pub fn entity_type_rows(root: &Path) -> Arc<EntityTypeRows> {
    let mut cache = ROWS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, rows)) = cache.iter().find(|(cached, _)| cached == root) {
        return rows.clone();
    }
    let bytes = std::fs::read(root.join("entity-types.json")).unwrap_or_default();
    let rows = Arc::new(EntityTypeRows::parse(&bytes));
    cache.push((root.to_owned(), rows.clone()));
    rows
}

impl EntityTypeRows {
    /// Whether `id` is an entity type.
    pub fn contains(&self, id: &str) -> bool {
        self.ids.binary_search_by(|t| t.as_str().cmp(id)).is_ok()
    }
    /// `EntityType.getDimensions()` of `id`: its width, height and eye height.
    pub fn dimensions(&self, id: &str) -> Option<[f32; 3]> {
        let i = self
            .dimensions
            .binary_search_by(|(t, _)| t.as_str().cmp(id))
            .ok()?;
        Some(self.dimensions[i].1)
    }
    /// Whether `id` is a living entity type.
    pub fn is_living(&self, id: &str) -> bool {
        self.living.binary_search_by(|t| t.as_str().cmp(id)).is_ok()
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;

    #[test]
    fn a_row_with_attributes_is_a_living_type() {
        let rows = EntityTypeRows::parse(
            br#"{"minecraft:oak_boat": {"width": 1.375},
                 "minecraft:player": {"width": 0.6, "attributes": {}},
                 "minecraft:zombie": {"width": 0.6, "height": 1.95, "eye_height": 1.74,
                                      "attributes": {"minecraft:armor": 2.0}},
                 "ruby:golem": {"template": "minecraft:zombie", "attributes": {}}}"#,
        );
        assert_eq!(
            rows.ids,
            [
                "minecraft:oak_boat",
                "minecraft:player",
                "minecraft:zombie",
                "ruby:golem",
            ]
        );
        assert_eq!(
            rows.non_player_ids,
            ["minecraft:oak_boat", "minecraft:zombie", "ruby:golem"]
        );
        assert!(rows.contains("minecraft:oak_boat"));
        assert!(!rows.contains("nope:ghost"));
        assert!(!rows.is_living("minecraft:oak_boat"));
        assert!(rows.is_living("minecraft:zombie"));
        assert!(rows.is_living("ruby:golem"));
        // A row without all three sizes has none.
        assert_eq!(rows.dimensions("minecraft:zombie"), Some([0.6, 1.95, 1.74]));
        assert_eq!(rows.dimensions("minecraft:oak_boat"), None);
    }

    /// A content mod's type has its template's row, sizes included; a row without all three
    /// sizes, or a type with no row, answers none.
    #[test]
    fn a_mod_types_row_carries_its_templates_size() {
        let rows = EntityTypeRows::parse(
            br#"{"minecraft:cow": {"width": 0.9, "height": 1.4, "eye_height": 1.3,
                                   "attributes": {}},
                 "ruby:moo": {"template": "minecraft:cow", "width": 0.9, "height": 1.4,
                              "eye_height": 1.3, "attributes": {}},
                 "ruby:half": {"template": "minecraft:cow", "width": 0.9, "attributes": {}}}"#,
        );
        assert_eq!(rows.dimensions("ruby:moo"), Some([0.9, 1.4, 1.3]));
        assert_eq!(rows.dimensions("ruby:half"), None);
        assert_eq!(rows.dimensions("ruby:absent"), None);
    }
}
