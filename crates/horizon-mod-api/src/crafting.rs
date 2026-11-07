//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
use horizon_items::items::Items;
use horizon_world::native_load_context::LoadContext;
use serde_json::Value;
use std::collections::HashSet;
/// A recipe ingredient of a mod's own kind: the entry named a type in a
/// namespace outside `minecraft:`, and the loader kept it for this call.
/// The kind and the JSON the file wrote are here; fill `items` with the
/// item ids the ingredient matches (item ids only: components are not
/// checked here; a match-time hook follows separately). Duplicates are
/// removed keeping first-seen order. The load calls it once per occurrence
/// (nested ones included), before any table parses the recipes; the
/// answer must be the same every time. An unclaimed kind leaves the list
/// empty and the recipe fails, as an unknown kind does.
#[inline(never)]
pub fn native_ingredient(
    context: &LoadContext,
    kind: &str,
    data: &serde_json::Map<String, Value>,
    items: &mut Vec<String>,
) {
    crate::dispatch_ingredient(&crate::OwningLoad::new(context), kind, data, items);
}
/// Items a mod adds to one of the recipe property sets: the items a furnace,
/// blast furnace, smoker, campfire, smithing table slot or brewing stand slot
/// accepts (`RecipeManager.RECIPE_PROPERTY_SETS`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RecipePropertyAddition {
    /// The set's key, `minecraft:furnace_input` for instance.
    pub set: String,
    /// The item ids to add.
    pub items: Vec<String>,
}
impl RecipePropertyAddition {
    /// `items` added to the set named `set`.
    pub fn new(set: impl Into<String>, items: Vec<String>) -> Self {
        Self {
            set: set.into(),
            items,
        }
    }
}
/// Items a mod's own recipe type takes in a station slot, added to the
/// recipe property sets (`RecipeManager.RECIPE_PROPERTY_SETS`): what a
/// furnace, blast furnace, smoker, campfire, smithing table slot or brewing
/// stand slot accepts, and what a client predicts it accepts. Push one
/// `RecipePropertyAddition` per set to add to; the vanilla recipes' own
/// items stay in. Called once per load, after the recipes are resolved, with
/// the `DataLoadContext` of the load that owns them (`origin()`
/// `ServerDataLoad` on a server, `ClientPreload` on a client's own load),
/// and again after `/reload`. An unknown set key, an item that is air or
/// not a registered item is logged and skipped. The answer must be the same
/// every time.
#[inline(never)]
pub fn native_recipe_property_sets(
    context: &LoadContext,
    additions: &mut Vec<RecipePropertyAddition>,
) {
    crate::dispatch_recipe_property_sets(&crate::OwningLoad::new(context), additions);
}
/// A mod's own ingredient kind, matching a concrete stack at craft time:
/// the ingredient's `test` calls here with the kind, its JSON and the
/// stack being matched, `accept` already true (the load-time list named
/// the item). Set `accept` to false to refuse the stack — its components
/// are the usual reason. The hook may be called many times a craft; the
/// answer must be the same for the same stack.
#[inline(never)]
pub fn native_ingredient_test(
    context: &horizon_world::native_load_context::LoopContext,
    kind: &str,
    data: &serde_json::Map<String, Value>,
    stack: &ItemStack,
    accept: &mut bool,
) {
    crate::dispatch_ingredient_test(&crate::OwningLoop::new(context), kind, data, stack, accept);
}
/// Where `Ingredient::resolve` leaves the hook's answer inside the occurrence
/// it resolved. Written and read only here; the loads that own the sources
/// never send or save it.
const RESOLVED_ITEMS: &str = "horizon:resolved_items";

/// The kind an ingredient object names: `neoforge:ingredient_type` in
/// current NeoForge, `type` in older packs.
fn ingredient_kind(o: &serde_json::Map<String, Value>) -> Option<&str> {
    o.get("neoforge:ingredient_type")
        .or_else(|| o.get("type"))?
        .as_str()
}

/// A mod's own kind: a namespaced type that is not minecraft's.
fn is_custom(kind: &str) -> bool {
    kind.contains(':') && !kind.starts_with("minecraft:")
}

