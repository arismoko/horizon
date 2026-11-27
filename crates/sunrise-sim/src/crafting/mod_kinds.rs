//! Registered serializer resolution, fatal unknown IDs and shared grid dispatch.
use super::*;
use mod_api::{__private::LoadedCraftingRecipe, CraftingGrid};
use std::result::Result;

#[derive(Clone)]
pub struct ModRecipe {
    pub serializer: String,
    loaded: LoadedCraftingRecipe,
}
impl std::fmt::Debug for ModRecipe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModRecipe")
            .field("serializer", &self.serializer)
            .finish_non_exhaustive()
    }
}
use sunrise_items::recipe_sources::source_path;
/// Resolve the file's serializer before parsing, after source conditions have run.
pub fn recipe_type(file: &str, serializer: &str) -> Result<String, String> {
    if super::VANILLA_RECIPE_SERIALIZERS.contains(&serializer) {
        return Ok(if serializer.starts_with("minecraft:crafting_") {
            "minecraft:crafting"
        } else if serializer.starts_with("minecraft:smithing_") {
            "minecraft:smithing"
        } else {
            serializer
        }
        .into());
    }
    mod_api::__private::parts_installation::recipe_serializer(serializer)
        .map(|(recipe_type, _)| recipe_type)
        .ok_or_else(|| {
            format!(
                "recipe {}, unregistered recipe serializer {serializer}",
                source_path(file)
            )
        })
}

#[cfg(test)]
pub(super) fn load(file: &str, json: &Value) -> Option<Recipe> {
    decode(file, json).map(Recipe::Mod)
}
pub(super) fn decode(file: &str, json: &Value) -> Option<ModRecipe> {
    decode_reported(&source_path(file), json, &mut |message| {
        log::error!("{message}")
    })
}
#[cfg(test)]
fn load_reported(file: &str, json: &Value, report: &mut dyn FnMut(String)) -> Option<Recipe> {
    decode_reported(file, json, report).map(Recipe::Mod)
}
fn decode_reported(file: &str, json: &Value, report: &mut dyn FnMut(String)) -> Option<ModRecipe> {
    let kind = json["type"].as_str()?;
    let (_, serializer) = mod_api::__private::parts_installation::recipe_serializer(kind)?;
    let mut object = json.as_object()?.clone();
    object.remove("type");
    let decoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        serializer.decode(&Value::Object(object).to_string())
    }))
    .unwrap_or_else(|_| Err("serializer panicked".into()));
    match decoded {
        Ok(loaded) => Some(ModRecipe {
            serializer: kind.into(),
            loaded,
        }),
        Err(error) => {
            report(format!("recipe {file}, crafting kind {kind}: {error}"));
            None
        }
    }
}
fn snapshot(grid: &[Option<ItemStack>], size: usize) -> Option<CraftingGrid> {
    snapshot_dimensions(grid, size, size, size)
}
fn snapshot_dimensions(
    grid: &[Option<ItemStack>],
    size: usize,
    width: usize,
    height: usize,
) -> Option<CraftingGrid> {
    let width_u32 = u32::try_from(width).ok()?;
    let height_u32 = u32::try_from(height).ok()?;
    if width.checked_mul(height)? == 0 {
        return CraftingGrid::new(width_u32, height_u32, Vec::new()).ok();
    }
    if size < width || height.checked_mul(size).is_none_or(|n| n > grid.len()) {
        return None;
    }
    let cells = (0..height)
        .flat_map(|y| (0..width).map(move |x| y * size + x))
        .map(|i| match grid.get(i)? {
            Some(stack) => to_api(stack).map(Some),
            None => Some(None),
        })
        .collect::<Option<Vec<_>>>()?;
    CraftingGrid::new(width_u32, height_u32, cells).ok()
}

fn to_api(stack: &ItemStack) -> Option<mod_api::ItemStack> {
    let key = mod_api::__private::item_key(&stack.item)?;
    let mut out = mod_api::ItemStack::new(key, stack.count)?;
    if stack.carries_component(None, "minecraft:damage") {
        out.set_damage(stack.damage);
    }
    if let Some(Tag::Compound(patch)) = crate::item_data::encode_extra_components(stack) {
        out.set_components(patch).ok()?;
    }
    Some(out)
}
fn from_api(stack: mod_api::ItemStack) -> Option<ItemStack> {
    let item = mod_api::__private::item_key_id(stack.item())?;
    if stack.count() > 99 {
        return None;
    }
    let mut patch = stack.components().map(|p| p.as_nbt().clone());
    if stack.damage_component() == Some(0) {
        patch
            .get_or_insert_with(Default::default)
            .insert("minecraft:damage".into(), Tag::Int(0));
    }
    let patch = patch.map(Tag::Compound);
    let (_, extra) = crate::item_data::decode_components(patch.as_ref());
    Some(ItemStack {
        item: item.into(),
        count: stack.count(),
        damage: stack.damage(),
        extra,
    })
}
/// Borrows the pre-take grid and its owning default rule for this one callback.
struct CraftingRemainderView<'a> {
    context: mod_api::OwningLoop<'a>,
    grid: &'a [Option<ItemStack>],
    size: usize,
    width: usize,
    height: usize,
    default: &'a dyn Fn(&ItemStack) -> Option<ItemStack>,
}
impl mod_api::LoopContext for CraftingRemainderView<'_> {
    fn is_in_tag(
        &self,
        item: mod_api::ItemKey,
        tag: mod_api::TagKey<mod_api::ItemKey>,
    ) -> Option<bool> {
        self.context.is_in_tag(item, tag)
    }
    fn items_in_tag(&self, tag: mod_api::TagKey<mod_api::ItemKey>) -> Option<&[mod_api::ItemKey]> {
        self.context.items_in_tag(tag)
    }
    fn mod_recipe_property_contains(&self, set: &str, item: mod_api::ItemKey) -> bool {
        self.context.mod_recipe_property_contains(set, item)
    }
    fn side(&self) -> mod_api::LoopSide {
        self.context.side()
    }
    fn registry(&self, name: &str) -> Option<&[String]> {
        self.context.registry(name)
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str> {
        self.context.setting(mod_id, key)
    }
    fn mods(&self) -> &mod_api::LoadedMods {
        self.context.mods()
    }
}
impl mod_api::CraftingRemainders for CraftingRemainderView<'_> {
    fn default_remainder(&self, cell: u32) -> Option<mod_api::ItemStack> {
        let cell = usize::try_from(cell).ok()?;
        if self
            .width
            .checked_mul(self.height)
            .is_none_or(|n| cell >= n)
        {
            return None;
        }
        let stack = self
            .grid
            .get(cell / self.width * self.size + cell % self.width)?
            .as_ref()?;
        (self.default)(stack).and_then(|stack| to_api(&stack))
    }
}

