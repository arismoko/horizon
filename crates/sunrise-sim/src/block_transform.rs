//! The `minecraft:block_transformer` registry (axe, hoe, shovel) that
//! `BlockTransformer.transformBlock` reads: each transform's rule based state
//! provider is evaluated against the clicked block and the block above.
//! `tools/test_block_transformer_parity.py` compares every block state with
//! vanilla.
use crate::items::Items;
use serde_json::Value;
use std::collections::HashMap;
use sunrise_core::identifier::Identifier;
use sunrise_world::{chunk::BlockState, registry::BlockRegistry};

enum Predicate {
    Blocks {
        names: Vec<String>,
        offset: [i32; 3],
    },
    Tag {
        tag: String,
        offset: [i32; 3],
    },
    AllOf(Vec<Predicate>),
    AnyOf(Vec<Predicate>),
}

enum Provider {
    Rules(Vec<(Predicate, Provider)>),
    State(BlockState),
    /// `CopyPropertiesProvider`: the source with the clicked block's properties.
    Copy(Box<Provider>),
}

/// One `BlockTransformData`.
pub struct Transform {
    provider: Provider,
    pub disallowed_faces: Vec<String>,
    pub sound: String,
    pub particle: String,
    pub loot: Option<String>,
    pub drop_strategy: String,
    pub update_from_neighbors: bool,
    pub transform_type: String,
    pub consume_on_use: bool,
    pub damage: u32,
}

#[derive(Default)]
pub struct Transformers {
    transformers: HashMap<String, Vec<Transform>>,
}

/// What the transformer reads of the world: the block state at an offset from
/// the clicked position.
pub trait Around {
    fn state(&self, offset: [i32; 3]) -> BlockState;
}

/// How a transform's state objects become block states: with the block registry once the
/// world builds its tables, and as written (names and properties only) when a data pack's
/// file is decoded, before any block registry is at hand.
type States<'a> = &'a dyn Fn(&Value) -> std::result::Result<BlockState, String>;

fn as_written(value: &Value) -> std::result::Result<BlockState, String> {
    let name = value["id"]
        .as_str()
        .or_else(|| value["Name"].as_str())
        .ok_or("a block state needs an id")?;
    let mut state = BlockState {
        name: name.into(),
        properties: Default::default(),
    };
    for (k, v) in value["properties"].as_object().into_iter().flatten() {
        let v = v.as_str().ok_or(format!("property {k}: not a string"))?;
        state.properties.insert(k.clone(), v.to_owned());
    }
    Ok(state)
}

fn resolved<'a>(
    registry: &'a BlockRegistry,
) -> impl Fn(&Value) -> std::result::Result<BlockState, String> + 'a {
    move |value| {
        let written = as_written(value)?;
        let default = registry
            .default_state(&written.name)
            .ok_or(format!("unknown block {}", written.name))?;
        let mut state = registry
            .state(default)
            .ok_or(format!("unknown block {}", written.name))?
            .clone();
        for (k, v) in written.properties {
            state.properties.insert(k, v);
        }
        registry
            .resolve(&state)
            .map_err(|_| format!("{} has no such state", written.name))?;
        Ok(state)
    }
}

fn predicate(value: &Value) -> std::result::Result<Predicate, String> {
    let kind = value["type"].as_str().ok_or("a predicate needs a type")?;
    let offset = |value: &Value| -> std::result::Result<[i32; 3], String> {
        let Some(list) = value.get("offset") else {
            return Ok([0; 3]);
        };
        let list = list.as_array().filter(|l| l.len() == 3);
        let at = |i: usize| {
            list.and_then(|l| l[i].as_i64())
                .and_then(|v| i32::try_from(v).ok())
                .ok_or("offset: three ints are required".to_owned())
        };
        Ok([at(0)?, at(1)?, at(2)?])
    };
    let list = |key: &str| -> std::result::Result<Vec<Predicate>, String> {
        value[key]
            .as_array()
            .ok_or(format!("{kind}: {key} is required"))?
            .iter()
            .map(predicate)
            .collect()
    };
    Ok(match kind {
        "minecraft:matching_blocks" => Predicate::Blocks {
            names: match &value["blocks"] {
                Value::String(s) => vec![s.clone()],
                Value::Array(a) => a
                    .iter()
                    .map(|b| b.as_str().map(str::to_owned).ok_or("blocks: not a string"))
                    .collect::<std::result::Result<_, _>>()?,
                _ => return Err("matching_blocks: blocks is required".into()),
            },
            offset: offset(value)?,
        },
        "minecraft:matching_block_tag" => Predicate::Tag {
            tag: value["tag"]
                .as_str()
                .ok_or("matching_block_tag: tag is required")?
                .to_owned(),
            offset: offset(value)?,
        },
        "minecraft:all_of" => Predicate::AllOf(list("predicates")?),
        "minecraft:any_of" => Predicate::AnyOf(list("predicates")?),
        other => return Err(format!("unsupported block predicate {other}")),
    })
}