/// Takes the answer `resolve` left out of `data`.
fn resolved_items(data: &mut serde_json::Map<String, Value>) -> Option<Vec<String>> {
    let answer = data.remove(RESOLVED_ITEMS)?;
    answer
        .as_array()?
        .iter()
        .map(|i| i.as_str().map(str::to_owned))
        .collect()
}
/// Keep first-seen order, drop the rest.
// hidden: a game-side helper, not part of the mod surface
#[doc(hidden)]
pub fn dedup_keep_order(items: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    items.retain(|i| seen.insert(i.clone()));
}
/// The walk both ingredient kinds share: a string (an id or `#tag`, as
/// `tags` resolves it), a list of them, or NeoForge's three shapes with
/// their children walked the same way, each answered by the flat list of
/// ids it matches in the order the sources named them. A kind outside the
/// shapes is answered by the wrapper's `custom` as the walk meets it, so a
/// shape sees its ids like any child's: the item ingredient gives the list
/// its hook left at load, a fluid one none, which fails the file. An empty
/// result is no ingredient, as an empty list or an unloaded tag is.
#[derive(Clone, Debug)]
// hidden: shared parser output for the game-owned ingredient adapter, not a mod-facing recipe.
#[doc(hidden)]
#[non_exhaustive]
pub struct WalkedIngredient {
    /// The ids it accepts, however the source shape named them.
    pub set: HashSet<String>,
    /// The same ids in the sources' order (`HolderSet`).
    pub order: Vec<String>,
    /// NeoForge's shapes, with their children walked.
    pub children: Option<(Shape, Vec<Self>)>,
    /// A kind outside the standard shapes: the kind and its JSON, as the
    /// file wrote it, without the wrapper's answer.
    pub custom: Option<(String, serde_json::Map<String, Value>)>,
}
/// The wrapper's answer for a kind outside the standard shapes: the ids it
/// matches, taken out of its JSON, or none to fail the file.
// hidden: the game-owned ingredient adapter supplies custom ids while walking JSON.
#[doc(hidden)]
pub type CustomIds<'a> = &'a dyn Fn(&mut serde_json::Map<String, Value>) -> Option<Vec<String>>;
impl WalkedIngredient {
    /// The walk of `v`: the shapes' flat lists in the sources' order, their
    /// children kept for matching a concrete stack.
    pub fn walk(
        v: &Value,
        tags: &std::collections::HashMap<String, Vec<String>>,
        custom: CustomIds<'_>,
    ) -> Option<Self> {
        Self::walk_resolved(v, tags, custom, &|id| Some(id.to_owned()))
    }

