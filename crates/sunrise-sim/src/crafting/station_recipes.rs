//! The stonecutting and smithing recipes: the one canonical JSON parse of
//! `StonecuttingRecipe`, `SmithingTransformRecipe` and `SmithingTrimRecipe`
//! shared by the recipe loader, the station tables and the mod recipe view.
use crate::crafting::{Ingredient, RecipeRule, RecipeTrimPattern, Template};
use crate::items::ItemStack;
use serde_json::Value;

#[derive(Clone, Debug)]
pub enum Smithing {
    Transform {
        id: String,
        template: Option<Ingredient>,
        base: Ingredient,
        addition: Option<Ingredient>,
        result: Template,
    },
    Trim {
        id: String,
        template: Ingredient,
        base: Ingredient,
        addition: Ingredient,
        pattern: RecipeTrimPattern,
    },
}
#[derive(Clone, Debug)]
pub struct Stonecutting {
    pub id: String,
    pub input: Ingredient,
    pub result: ItemStack,
}
/// One canonical parse shared by the station tables and the mod recipe view.
#[derive(Clone, Debug)]
pub(crate) enum ParsedStationRecipe {
    Smithing(Smithing),
    Stonecutting(Stonecutting),
}
impl ParsedStationRecipe {
    pub(crate) fn parse(
        id: String,
        v: &Value,
        ingredient: impl Fn(&Value) -> Option<Ingredient>,
    ) -> Option<Self> {
        match v["type"].as_str().unwrap_or("") {
            "minecraft:smithing_transform" => {
                // Only omitted ingredients mean empty slots; malformed present
                // ingredients invalidate the recipe rather than becoming optional.
                let parse_optional = |key: &str| match v.get(key) {
                    None => Some(None),
                    Some(value) => ingredient(value).map(Some),
                };
                Some(Self::Smithing(Smithing::Transform {
                    id,
                    template: parse_optional("template")?,
                    base: ingredient(&v["base"])?,
                    addition: parse_optional("addition")?,
                    result: Template::parse(&v["result"])?,
                }))
            }
            "minecraft:smithing_trim" => Some(Self::Smithing(Smithing::Trim {
                id,
                template: ingredient(&v["template"])?,
                base: ingredient(&v["base"])?,
                addition: ingredient(&v["addition"])?,
                // Use the same holder codec as item components, collapsing
                // unstyled literal text once at the recipe boundary before
                // publishing the typed snapshot.
                pattern: match crate::item_data::trim::pattern_from_nbt(
                    &crate::crafting::json_tag(v.get("pattern")?),
                )? {
                    crate::item_data::trim::Holder::Named(id) => {
                        RecipeTrimPattern::Reference(id.to_string())
                    }
                    crate::item_data::trim::Holder::Direct(pattern) => RecipeTrimPattern::Inline {
                        asset_id: pattern.asset_id.to_string(),
                        description: pattern.description,
                        decal: pattern.decal,
                    },
                },
            })),
            "minecraft:stonecutting" => Some(Self::Stonecutting(Stonecutting {
                id,
                input: ingredient(&v["ingredient"])?,
                result: Template::parse(&v["result"])
                    .filter(|template| template.item.is_some())?
                    .create(),
            })),
            _ => None,
        }
    }

    pub(crate) fn rule(&self) -> RecipeRule {
        match self {
            Self::Stonecutting(recipe) => RecipeRule::Stonecutting {
                input: recipe.input.clone(),
                result: recipe.result.clone(),
            },
            Self::Smithing(Smithing::Transform {
                template,
                base,
                addition,
                result,
                ..
            }) => RecipeRule::SmithingTransform {
                template: template.clone(),
                base: base.clone(),
                addition: addition.clone(),
                result: result.view(),
            },
            Self::Smithing(Smithing::Trim {
                template,
                base,
                addition,
                pattern,
                ..
            }) => RecipeRule::SmithingTrim {
                template: template.clone(),
                base: base.clone(),
                addition: addition.clone(),
                pattern: pattern.clone(),
            },
        }
    }
}
