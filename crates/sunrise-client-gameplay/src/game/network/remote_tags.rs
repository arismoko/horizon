//! The server's tags on this client (`ClientPacketListener.handleUpdateTags`, and configuration's
//! `RegistryDataCollector`): install gameplay tags, then notify native mods.
use crate::game::ClientGame;
use std::sync::Arc;
use sunrise_items::items::{Items, TagRegistry};
use sunrise_protocol::{item::Names, packets::configuration::UpdateTags, registry::SyncedNames};

/// The registries whose tags the tag helpers read.
pub(in crate::game) const READ_REGISTRIES: [(&str, TagRegistry); 9] = [
    ("minecraft:block", TagRegistry::Block),
    ("minecraft:item", TagRegistry::Item),
    ("minecraft:fluid", TagRegistry::Fluid),
    ("minecraft:entity_type", TagRegistry::EntityType),
    ("minecraft:worldgen/biome", TagRegistry::Biome),
    ("minecraft:enchantment", TagRegistry::Enchantment),
    ("minecraft:worldgen/structure", TagRegistry::Structure),
    ("minecraft:potion", TagRegistry::Potion),
    ("minecraft:world_clock", TagRegistry::WorldClock),
];
/// The synced registry whose tags `Tags::damage_type_in_tag` reads.
pub(in crate::game) const DAMAGE_TYPE: &str = "minecraft:damage_type";

