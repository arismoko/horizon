//! Advancement file codecs; reject only the owning malformed file.
use super::*;
pub(super) fn parse(id: &str, v: &Value) -> Option<Advancement> {
    if let Err(error) = validate(v) {
        log::warn!("advancement {id} rejected: {error}");
        return None;
    }
    let criteria: Vec<Criterion> = v["criteria"]
        .as_object()?
        .iter()
        .map(|(name, c)| Criterion {
            name: name.clone(),
            trigger: c["trigger"]
                .as_str()
                .map_or(String::new(), |t| full(t).into_owned()),
            conditions: LazyJson::new(&c["conditions"]),
        })
        .collect();
    let requirements = match v["requirements"].as_array() {
        Some(r) => r
            .iter()
            .map(|group| {
                group
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect()
            })
            .collect(),
        // AdvancementRequirements.allOf: every criterion.
        None => criteria.iter().map(|c| vec![c.name.clone()]).collect(),
    };
    let d = &v["display"];
    let display = d.is_object().then(|| Display {
        icon: d["icon"]["id"]
            .as_str()
            .unwrap_or("minecraft:air")
            .to_owned(),
        title: d["title"].clone(),
        description: d["description"].clone(),
        frame: frame(&d["frame"]),
        background: d["background"].as_str().map(str::to_owned),
        show_toast: d["show_toast"].as_bool().unwrap_or(true),
        announce: d["announce_to_chat"].as_bool().unwrap_or(true),
        hidden: d["hidden"].as_bool().unwrap_or(false),
    });
    let r = &v["rewards"];
    let strings = |v: &Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect()
    };
    Some(Advancement {
        id: id.to_owned(),
        parent: v["parent"].as_str().map(|p| full(p).into_owned()),
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

fn validate(value: &Value) -> Result<(), String> {
    use horizon_world::loot::{
        validate_entity_predicate, validate_item_predicate, validate_loot_condition,
    };
    horizon_world::item_defaults::validate_stack(&value["display"]["icon"])?;
    horizon_world::item_defaults::validate_text(&value["display"]["title"])?;
    horizon_world::item_defaults::validate_text(&value["display"]["description"])?;
    for criterion in value["criteria"]
        .as_object()
        .into_iter()
        .flat_map(|map| map.values())
    {
        let trigger = criterion["trigger"].as_str().unwrap_or("");
        if trigger.contains(':') && !trigger.starts_with("minecraft:") {
            continue; // A native trigger owns its schema.
        }
        let c = &criterion["conditions"];
        for key in ["item", "rod", "fired_from_weapon"] {
            validate_item_predicate(&c[key])?;
        }
        for key in ["items", "ingredients"] {
            for item in c[key].as_array().into_iter().flatten() {
                validate_item_predicate(item)?;
            }
        }
        for key in [
            "player",
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
    }
    Ok(())
}
