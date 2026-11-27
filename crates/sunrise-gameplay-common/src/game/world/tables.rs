//! Read-only world content tables shared by server and client games.
use super::*;
use crate::game::{
    native_data_maps_common, native_hooks::supports_enchantment::GameHooks as _,
    side_ops::tables::SideOps as _,
};
use std::sync::LazyLock;
use sunrise_core::nbt::Tag;
use sunrise_sim::{menu::Env, neoforge_mob_maps::NeoForgeMobMaps};
use sunrise_world::level::data::BlockSet;

/// [`Game::level_suffocation`] from the fields it reads, for a caller that holds other fields: the
/// world's own tables (its datapacks' `causes_suffocation`), else the built-in ones.
#[allow(dead_code)]
pub fn level_suffocation_of<'a>(
    world_tables: &'a Option<WorldTables>,
    res: &'a Resources,
) -> &'a sunrise_world::suffocation::SuffocationRegistry<'static> {
    world_tables
        .as_ref()
        .map_or_else(|| res.suffocation(), |t| &t.suffocation)
}

fn loaded_data_maps(
    tables: Option<&WorldTables>,
) -> std::result::Result<&native_data_maps_common::DataMaps, mod_api::DataMapError> {
    tables
        .map(|tables| &tables.data_maps)
        .ok_or(mod_api::DataMapError::Unavailable)
}

pub trait GameItems<'__sg>: crate::game::SideGame<'__sg> {
    /// Server reads active station enchantments; client reads synced remote enchantments or active items.
    fn item_enchantments<'s>(&'s self) -> &'s Arc<sunrise_items::enchant::Enchantments>
    where
        '__sg: 's,
    {
        if self.is_logical_server() {
            &self.stations().enchantments
        } else if let Some(ench) = self.side_state().side_remote_enchantments() {
            ench
        } else {
            &self.items().enchantments
        }
    }

    /// The content tables the world plays with: its data packs' over the
    /// game's own when it has packs, `Resources.rest`'s own otherwise.
    fn items<'s>(&'s self) -> &'s Items
    where
        '__sg: 's,
    {
        self.server_items
            .as_deref()
            .or_else(|| self.side_world_tables().map(|t| t.items.as_ref()))
            .unwrap_or_else(|| self.res.items())
    }

    /// The level's suffocation rules, for its mobs and its players.
    fn level_suffocation<'s>(
        &'s self,
    ) -> &'s sunrise_world::suffocation::SuffocationRegistry<'static>
    where
        '__sg: 's,
    {
        self.side_world_tables()
            .map_or_else(|| self.res.suffocation(), |t| &t.suffocation)
    }

    fn recipes<'s>(&'s self) -> &'s Recipes
    where
        '__sg: 's,
    {
        self.side_recipes()
    }

    fn active_providers<'s>(&'s self) -> &'s Providers
    where
        '__sg: 's,
    {
        self.side_world_tables()
            .map_or_else(|| self.res.providers(), |t| &t.providers)
    }

    fn stations<'s>(&'s self) -> &'s sunrise_sim::stations::StationData
    where
        '__sg: 's,
    {
        self.side_world_tables()
            .map_or_else(|| self.res.stations(), |t| &t.stations)
    }
}
impl<'__sg, T: crate::game::SideGame<'__sg>> GameItems<'__sg> for T {}

pub(in crate::game) static DEFAULT_CLIENT_FEATURES: LazyLock<Arc<[String]>> =
    LazyLock::new(|| Arc::from([String::from("minecraft:vanilla")]));

pub fn default_client_features() -> &'static Arc<[String]> {
    &DEFAULT_CLIENT_FEATURES
}

/// Kinds of contextual block provider dispatched by the game.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BlockProviderQuery {
    EnchantPower,
    LightEmission,
}

#[derive(PartialEq, Eq)]
pub(in crate::game) struct BlockQueryKey {
    pub(in crate::game) game: *const (),
    pub(in crate::game) state: BlockStateId,
    pub(in crate::game) query: BlockProviderQuery,
}

