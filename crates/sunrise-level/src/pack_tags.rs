//! Tags as resource packs' files name them (`#minecraft:logs`): the ids
//! `data/<namespace>/tags/<registry>/<path>.json` holds, nested tags included, read from a pack
//! overlay root (which links the vanilla data) and kept once read.
use serde_json::Value as Json;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use sunrise_assets::resource_source::ResourceSource;
use sunrise_world::tag_layers::TagLayers;

/// Bind tags read from a resource source (directory, archive or indexed pack).
pub fn bind(
    root: &ResourceSource,
    registry: &str,
    contains: impl Fn(&str) -> bool,
) -> HashMap<String, Vec<String>> {
    let kind = format!("tags/{registry}");
    let separator = format!("/{kind}/");
    let mut raw = sunrise_world::tag_layers::convention::layers(registry);
    for file in root.list_under("data", &[&kind]) {
        let Some((namespace, name)) = file
            .strip_prefix("data/")
            .and_then(|s| s.split_once(&separator))
        else {
            continue;
        };
        let Some(name) = name.strip_suffix(".json") else {
            continue;
        };
        let Some(json) = root
            .read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Json>(&bytes).ok())
        else {
            continue;
        };
        if let Err(err) = raw
            .entry(format!("{namespace}:{name}"))
            .or_default()
            .push_file(json)
        {
            log::warn!("Sunrise: couldn't read tag file {file}: {err}");
        }
    }
    TagLayers::bind(registry, &raw, contains)
}

type TagsByName = HashMap<String, Arc<HashSet<String>>>;

/// The tags under one root, bound on first use of a registry.
#[derive(Debug)]
pub struct Tags {
    root: ResourceSource,
    cache: Mutex<HashMap<String, TagsByName>>,
    report: Json,
    empty: Arc<HashSet<String>>,
}

impl Tags {
    pub fn new(root: &ResourceSource) -> Self {
        Self {
            root: root.clone(),
            cache: Mutex::default(),
            report: root
                .read("registries.json")
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default(),
            empty: Arc::default(),
        }
    }

    /// The ids in `tag` (`minecraft:logs`, without the `#`) of `registry` (`block`,
    /// `entity_type`, `worldgen/biome`...); empty for an unbound tag.
    pub fn members(&self, registry: &str, tag: &str) -> Arc<HashSet<String>> {
        let name = sunrise_core::identifier::Identifier::parse(tag).map(|id| id.to_string());
        let Ok(name) = name else {
            return self.empty.clone();
        };
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let tags = cache.entry(registry.to_owned()).or_insert_with(|| {
            let reported = self.report[format!("minecraft:{registry}")]["entries"].as_object();
            bind(&self.root, registry, |id| {
                if let Some(entries) = reported {
                    entries.contains_key(id)
                } else if matches!(registry, "dimension_type" | "worldgen/biome") {
                    id.split_once(':').is_some_and(|(ns, name)| {
                        self.root
                            .contains(&format!("data/{ns}/{registry}/{name}.json"))
                    })
                } else {
                    // Standalone visual pack sources don't necessarily include a registry report.
                    true
                }
            })
            .into_iter()
            .map(|(name, ids)| (name, Arc::new(ids.into_iter().collect())))
            .collect()
        });
        tags.get(&name).unwrap_or(&self.empty).clone()
    }

    /// Whether `id` is in `tag` of `registry`.
    pub fn contains(&self, registry: &str, tag: &str, id: &str) -> bool {
        self.members(registry, tag).contains(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_tags_optional_entries_and_cycles_resolve() {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("sunrise-pack-tags-{}-{n}", std::process::id()));
        let write = |path: &str, json: serde_json::Value| {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, json.to_string()).unwrap();
        };
        write(
            "data/minecraft/tags/block/logs.json",
            serde_json::json!({"values": ["oak_log", "#pack:fancy", {"id": "mod:log", "required": false}, {"id":"minecraft:oak_log", "remove":true}, "minecraft:oak_log"], "remove":["minecraft:cherry_log"]}),
        );
        // A tag that includes the one including it.
        write(
            "data/pack/tags/block/fancy.json",
            serde_json::json!({"values": ["minecraft:cherry_log", {"id":"#minecraft:logs","required":false}]}),
        );
        let tags = Tags::new(&ResourceSource::directory(root.clone()));
        let mut logs: Vec<String> = tags.members("block", "logs").iter().cloned().collect();
        logs.sort();
        assert_eq!(logs, ["minecraft:oak_log", "mod:log"]);
        assert!(tags.contains("block", "minecraft:logs", "minecraft:oak_log"));
        // Another registry's tag of the same name, and a tag no file defines, are empty.
        assert!(tags.members("entity_type", "logs").is_empty());
        assert!(tags.members("block", "pack:none").is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
