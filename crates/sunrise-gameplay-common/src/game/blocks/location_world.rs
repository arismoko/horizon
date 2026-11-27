//! The level as a `LocationPredicate` reads it from inside a block behaviour:
//! the same `advancements::World` the advancement triggers and
//! `/execute if predicate` evaluate against, built from the level's own
//! field borrows (the behaviour holds the game's world mutably, so the
//! game's view cannot be used).
use crate::game::{advancements_common::TagSources, end_state::EndState};
use std::collections::{BTreeMap, HashMap};
use sunrise_core::nbt::Tag;
use sunrise_gameplay_common::{
    hoppers::{HopperState, Pos, hopper_cooldown_in},
    structures::structures_reaching_in,
};
use sunrise_items::{
    blocks::{BlockFacts, Fluid},
    items::Items,
};
use sunrise_level::biome::Biomes;
use sunrise_sim::advancements::World;
use sunrise_world::{block_collision::BlockCollisionRegistry, registry::BlockRegistry};

/// What a location predicate reads of the game beyond the level's own fields.
#[derive(Clone, Copy)]
pub struct LocationRefs<'w> {
    pub dimension: &'static str,
    pub end: &'w EndState,
    pub tags: TagSources<'w>,
}

/// One block behaviour's level, read for a location predicate.
pub(in crate::game) struct LevelLocation<'a> {
    pub(in crate::game) world: &'a sunrise_level::world::World,
    pub(in crate::game) registry: &'a BlockRegistry,
    pub(in crate::game) facts: &'a BlockFacts,
    pub(in crate::game) biomes: &'a Biomes,
    pub(in crate::game) items: &'a Items,
    pub(in crate::game) collision: &'a BlockCollisionRegistry<'a>,
    pub(in crate::game) hoppers: &'a HashMap<Pos, HopperState>,
    pub(in crate::game) raining: bool,
    pub(in crate::game) thundering: bool,
    pub(in crate::game) refs: LocationRefs<'a>,
}

impl sunrise_sim::beehive::SmokeLevel for LevelLocation<'_> {
    fn is_lit_campfire(&self, at: [i32; 3]) -> bool {
        let state = self.world.block(at[0], at[1], at[2]);
        self.items
            .block_tags
            .get("minecraft:campfires")
            .is_some_and(|set| set.contains(&*self.facts.get(state).name))
            && self
                .registry
                .state(state)
                .and_then(|s| s.properties.get("lit"))
                .map(String::as_str)
                == Some("true")
    }
    fn collision_boxes(&self, at: [i32; 3]) -> Vec<sunrise_core::geometry::Aabb> {
        sunrise_sim::mobs::living::collision_boxes_in(
            self.world,
            self.collision,
            at[0],
            at[1],
            at[2],
        )
    }
}

impl World for LevelLocation<'_> {
    fn is_loaded(&self, [x, y, z]: [i32; 3]) -> bool {
        self.world.is_loaded_at(x, y, z)
    }
    fn dimension(&self) -> &str {
        self.refs.dimension
    }
    fn weather(&self) -> (bool, bool) {
        (self.raining, self.thundering)
    }
    fn biome(&self, p: [i32; 3]) -> String {
        self.world
            .chunks
            .get(&(p[0] >> 4, p[2] >> 4))
            .map(|c| self.biomes.get(c.biome(p[0], p[1], p[2])).name.clone())
            .unwrap_or_default()
    }
    fn structures(&self, p: [i32; 3]) -> Vec<String> {
        structures_reaching_in(self.world, &|| self.refs.end.locator(), p, &|_| true)
            .into_iter()
            .filter(|(_, piece)| *piece)
            .map(|(name, _)| name)
            .collect()
    }
    fn block_state(&self, [x, y, z]: [i32; 3]) -> Option<&sunrise_world::chunk::BlockState> {
        self.registry.state(self.world.block(x, y, z))
    }
    fn block(&self, p: [i32; 3]) -> (String, BTreeMap<String, String>) {
        let state = self.world.block(p[0], p[1], p[2]);
        let props = self
            .registry
            .state(state)
            .map(|s| {
                s.properties
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        (self.facts.get(state).name.to_string(), props)
    }
    fn block_property_values(&self, block: &str, property: &str) -> Option<Vec<String>> {
        let values = self.registry.property_values(block, property);
        (!values.is_empty()).then(|| values.into_iter().map(str::to_owned).collect())
    }
    fn fluid(&self, p: [i32; 3]) -> Option<String> {
        match self.facts.get(self.world.block(p[0], p[1], p[2])).fluid {
            Fluid::Water => Some("minecraft:water".into()),
            Fluid::Lava => Some("minecraft:lava".into()),
            _ => None,
        }
    }
    /// `Level.canSeeSky`: sky light 15 at the position, in a level with sky light.
    fn can_see_sky(&self, p: [i32; 3]) -> bool {
        self.world.shape.sky_light && self.world.light(p[0], p[1], p[2]) >> 4 == 15
    }
    fn light(&self, p: [i32; 3]) -> i32 {
        let l = self.world.light(p[0], p[1], p[2]);
        i32::from((l >> 4).max(l & 15))
    }
    fn smokey(&self, p: [i32; 3]) -> bool {
        sunrise_sim::beehive::smokey(self, p)
    }
    /// `BlockEntity.saveWithFullMetadata` of the stored block entity, as
    /// `Game::block_entity_nbt` reads it, without what the game keeps in its
    /// tick tables (a comparator's output, a moving piston) or a mod's
    /// `saveAdditional`: those blocks read as stored.
    fn block_entity_nbt(&self, p: [i32; 3]) -> Option<Tag> {
        if let Some(tag) = super::decorated_pot::pot_data_in(self.world, self.facts, p) {
            return Some(tag);
        }
        let key = (p[0], p[1], p[2]);
        let name = self
            .facts
            .get(self.world.block(p[0], p[1], p[2]))
            .name
            .clone();
        let Some(entity) = self.world.block_entities.get(&key) else {
            return crate::game::spill_common::saved_full_metadata(self.world, p);
        };
        let current = crate::game::commands_common::stored_block_entity_data(
            self.world,
            key,
            &name,
            entity,
            self.items,
            || hopper_cooldown_in(self.hoppers, self.world, key),
        );
        Some(crate::game::spill_common::live_full_metadata(
            self.world, key, entity, &name, self.items, current,
        ))
    }
    fn in_tag(&self, registry: &str, tag: &str, id: &str) -> bool {
        self.refs.tags.contains(registry, tag, id)
    }
}

impl crate::game::GameLevel<'_> {
    /// This level as its location predicates and loot rolls read it.
    pub(in crate::game) fn location_view(&self) -> LevelLocation<'_> {
        LevelLocation {
            world: &*self.world,
            registry: self.registry,
            facts: self.facts,
            biomes: self.biomes,
            items: self.items,
            collision: self.collision,
            hoppers: &*self.positions.hoppers,
            raining: self.raining,
            thundering: self.thundering,
            refs: self.location,
        }
    }
}