impl ClientGame<'_> {
    /// `ClientboundUpdateTagsPacket` in play.
    pub fn remote_update_tags(&mut self, tags: &UpdateTags) {
        if let Some(r) = self.client.remote.as_mut() {
            Arc::make_mut(&mut r.registries).add_tags(tags);
        }
        self.install_server_items();
        self.client_refresh_infiniburn();
        self.refresh_creative_search_tags();
        self.install_remote_tags();
        self.refresh_face_visibility_tags();
        if self.common.recipe_tables.server_stations.is_some()
            && tags
                .registries
                .iter()
                .any(|tags| tags.registry == "minecraft:item")
        {
            self.refresh_remote_stonecutter();
        }
        if tags
            .registries
            .iter()
            .any(|tags| tags.registry == "minecraft:timeline")
        {
            if let Some(r) = self.client.remote.as_mut() {
                r.level_environments.clear();
            }
            self.build_level_environments();
            self.sync_timeline_registries();
        }
    }

    pub(in crate::game::client_side) fn refresh_creative_search_tags(&mut self) {
        let tags = self.client.remote.as_ref().and_then(|r| {
            let ids = r.builtin.get("minecraft:item")?;
            let tags = r.registries.tags.get("minecraft:item")?;
            Some(
                tags.iter()
                    .map(|(tag, members)| {
                        let items = members
                            .iter()
                            .filter_map(|&id| ids.name(id).map(str::to_owned))
                            .collect();
                        (tag.clone(), items)
                    })
                    .collect(),
            )
        });
        self.client.creative_screen.0.set_item_tags(tags.as_ref());
    }

    /// Make the server's tags what native mods read, then tell them. Snapshot client
    /// enchant answers only after the tags, rebound parts and tag listeners are published.
    pub fn install_remote_tags(&mut self) {
        if mod_api::__private::mods().is_empty() {
            self.resolve_enchant_particles();
            return;
        }
        let Some(r) = self.client.remote.as_mut() else {
            return;
        };
        let damage_types = r.registries.registries.get(DAMAGE_TYPE);
        if r.registries.tags.is_empty() && damage_types.is_none() {
            r.tags = None;
            self.refresh_recipe_property_context();
            return;
        }
        let names = SyncedNames {
            builtin: &r.builtin,
            dynamic: &r.registries,
        };
        let mut items = Items::empty();
        for (key, registry) in READ_REGISTRIES {
            let Some(tags) = r.registries.tags.get(key) else {
                continue;
            };
            let members = |ids: &[i32]| ids.iter().filter_map(|&id| names.name(key, id)).collect();
            let tags = tags.iter().map(|(tag, ids)| (tag.clone(), members(ids)));
            items.set_tags(registry, tags.collect());
        }
        // The damage type registry and its tags, for `Tags::damage_type_in_tag`. A registry
        // sent with no tag group binds its types with no tags, as vanilla binds received
        // elements whose tag payload was omitted.
        if let Some(types) = damage_types {
            let members = |ids: &[i32]| -> Vec<String> {
                ids.iter()
                    .filter_map(|&id| names.name(DAMAGE_TYPE, id))
                    .collect()
            };
            items.bind_damage_registry(
                types.iter().map(|(name, _)| name.clone()).collect(),
                r.registries
                    .tags
                    .get(DAMAGE_TYPE)
                    .into_iter()
                    .flatten()
                    .map(|(tag, ids)| (tag.clone(), members(ids)))
                    .collect(),
            );
        }
        super::class_tags::install(&mut items, self.common.res.facts());
        // Rebuild scoped parts against the received tags before refreshing state-only facts.
        let mut part_items = self.items().clone();
        let names = items.block_state_names();
        part_items.block_tags = items.block_tags.clone();
        part_items.rebuild_block_tag_flags(names);
        part_items.item_tags = items.item_tags.clone();
        let parts = match self.prepare_parts(&part_items) {
            Ok(parts) => parts,
            Err(error) => {
                log::warn!("Sunrise: client tag parts refused: {error}");
                return;
            }
        };
        if let Some(r) = self.client.remote.as_mut() {
            r.tags = Some(items);
        }
        self.bind_parts(parts);
        self.refresh_recipe_property_context();
        self.announce_data_bound(mod_api::TagsUpdateCause::ClientPacketReceived {
            integrated_server: self.client.integrated.is_some(),
        });
        self.resolve_primary_items();
        self.resolve_enchant_particles();
    }

    /// Recompile tag-based block face rules (`skip_rendering` / `hides_neighbor_face`) against
    /// the client's synced `minecraft:block` tags and remesh loaded sections when they change.
    pub(in crate::game::client_side) fn refresh_face_visibility_tags(&mut self) {
        if !self
            .client
            .client_pack
            .context
            .mesh
            .tables
            .visibility
            .has_tag_rules()
        {
            return;
        }
        let block_tags: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> = {
            let Some(r) = self.client.remote.as_ref() else {
                return;
            };
            let Some(raw_tags) = r.registries.tags.get("minecraft:block") else {
                return;
            };
            let names = SyncedNames {
                builtin: &r.builtin,
                dynamic: &r.registries,
            };
            raw_tags
                .iter()
                .map(|(tag, ids)| {
                    let members = ids
                        .iter()
                        .filter_map(|&id| names.name("minecraft:block", id))
                        .collect();
                    (tag.clone(), members)
                })
                .collect()
        };
        let Some(visibility) = self
            .client
            .client_pack
            .context
            .mesh
            .tables
            .visibility
            .rebuild_for_tags(self.common.pack.assets.registry, &block_tags)
        else {
            return;
        };
        let old = &self.client.client_pack.context;
        let tables = Arc::new(old.mesh.tables.with_visibility(visibility.clone()));
        self.client.client_pack.context =
            Arc::new(sunrise_client_render::streaming::WorkerContext {
                chunk: old.chunk.clone(),
                mesh: sunrise_client_render::streaming::MeshWorkerContext {
                    tables,
                    ..old.mesh.clone()
                },
            });
        self.client_res()
            .block_entity_render()
            .set_visibility(visibility);
        let context = self.worker_context();
        self.client
            .streaming
            .reload_resources(&mut self.common.world, context);
    }
}

use sunrise_gameplay_common::game::command_registries::CommandRegistriesGameExt1 as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::native_data_bound::NativeDataBoundGameExt1 as _;