fn provider(value: &Value, states: States) -> std::result::Result<Provider, String> {
    if !value.is_object() {
        return Err("a block state provider must be an inline object".into());
    }
    match value["type"].as_str() {
        Some("minecraft:rule_based") => {
            let mut rules = Vec::new();
            for rule in value["rules"]
                .as_array()
                .ok_or("rule_based: rules is required")?
            {
                rules.push((
                    predicate(&rule["if_true"])?,
                    provider(&rule["then"], states)?,
                ));
            }
            Ok(Provider::Rules(rules))
        }
        Some("minecraft:copy_properties") => Ok(Provider::Copy(Box::new(provider(
            &value["source"],
            states,
        )?))),
        Some(other) => Err(format!("unsupported block state provider {other}")),
        None => states(value).map(Provider::State),
    }
}

/// `BlockTransformer.DIRECT_CODEC`: one to 200 `BlockTransformData`.
fn transforms(
    value: &Value,
    states: States,
    sound_exists: &dyn Fn(&str) -> bool,
) -> std::result::Result<Vec<Transform>, String> {
    let list = value.as_array().ok_or("not a list")?;
    if !(1..=200).contains(&list.len()) {
        return Err(format!(
            "a list of 1 to 200 transforms is required, found {}",
            list.len()
        ));
    }
    list.iter()
        .enumerate()
        .map(|(index, t)| transform(t, states, sound_exists).map_err(|e| format!("[{index}]: {e}")))
        .collect()
}

fn transform(
    t: &Value,
    states: States,
    sound_exists: &dyn Fn(&str) -> bool,
) -> std::result::Result<Transform, String> {
    if !t.is_object() {
        return Err("not a map".into());
    }
    let choice =
        |key: &str, allowed: &[&str], default: &str| -> std::result::Result<String, String> {
            match t.get(key) {
                None => Ok(default.to_owned()),
                Some(v) => v
                    .as_str()
                    .filter(|s| allowed.contains(s))
                    .map(str::to_owned)
                    .ok_or(format!("{key}: expected one of {allowed:?}")),
            }
        };
    let flag = |key: &str| -> std::result::Result<bool, String> {
        t.get(key).map_or(Ok(true), |v| {
            v.as_bool().ok_or(format!("{key}: not a boolean"))
        })
    };
    let identifier = |key: &str, v: &Value| -> std::result::Result<String, String> {
        let s = v.as_str().ok_or(format!("{key}: not a string"))?;
        Identifier::parse(s).map_err(|e| format!("{key}: {e}"))?;
        Ok(s.to_owned())
    };
    // `SoundEvent.CODEC`: a registered name, or an inline `sound_id` event.
    let sound = match t.get("sound") {
        None => "minecraft:empty".to_owned(),
        Some(Value::String(name)) => {
            if !sound_exists(name) {
                return Err(format!("sound: unknown holder {name}"));
            }
            name.clone()
        }
        Some(Value::Object(inline)) => identifier(
            "sound.sound_id",
            inline.get("sound_id").unwrap_or(&Value::Null),
        )?,
        Some(_) => return Err("sound: not a sound event".into()),
    };
    let disallowed_faces = match t.get("disallowed_faces") {
        None => Vec::new(),
        Some(faces) => faces
            .as_array()
            .ok_or("disallowed_faces: not a list")?
            .iter()
            .map(|face| {
                face.as_str()
                    .filter(|f| ["down", "up", "north", "south", "west", "east"].contains(f))
                    .map(str::to_owned)
                    .ok_or(format!("disallowed_faces: unknown direction {face}"))
            })
            .collect::<std::result::Result<_, _>>()?,
    };
    let damage = match t.get("item_damage_per_use") {
        None => 0,
        Some(v) => v
            .as_i64()
            .and_then(|v| u32::try_from(v).ok())
            .filter(|v| i32::try_from(*v).is_ok())
            .ok_or("item_damage_per_use: a non-negative int is required")?,
    };
    Ok(Transform {
        provider: provider(&t["block_state_provider"], states)
            .map_err(|e| format!("block_state_provider: {e}"))?,
        disallowed_faces,
        sound,
        particle: choice("particle", &["none", "scrape", "wax_on", "wax_off"], "none")?,
        loot: t.get("loot").map(|v| identifier("loot", v)).transpose()?,
        drop_strategy: choice(
            "drop_strategy",
            &["clicked_face", "from_middle"],
            "from_middle",
        )?,
        update_from_neighbors: flag("update_from_neighbors")?,
        transform_type: choice(
            "transform_type",
            &["single_block", "copper_chest"],
            "single_block",
        )?,
        consume_on_use: flag("consume_on_use")?,
        damage,
    })
}

