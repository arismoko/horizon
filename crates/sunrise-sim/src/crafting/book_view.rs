//! What the recipe book shows of a recipe: its `RecipeDisplay` layout and the
//! facts the server's `RecipeManager` keeps per recipe (special, display count,
//! station, notification, result). The book consumes them; the recipe loader
//! produces them.
use crate::crafting::Ingredient;

/// `RecipeDisplay`: how a recipe is laid out.
#[derive(Clone, Debug)]
pub enum Display {
    Shaped {
        width: usize,
        height: usize,
        cells: Vec<Option<Ingredient>>,
    },
    Shapeless(Vec<Ingredient>),
    /// A smelting-type recipe: its input; the fuel is any fuel.
    Furnace(Ingredient),
}

#[derive(Clone, Debug)]
pub(crate) struct KnownEntry {
    pub(crate) special: bool,
    pub(crate) displays: usize,
    /// `RecipeDisplay.craftingStation`: the workstation item of its displays.
    pub(crate) station: &'static str,
    /// `Recipe.showNotification`: unlocking it shows the `RecipeToast`.
    pub(crate) notify: bool,
    /// `result.id`, for the recipes the book doesn't list.
    pub(crate) result: Option<String>,
}

/// `RecipeDisplay.craftingStation` of a recipe type.
pub(crate) fn crafting_station(kind: &str) -> &'static str {
    match kind {
        "minecraft:smelting" => "minecraft:furnace",
        "minecraft:blasting" => "minecraft:blast_furnace",
        "minecraft:smoking" => "minecraft:smoker",
        "minecraft:campfire_cooking" => "minecraft:campfire",
        "minecraft:stonecutting" => "minecraft:stonecutter",
        "minecraft:brewing" => "minecraft:brewing_stand",
        "minecraft:smithing_transform" | "minecraft:smithing_trim" => "minecraft:smithing_table",
        _ => "minecraft:crafting_table",
    }
}
