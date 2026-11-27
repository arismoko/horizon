//! Advancement file codecs; reject only the owning malformed file.
use super::*;

pub(super) struct PredicateSource<'a, 'o> {
    predicates: Option<&'a HashMap<String, Value>>,
    loaded_predicates: Option<HashMap<String, Value>>,
    captured_types: Option<sunrise_world::loot::types::Types>,
    bound: HashMap<String, BoundCondition>,
    data: Option<&'a Path>,
    overlay: Option<&'a sunrise_world::loot::PackOverlay<'o>>,
}

impl<'a, 'o> PredicateSource<'a, 'o> {
    pub(super) fn from_disk(
        data: &'a Path,
        overlay: &'a sunrise_world::loot::PackOverlay<'o>,
    ) -> Self {
        Self {
            predicates: None,
            loaded_predicates: None,
            captured_types: None,
            bound: HashMap::new(),
            data: Some(data),
            overlay: Some(overlay),
        }
    }

    pub(super) fn from_predicates(predicates: &'a HashMap<String, Value>) -> Self {
        Self {
            predicates: Some(predicates),
            loaded_predicates: None,
            captured_types: None,
            bound: HashMap::new(),
            data: None,
            overlay: None,
        }
    }

    #[cfg(test)]
    pub(super) fn empty() -> Self {
        Self {
            predicates: None,
            loaded_predicates: Some(HashMap::new()),
            captured_types: None,
            bound: HashMap::new(),
            data: None,
            overlay: None,
        }
    }

    /// Bind `player` predicates against `types` instead of the process's captured registry.
    #[cfg(test)]
    pub(super) fn with_types(mut self, types: sunrise_world::loot::types::Types) -> Self {
        self.captured_types = Some(types);
        self
    }

    fn bind_player(&mut self, value: &Value) -> Result<BoundCondition, String> {
        let predicates = match self.predicates {
            Some(predicates) => predicates,
            None => self
                .loaded_predicates
                .get_or_insert_with(|| match (self.data, self.overlay) {
                    (Some(data), Some(overlay)) => load_data_predicates(data, overlay),
                    _ => HashMap::new(),
                }),
        };
        let types = self
            .captured_types
            .get_or_insert_with(sunrise_world::loot::types::Types::capture);
        types
            .validate_condition_field(value, predicates, "conditions.player")
            .map_err(|error| error.to_string())?;
        bind_condition(value, predicates, types, &mut self.bound)
    }
}

fn load_data_predicates(
    data: &Path,
    overlay: &sunrise_world::loot::PackOverlay<'_>,
) -> HashMap<String, Value> {
    let mut out = HashMap::new();
    if let Ok(namespaces) = std::fs::read_dir(data) {
        for ns in namespaces.flatten() {
            let Ok(folder) = sunrise_world::json_folder::read(&ns.path().join("predicate")) else {
                continue;
            };
            let ns = ns.file_name().to_string_lossy().into_owned();
            for file in &folder.files {
                let id = format!("{ns}:{}", file.name);
                if let Ok(v) = serde_json::from_slice::<Value>(file.bytes().unwrap_or_default()) {
                    out.insert(id, v);
                }
            }
        }
    }
    for (id, files) in overlay.kind("predicate") {
        match serde_json::from_slice::<Value>(sunrise_world::loot::PackOverlay::top(files)) {
            Ok(v) => {
                out.insert(id.to_owned(), v);
            }
            Err(e) => {
                out.remove(id);
                log::warn!("Sunrise: pack predicate {id} not loaded: {e}");
            }
        }
    }
    out.retain(|id, v| overlay.conditions.allows_id(v, "predicate", id));
    out
}

#[cfg(test)]
pub(super) fn parse(id: &str, v: &Value) -> Option<Advancement> {
    parse_with(id, v, &mut PredicateSource::empty()).ok()
}