/// `BlockState.withPropertiesOf`: the properties both blocks have.
fn with_properties_of(
    registry: &BlockRegistry,
    mut state: BlockState,
    old: &BlockState,
) -> Option<BlockState> {
    let defaults = registry
        .default_state(&state.name)
        .and_then(|id| registry.state(id))?;
    for (k, v) in &old.properties {
        if defaults.properties.contains_key(k) {
            let mut candidate = state.clone();
            candidate.properties.insert(k.clone(), v.clone());
            if registry.resolve(&candidate).is_ok() {
                state = candidate;
            }
        }
    }
    Some(state)
}

impl Predicate {
    fn test(&self, items: &Items, around: &dyn Around) -> bool {
        match self {
            Self::Blocks { names, offset } => {
                let state = around.state(*offset);
                names.iter().any(|name| match name.strip_prefix('#') {
                    Some(tag) => items
                        .block_tags
                        .get(tag)
                        .is_some_and(|members| members.contains(&state.name)),
                    None => *name == state.name,
                })
            }
            Self::Tag { tag, offset } => {
                let state = around.state(*offset);
                items
                    .block_tags
                    .get(tag)
                    .is_some_and(|members| members.contains(&state.name))
            }
            Self::AllOf(all) => all.iter().all(|p| p.test(items, around)),
            Self::AnyOf(any) => any.iter().any(|p| p.test(items, around)),
        }
    }
}

impl Provider {
    fn state(
        &self,
        items: &Items,
        registry: &BlockRegistry,
        around: &dyn Around,
    ) -> Option<BlockState> {
        match self {
            Self::Rules(rules) => rules
                .iter()
                .find(|(p, _)| p.test(items, around))
                .and_then(|(_, then)| then.state(items, registry, around)),
            Self::State(state) => Some(state.clone()),
            Self::Copy(source) => {
                let state = source.state(items, registry, around)?;
                with_properties_of(registry, state, &around.state([0, 0, 0]))
            }
        }
    }
}

impl Transformers {
    /// `BlockTransformer.DIRECT_CODEC` over one pack file's value, as the registry load decodes it:
    /// any error refuses the load. Blocks and their states are checked against the block
    /// registry by [`Self::build`].
    pub fn validate(
        value: &Value,
        sound_exists: &dyn Fn(&str) -> bool,
    ) -> std::result::Result<(), String> {
        transforms(value, &as_written, sound_exists).map(drop)
    }

