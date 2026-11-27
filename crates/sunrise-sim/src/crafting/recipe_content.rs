//! NeoForge's full recipe-content route, separate from vanilla book displays.
use super::{
    DecodedRecipe, Ingredient, ParsedStationRecipe, Recipes, ResolvedRecipe, Smithing, Special,
    qualified,
};
use serde_json::{Map, Value, json};
use sunrise_core::identifier::Identifier;

impl Recipes {
    /// Encode selected recipes from the published generation. Vanilla serializers
    /// write resolved holder sets; mod serializers write their own opaque values.
    pub fn recipe_content(&self, types: &[String]) -> Vec<(String, Value)> {
        self.sources
            .iter()
            .filter(|recipe| {
                recipe
                    .recipe_type
                    .as_ref()
                    .is_some_and(|kind| types.contains(kind))
            })
            .filter_map(|recipe| match recipe.write_content() {
                Ok(Some(value)) => Some((recipe.name.clone(), value)),
                Ok(None) => None, // No value, or shared callback dispatch already reported the failure.
                Err(error) => {
                    log::error!("recipe {}: {error}", qualified(&recipe.name));
                    None
                }
            })
            .collect()
    }
}

impl Ingredient {
    /// `Ingredient.getValuesForSync`: default display order can omit candidates
    /// that only match with components. Send the entire set, not the preview.
    fn write_candidates(&self) -> Value {
        json!(self.content_candidates())
    }

    pub(crate) fn content_candidates(&self) -> Vec<&str> {
        let mut order: Vec<_> = self.order.iter().map(String::as_str).collect();
        if order.len() == self.items.len() {
            return order;
        }
        let seen: std::collections::HashSet<_> = order.iter().copied().collect();
        let mut rest: Vec<_> = self
            .items
            .iter()
            .map(String::as_str)
            .filter(|id| !seen.contains(id))
            .collect();
        rest.sort_unstable();
        order.extend(rest);
        order
    }

    pub(super) fn decode_candidates(value: &Value) -> std::result::Result<Self, String> {
        let Some(list) = value.as_array() else {
            return Err("recipe content ingredient must be a resolved item list".into());
        };
        let mut items = std::collections::HashSet::with_capacity(list.len());
        let mut order = Vec::with_capacity(list.len());
        for entry in list {
            let Some(id) = entry.as_str().filter(|id| Identifier::parse(id).is_ok()) else {
                return Err(format!(
                    "invalid item id in recipe content ingredient: {entry}"
                ));
            };
            if items.insert(id.to_owned()) {
                order.push(id.to_owned());
            }
        }
        // An empty holder set carries no candidate matches, not a missing slot.
        let mut ingredient = Self::of(Vec::new());
        ingredient.items = items;
        ingredient.order = order;
        Ok(ingredient)
    }
}