    // hidden: the game-owned adapter resolves ids using its own load-time registry.
    #[doc(hidden)]
    pub fn walk_resolved(
        v: &Value,
        tags: &std::collections::HashMap<String, Vec<String>>,
        custom: CustomIds<'_>,
        resolve: &dyn Fn(&str) -> Option<String>,
    ) -> Option<Self> {
        let mut list: Vec<String> = Vec::new();
        let mut valid = true;
        let mut add = |s: &str| {
            let more: Vec<String> = match s.strip_prefix('#') {
                Some(tag) => tags.get(tag).cloned().unwrap_or_default(),
                None => vec![s.to_owned()],
            };
            for i in more {
                match resolve(&i) {
                    Some(i) if !list.contains(&i) => list.push(i),
                    Some(_) => {}
                    None => valid = false,
                }
            }
        };
        match v {
            Value::String(s) => add(s),
            Value::Array(a) => a.iter().filter_map(Value::as_str).for_each(&mut add),
            Value::Object(o) => return Self::walk_object(o, tags, custom, resolve),
            _ => return None,
        }
        valid.then(|| Self::flat(list)).flatten()
    }
    /// A plain list's answer: its ids in order, no shapes.
    fn flat(list: Vec<String>) -> Option<Self> {
        (!list.is_empty()).then(|| Self {
            set: list.iter().cloned().collect(),
            order: list,
            children: None,
            custom: None,
        })
    }
    /// NeoForge's ingredient types (`NeoForgeRegistries.Keys.INGREDIENT_TYPES`),
    /// the standard shapes mods' recipes carry, each answered by the flat id
    /// list it matches, with the children kept for matching a concrete
    /// stack. A kind outside these is the wrapper's `custom`; one of
    /// `minecraft:`'s the wrapper does not know still fails the file.
    fn walk_object(
        o: &serde_json::Map<String, Value>,
        tags: &std::collections::HashMap<String, Vec<String>>,
        custom: CustomIds<'_>,
        resolve: &dyn Fn(&str) -> Option<String>,
    ) -> Option<Self> {
        // Current NeoForge marks a custom ingredient with
        // `neoforge:ingredient_type`; `type` is the older packs' key.
        let kind = ingredient_kind(o)?;
        let children = || {
            o.get("children")
                .or_else(|| o.get("ingredients"))
                .and_then(Value::as_array)
        };
        // The shapes' walked children, for matching at craft time: every
        // arm either fills it or returns.
        let nested;
        let kept = match kind {
            // `CompoundIngredient`: any of its children matching the stack
            // is enough — the union of their ids.
            "neoforge:compound" => {
                let subs: Vec<Self> = children()?
                    .iter()
                    .map(|s| Self::walk_resolved(s, tags, custom, resolve))
                    .collect::<Option<_>>()?;
                let mut seen = Vec::new();
                for sub in &subs {
                    for id in &sub.order {
                        if !seen.contains(id) {
                            seen.push(id.clone());
                        }
                    }
                }
                nested = Some((Shape::Compound, subs));
                seen
            }
            // `IntersectionIngredient`: every child must match the same
            // stack — the ids all of them name.
            "neoforge:intersection" => {
                let subs: Vec<Self> = children()?
                    .iter()
                    .map(|s| Self::walk_resolved(s, tags, custom, resolve))
                    .collect::<Option<_>>()?;
                nested = Some((Shape::Intersection, subs.clone()));
                subs.first()?
                    .order
                    .iter()
                    .filter(|i| subs[1..].iter().all(|s| s.set.contains(*i)))
                    .cloned()
                    .collect()
            }
            // The base's ids without the subtracted's.
            "neoforge:difference" => {
                let base = Self::walk_resolved(o.get("base")?, tags, custom, resolve)?;
                let gone = Self::walk_resolved(o.get("subtracted")?, tags, custom, resolve)?;
                nested = Some((Shape::Difference, vec![base.clone(), gone.clone()]));
                base.order
                    .iter()
                    .filter(|i| !gone.set.contains(*i))
                    .cloned()
                    .collect()
            }
            // A kind outside these shapes: the wrapper answers it — the
            // item ingredient asks its hook, a fluid one fails the file.
            k if is_custom(k) => {
                let mut data = o.clone();
                let list = custom(&mut data)?;
                return (!list.is_empty()).then(|| Self {
                    set: list.iter().cloned().collect(),
                    order: list,
                    children: None,
                    custom: Some((k.to_owned(), data)),
                });
            }
            _ => return None,
        };
        (!kept.is_empty()).then(|| Self {
            set: kept.iter().cloned().collect(),
            order: kept,
            children: nested,
            custom: None,
        })
    }
}
/// A set of accepted items (expanded from an item, list or tag), and the
/// items in its `HolderSet` order (`Ingredient.display`'s `TagSlotDisplay`).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct Ingredient {
    /// The items it accepts, however the source shape named them.
    pub items: HashSet<String>,
    /// The same items in `HolderSet` order (`display`).
    pub order: Vec<String>,
    /// A mod's own kind, for matching at craft time: the kind and its
    /// JSON, as the load-time hook saw them (`native_ingredient_test`).
    pub custom: Option<(String, serde_json::Map<String, Value>)>,
    /// NeoForge's shapes, with their children parsed: matching a concrete
    /// stack asks each child its own `test` (a mod's kind among them),
    /// as on NeoForge — the flat list above only names items.
    pub children: Option<(Shape, Vec<Self>)>,
}
/// How a shape's children combine at match time.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
// exhaustive: a closed set the game matches without a wildcard
pub enum Shape {
    /// Any child matching the stack is enough.
    Compound,
    /// Every child must match it.
    Intersection,
    /// The first child must, and the second must not.
    Difference,
}
impl Ingredient {
    /// The items of a list, in order.
    pub fn of(items: Vec<String>) -> Self {
        Self {
            items: items.iter().cloned().collect(),
            order: items,
            custom: None,
            children: None,
        }
    }
    pub fn parse(v: &Value, items: &Items) -> Option<Self> {
        // A mod's own kind: `resolve` asked the hook once at load and left
        // the item list it matches with the occurrence; one no load
        // resolved fails the file.
        Self::of_walked(WalkedIngredient::walk(
            v,
            &items.item_tag_order,
            &resolved_items,
        )?)
    }
    /// The walk of an item ingredient's answer: NeoForge's standard shapes
    /// keep their children, and a mod's own kind keeps its kind and JSON,
    /// since the load-time list only names items and matching a concrete
    /// stack (its components) is the match-time hook's.
    fn of_walked(walked: WalkedIngredient) -> Option<Self> {
        // The shapes' children walk the same way; a child no wrapper
        // answers fails the file.
        let children = match walked.children {
            Some((shape, kids)) => Some((
                shape,
                kids.into_iter()
                    .map(Self::of_walked)
                    .collect::<Option<_>>()?,
            )),
            None => None,
        };
        (!walked.order.is_empty()).then(|| Self {
            items: walked.set,
            order: walked.order,
            custom: walked.custom,
            children,
        })
    }
    /// `Ingredient.test`: whether `stack` fits: a stack the list names, and
    /// for a mod's own kind one the match-time hook accepts
    /// (`ICustomIngredient.test`). The game answers with the loop context of
    /// the side `host` runs on; a host without a game refuses every stack.
    pub fn test(&self, host: &dyn Host, stack: &ItemStack) -> bool {
        backend_of(host).is_some_and(|backend| backend.recipe_ingredient_test(self, stack))
    }
}
/// A recipe in force (`RecipeManager.getAllRecipesFor`), whatever its type
/// (vanilla's or a mod's own): its typed parts, and its file for whatever a
/// mod's own type keeps there. Plain data.
///
/// A recipe the game could not parse (an ingredient it cannot read, no
/// result) is not in force and is not listed.
///
/// `ingredients` and `results` are filled from the game's own parse of the
/// shaped, shapeless and cooking types: a shaped recipe's pattern cells row
/// by row (empty cells left out), a shapeless recipe's list, a cooking
/// recipe's `ingredient`, and the result as `results[0]`. Stonecutting,
/// smithing and special crafting recipes instead expose their typed [`RecipeRule`]
/// in `rule`, keeping these two lists empty. Dynamic outputs are rules/templates,
/// not fixed stacks; use [`LevelRef::craft`] to assemble a crafting grid.
/// A mod's own type leaves these fields empty and keeps its `json`.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Recipe {
    /// `<namespace>:<path>`, its place under `data/<namespace>/recipe/`.
    pub id: String,
    /// The recipe type, `minecraft:smelting` or `mymod:crusher`.
    pub kind: String,
    /// The recipe book group, empty for none.
    pub group: String,
    /// What it takes, as above.
    pub ingredients: Vec<Ingredient>,
    /// What it makes, as above.
    pub results: Vec<ItemStack>,
    /// Ticks a cooking recipe (`smelting`, `blasting`, `smoking`,
    /// `campfire_cooking`) takes, its file's `cookingtime`; `None` for any
    /// other type.
    pub cooking_time: Option<u32>,
    /// Experience a cooking recipe gives, `None` for any other type.
    pub experience: Option<f32>,
    /// Parsed stonecutting, smithing or special crafting rule. `None` for
    /// shaped, shapeless, cooking and mod-defined types. The ingredient roles,
    /// bounds and templates come from the game's canonical parse.
    pub rule: Option<RecipeRule>,
    /// The recipe's file, as the data pack wrote it.
    pub json: Arc<Value>,
}
impl Recipe {
    /// The recipe `id` of type `kind` with its file `json`, the typed parts empty.
    pub fn new(id: impl Into<String>, kind: impl Into<String>, json: Arc<Value>) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            group: String::new(),
            ingredients: Vec::new(),
            results: Vec::new(),
            cooking_time: None,
            experience: None,
            rule: None,
            json,
        }
    }
}

/// What a crafting grid makes (`CraftingRecipe.assemble` and `getRemainingItems`), as
/// [`LevelRef::craft`] answers it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CraftResult {
    /// The stack the recipe makes.
    pub output: ItemStack,
    /// What each slot of the grid keeps once the craft takes one item from it, row by row as
    /// the grid was given: an empty bucket for a milk bucket, `None` for most.
    pub remainders: Vec<Option<ItemStack>>,
}
impl CraftResult {
    /// New.
    pub fn new(output: ItemStack, remainders: Vec<Option<ItemStack>>) -> Self {
        Self { output, remainders }
    }
}