thread_local! {
    pub(in crate::game) static ACTIVE_BLOCK_QUERIES: RefCell<Vec<BlockQueryKey>> = const { RefCell::new(Vec::new()) };
}

pub(in crate::game) struct PopBlockQuery;

impl Drop for PopBlockQuery {
    fn drop(&mut self) {
        ACTIVE_BLOCK_QUERIES.with(|active| {
            active.borrow_mut().pop();
        });
    }
}

pub trait WorldTablesExt<'__sg>: crate::game::SideGame<'__sg> {
    fn pack_files<'s>(&'s self) -> sunrise_world::loot::PackOverlay<'s>
    where
        '__sg: 's,
    {
        match self.side_world_tables() {
            Some(t) => sunrise_world::loot::PackOverlay {
                conditions: &t.conditions,
                files: &t.files,
                priority_layers: &t.priority_layers,
            },
            None => sunrise_world::loot::PackOverlay::NONE,
        }
    }

    fn active_enchantment_definitions<'s>(&'s self) -> &'s sunrise_items::enchant::Enchantments
    where
        '__sg: 's,
    {
        self.item_enchantments()
    }

    fn shared_items(&self) -> Arc<Items> {
        if let Some(items) = &self.server_items {
            Arc::clone(items)
        } else if let Some(tables) = self.side_world_tables() {
            Arc::clone(&tables.items)
        } else {
            self.res.shared_items()
        }
    }

    fn station_recipes<'s>(&'s self) -> &'s sunrise_sim::stations::StationRecipes
    where
        '__sg: 's,
    {
        if self.is_logical_server() {
            &self.stations().recipes
        } else {
            self.recipe_tables
                .server_stations
                .as_ref()
                .unwrap_or(&self.stations().recipes)
        }
    }

    fn mob_maps<'s>(&'s self) -> &'s NeoForgeMobMaps
    where
        '__sg: 's,
    {
        self.mob_map_snapshot().as_ref()
    }

    fn mob_map_snapshot<'s>(&'s self) -> &'s Arc<NeoForgeMobMaps>
    where
        '__sg: 's,
    {
        if let Some(tables) = self.side_world_tables() {
            &tables.data_maps.mob_maps
        } else {
            NeoForgeMobMaps::vanilla()
        }
    }

    fn enabled_feature_ids<'s>(&'s self) -> &'s Arc<[String]>
    where
        '__sg: 's,
    {
        self.side_state()
            .side_level_features(self.level.enabled_features.as_arc())
    }

    fn limited_crafting(&self) -> bool {
        if self.is_logical_server() {
            self.ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::LIMITED_CRAFTING)
        } else {
            self.side_state().side_remote_limited_crafting()
        }
    }

    fn menu_env<'s>(&'s self) -> Env<'s>
    where
        '__sg: 's,
    {
        Env {
            crafting_remainder: Some(self),
            player: self.side_acting_player_uuid(),
            player_entity_id: Some(self.own_entity_id()),
            loop_context: &self.loop_context,
            items: self.items(),
            recipes: self.recipes(),
            brewing: &self.stations().brewing,
            banners: &self.stations().banners,
            station_recipes: self.station_recipes(),
            enchantments: self.item_enchantments(),
            names: &self.pack.names,
            trade: None,
            maps: None,
            limited_crafting: self.limited_crafting(),
            can_equip: None,
            known_recipes: Some(&self.level.recipe_book),
            slot_rules: Some(self),
            stacked: None,
        }
    }

    fn data_map_value<'s>(
        &'s self,
        registry: &str,
        entry: &str,
        map: &str,
    ) -> std::result::Result<Option<&'s Tag>, mod_api::DataMapError>
    where
        '__sg: 's,
    {
        loaded_data_maps(self.side_world_tables())?.value(
            registry,
            map,
            &native_data_maps_common::full_id(entry),
        )
    }

    fn data_map_entries<'s>(
        &'s self,
        registry: &str,
        map: &str,
    ) -> std::result::Result<&'s [(String, Tag)], mod_api::DataMapError>
    where
        '__sg: 's,
    {
        loaded_data_maps(self.side_world_tables())?.entries(registry, map)
    }

    fn loaded_biome_keys(&self) -> Option<&mod_api::__private::BiomeKeys> {
        loaded_data_maps(self.side_world_tables())
            .ok()
            .map(|maps| &*maps.biome_keys)
    }

    fn biome(
        &self,
        id: &str,
    ) -> std::result::Result<mod_api::BiomeView<'_>, mod_api::BiomeKeyError> {
        let maps = loaded_data_maps(self.side_world_tables())
            .map_err(|_| mod_api::BiomeKeyError::Unavailable)?;
        maps.biome_keys.view(id)
    }

    fn biome_view(
        &self,
        key: &mod_api::BiomeKey,
    ) -> std::result::Result<mod_api::BiomeView<'_>, mod_api::BiomeKeyError> {
        let maps = loaded_data_maps(self.side_world_tables())
            .map_err(|_| mod_api::BiomeKeyError::Unavailable)?;
        maps.biome_keys.view_key(key)
    }

    fn biome_view_id(
        &self,
        view: mod_api::BiomeView<'_>,
    ) -> std::result::Result<&str, mod_api::BiomeKeyError> {
        let maps = loaded_data_maps(self.side_world_tables())
            .map_err(|_| mod_api::BiomeKeyError::Unavailable)?;
        maps.biome_keys.id(view)
    }

    fn mod_recipe_property_contains(&self, set: &str, item: mod_api::ItemKey) -> bool {
        mod_api::__private::item_key_id(item)
            .is_some_and(|name| self.loop_context.mod_recipe_property_contains(set, name))
    }

    fn refresh_recipe_property_context(&mut self) {
        let mut sets = HashMap::<String, HashSet<String>>::new();
        if !self.is_logical_server() {
            if let Some(server) = &self.recipe_tables.server_properties.sets
                && let Some(item_ids) = self.side_state().side_remote_item_registry()
            {
                for (key, ids) in server
                    .iter()
                    .filter(|(k, _)| sunrise_sim::lan::recipes::vanilla_property_set(k).is_none())
                {
                    let items = ids
                        .iter()
                        .filter_map(|id| item_ids?.name(*id))
                        .filter(|name| *name != "minecraft:air")
                        .map(str::to_owned)
                        .collect();
                    sets.insert(key.clone(), items);
                }
            }
        } else if self.side_world_tables().is_some() || self.res.rest.get().is_some() {
            for (key, names) in self.recipes().custom_property_sets() {
                let items = names
                    .iter()
                    .filter(|name| self.res.items().get(name).is_some())
                    .cloned()
                    .collect();
                sets.insert(key.to_owned(), items);
            }
        }
        let tags = self
            .native_tag_items()
            .filter(|items| items.tags_loaded(sunrise_items::items::TagRegistry::Item))
            .and_then(|items| {
                let registry = self
                    .items()
                    .registry_order
                    .iter()
                    .map(AsRef::as_ref)
                    .collect();
                mod_api::__private::BoundItemTags::bind(&items.item_tag_order, &registry)
            });
        let context = Arc::make_mut(&mut self.loop_context);
        context.set_recipe_properties(Arc::new(sets));
        context.set_item_tags(tags);
        if self.is_logical_server() {
            {
                let (common, state) = self.common_and_state_mut();
                state.side_install_lan_loop_context(&common.loop_context);
            };
        }
    }

    fn native_tag_items<'s>(&'s self) -> Option<&'s Items>
    where
        '__sg: 's,
    {
        if !self.is_logical_server() {
            if <Self::State as crate::game::Side>::IS_SERVER {
                return None;
            }
            return self.side_state().side_game_tag_items(&self.res);
        }
        self.side_tags_ready().then(|| self.items())
    }

    fn resolve_level_infiniburn(
        &self,
        facts: &sunrise_world::dimension_type::DimensionFacts,
    ) -> Arc<BlockSet> {
        let name = facts
            .infiniburn
            .strip_prefix('#')
            .unwrap_or(&facts.infiniburn);
        self.resolve_block_tag(name)
    }

    fn resolve_golem_heads(&self) -> Arc<BlockSet> {
        self.resolve_block_tag("sunrise:golem_heads")
    }

    fn resolve_block_tag(&self, name: &str) -> Arc<BlockSet> {
        let mut set = BlockSet::default();
        if let Some(members) = self.items().block_tags.get(name) {
            for member in members {
                if let Some(block) = self.res.behaviours().table.block_by_name(member) {
                    set.insert(block);
                }
            }
        }
        Arc::new(set)
    }

    fn prepare_cauldron_interactions(
        &self,
        items: &Items,
    ) -> Result<Arc<cauldron_interactions::Table>> {
        use mod_api::__private::game::RegistrationContextNew;
        let mut registrar = cauldron_interactions::Registrar::new(items, self.pack.assets.registry);
        crate::game::native_hooks::native_register_cauldron_interaction(
            &mod_api::RegistrationContext::new(),
            &mut registrar,
        );
        registrar.finish()
    }

    fn prepare_parts(&self, items: &Items) -> Result<PreparedParts> {
        #[cfg(not(any(test, feature = "test-support")))]
        if !mod_api::__private::content_owners_published() {
            return Err("Mod parts refused: content ownership has not been published".into());
        }
        let registry = self.pack.assets.registry;
        let block_tags = &items.block_tags;
        let parts = mod_api::__private::prepare_parts(
            block_tags,
            &items.item_tags,
            mod_api::__private::KnownIds {
                block: &|id| registry.default_state(id).is_some(),
                item: &|id| items.items.contains_key(id),
            },
        )
        .map_err(|refused| match refused {
            mod_api::__private::ResolvePartsError::ItemKeys(errors) => format!(
                "Mod item keys refused:\n{}",
                errors
                    .into_iter()
                    .map(|e| format!("mod {} requested unknown item {}", e.mod_id, e.id))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            mod_api::__private::ResolvePartsError::Parts(errors) => {
                format!("Mod parts refused:\n{}", errors.join("\n"))
            }
            _ => "Mod parts refused".to_owned(),
        })?;
        let block_states = api_values::prepare_block_states(registry)?;
        let cauldrons = self.prepare_cauldron_interactions(items)?;
        Ok(PreparedParts {
            cauldrons,
            block_states,
            parts,
        })
    }

    fn resolve_parts(&self) -> Result<()> {
        let prepared = self.prepare_parts(self.items())?;
        if !self.is_logical_server() && self.side_state().side_remote_tags_present() {
            self.bind_parts(prepared);
        } else {
            self.install_parts(prepared);
        }
        self.side_resolve_enchant_particles();
        Ok(())
    }

    fn install_parts(&self, prepared: PreparedParts) {
        self.bind_parts(prepared);
        self.resolve_primary_items();
    }

    fn bind_parts(&self, prepared: PreparedParts) {
        if self.is_logical_server() {
            self.side_state()
                .side_publish_cauldron_registrations(&prepared.cauldrons);
        }
        cauldron_interactions::install(prepared.cauldrons);
        if self.is_logical_server() {
            native_worldgen_parts::publish(
                prepared.parts.as_ref(),
                &prepared.block_states,
                self.pack.assets.registry,
            );
        }
        api_values::install_block_states(&self.tick, prepared.block_states);
        api_values::install_resolved_parts(&self.tick, prepared.parts);
        self.mark_collision_shape_states();
        self.side_resolve_block_paths();
        self.resolve_piston_reactions();
    }

    fn resolve_primary_items(&self) {
        let mut support = sunrise_world::enchantment_answers::Answers::new();
        let enchantments = self.item_enchantments();
        let items = self.items();
        if mod_api::__private::has_part_hook(mod_api::HookId::SupportsEnchantment) {
            for item in items.items.keys() {
                if !mod_api::__private::item_listens(mod_api::HookId::SupportsEnchantment, item) {
                    continue;
                }
                let stack = ItemStack::new(item, 1);
                for enchantment in &enchantments.list {
                    let mut answer = None;
                    self.native_supports_enchantment(&stack, &enchantment.id, &mut answer);
                    if let Some(supported) = answer {
                        support
                            .entry(item.clone())
                            .or_default()
                            .insert(enchantment.id.clone(), supported);
                    }
                }
            }
        }
        for table in self.enchantment_snapshots() {
            table.support_answers().install(support.clone());
        }
        self.side_resolve_primary_items_server(support);
    }

    fn clear_world_parts(&self) {
        for table in self.enchantment_snapshots() {
            table.support_answers().clear();
            if self.is_logical_server() {
                table.primary_answers().clear();
            }
        }
        self.side_clear_loot_enchantment_answers();
        mod_api::__private::clear_game_parts();
    }

    fn enchantment_snapshots<'s>(
        &'s self,
    ) -> impl Iterator<Item = &'s sunrise_items::enchant::Enchantments>
    where
        '__sg: 's,
    {
        let stations = &self.stations().enchantments;
        let items = &self.items().enchantments;
        let active = self.item_enchantments();
        let other = (!Arc::ptr_eq(stations, items)).then_some(items);
        let synced =
            (!Arc::ptr_eq(active, stations) && !Arc::ptr_eq(active, items)).then_some(active);
        std::iter::once(stations)
            .chain(other)
            .chain(synced)
            .map(|table| table.as_ref())
    }

    fn with_block_provider_query<R>(
        &self,
        state: BlockStateId,
        query: BlockProviderQuery,
        ask: impl FnOnce() -> R,
    ) -> Option<R> {
        let key = BlockQueryKey {
            game: std::ptr::from_ref(self).cast(),
            state,
            query,
        };
        ACTIVE_BLOCK_QUERIES.with(|active| {
            let mut active = active.borrow_mut();
            if active.contains(&key) {
                return None;
            }
            active.push(key);
            drop(active);
            let _pop = PopBlockQuery;
            Some(ask())
        })
    }

    fn block_name(&self, p: [i32; 3]) -> Arc<str> {
        self.res
            .facts()
            .get(self.world.block(p[0], p[1], p[2]))
            .name
            .clone()
    }

    fn dispenser_block_name(&self, p: (i32, i32, i32)) -> Arc<str> {
        self.res
            .facts()
            .get(self.world.block(p.0, p.1, p.2))
            .name
            .clone()
    }

    fn in_block_tag(&self, tag: &str, block: &str) -> bool {
        self.items()
            .block_tags
            .get(tag)
            .is_some_and(|s| s.contains(block))
    }

    fn named_block_in_tag(&self, tag: &str, block: &str) -> bool {
        self.items()
            .block_tags
            .get(tag)
            .is_some_and(|s| s.contains(block))
    }

    fn item_in_tag(&self, tag: &str, item: &str) -> bool {
        self.items()
            .item_tags
            .get(tag)
            .is_some_and(|t| t.contains(item))
    }

    fn biome_name_at(&self, p: sunrise_core::geometry::Vec3) -> Option<String> {
        let (x, y, z) = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        let c = self.world.chunks.get(&(x >> 4, z >> 4))?;
        Some(world_biomes!(self).name(c.biome(x, y, z)).to_owned())
    }

    fn enchantments_compatible(&self, a: &str, b: &str) -> bool {
        self.item_enchantments().compatible(a, b)
    }

    fn item_has_enchantment_effect(&self, stack: &ItemStack, effect: &str) -> bool {
        self.item_enchantments().has_component(stack, effect)
    }
}
impl<'__sg, T: crate::game::SideGame<'__sg>> WorldTablesExt<'__sg> for T {}

use crate::game::side_ops::tables::TablesSideGameExt as _;

use crate::game::side_ops::world::WorldSideGameExt as _;

use crate::game::player_identity::PlayerIdentityGameExt1 as _;
