//! A level's shape, chunk format and facts, from the dimension type its server synced: what the
//! player's level takes on entering it (`enter_remote_dimension`), and a viewed level's world on
//! its `open` (`viewed_worlds`). Only the player's level also takes the type's environment
//! attributes (`build_level_environments`).
use super::Remote;
use std::sync::{Arc, Weak};
use sunrise_level::{chunk::LevelShape, environment::Environment};
use sunrise_protocol::{packets::play, registry::DimensionType};
use sunrise_world::dimension_type::DimensionFacts;
use {crate::game::client_side::remote_chunks::ChunkFormat, sunrise_gameplay_common::game::Dim};

/// What a level of the server is, from its synced dimension type.
pub(in crate::game) struct LevelFormat {
    pub(in crate::game) dimension: DimensionType,
    pub(in crate::game) shape: LevelShape,
    /// The format its chunk packets decode with.
    pub(in crate::game) chunk_format: Arc<ChunkFormat>,
    /// Its facts (what it does to fluids, mobs, beds and time); `None` when the server sent no
    /// type the game takes.
    pub(in crate::game) facts: Option<Arc<DimensionFacts>>,
    /// The synchronized dimension type's name and JSON, which the level's environment attributes
    /// start from (`Game::build_level_environments`); `None` only when neither the server nor the
    /// client's own resources have data for it (`fill_known_entries` fills a known pack's entries
    /// from the client's files).
    pub(in crate::game) synced_type: Option<(String, serde_json::Value)>,
}

/// The environments `Game::build_level_environments` built from one synchronized dimension type,
/// and what they were built from.
pub(in crate::game) struct LevelEnvironments {
    /// The type's JSON both were built from.
    pub(in crate::game) json: serde_json::Value,
    /// Over the client's own environment of the type: gameplay's.
    pub(in crate::game) own: Option<Arc<Environment>>,
    /// The packs' stacks `pack` was built over (`None` without pack modifiers). A weak handle:
    /// a reload makes new stacks, and the old ones' address is not reused while it lives.
    pub(in crate::game) stacks: Option<Weak<[Environment; 3]>>,
    /// Over the packs' stack of the type: the frame's.
    pub(in crate::game) pack: Option<Arc<Environment>>,
}

/// The frame's environment of the synchronized type `name` with `json`: the type over the packs'
/// stack of that type, or `None` without pack modifiers or for a type with no pack stack.
pub(in crate::game) fn pack_environment(
    name: &str,
    json: &serde_json::Value,
    stacks: Option<&Arc<[Environment; 3]>>,
    timelines: &sunrise_level::environment::TimelineTable,
) -> Option<Arc<Environment>> {
    let base = match name {
        "minecraft:overworld" => &stacks?[0],
        "minecraft:the_nether" => &stacks?[1],
        "minecraft:the_end" => &stacks?[2],
        _ => return None,
    };
    Environment::with_type(base, json, timelines).map(Arc::new)
}

/// Whether `built` are the packs' current stacks `now`.
pub(in crate::game) fn same_stacks(
    built: Option<&Weak<[Environment; 3]>>,
    now: Option<&Arc<[Environment; 3]>>,
) -> bool {
    match (built, now) {
        (None, None) => true,
        (Some(built), Some(now)) => std::ptr::eq(built.as_ptr(), Arc::as_ptr(now)),
        _ => false,
    }
}

impl Remote {
    /// The format and facts of the level `spawn` names, from the server's dimension type. The
    /// facts are cached by level and named type.
    pub(in crate::game) fn level_format(&mut self, spawn: &play::SpawnInfo) -> LevelFormat {
        let dimension = self.registries.dimension(spawn.dimension_type);
        // The server's dimension type gives the level's shape, and its
        // chunks the shape's column.
        let shape = LevelShape::new(
            dimension.min_y,
            dimension.height,
            dimension.logical_height,
            dimension.has_skylight,
            dimension.has_ceiling,
        )
        .unwrap_or_else(|_| Dim::from_id(&dimension.name).shape());
        let chunk_format = Arc::new(
            self.chunk_format
                .with_dimension(&dimension)
                .in_column(shape.column()),
        );
        // The server's dimension type gives the level's facts (what it does to fluids, mobs,
        // beds and time), not only its shape: a level of a mod the server runs is no
        // longer taken for the Overworld.
        let mut facts = None;
        let synced_type = self.registries.dimension_json(spawn.dimension_type);
        if let Some((name, json)) = &synced_type {
            let cached = self
                .facts_cache
                .get(&spawn.dimension)
                .filter(|(old_name, old, _)| old_name == name && old == json)
                .map(|(_, _, facts)| facts.clone());
            facts = cached.or_else(|| {
                let base = DimensionFacts::base_for_type(name);
                let facts = (|| {
                    for field in ["coordinate_scale", "ambient_light"] {
                        if json.get(field).is_some()
                            && !json[field].as_f64().is_some_and(f64::is_finite)
                        {
                            return Err(format!("invalid {field}"));
                        }
                    }
                    if json.get("monster_spawn_block_light_limit").is_some()
                        && json["monster_spawn_block_light_limit"].as_i64().is_none()
                    {
                        return Err("invalid monster_spawn_block_light_limit".into());
                    }
                    DimensionFacts::patched(base, name, json)
                })()
                .map_err(|why| log::warn!("the server's dimension type {name} is refused ({why})"))
                .unwrap_or_else(|_| base.clone());
                if !super::level_format::sane_facts(&facts) {
                    log::warn!(
                        "the server's dimension type {name} is refused (a value out of range)"
                    );
                    return Some(Arc::new(base.clone()));
                }
                let facts = Arc::new(facts);
                self.facts_cache.insert(
                    spawn.dimension.clone(),
                    (name.clone(), json.clone(), facts.clone()),
                );
                Some(facts)
            });
        }
        LevelFormat {
            dimension,
            shape,
            chunk_format,
            facts,
            synced_type,
        }
    }
}

/// Whether a synchronized dimension type's values are inside the codec's ranges
/// (`DimensionType.java`: a coordinate scale of `1.0E-5F` to `3.0E7`, a monster spawn block light
/// limit of 0 to 15; the ambient light has no range), so a portal's ratio of two scales is finite.
pub(in crate::game) fn sane_facts(facts: &DimensionFacts) -> bool {
    (f64::from(1.0e-5_f32)..=3.0e7).contains(&facts.coordinate_scale)
        && (0..=15).contains(&facts.monster_spawn_block_light_limit)
        && facts.ambient_light.is_finite()
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::sane_facts;
    use sunrise_world::dimension_type::DimensionFacts;

    /// A type outside the codec's ranges is not taken; the vanilla ones are.
    #[test]
    fn a_type_outside_the_codecs_ranges_is_refused() {
        let mut facts = DimensionFacts::overworld().clone();
        assert!(sane_facts(&facts));
        assert!(sane_facts(DimensionFacts::of_builtin_level(
            sunrise_core::LevelKey::NETHER
        )));
        for scale in [0.0, f64::from_bits(1), -1.0, 3.1e7, f64::NAN] {
            facts.coordinate_scale = scale;
            assert!(!sane_facts(&facts), "{scale}");
        }
        facts.coordinate_scale = 1.0;
        // the codec sets no range on the ambient light
        facts.ambient_light = 1.5;
        assert!(sane_facts(&facts));
        facts.monster_spawn_block_light_limit = 16;
        assert!(!sane_facts(&facts));
    }
}
