use crate::game::ClientGame;
use std::{collections::HashMap, sync::Arc};
use sunrise_items::items::EntityTypeTags;

#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// Apply synced block, item, fluid, and entity-type tags plus generated class tags on clients.
    pub fn install_server_items(&mut self) {
        let Some(remote) = self.client.remote.as_ref() else {
            return;
        };
        let names = |registry: &str| -> Option<HashMap<String, Vec<String>>> {
            let ids = remote.builtin.get(registry)?;
            let tags = remote
                .registries
                .tags
                .get(registry)?
                .iter()
                .map(|(name, members)| {
                    let members = members.iter().filter_map(|&id| ids.name(id));
                    (name.clone(), members.map(str::to_owned).collect())
                });
            Some(tags.collect())
        };
        let (blocks, items) = (names("minecraft:block"), names("minecraft:item"));
        let base = self.common.res.items();
        let mut items = base.with_tags(blocks, items);
        if let Some(fluids) = names("minecraft:fluid") {
            items.set_tags(sunrise_items::items::TagRegistry::Fluid, fluids);
        }
        sunrise_gameplay_common::game::class_tags::install(&mut items, self.common.res.facts());
        let tags = EntityTypeTags::from_ordered(
            names(EntityTypeTags::REGISTRY).unwrap_or_default(),
            base.entity_type_tags.ids().clone(),
        );
        items.set_entity_type_tags(tags);
        if let Some(enchantments) = self
            .client
            .remote
            .as_mut()
            .and_then(|r| r.enchantments.as_mut())
        {
            Arc::make_mut(enchantments).entity_tags = items.entity_type_tags.members().clone();
        }
        self.common.server_items = Some(Arc::new(items));
        self.install_remote_registry_snapshots();
    }

    /// The synchronized registries gameplay and drawing read, from the server's registry packet:
    /// the block transformers, the sulfur cube archetypes and the decorated pot patterns. An
    /// entry the server sent without data (a known pack's) is the resource root's own file.
    /// Renderer-only pot patterns apply to both integrated and remote sessions; the integrated
    /// server shares the gameplay tables its server thread installed.
    fn install_remote_registry_snapshots(&self) {
        use sunrise_sim::mobs::sulfur_cube;
        let Some(remote) = self.client.remote.as_ref() else {
            return;
        };
        let entries = |registry: &str| -> Vec<(String, serde_json::Value)> {
            let kind = registry.trim_start_matches("minecraft:");
            let mut found: Vec<_> = remote
                .registries
                .registries
                .get(registry)
                .into_iter()
                .flatten()
                .filter_map(|(name, data)| {
                    let value = match data {
                        Some(tag) => sunrise_game_data::scoreboard::tag_to_json(tag),
                        None => {
                            let (namespace, path) = name.split_once(':')?;
                            let file = self
                                .common
                                .options
                                .resources
                                .join(format!("data/{namespace}/{kind}/{path}.json"));
                            let read = std::fs::read(&file)
                                .map_err(|error| error.to_string())
                                .and_then(|bytes| {
                                    serde_json::from_slice(&bytes)
                                        .map_err(|error| error.to_string())
                                });
                            match read {
                                Ok(value) => value,
                                Err(error) => {
                                    log::error!(
                                        "Sunrise: the server's {registry} entry {name} is not readable from {}: {error}",
                                        file.display()
                                    );
                                    return None;
                                }
                            }
                        }
                    };
                    Some((name.clone(), value))
                })
                .collect();
            found.sort_by(|(a, _), (b, _)| {
                let key = |name: &str| {
                    let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
                    (path.to_owned(), namespace.to_owned())
                };
                key(a).cmp(&key(b))
            });
            found
        };
        let pot_patterns: HashMap<String, String> = entries("minecraft:decorated_pot_pattern")
            .into_iter()
            .filter_map(|(id, value)| Some((id, value["asset_id"].as_str()?.to_owned())))
            .collect();
        self.install_pot_patterns(pot_patterns);
        if self.client.integrated.is_some() {
            return;
        }
        let transformers = entries("minecraft:block_transformer");
        match sunrise_sim::block_transform::Transformers::build(
            transformers.iter().map(|(id, value)| (id.as_str(), value)),
            self.common.pack.assets.registry,
        ) {
            Ok(built) => self.common.res.set_transformers(Arc::new(built)),
            Err(error) => {
                log::error!("Sunrise: the server's block transformers: {error}");
                self.common.res.set_transformers(Arc::default());
            }
        }
        let archetypes = entries("minecraft:sulfur_cube_archetype");
        let tags = self
            .common
            .server_items
            .as_ref()
            .map_or_else(HashMap::new, |items| items.item_tags.clone());
        sulfur_cube::install(sulfur_cube::build(
            &archetypes
                .iter()
                .map(|(id, value)| (id.clone(), value))
                .collect::<Vec<_>>(),
            &tags,
        ));
    }
}