    /// The world's `block_transformer` registry: its entries as the load admitted them
    /// (`id` -> file value), with their blocks and states bound to `registry`.
    pub fn build<'a>(
        entries: impl IntoIterator<Item = (&'a str, &'a Value)>,
        registry: &BlockRegistry,
    ) -> std::result::Result<Self, String> {
        let states = resolved(registry);
        let mut transformers = HashMap::new();
        for (id, value) in entries {
            let list = transforms(value, &states, &|_| true).map_err(|e| format!("{id}: {e}"))?;
            transformers.insert(id.to_owned(), list);
        }
        Ok(Self { transformers })
    }

    /// `BlockTransformer.transformBlock` without the world writes: the first
    /// transform that allows the clicked face and provides a state, with its
    /// index.
    pub fn apply<'a>(
        &'a self,
        items: &Items,
        registry: &BlockRegistry,
        id: &str,
        face: &str,
        around: &dyn Around,
    ) -> Option<(usize, &'a Transform, BlockState)> {
        self.transformers
            .get(id)?
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.disallowed_faces.iter().any(|f| f == face))
            .find_map(|(i, t)| Some((i, t, t.provider.state(items, registry, around)?)))
    }

    /// `HoneycombItem.getWaxed`: the waxed twin of a copper block, found from
    /// the axe's wax-off transforms (`WAX_OFF_BY_BLOCK` is the inverse of
    /// `WAXABLES`).
    pub fn waxed(&self, registry: &BlockRegistry, old: &BlockState) -> Option<BlockState> {
        let (names, _) = self
            .wax_pairs()
            .find(|(_, unwaxed)| unwaxed.name == old.name)?;
        let target = registry.state(registry.default_state(names.first()?)?)?;
        with_properties_of(registry, target.clone(), old)
    }

    /// `HoneycombItem.WAX_OFF_BY_BLOCK`: the unwaxed twin of `old`, read
    /// from the same axe rules as `waxed`, with shared properties copied.
    pub fn unwaxed(&self, registry: &BlockRegistry, old: &BlockState) -> Option<BlockState> {
        let (_, target) = self
            .wax_pairs()
            .find(|(names, _)| names.contains(&old.name))?;
        with_properties_of(registry, target.clone(), old)
    }

    fn wax_pairs(&self) -> impl Iterator<Item = (&[String], &BlockState)> {
        self.transformers
            .get("minecraft:axe")
            .into_iter()
            .flatten()
            .filter_map(|transform| match &transform.provider {
                Provider::Rules(rules) if transform.particle == "wax_off" => Some(rules),
                _ => None,
            })
            .flatten()
            .filter_map(|(predicate, then)| {
                let (Predicate::Blocks { names, .. }, Provider::Copy(source)) = (predicate, then)
                else {
                    return None;
                };
                let Provider::State(unwaxed) = &**source else {
                    return None;
                };
                (!names.is_empty()).then_some((names.as_slice(), unwaxed))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sunrise_core::nbt::Tag;

    /// air, stone and dirt.
    fn registry() -> BlockRegistry {
        let state = |id: &str| Tag::compound([("id", Tag::String(id.into()))]);
        BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
            (
                "states",
                Tag::List(vec![
                    state("minecraft:air"),
                    state("minecraft:stone"),
                    state("minecraft:dirt"),
                ]),
            ),
            (
                "defaults",
                Tag::compound([
                    ("minecraft:air", Tag::Int(0)),
                    ("minecraft:stone", Tag::Int(1)),
                    ("minecraft:dirt", Tag::Int(2)),
                ]),
            ),
        ]))
        .unwrap()
    }

    struct Stone;
    impl Around for Stone {
        fn state(&self, _: [i32; 3]) -> BlockState {
            BlockState {
                name: "minecraft:stone".into(),
                properties: Default::default(),
            }
        }
    }

    fn hammer(loot: &str) -> Value {
        json!([{
            "block_state_provider": {"type": "minecraft:rule_based", "rules": [{
                "if_true": {"type": "minecraft:matching_blocks", "blocks": "minecraft:stone"},
                "then": {"id": "minecraft:dirt"}
            }]},
            "sound": {"sound_id": "test:hammer"},
            "loot": loot,
            "drop_strategy": "clicked_face",
            "item_damage_per_use": 3
        }])
    }

    #[test]
    fn a_pack_entry_is_a_transformer_with_its_own_loot_table_and_drop_strategy() {
        let registry = registry();
        let entry = hammer("test:hammer_drops");
        let transformers = Transformers::build([("test:hammer", &entry)], &registry).unwrap();
        let (index, transform, state) = transformers
            .apply(&Items::default(), &registry, "test:hammer", "up", &Stone)
            .unwrap();
        assert_eq!((index, state.name.as_str()), (0, "minecraft:dirt"));
        assert_eq!(transform.loot.as_deref(), Some("test:hammer_drops"));
        assert_eq!(transform.drop_strategy, "clicked_face");
        assert_eq!(transform.damage, 3);
        assert!(
            transformers
                .apply(&Items::default(), &registry, "minecraft:axe", "up", &Stone)
                .is_none()
        );
    }

    #[test]
    fn an_entry_that_names_an_unknown_block_or_breaks_the_codec_is_refused() {
        let registry = registry();
        let mut unknown = hammer("test:hammer_drops");
        unknown[0]["block_state_provider"]["rules"] = json!([{
            "if_true": {"type": "minecraft:matching_blocks", "blocks": "minecraft:stone"},
            "then": {"id": "minecraft:nonexistent"}
        }]);
        unknown[0]["block_state_provider"]["type"] = json!("minecraft:rule_based");
        let error = Transformers::build([("test:unknown", &unknown)], &registry)
            .err()
            .unwrap();
        assert!(
            error.contains("test:unknown") && error.contains("minecraft:nonexistent"),
            "{error}"
        );
        let mut bad = hammer("test:hammer_drops");
        bad[0]["drop_strategy"] = json!("from_the_side");
        assert!(
            Transformers::validate(&bad, &|_| true)
                .unwrap_err()
                .contains("drop_strategy")
        );
        assert!(Transformers::validate(&json!([]), &|_| true).is_err());
    }
}
