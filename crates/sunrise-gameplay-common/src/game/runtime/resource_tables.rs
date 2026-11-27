//! The game's shared read-only tables: `Resources` (blocks, items, recipes, mob types, biomes),
//! `PackResources`.
use super::*;
use sunrise_level::chunk_streamer::ChunkWorkerContext;

impl Resources {
    pub fn rest(&self) -> &WorldRest {
        self.rest
            .get()
            .expect("world resources joined before entry")
    }
    pub fn facts(&self) -> &BlockFacts {
        &self.core.facts
    }
    pub fn item_defaults(&self) -> &sunrise_world::item_defaults::ItemDefaults {
        self.core.item_data.defaults()
    }
    pub fn items(&self) -> &Items {
        &self.rest().items
    }
    pub fn shared_items(&self) -> Arc<Items> {
        self.rest().items.clone()
    }
    pub fn recipes(&self) -> &Recipes {
        &self.rest().recipes
    }
    pub fn transformers(&self) -> Arc<sunrise_sim::block_transform::Transformers> {
        self.rest()
            .transformers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    /// Publish the open world's `block_transformer` registry, replacing the last world's.
    pub fn set_transformers(&self, transformers: Arc<sunrise_sim::block_transform::Transformers>) {
        *self
            .rest()
            .transformers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = transformers;
    }
    pub fn biomes(&self) -> &Arc<Biomes> {
        &self.core.biomes
    }
    pub fn environment(&self) -> &Environment {
        &self.core.environment
    }
    /// The environment of a dimension type id; a type with no table of its own gets the overworld's.
    pub fn environment_of(&self, dimension_type: &str) -> &Environment {
        match dimension_type {
            "minecraft:the_nether" => self.nether_environment(),
            "minecraft:the_end" => self.end_environment(),
            _ => self.environment(),
        }
    }
    pub fn mob_types(&self) -> &HashMap<String, Arc<MobType>> {
        &self.rest().mob_types
    }
    /// The player's, the armor stand's and the dragon's registry keys, resolved with these
    /// resources.
    pub fn living_type_keys(&self) -> sunrise_sim::health::LivingTypeKeys {
        self.rest().living_type_keys
    }
    pub fn mob_data(&self) -> &sunrise_sim::mobs::MobData {
        &self.rest().mob_data
    }
    pub fn providers(&self) -> &Providers {
        &self.rest().providers
    }
    pub fn test_environments(&self) -> &HashMap<String, crate::test_environments::Environment> {
        &self.rest().test_environments
    }
    pub fn test_instances(&self) -> &HashMap<String, crate::test_instances::TestDef> {
        &self.rest().test_instances
    }
    pub fn nether_environment(&self) -> &Environment {
        &self.rest().nether_environment
    }
    pub(in crate::game) fn end_environment(&self) -> &Environment {
        &self.rest().end_environment
    }
    pub fn behaviours(&self) -> &Behaviours {
        &self.rest().behaviours
    }
    pub fn collision(&self) -> &BlockCollisionRegistry<'static> {
        &self.core.collision
    }
    pub(in crate::game) fn suffocation(
        &self,
    ) -> &sunrise_world::suffocation::SuffocationRegistry<'static> {
        &self.core.suffocation
    }
    pub fn motion(&self) -> &MotionResources {
        &self.core.motion
    }
    /// The prepared vanilla variant tables for families on `REGISTRY_KINDS`.
    pub fn variants(&self) -> &sunrise_sim::mobs::variants::VariantTables {
        &self.core.variants
    }
    pub fn threads(&self) -> usize {
        self.core.threads
    }
    pub fn stations(&self) -> &sunrise_sim::stations::StationData {
        &self.rest().stations
    }
}

pub struct PackParts {
    pub assets: Arc<Assets>,
    pub biomes: Arc<Biomes>,
    pub light: Arc<LightTables>,
    pub item_models: Arc<sunrise_assets::item_model::ItemModels>,
    pub item_ids: Arc<HashMap<String, i32>>,
}

impl PackResources {
    pub fn chunk_context_with(&self, biomes: Arc<Biomes>) -> Arc<ChunkWorkerContext> {
        let old = &self.chunk_context;
        if Arc::ptr_eq(&old.biomes, &biomes) {
            return old.clone();
        }
        Arc::new(ChunkWorkerContext {
            assets: old.assets.clone(),
            biomes,
            light: old.light.clone(),
        })
    }

    pub fn assemble(
        root: &sunrise_assets::resource_source::ResourceRoot,
        parts: PackParts,
    ) -> Result<Self> {
        let PackParts {
            assets,
            biomes,
            light,
            item_models,
            item_ids,
        } = parts;
        let names =
            Names::load(root.source()).with_item_names(&item_models.prototypes, assets.registry);
        Ok(Self {
            chunk_context: Arc::new(ChunkWorkerContext {
                assets: assets.clone(),
                biomes,
                light,
            }),
            block_registries: native_block_states::block_registries(assets.registry),
            assets,
            source: root.source().clone(),
            data_source: root.data().clone(),
            data_changes: root.data_changes().clone(),
            additional_data: Arc::from([]),
            mod_layers: root.mod_layers().clone(),
            names: Arc::new(names),
            item_models,
            item_ids,
        })
    }

    pub(in crate::game) fn item_registry_id(&self, item: &str) -> Option<u32> {
        sunrise_sim::item_models::item_registry_id(&self.item_ids, item)
    }
}