impl ResolvedRecipe {
    fn write_content(&self) -> std::result::Result<Option<Value>, String> {
        use super::Recipe;
        let serializer = &self.file["type"];
        let mut out = Map::from_iter([("type".into(), serializer.clone())]);
        let result = |key: &str, id: Option<&str>, count: u32| {
            let mut result = Map::from_iter([("count".into(), count.into())]);
            if let Some(id) = id {
                result.insert("id".into(), id.into());
            }
            // Component JSON is the component codec's data, not a recipe field
            // to reinterpret. Keep its exact representation (e.g. JSON booleans).
            if let Some(components) = self.file[key].get("components") {
                result.insert("components".into(), components.clone());
            }
            Value::Object(result)
        };
        if let Some(group) = self.group() {
            out.insert("group".into(), group.into());
        }
        match &self.decoded {
            DecodedRecipe::Crafting {
                recipe,
                category,
                show_notification,
                shaped_pattern,
                ..
            } => {
                out.insert("category".into(), json!(category));
                out.insert("show_notification".into(), json!(show_notification));
                let template = match recipe {
                    Recipe::Shaped { result, .. } => {
                        let pattern = shaped_pattern.as_ref().expect("decoded shaped pattern");
                        let keys: Map<_, _> = pattern
                            .keys
                            .iter()
                            .map(|(key, ingredient)| {
                                (key.to_string(), ingredient.write_candidates())
                            })
                            .collect();
                        out.insert("key".into(), Value::Object(keys));
                        out.insert("pattern".into(), json!(pattern.rows));
                        result
                    }
                    Recipe::Shapeless {
                        ingredients,
                        result,
                    } => {
                        out.insert(
                            "ingredients".into(),
                            Value::Array(
                                ingredients
                                    .iter()
                                    .map(Ingredient::write_candidates)
                                    .collect(),
                            ),
                        );
                        result
                    }
                    Recipe::Special(special) => {
                        let (fields, template) = match special {
                            Special::Transmute {
                                input,
                                material,
                                min,
                                max,
                                add_count,
                                result,
                            } => {
                                out.insert(
                                    "material_count".into(),
                                    json!({"min": min, "max": max}),
                                );
                                out.insert("add_material_count_to_result".into(), json!(add_count));
                                (vec![("input", input), ("material", material)], result)
                            }
                            Special::BookCloning {
                                source,
                                material,
                                min,
                                max,
                                result,
                            } => {
                                out.insert(
                                    "allowed_generations".into(),
                                    json!({"min": min, "max": max}),
                                );
                                (vec![("source", source), ("material", material)], result)
                            }
                            Special::FireworkStar {
                                dye,
                                fuel,
                                trail,
                                twinkle,
                                shapes,
                                result,
                            } => {
                                let shapes: Map<_, _> = shapes
                                    .iter()
                                    .map(|(shape, ingredient)| {
                                        (shape.name().into(), ingredient.write_candidates())
                                    })
                                    .collect();
                                out.insert("shapes".into(), Value::Object(shapes));
                                (
                                    vec![
                                        ("dye", dye),
                                        ("fuel", fuel),
                                        ("trail", trail),
                                        ("twinkle", twinkle),
                                    ],
                                    result,
                                )
                            }
                            Special::BannerDuplicate { banner, result } => {
                                (vec![("banner", banner)], result)
                            }
                            Special::ShieldDecoration {
                                banner,
                                target,
                                result,
                            } => (vec![("banner", banner), ("target", target)], result),
                            Special::Dye {
                                target,
                                dye,
                                result,
                            } => (vec![("target", target), ("dye", dye)], result),
                            Special::Imbue {
                                source,
                                material,
                                result,
                            } => (vec![("source", source), ("material", material)], result),
                            Special::FireworkRocket {
                                shell,
                                fuel,
                                star,
                                result,
                            } => (
                                vec![("shell", shell), ("fuel", fuel), ("star", star)],
                                result,
                            ),
                            Special::FireworkStarFade {
                                dye,
                                target,
                                result,
                            } => (vec![("dye", dye), ("target", target)], result),
                            Special::MapExtending {
                                map,
                                material,
                                result,
                            } => (vec![("map", map), ("material", material)], result),
                            Special::DecoratedPot {
                                back,
                                left,
                                right,
                                front,
                                result,
                            } => (
                                vec![
                                    ("back", back),
                                    ("left", left),
                                    ("right", right),
                                    ("front", front),
                                ],
                                result,
                            ),
                            Special::RepairItem => return Ok(Some(Value::Object(out))),
                        };
                        out.extend(
                            fields
                                .iter()
                                .map(|(key, i)| ((*key).into(), i.write_candidates())),
                        );
                        template
                    }
                    Recipe::Mod(_) => {
                        unreachable!("mod serializers have their own decoded variant")
                    }
                };
                out.insert(
                    "result".into(),
                    result("result", template.item.as_deref(), template.count),
                );
            }
            DecodedRecipe::Cooking(cook) => {
                out.insert("category".into(), json!(cook.category));
                out.insert("show_notification".into(), json!(cook.show_notification));
                out.insert("ingredient".into(), cook.ingredient.write_candidates());
                out.insert(
                    "result".into(),
                    result("result", Some(&cook.result.item), cook.result.count),
                );
                out.insert("experience".into(), json!(cook.experience));
                out.insert("cookingtime".into(), json!(cook.time));
            }
            DecodedRecipe::Station {
                recipe,
                category,
                show_notification,
                ..
            } => {
                out.insert("category".into(), json!(category));
                out.insert("show_notification".into(), json!(show_notification));
                match recipe {
                    ParsedStationRecipe::Stonecutting(cut) => {
                        out.insert("ingredient".into(), cut.input.write_candidates());
                        out.insert(
                            "result".into(),
                            result("result", Some(&cut.result.item), cut.result.count),
                        );
                    }
                    ParsedStationRecipe::Smithing(Smithing::Transform {
                        template,
                        base,
                        addition,
                        result: output,
                        ..
                    }) => {
                        for (key, ingredient) in [
                            ("template", template.as_ref()),
                            ("base", Some(base)),
                            ("addition", addition.as_ref()),
                        ] {
                            if let Some(ingredient) = ingredient {
                                out.insert(key.into(), ingredient.write_candidates());
                            }
                        }
                        out.insert(
                            "result".into(),
                            result("result", output.item.as_deref(), output.count),
                        );
                    }
                    ParsedStationRecipe::Smithing(Smithing::Trim {
                        template,
                        base,
                        addition,
                        ..
                    }) => {
                        for (key, ingredient) in [
                            ("template", template),
                            ("base", base),
                            ("addition", addition),
                        ] {
                            out.insert(key.into(), ingredient.write_candidates());
                        }
                        // The pattern has already passed the trim holder codec.
                        out.insert("pattern".into(), self.file["pattern"].clone());
                    }
                }
            }
            DecodedRecipe::Brewing {
                recipe,
                input,
                reagent,
                category,
                ..
            } => {
                out.insert("category".into(), json!(category));
                for (key, ingredient, potion) in [
                    ("input", input, &recipe.input),
                    ("reagent", reagent, &recipe.reagent),
                ] {
                    let mut value = json!({"item": ingredient.write_candidates()});
                    if let Some(potions) = &potion.potions {
                        let mut potions: Vec<_> = potions.iter().collect();
                        potions.sort_unstable();
                        value["potion_contents"] = json!({"potions": potions});
                    }
                    out.insert(key.into(), value);
                }
                out.insert(
                    "output".into(),
                    result("output", Some(&recipe.output.item), recipe.output.count),
                );
            }
            DecodedRecipe::Mod(slot) => {
                let Some(recipe) =
                    slot.get_or_init(|| super::mod_kinds::decode(&self.name, &self.file))
                else {
                    return Ok(None);
                };
                let Some(text) = recipe.written()? else {
                    return Ok(None);
                };
                let Value::Object(mut object) =
                    serde_json::from_str::<Value>(&text).map_err(|e| e.to_string())?
                else {
                    return Err(format!(
                        "serializer {} wrote a non-object",
                        recipe.serializer
                    ));
                };
                object.insert("type".into(), serializer.clone());
                return Ok(Some(Value::Object(object)));
            }
            DecodedRecipe::Ignored => return Ok(None),
        }
        Ok(Some(Value::Object(out)))
    }
}