impl ModRecipe {
    pub fn assemble(
        &self,
        context: &LoopContext,
        grid: &[Option<ItemStack>],
        size: usize,
    ) -> Option<Option<ItemStack>> {
        self.assemble_owned(context, snapshot(grid, size)?)
    }
    fn assemble_owned(
        &self,
        context: &LoopContext,
        input: CraftingGrid,
    ) -> Option<Option<ItemStack>> {
        let reader = sunrise_level::api_context::LoopReader(context);
        let context = reader.context();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.loaded
                .matches(&context, input.clone())
                .then(|| self.loaded.assemble(&context, input).and_then(from_api))
        }))
        .ok()
        .flatten()
    }
    pub fn written(&self) -> Result<Option<String>, String> {
        self.loaded.write()
    }
}

#[cfg(test)]
#[path = "mod_kinds_tests.rs"]
pub(crate) mod tests;

impl Recipes {
    /// Lookup preserves the dimensions supplied to recipe_for, including rectangles.
    pub fn find_with_dimensions(
        &self,
        context: &LoopContext,
        grid: &[Option<ItemStack>],
        size: usize,
        width: usize,
        height: usize,
        map_scale: &dyn Fn(i32) -> Option<u8>,
    ) -> Option<(&str, ItemStack)> {
        let vanilla = Input::of_rect(grid, size, width, height);
        if vanilla.is_none() && !self.mod_crafting {
            return None;
        }
        let input = self
            .mod_crafting
            .then(|| snapshot_dimensions(grid, size, width, height))
            .flatten();
        let ctx = Context {
            recipes: self,
            context,
            map_scale,
        };
        for (id, recipe) in &self.crafting {
            let result = match recipe {
                Recipe::Mod(recipe) => input
                    .as_ref()
                    .and_then(|input| recipe.assemble_owned(context, input.clone())),
                _ => vanilla
                    .as_ref()
                    .and_then(|input| Self::assemble_if_matches(&ctx, input, recipe)),
            };
            if let Some(output) = result {
                return output.map(|output| (id.as_str(), output));
            }
        }
        None
    }
    /// Remainders for a rectangular host lookup, placed back on the padded grid.
    pub fn mod_remainders_rect(
        &self,
        context: &LoopContext,
        id: &str,
        grid: &[Option<ItemStack>],
        size: usize,
        width: usize,
        height: usize,
        default: &dyn Fn(&ItemStack) -> Option<ItemStack>,
    ) -> Vec<Option<ItemStack>> {
        let fallback = || {
            grid.iter()
                .map(|cell| cell.as_ref().and_then(default))
                .collect()
        };
        let Some(Recipe::Mod(recipe)) = self.recipe(id) else {
            return fallback();
        };
        let Some(input) = snapshot_dimensions(grid, size, width, height) else {
            return fallback();
        };
        let reader = sunrise_level::api_context::LoopReader(context);
        let view = CraftingRemainderView {
            context: reader.context(),
            grid,
            size,
            width,
            height,
            default,
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            recipe.loaded.remaining_items(&view, input)
        }))
        .ok()
        .flatten();
        let Some(expected) = width.checked_mul(height) else {
            return fallback();
        };
        let Some(result) = result.filter(|r| r.len() == expected) else {
            return fallback();
        };
        let mut placed = vec![None; grid.len()];
        for (i, cell) in result.into_iter().enumerate() {
            placed[i / width * size + i % width] = match cell {
                Some(stack) => match from_api(stack) {
                    Some(stack) => Some(stack),
                    None => return fallback(),
                },
                None => None,
            };
        }
        placed
    }
}

/// Recipe types selected for sync; registered crafting serializers share crafting.
pub fn sync_recipe_types(requested: &[String]) -> Vec<String> {
    let mut types = requested.to_vec();
    if !mod_api::__private::parts_installation::crafting_recipe_kinds().is_empty() {
        types.push("minecraft:crafting".into());
    }
    types.sort();
    types.dedup();
    types
}