/// `Advancement.CODEC`: the reason a file is refused, which fails the load.
pub(super) fn parse_with(
    id: &str,
    v: &Value,
    source: &mut PredicateSource<'_, '_>,
) -> Result<Advancement, String> {
    sunrise_world::item_defaults::validate_stack(&v["display"]["icon"])
        .and_then(|()| sunrise_world::item_defaults::validate_text(&v["display"]["title"]))
        .and_then(|()| sunrise_world::item_defaults::validate_text(&v["display"]["description"]))?;
    let criterion_map = v["criteria"]
        .as_object()
        .ok_or("criteria: expected an object")?;
    if criterion_map.is_empty() {
        return Err("Advancement criteria cannot be empty".into());
    }
    let mut criteria = Vec::new();
    for (name, criterion) in criterion_map {
        let Some(trigger) = criterion["trigger"]
            .as_str()
            .and_then(|key| sunrise_core::identifier::Identifier::parse(key).ok())
            .filter(|key| !key.path().is_empty())
        else {
            return Err(format!("criterion {name} has an invalid trigger id"));
        };
        let builtin = is_builtin_trigger(trigger.as_str());
        let conditions = &criterion["conditions"];
        let player = if builtin {
            validate_builtin_conditions(conditions).map(|()| None)
        } else if conditions.is_null() || conditions.is_object() {
            // SimpleInstance owns only `player`. The trigger's other fields are opaque to the game.
            let player = &conditions["player"];
            if player.is_null() {
                Ok(None)
            } else {
                source.bind_player(player).map(Some)
            }
        } else {
            Err("conditions: expected an object".into())
        };
        let player = player.map_err(|error| format!("criterion {name}: {error}"))?;
        criteria.push(Criterion {
            name: name.clone(),
            trigger: trigger.as_str().to_owned(),
            conditions: if builtin {
                LazyJson::new(conditions)
            } else {
                conditions.clone().into()
            },
            player,
        });
    }
    let requirements = if v["requirements"].is_null() {
        // AdvancementRequirements.allOf: every criterion.
        criteria.iter().map(|c| vec![c.name.clone()]).collect()
    } else {
        // AdvancementRequirements.CODEC: a list of lists of strings, which
        // `AdvancementRequirements.validate` matches exactly to the criteria.
        let groups: Vec<Vec<String>> = serde_json::from_value(v["requirements"].clone())
            .map_err(|error| format!("requirements: {error}"))?;
        let named: BTreeSet<&str> = groups.iter().flatten().map(String::as_str).collect();
        let expected: BTreeSet<&str> = criteria.iter().map(|c| c.name.as_str()).collect();
        if named != expected {
            return Err(format!(
                "Advancement completion requirements did not exactly match specified criteria. Missing: {:?}. Unknown: {:?}",
                expected.difference(&named).collect::<Vec<_>>(),
                named.difference(&expected).collect::<Vec<_>>(),
            ));
        }
        groups
    };
    let d = &v["display"];
    let display = d.is_object().then(|| Display {
        icon: crate::crafting::recipe_result(&d["icon"])
            .unwrap_or_else(|| crate::items::ItemStack::new("minecraft:air", 1)),
        title: d["title"].clone(),
        description: d["description"].clone(),
        frame: frame(&d["frame"]),
        background: d["background"].as_str().map(str::to_owned),
        show_toast: d["show_toast"].as_bool().unwrap_or(true),
        announce: d["announce_to_chat"].as_bool().unwrap_or(true),
        hidden: d["hidden"].as_bool().unwrap_or(false),
    });
    // `Advancement.validate`.
    let parent = v["parent"].as_str().map(|p| full(p).into_owned());
    let has_background = display.as_ref().is_some_and(|d| d.background.is_some());
    if parent.is_none() && display.is_some() && !has_background {
        return Err("Visible advancement roots must have background".into());
    }
    if parent.is_some() && has_background {
        return Err("Only advancement roots can have background".into());
    }
    let r = &v["rewards"];
    let strings = |v: &Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect()
    };
    Ok(Advancement {
        id: id.to_owned(),
        parent,
        display,
        criteria,
        requirements,
        rewards: Rewards {
            recipes: strings(&r["recipes"]),
            experience: r["experience"].as_i64().unwrap_or(0) as i32,
            loot: strings(&r["loot"]),
            function: r["function"].as_str().map(|id| full(id).into_owned()),
        },
    })
}

fn validate_builtin_conditions(c: &Value) -> Result<(), String> {
    use sunrise_world::loot::{
        validate_entity_predicate, validate_item_predicate, validate_loot_condition,
    };
    validate_loot_condition(&c["player"])?;
    for key in ["item", "rod", "fired_from_weapon"] {
        validate_item_predicate(&c[key])?;
    }
    for key in ["items", "ingredients"] {
        for item in c[key].as_array().into_iter().flatten() {
            validate_item_predicate(item)?;
        }
    }
    for key in [
        "bystander",
        "cause",
        "child",
        "entity",
        "lightning",
        "location",
        "parent",
        "partner",
        "projectile",
        "source",
        "villager",
        "zombie",
        "victims",
    ] {
        validate_loot_condition(&c[key])?;
    }
    for damage in [&c["killing_blow"], &c["damage"]["type"]] {
        validate_entity_predicate(&damage["direct_entity"])?;
        validate_entity_predicate(&damage["source_entity"])?;
    }
    validate_loot_condition(&c["damage"]["source_entity"])?;
    for location in [&c["start_position"], &c["location"]] {
        validate_item_predicate(&location["block"])?;
    }
    Ok(())
}

