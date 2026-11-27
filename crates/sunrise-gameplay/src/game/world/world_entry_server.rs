use super::*;
use sunrise_gameplay_common::game::command_registries::CommandRegistriesGameExt1 as _;
use sunrise_gameplay_common::game::{api_values, load_conditions};
use sunrise_items::items::{EntityTypeTags, LoadEntityTags};
use {
    crate::game::server_side::global_loot_modifiers,
    sunrise_gameplay_common::game::native_data_maps_common::DataMaps,
    sunrise_gameplay_common::game::native_data_maps_common::WorldRegistries,
    sunrise_gameplay_common::game::world_entry::next_biome_keys_session,
};

impl crate::game::ServerGame<'_> {
    /// Reload the content tables with the world's packs, as
    /// `/reload` reloads a server's (`ReloadableServerResources`).
    /// Publish tables before function admission, leaving advancement rewards queued
    /// until the reload's new function store is available.
    pub fn install_world_tables(&mut self, prepared: PreparedWorldReload) {
        let PreparedWorldReload {
            structure_spawns,
            tables,
            advancements,
            health,
            parts,
            wire,
            wire_tags,
            datapack_registries,
            datapack_vanilla_values,
            data,
        } = prepared;
        self.server.fns.0.data = data;
        self.close_stonecutters_for_reload();
        lan::sync::forget_server_sources();
        if let Some(tables) = &tables
            && let Some(sources) = &tables.reload_sources
        {
            let overlay = sunrise_world::loot::PackOverlay {
                conditions: &tables.conditions,
                files: &tables.files,
                priority_layers: &tables.priority_layers,
            };
            lan::sync::install_server_sources(
                &self.common.options.resources,
                &self.common.res.core.item_data,
                &overlay,
                sources.clone(),
            );
        }
        self.common.mods.datapack_registries = datapack_registries.map(Arc::new);
        self.common.mods.datapack_vanilla_values = datapack_vanilla_values.map(Arc::new);
        self.common.health_resources = health;
        self.server.world_tables.0 = tables;
        // Publish the paired item and world evaluators before any callbacks.
        self.load_server_loot();
        self.server.data_map_registries = self
            .server
            .world_tables
            .0
            .as_ref()
            .map_or_else(Vec::new, |t| t.data_maps.registries());
        self.install_block_maps();
        self.install_registry_snapshots();
        if let Err(error) = self.install_fluid_interactions() {
            log::error!("Sunrise: {error}");
        }
        self.install_profession_save_snapshots();
        self.install_level_environments();
        self.sync_timeline_registries();
        self.share_mob_maps();
        self.share_biome_keys();
        self.common.world.pois.clear();
        for world in self.server.levels.0.parked_worlds() {
            world.pois.clear();
        }
        sunrise_sim::datapack_net::serve(self.common.mods.datapack_registries.clone());
        self.refresh_recipe_property_context();
        self.server.structure_spawns = structure_spawns;
        self.refresh_infiniburn();
        self.install_parts(parts);
        self.common.damage_tag_generation = self.common.damage_tag_generation.wrapping_add(1);
        // Before the players are sent the reloaded data below.
        self.sync_recipe_content();
        // PlayerList.reloadResources: the tags and recipes go to the players
        // and their advancements reload. The synchronized registries (the
        // dialogs, and the commands' ids of them) stay the hub's.
        lan::sync::install_wire_reload(
            &self.common.options.resources,
            &self.common.res.core.item_data,
            &self.pack_files(),
            wire.clone(),
            advancements.clone(),
            wire_tags,
        );
        if self.server.lan.is_some()
            && let Some(sync) = wire
        {
            self.with_advancement_hub(|hub, owners| hub.reload_resources(sync, owners));
        }
        // Clients need UpdateTags before decoding datapack snapshots with holder sets.
        if !self
            .server
            .lan
            .as_ref()
            .is_some_and(|lan| lan.hub.native_reload_pending)
        {
            self.send_datapack_snapshots();
        }
        self.common.adv.tree = advancements;
        self.common.adv.reset_tree();
        self.announce_data_bound(mod_api::TagsUpdateCause::ServerDataLoad { reload: true });
        self.announce_data_maps_updated(mod_api::DataMapsUpdateCause::ServerReload);
    }
    /// Build and publish one server snapshot for this world load or reload.
    pub fn load_server_loot(&mut self) {
        if let Some(tables) = &self.server.world_tables.0 {
            self.common.server_items = Some(tables.items.clone());
            self.copy_loot_enchantment_answers(&tables.loot);
            self.server.server_loot.0.set(Some(tables.loot.clone()));
            global_loot_modifiers::bind_retained(
                &self.server.reload_resources.0,
                &tables.loot_modifier_manager,
            );
            return;
        }
        self.common.server_items = None;
        let overlay = pack_files(&self.server.world_tables.0);
        let native = if mod_api::__private::mods().is_empty() {
            None
        } else {
            match lan::sync::server_data_load(
                &self.common.options.resources,
                &self.common.res.core.item_data,
            ) {
                Ok(context) => Some(context),
                Err(error) => {
                    log::warn!("Sunrise: server loot inventory unavailable: {error}");
                    self.server.server_loot.0.set(None);
                    return;
                }
            }
        };
        let loot = match sunrise_world::loot::LootData::load_with_damage(
            &self.common.options.resources,
            &overlay,
            self.common.res.item_defaults(),
            &self.common.health_resources.damage,
            &self.items().ordered_tags(),
            native.as_ref(),
        ) {
            Ok(loot) => Some(Arc::new(loot)),
            Err(error) => {
                log::warn!("Sunrise: server loot tables unavailable: {error}");
                None
            }
        };
        if let Some(loot) = &loot {
            let mut items = self.items().clone();
            items.set_loot_data(loot);
            self.common.server_items = Some(Arc::new(items));
            self.copy_loot_enchantment_answers(loot);
        }
        self.server.server_loot.0.set(loot);
    }
    /// A world open's tables, and the first generation of the registries the mods declared.
    pub fn open_world_tables(
        &mut self,
        staged: super::condition_reload_state::StagedWorldTables,
    ) -> Result<()> {
        self.common.world_biomes_base = staged
            .tables
            .as_ref()
            .map(|tables| tables.biomes.table.clone());
        self.common.world_biomes = self.common.world_biomes_base.clone();
        self.server.world_tables.0 = staged.tables;
        self.common.health_resources = staged.health;
        self.common.mods.datapack_registries = staged.datapack_registries.map(Arc::new);
        self.common.mods.datapack_vanilla_values = staged.datapack_vanilla_values.map(Arc::new);
        self.server.fns.0.data = staged.data;
        self.common.adv.tree = staged.advancements;
        self.common.adv.reset_tree();
        self.load_server_loot();
        self.server.data_map_registries = self
            .server
            .world_tables
            .0
            .as_ref()
            .map_or_else(Vec::new, |t| t.data_maps.registries());
        self.install_block_maps();
        self.install_registry_snapshots();
        self.install_fluid_interactions()?;
        self.install_profession_save_snapshots();
        self.install_level_environments();
        self.sync_timeline_registries();
        self.share_mob_maps();
        self.share_biome_keys();
        self.sync_test_registry_ids();
        sunrise_sim::datapack_net::serve(self.common.mods.datapack_registries.clone());
        Ok(())
    }

    /// Publish what gameplay and drawing read of the open world's data-driven registries, from
    /// the tables just opened: the block transformers, the trial spawner configs, the sulfur cube
    /// archetypes and the decorated pot patterns, replacing the last world's.
    pub(in crate::game) fn install_registry_snapshots(&self) {
        use sunrise_sim::mobs::{sulfur_cube, trial_spawner};
        let Some(tables) = self.server.world_tables.0.as_ref() else {
            return;
        };
        let registries = &tables.packed_registries;
        self.common
            .res
            .set_transformers(tables.transformers.clone());
        trial_spawner::install(
            registries
                .validated("minecraft:trial_spawner")
                .into_iter()
                .filter_map(|(id, value)| {
                    let config =
                        <trial_spawner::TrialConfig as trial_spawner::TrialConfigExt>::decode(
                            &sunrise_game_data::scoreboard::json_to_tag(value),
                        )?;
                    Some((id, config))
                })
                .collect(),
        );
        sulfur_cube::install(sulfur_cube::build(
            &registries.validated("minecraft:sulfur_cube_archetype"),
            &tables.items.item_tags,
        ));
    }

    /// The resource root's own `block_transformer` files as the world's registry, for tests that
    /// boot a game without opening a world.
    #[cfg(any(test, feature = "test-support"))]
    pub fn install_root_block_transformers(&self) {
        let dir = self
            .common
            .options
            .resources
            .join("data/minecraft/block_transformer");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return;
        };
        let files: Vec<_> = entries
            .filter_map(|entry| {
                let path = entry.unwrap().path();
                (path.extension().and_then(|e| e.to_str()) == Some("json")).then(|| {
                    let id = format!("minecraft:{}", path.file_stem().unwrap().to_string_lossy());
                    let value =
                        serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap())
                            .unwrap();
                    (id, value)
                })
            })
            .collect();
        let transformers = sunrise_sim::block_transform::Transformers::build(
            files.iter().map(|(id, value)| (id.as_str(), value)),
            self.common.pack.assets.registry,
        )
        .unwrap();
        self.common.res.set_transformers(Arc::new(transformers));
    }

    /// Publishes every level's dimension type overrides and the world's `minecraft:timeline`
    /// table to `Environment::set_level_attributes` so `EnvironmentAttributeSystem` reads
    /// pack timelines for built-in and custom levels alike.
    pub(super) fn install_level_environments(&self) {
        let Some(tables) = self.server.world_tables.0.as_ref() else {
            return;
        };
        let overrides = &tables.biomes.overrides;
        let levels = [
            "minecraft:overworld",
            "minecraft:the_nether",
            "minecraft:the_end",
        ]
        .into_iter()
        .chain(self.server.level_shapes.extra.iter().map(|level| level.id));
        let attributes = levels
            .map(|level| {
                let json = overrides
                    .level_type_json(level)
                    .map_or(serde_json::Value::Null, |(_, json)| json);
                (level.to_owned(), json)
            })
            .collect();
        sunrise_level::environment::Environment::set_level_attributes(
            attributes,
            tables.timelines.clone(),
        );
    }
    /// Reload the content tables with the world's packs, as
    /// `/reload` reloads a server's (`ReloadableServerResources`).
    pub fn prepare_world_reload(
        &mut self,
        stack: &sunrise_gameplay_common::game::function_pack_stack::PackStack,
        root: &std::path::Path,
    ) -> Result<PreparedWorldReload> {
        let super::condition_reload_state::StagedWorldTables {
            tables,
            advancements,
            health,
            datapack_registries,
            datapack_vanilla_values,
            data,
            ..
        } = self.stage_world_tables(stack, true, root)?;
        let items = tables.as_ref().map_or_else(
            || self.common.res.shared_items(),
            |tables| tables.items.clone(),
        );
        let parts = self.prepare_parts(&items)?;
        let wire_tags = sunrise_sim::lan::data::WorldTags::new(items.clone(), &health.damage);
        let overlay = pack_files(&tables);
        let kept = self
            .server
            .lan
            .as_ref()
            .map(|host| host.hub.lan.shared.sync())
            .or_else(lan::sync::cached_reload_sync);
        let wire = if let Some(kept) = kept {
            let sources = match tables
                .as_ref()
                .and_then(|tables| tables.reload_sources.clone())
            {
                Some(sources) => sources,
                None => {
                    let sources = lan::sync::prepare_server_sources(
                        root,
                        &self.common.res.core.item_data,
                        &items,
                        &overlay,
                    )?;
                    sources.check()?;
                    sources
                }
            };
            Some(
                lan::sync::prepare_wire_reload(
                    root,
                    &self.common.res.core.item_data,
                    &overlay,
                    &kept,
                    &sources,
                    &advancements,
                    &wire_tags,
                )
                .map_err(|error| format!("wire reload: {error}"))?,
            )
        } else {
            None
        };
        overlay.conditions.check_errors()?;
        Ok(PreparedWorldReload {
            advancements,
            structure_spawns: sunrise_sim::mobs::spawn_selection::StructureSpawns::load(root),
            tables,
            health,
            parts,
            wire,
            wire_tags,
            datapack_registries,
            datapack_vanilla_values,
            data,
        })
    }
    #[cfg(any(test, feature = "test-support"))]
    #[inline(always)]
    pub fn resolve_block_paths(&self) {
        self.server_resolve_block_paths();
    }
    #[inline(always)]
    pub(in crate::game) fn server_acting_player_uuid(&self) -> Option<u128> {
        self.server.lan_acting_uuid
    }
    #[inline(always)]
    pub(in crate::game) fn server_clear_random_tick_sections(&mut self) {
        self.server.random_tick_sections.0.clear();
    }
    pub(in crate::game) fn server_resolve_block_paths(&self) {
        use mod_api::{HookId, PathComputationType};
        use sunrise_sim::mobs::data::PartAnswers;
        let data = self.common.res.mob_data();
        let pathfindable =
            Self::resolve_per_state(HookId::IsPathfindable, data.states.len(), |state| {
                PathComputationType::ALL
                    .into_iter()
                    .filter(|&kind| {
                        let mut pathfindable = data.block_pathfindable(state, kind);
                        self.native_is_pathfindable(state, kind, &mut pathfindable);
                        pathfindable
                    })
                    .fold(0, |bits, kind| bits | PartAnswers::pathfindable_bit(kind))
            });
        data.resolve_parts(
            self.common.res.facts(),
            &PartAnswers {
                pathfindable: pathfindable.as_deref(),
            },
        );
    }
    #[inline(always)]
    pub(in crate::game) fn server_tags_ready(&self) -> bool {
        self.server.session_lock.0.is_some()
    }
    #[inline(always)]
    pub(in crate::game) fn server_world_tables(&self) -> Option<&WorldTables> {
        self.server.world_tables.0.as_ref()
    }
    /// Load the world's data packs' content tables (`None`: the world has no
    /// packs, and `Resources.rest` stays in use). This re-reads the tables,
    /// so it runs once per world open and on `/reload`.
    pub fn stage_world_tables(
        &mut self,
        stack: &sunrise_gameplay_common::game::function_pack_stack::PackStack,
        reload: bool,
        root: &std::path::Path,
    ) -> Result<super::condition_reload_state::StagedWorldTables> {
        // The ask of this load, both here and on `/reload`: a native mod
        // recomputes the pack files it contributes every time.
        let mut contributed = Vec::new();
        self.native_data_pack_files(&mut contributed);
        let modifier_files = global_loot_modifiers::contributed(&contributed);
        let data = Arc::new(
            stack.data_packs(
                contributed
                    .into_iter()
                    .filter(|(key, _)| !key.starts_with("loot_modifiers/"))
                    .collect(),
            ),
        );
        let functions::OverlayFiles {
            files,
            read_errors,
            priority_layers,
        } = functions::overlay_files(&data);
        let inventory = sunrise_mod_package::enabled::EnabledMods::read(root)?;
        let base_conditions = load_conditions::conditions_for_mods(&self.common.mods, &inventory)?;
        // Tag files are not conditional. Collect the candidate's static holders
        // and pending tags before registry codecs can inspect them.
        let tag_files = sunrise_world::loot::PackOverlay::new(&files);
        let mut items = Items::load_registry(
            root,
            self.common.pack.assets.registry,
            self.common.res.facts(),
            &tag_files,
            self.common.res.item_defaults(),
            if reload {
                LoadEntityTags::Reload(&self.items().entity_type_tags)
            } else {
                LoadEntityTags::Frozen(self.common.res.items().entity_type_tags.ids())
            },
        )?;
        if reload {
            items.retain_unbound_tags(self.items());
        }
        class_tags::install(&mut items, self.common.res.facts());
        let mut view = load_conditions::RegistryView {
            ids: Default::default(),
            tags: items
                .bound_tags()
                .map(|(registry, tags)| {
                    let tags = tags.map(|(tag, held)| (tag.to_owned(), held));
                    (registry.to_owned(), tags.collect())
                })
                .collect(),
        };
        use sunrise_sim::lan::data::{BIOME_REGISTRY, REGISTRY_KINDS, SyncData};
        let builtin = sunrise_protocol::registry::Builtin::find(root)
            .map(|path| sunrise_protocol::registry::Builtin::load(&path))
            .transpose()?
            .map(sunrise_sim::lan::data::with_custom_stats)
            .unwrap_or_default();
        for (key, ids) in &builtin.registries {
            view.ids.insert(
                key.clone(),
                ids.names
                    .iter()
                    .filter(|id| !id.is_empty())
                    .cloned()
                    .collect(),
            );
        }
        view.ids.insert(
            "minecraft:item".into(),
            items.items.keys().map(ToString::to_string).collect(),
        );
        view.ids.insert(
            "minecraft:block".into(),
            self.common
                .res
                .facts()
                .states
                .iter()
                .map(|state| state.name.to_string())
                .collect(),
        );
        view.ids.insert(
            EntityTypeTags::REGISTRY.into(),
            items.entity_type_tags.ids().iter().cloned().collect(),
        );
        let mut pack_tags = HashMap::new();
        let mut banner_tags = std::collections::BTreeMap::new();
        let gameplay_tags = WorldTables::PACK_TAG_REGISTRIES
            .into_iter()
            .chain(sunrise_sim::neoforge_mob_maps::MAPS.map(|definition| definition.directory))
            .filter(|directory| !matches!(*directory, "item" | "entity_type" | "damage_type"))
            .collect::<std::collections::BTreeSet<_>>();
        let mut timeline_tags = std::collections::BTreeMap::new();
        let mut bind_tags = |registry: &str,
                             view: &mut load_conditions::RegistryView|
         -> Result<()> {
            let ids = view.ids.get(registry);
            let tags =
                sunrise_sim::lan::data::registry_tag_bindings(root, &tag_files, registry, |id| {
                    ids.is_some_and(|ids| ids.contains(id))
                })?;
            if registry == "minecraft:timeline" {
                timeline_tags = tags.clone();
            }
            view.tags.insert(
                registry.to_owned(),
                tags.iter()
                    .map(|(tag, members)| (tag.clone(), !members.is_empty()))
                    .collect(),
            );
            if registry == "minecraft:banner_pattern" {
                banner_tags.clone_from(&tags);
            }
            if let Some(directory) = registry
                .strip_prefix("minecraft:")
                .and_then(|name| gameplay_tags.get(name))
            {
                pack_tags.insert(
                    *directory,
                    tags.into_iter()
                        .map(|(tag, members)| (tag, members.into_iter().collect::<HashSet<_>>()))
                        .collect(),
                );
            }
            Ok(())
        };
        // Item, block, fluid and entity type tags already came from Items' loader.
        let static_registries = builtin
            .registries
            .keys()
            .map(String::as_str)
            .chain(gameplay_tags.iter().filter_map(|directory| {
                view.ids
                    .get_key_value(&format!("minecraft:{directory}"))
                    .map(|(key, _)| key.as_str())
            }))
            .filter(|&key| {
                !matches!(key, BIOME_REGISTRY | "minecraft:damage_type")
                    && !REGISTRY_KINDS.iter().any(|table| table.key == key)
                    && !view.tags.contains_key(key)
            })
            .map(str::to_owned)
            .collect::<std::collections::BTreeSet<_>>();
        for registry in &static_registries {
            bind_tags(registry, &mut view)?;
        }
        // WORLD_REGISTRIES codecs see static holders (not their own pending entries)
        // and NeoForge's ResourceManagerRegistryLoadTask uses VANILLA_SET.
        let frozen = self.server.world_tables.0.as_ref().filter(|_| reload);
        let fixed = REGISTRY_KINDS.iter().map(|table| table.key);
        for registry in fixed
            .clone()
            .chain(["minecraft:damage_type", BIOME_REGISTRY])
        {
            view.ids.remove(registry);
            view.tags.remove(registry);
        }
        let registry_base = base_conditions.with_features(["minecraft:vanilla"]);
        let registry_conditions = view.conditions(&registry_base);
        let registry_overlay = sunrise_world::loot::PackOverlay {
            conditions: &registry_conditions,
            files: &files,
            priority_layers: &priority_layers,
        };
        let base = if reload {
            &self.common.health_resources
        } else {
            &self.common.res.motion().health
        };
        let health = base.with_packs(root, &registry_overlay, reload)?;
        let (packed_registry_definitions, packed_admission) = if let Some(tables) = frozen {
            (tables.packed_registries.clone(), Ok(()))
        } else {
            let (packed, admission) = SyncData::stage_registry_definitions(
                root,
                self.common.res.item_defaults(),
                &registry_overlay,
                &read_errors,
                &items.component_registry,
                |id| {
                    items
                        .component_registry
                        .ids("minecraft:sound_event")
                        .binary_search_by(|sound| sound.as_str().cmp(id))
                        .is_ok()
                },
            );
            (Arc::new(packed), admission)
        };
        let variants = frozen
            .map(|tables| tables.variants.clone())
            .unwrap_or_else(|| Arc::new(packed_registry_definitions.variant_tables()));
        let biomes = if let Some(tables) = frozen {
            tables.biomes.clone()
        } else {
            Arc::new(condition_reload_state::WorldBiomeRegistry::load(
                self.common.res.biomes(),
                root,
                self.common.pack.assets.registry,
                stack,
                &data,
                &registry_overlay,
                &read_errors,
            )?)
        };
        let frozen_tests = frozen.map(|tables| {
            (
                tables.test_environments.clone(),
                tables.test_instances.clone(),
            )
        });
        view.ids.insert(
            BIOME_REGISTRY.into(),
            biomes
                .table
                .registered_keys()
                .iter()
                .map(ToString::to_string)
                .collect(),
        );
        for (registry, id, admitted) in packed_registry_definitions.entries() {
            let ids = view.ids.entry(registry.to_owned()).or_default();
            if admitted {
                ids.insert(id);
            } else {
                ids.remove(&id);
            }
        }
        view.ids.insert(
            "minecraft:damage_type".into(),
            health
                .damage
                .entries()
                .map(|(_, definition)| definition.name.clone())
                .collect(),
        );
        for registry in fixed.chain([BIOME_REGISTRY]) {
            bind_tags(registry, &mut view)?;
        }
        // Painting.create's candidates: the world's painting_variant registry (the
        // root's variants under the packs' winners) and `#minecraft:placeable` over it.
        let painting_variants = {
            let all = packed_registry_definitions.painting_variants(
                sunrise_gameplay_common::painting_variants::PaintingVariants::load(root).all,
            );
            let mut tags = sunrise_sim::lan::data::registry_tag_bindings(
                root,
                &tag_files,
                "minecraft:painting_variant",
                |id| all.iter().any(|variant| &*variant.id == id),
            )?;
            let placeable = tags
                .remove("minecraft:placeable")
                .unwrap_or_default()
                .into_iter()
                .map(Arc::from)
                .collect();
            sunrise_gameplay_common::painting_variants::PaintingVariants { all, placeable }
        };
        // WORLD_REGISTRIES owns the structures; its raw definitions are not holders.
        // First open uses the same cached data as live features. Reload binds new tags
        // to the frozen owner, just as TagLoader does before server resource reload.
        let generator = if reload {
            self.registry_generator_data()
        } else {
            // This bundle can be unavailable independently of the resource tables.
            // Terrain preparation owns the existing diagnostics for it.
            sunrise_sim::vanillagen::shared_data_from(
                &root.join("worldgen"),
                &biomes.overrides,
                self.common.pack.assets.registry,
                stack.content.source(),
            )
            .ok()
        };
        if let Some(generator) = generator {
            const STRUCTURES: &str = "minecraft:worldgen/structure";
            let mut holders = std::collections::BTreeSet::new();
            if generator
                .visit_registry_ids(STRUCTURES, &mut |id| {
                    holders.insert(id.to_owned());
                    std::ops::ControlFlow::Continue(())
                })
                .is_some()
            {
                view.ids.insert(STRUCTURES.to_owned(), holders);
                bind_tags(STRUCTURES, &mut view)?;
            }
            // Placed features are holders too: the decoded ones (the codecs live generation
            // places with, not the source files) and their tags, for a downstream
            // `neoforge:registered` or `neoforge:tag_empty`. Reload keeps the frozen owner's.
            const PLACED: &str = "minecraft:worldgen/placed_feature";
            let live = if reload {
                self.common.res.behaviours().live_features()
            } else {
                sunrise_world::worldgen::live::LiveFeatures::new(&generator)
                    .ok()
                    .flatten()
                    .map(Arc::new)
            };
            if let Some(ids) = live
                .as_ref()
                .and_then(|features| features.registry_ids(PLACED))
            {
                view.ids.insert(PLACED.to_owned(), ids.clone());
                bind_tags(PLACED, &mut view)?;
            }
        }
        let damage_tags: HashMap<_, HashSet<_>> = health
            .damage
            .tags()
            .map(|tag| {
                (
                    tag.to_owned(),
                    health.damage.tag_members(tag).map(str::to_owned).collect(),
                )
            })
            .collect();
        view.tags.insert(
            "minecraft:damage_type".into(),
            damage_tags
                .iter()
                .map(|(tag, members)| (tag.clone(), !members.is_empty()))
                .collect(),
        );
        pack_tags.insert("damage_type", damage_tags);
        if !view.tags.contains_key("minecraft:villager_trade")
            && let Ok(tags) = sunrise_items::items::load_tags(root, "villager_trade", &tag_files)
        {
            view.tags.insert(
                "minecraft:villager_trade".to_owned(),
                tags.into_iter()
                    .map(|(tag, members)| (tag, !members.is_empty()))
                    .collect(),
            );
        }

        let datapack_candidate = self.common.stage_datapack_registries(
            stack,
            root,
            &registry_overlay,
            &mut view,
            &load_conditions::VanillaValueInputs {
                items: &items,
                tag_files: &tag_files,
                biomes: &biomes.table,
            },
        );
        // The downstream reload listeners see every completed producer and the
        // world's flags, not the codec's vanilla flags or a previous generation.
        let conditions = view
            .conditions(&registry_base.with_features(stack.features.iter().map(String::as_str)));
        let overlay = sunrise_world::loot::PackOverlay {
            conditions: &conditions,
            files: &files,
            priority_layers: &priority_layers,
        };
        // RegistryDataLoader.RELOADABLE_REGISTRIES: providers are staged and
        // published with the other reloadable data, including on /reload.
        let providers = Providers::load_with(root, &overlay);
        let mut data_map_types = Vec::new();
        self.native_data_map_types(&mut data_map_types);
        // Visit independently readable registries before consuming any result.
        // One rejected winner must not hide the other entries' diagnostics.
        let loot = sunrise_world::loot::LootData::load_candidate_with_damage(
            root,
            &overlay,
            self.common.res.core.item_data.defaults(),
            &health.damage,
            &items.ordered_tags(),
        );
        let advancements = Arc::new(match &loot {
            Ok(loot) => sunrise_sim::advancements::Tree::load_with_predicates(
                &root.join("data"),
                &overlay,
                loot.predicates(),
            ),
            Err(_) => sunrise_sim::advancements::Tree::load_with(&root.join("data"), &overlay),
        });
        let install_items_result = packed_registry_definitions.install_items(
            &mut items,
            root,
            &overlay,
            self.common.res.item_defaults(),
        );
        // The block registry binds each transform's blocks and states, so a name it does not
        // know refuses the load with the other registry errors.
        let transformers = sunrise_sim::block_transform::Transformers::build(
            packed_registry_definitions
                .validated("minecraft:block_transformer")
                .iter()
                .map(|(id, value)| (id.as_str(), *value)),
            self.common.pack.assets.registry,
        );
        // Item damage predicates use the same frozen registry as player hurt.
        Arc::make_mut(&mut items.enchantments).bind_damage_catalog(&health.damage);
        items.bind_component_damage_catalog(&health.damage);
        if let Ok(loot) = &loot {
            items.component_defaults = loot.component_defaults().clone();
        }
        // The recipe tables, the stations and the recipe book read one folder:
        // hold its files so it is read once.
        let _recipe_files = sunrise_world::json_folder::read(&root.join("data/minecraft/recipe"));
        let sources = if reload {
            lan::sync::prepare_server_sources(
                root,
                &self.common.res.core.item_data,
                &items,
                &overlay,
            )
        } else {
            lan::sync::load_sources(
                root,
                &self.common.res.core.item_data,
                &items,
                self.common.options.execution_role.data_load_origin(),
                &overlay,
            )
        };
        // Loot preparation resolves references after the native edits. Include
        // failures here even when a condition rejected the referenced entry.
        let loot = loot.and_then(|loot| {
            let context = lan::sync::server_data_load(root, &self.common.res.core.item_data)?;
            let data =
                loot.prepare((!mod_api::__private::mods().is_empty()).then_some(&context))?;
            Ok((data, context))
        });
        // A rejected early registry must not hide independent late load errors.
        // No table is installed, even if another owning loader succeeded.
        let errors = [
            packed_admission.err(),
            providers.as_ref().err().cloned(),
            install_items_result.err().map(|error| error.to_string()),
            transformers.as_ref().err().cloned(),
            conditions.check_errors().err(),
            (!advancements.refused.is_empty()).then(|| advancements.refused.join("\n")),
            loot.as_ref().err().map(ToString::to_string),
            datapack_candidate.as_ref().err().cloned(),
            sources.as_ref().err().map(ToString::to_string),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        if !errors.is_empty() {
            return Err(errors.join("\n").into());
        }
        let providers = providers?;
        // Bind the world-owned registries' ordered tags against the admitted entries
        // (including datapack and mod biomes, enchantments, structures, and world clocks).
        sunrise_gameplay_common::game::world_entry::bind_world_registry_tags(
            &mut items,
            root,
            &tag_files,
            &overlay,
            &biomes.table,
        )?;
        if reload {
            items.retain_unbound_tags(self.items());
        }
        let transformers = Arc::new(transformers?);
        let sources = sources?;
        let (mut loot, context) = loot?;
        let recipes = Recipes::from_resolved(root, &items, &overlay, &sources)?;
        let mut stations = sunrise_sim::stations::StationData::from_sources(
            root,
            &items,
            &overlay,
            sunrise_sim::banner::Banners::stage(
                root,
                self.common.res.item_defaults(),
                &packed_registry_definitions,
                banner_tags,
            ),
            (&sources, &recipes),
        )?;
        self.prepare_professions(root, &items, &overlay, &mut stations)?;
        // Use the same candidate's IDs and bound tags for data map targets.
        let (datapack_registries, datapack_vanilla_values) = match datapack_candidate? {
            Some((candidate, values)) => (
                Some(mod_api::DatapackRegistrySet::from_candidate(candidate)),
                values,
            ),
            None => (None, None),
        };
        let datapack_set = datapack_registries.as_ref();
        let registries = WorldRegistries::new([], datapack_set, &items, root, &overlay)
            .with_candidate(&view, &pack_tags)
            .with_biome_keys(biomes.table.registered_keys());
        let causes_suffocation = items
            .block_tags
            .get("minecraft:causes_suffocation")
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        let suffocation = sunrise_sim::mobs::suffocation::bind(
            self.common.res.core.collision.clone(),
            &causes_suffocation,
        )?;
        // Items already decoded the root's command export. Block-state-only
        // command metadata is not the registry universe, and professions and
        // biomes belong to the live world candidate rather than that export.
        for registry in ["minecraft:sound_event", "minecraft:game_event"] {
            if items.component_registry.ids(registry).is_empty() {
                return Err(format!(
                    "builtin data maps require {registry} in command-registries.json"
                )
                .into());
            }
        }
        let builtin = neoforge_mob_maps::load(
            &overlay,
            |directory| match directory {
                "item" => Some(&items.item_tags),
                "entity_type" => Some(items.entity_type_tags.members()),
                _ => pack_tags.get(directory),
            },
            |registry, id| match registry {
                "minecraft:item" => items.items.contains_key(id),
                "minecraft:villager_profession" => stations.professions.resolve(id).is_some(),
                "minecraft:worldgen/biome" => biomes.table.contains(id),
                EntityTypeTags::REGISTRY => items
                    .entity_type_tags
                    .ids()
                    .binary_search_by(|known| known.as_str().cmp(id))
                    .is_ok(),
                _ => items
                    .component_registry
                    .ids(registry)
                    .binary_search_by(|known| known.as_str().cmp(id))
                    .is_ok(),
            },
        )?;
        // The timeline tags the raid code reads bind from this world's packs too.
        let gameplay = Arc::new(sunrise_sim::patrol::GameplayAttributes::load(
            root, &overlay,
        )?);
        let data_maps = DataMaps::load(&data_map_types, &overlay, &registries)
            .with_builtins(builtin)
            .with_biome_keys(mod_api::__private::BiomeKeys::new(
                next_biome_keys_session(),
                biomes.table.registered_keys().iter().cloned(),
            ));
        drop(registries);
        // Vanilla reads tags of these registries too: `LootTable.LIST_CODEC` (advancement
        // rewards, the loot table trigger) and `Recipe.LIST_CODEC` (the recipe triggers).
        pack_tags.insert(
            "loot_table",
            loot.registry_tags("loot_table")
                .into_iter()
                .flatten()
                .map(|(tag, members)| (tag.clone(), members.iter().cloned().collect()))
                .collect(),
        );
        pack_tags.insert(
            "recipe",
            sunrise_world::tag_layers::load(root, "recipe", &overlay, |id| {
                stations.recipe_index.contains(id)
            })?
            .into_iter()
            .map(|(tag, members)| (tag, members.into_iter().collect()))
            .collect(),
        );
        // `HolderSetCodec.lookupTag`: an advancement naming a tag that is not bound is refused.
        let unbound = advancements.unbound_tags(|registry, tag| {
            pack_tags
                .get(registry)
                .is_some_and(|tags| tags.contains_key(tag))
        });
        if !unbound.is_empty() {
            return Err(unbound.join("\n").into());
        }
        // Check again after `StationData::from_sources` (`Trades::load`) and other late
        // condition readers, before live state (`resolve_enchantment_registry`) changes.
        conditions.check_errors()?;
        // The API's key order is live state as well: do not change it for a
        // candidate refused by the condition readers.
        api_values::resolve_enchantment_registry(&items)?;
        let serializers = self
            .common
            .mods
            .loot_modifier_serializers
            .get_or_init(global_loot_modifiers::Serializers::capture);
        let (modifiers, loot_modifier_manager) = serializers.prepare(
            global_loot_modifiers::files(root, stack, modifier_files),
            &conditions,
            &context,
            &loot,
        )?;
        loot.set_global_modifiers(modifiers);
        items.set_loot_data(&loot);
        let loot = Arc::new(loot);
        let (test_environments, test_instances) = if let Some(tables) = frozen_tests {
            tables
        } else {
            let mut functions = mod_api::GameTestFunctions::default();
            self.native_register_game_tests(&mut functions);
            sunrise_gameplay_common::game::test_registry_tables::build_game_test_tables(
                &packed_registry_definitions,
                &functions,
            )
        };
        Ok(super::condition_reload_state::StagedWorldTables {
            tables: Some(WorldTables {
                conditions,
                suffocation,
                files,
                priority_layers,
                items: Arc::new(items),
                timelines: sunrise_level::environment::TimelineTable {
                    timelines: packed_registry_definitions.timelines(),
                    tags: timeline_tags,
                },
                packed_registries: packed_registry_definitions,
                painting_variants,
                transformers,
                variants,
                providers,
                biomes,
                loot,
                loot_modifier_manager,
                recipes,
                stations,
                reload_sources: reload.then_some(sources),
                data_maps,
                pack_tags,
                gameplay,
                test_environments,
                test_instances,
            }),
            advancements,
            health,
            datapack_registries,
            datapack_vanilla_values,
            data,
        })
    }

    /// Sends each joined Sunrise client the snapshot of the set just published, built from the
    /// offer owned by its connection. A refused replacement ends that connection with its reason.
    pub(in crate::game) fn send_datapack_snapshots(&mut self) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        for guest in host
            .hub
            .guests
            .iter()
            .filter(|g| g.placed && g.participates_online() && g.brand == "sunrise")
        {
            sunrise_sim::datapack_net::reload_connection(&guest.sender);
        }
    }

    /// Close guest stonecutter windows before publishing a reloaded recipe generation.
    pub(in crate::game) fn close_stonecutters_for_reload(&mut self) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let open: Vec<_> = host
            .menus
            .iter()
            .filter_map(|(&conn, menus)| {
                menus
                    .station
                    .as_ref()
                    .filter(|menu| menu.kind == super::stations_common::Kind::Stonecutter)
                    .map(|_| conn)
            })
            .collect();
        for conn in open {
            self.guest_close_menus_and_tell(conn);
        }
    }
}

use sunrise_gameplay_common::game::native_data_bound::NativeDataBoundGameExt1 as _;

use sunrise_gameplay_common::game::primary_items::PrimaryItemsGameExt1 as _;