/// Bind the already validated graph in postorder. Keep each named node in the
/// load's identity map; every incoming edge clones only its shared pointer.
fn bind_condition(
    root: &Value,
    predicates: &HashMap<String, Value>,
    types: &sunrise_world::loot::types::Types,
    bound: &mut HashMap<String, BoundCondition>,
) -> Result<BoundCondition, String> {
    enum Work<'a> {
        Visit(&'a Value),
        FinishReference(String),
        FinishGroup(BoundConditionKind, usize),
    }
    let mut pending = vec![Work::Visit(root)];
    let mut values = Vec::new();
    while let Some(work) = pending.pop() {
        match work {
            Work::Visit(value) => {
                if let Some(id) = value.as_str() {
                    let id = full(id.strip_prefix(':').unwrap_or(id));
                    if let Some(node) = bound.get(&*id) {
                        values.push(node.clone());
                    } else {
                        let target = &predicates[&*id];
                        pending.push(Work::FinishReference(id.into_owned()));
                        pending.push(Work::Visit(target));
                    }
                    continue;
                }
                let kind = value["type"].as_str().expect("validated condition type");
                let kind = full(kind.strip_prefix(':').unwrap_or(kind));
                let (group, terms) = match kind.as_ref() {
                    "minecraft:inverted" => (
                        BoundConditionKind::Inverted,
                        std::slice::from_ref(&value["term"]),
                    ),
                    "minecraft:all_of" | "minecraft:any_of" => {
                        let group = if kind == "minecraft:all_of" {
                            BoundConditionKind::AllOf
                        } else {
                            BoundConditionKind::AnyOf
                        };
                        let terms = &value["terms"];
                        let terms = match terms {
                            Value::Array(list) => list.as_slice(),
                            one => std::slice::from_ref(one),
                        };
                        (group, terms)
                    }
                    leaf => {
                        values.push(BoundCondition::new(
                            bind_leaf(leaf, value, types)?,
                            Vec::new(),
                        ));
                        continue;
                    }
                };
                pending.push(Work::FinishGroup(group, terms.len()));
                pending.extend(terms.iter().rev().map(Work::Visit));
            }
            Work::FinishReference(id) => {
                bound.insert(id, values.last().expect("bound predicate").clone());
            }
            Work::FinishGroup(group, count) => {
                let terms = values.split_off(values.len() - count);
                values.push(BoundCondition::new(group, terms));
            }
        }
    }
    Ok(values.pop().expect("bound root condition"))
}

/// A leaf of a `player` predicate: one this evaluator answers, any other vanilla condition
/// (the shared loot evaluator tests it), or a mod's registered kind. Only a kind that is none of
/// these is refused, as vanilla's codec refuses an unknown type.
fn bind_leaf(
    kind: &str,
    value: &Value,
    types: &sunrise_world::loot::types::Types,
) -> Result<BoundConditionKind, String> {
    const ANSWERED: [&str; 7] = [
        "minecraft:entity_scores",
        "minecraft:entity_properties",
        "minecraft:location_check",
        "minecraft:match_block",
        "minecraft:block_state_property",
        "minecraft:match_tool",
        "minecraft:weather_check",
    ];
    if ANSWERED.contains(&kind) {
        return Ok(BoundConditionKind::Leaf {
            kind: kind.to_owned(),
            data: value.clone(),
        });
    }
    if sunrise_world::loot::types::is_vanilla_condition(kind) {
        let mut condition = value.clone();
        condition["type"] = Value::String(kind.to_owned());
        return Ok(BoundConditionKind::Loot(condition));
    }
    match types
        .bind_registered_condition(kind, value, "conditions.player")
        .map_err(|error| error.to_string())?
    {
        Some(entry) => Ok(BoundConditionKind::Registered(Arc::new(
            RegisteredCondition {
                kind: kind.to_owned(),
                entry,
            },
        ))),
        None => Err(format!(
            "conditions.player: condition type {kind} is not supported in an advancement"
        )),
    }
}
