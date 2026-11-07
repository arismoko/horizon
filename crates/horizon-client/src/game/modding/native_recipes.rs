//! Every item and recipe the game knows, made once for the mods that read
//! them (native mods' `native_recipes`, capability `recipes` of WebAssembly
//! mods, docs/modding/src/how-to/content-mods.md), and the recipe book's known displays; placing a
//! recipe and creative gifts. The recipes are the window's own data — the
//! content root's — with the server's recipes of the types its native mods
//! had it send (`horizon:recipe_content`) in their place, while the known
//! displays follow the server's book as the recipe book does.
use super::api_values::TryIntoApi;
use super::{
    Game,
    native_hooks::{
        NativeItem as RecipeItem, NativeRecipe as Recipe, NativeRecipeKind as RecipeKind,
        NativeRecipeStack as RecipeStack, NativeRecipes as Recipes,
    },
};
use horizon_client::{
    crafting::{Ingredient, qualified},
    items::ItemStack,
    recipe_book::{Category, Display},
    stations::Smithing,
};
use mod_api::{CraftResult, LevelRef};
use std::{collections::HashMap, sync::Arc};

/// The tables, made once, and the recipe book's known displays with the
/// book's version they are of.
/// The most recipes a server's recipe content may carry.
const MAX_SYNCED_RECIPES: usize = 1_000_000;

#[derive(Default)]
pub(super) struct RecipeTables {
    tables: Option<Arc<Recipes>>,
    pub(super) categories: Option<HashMap<String, mod_api::recipe_book::RecipeBookCategory>>,
    known: (Option<u64>, Arc<[i32]>),
    /// The server's recipes of the types its native mods sent (`horizon:
    /// recipe_content`) over the window's own: what `Game::recipes` reads on
    /// a client once they came.
    synced: Option<horizon_client::crafting::Recipes>,
    /// The recipe content's parts so far, until its last.
    incoming: Vec<(String, serde_json::Value)>,
    /// The types that came before the player did: the mods hear of them
    /// once the helpers can answer (`recipes_received_now`).
    received: Option<Vec<String>>,
}

/// Item id -> its index in [`Recipes::items`].
struct Index(HashMap<Arc<str>, u32>);
impl Index {
    fn stack(&self, item: &str, count: u32, components: String) -> Option<RecipeStack> {
        Some(RecipeStack {
            item: *self.0.get(item)?,
            count,
            components,
        })
    }
    fn of(&self, s: &ItemStack) -> Option<RecipeStack> {
        self.stack(&s.item, s.count, components(s))
    }
    fn any(&self, i: &Ingredient) -> Vec<RecipeStack> {
        i.order
            .iter()
            .filter_map(|id| self.stack(id, 1, String::new()))
            .collect()
    }
}

/// A stack's components beyond its item's, as vanilla's item syntax writes
/// them (`[minecraft:potion_contents={potion:"minecraft:swiftness"}]`), or "".
pub(super) fn components(s: &ItemStack) -> String {
    use horizon_core::nbt::Tag;
    let Some(Tag::Compound(entries)) = horizon_client::item_data::encode_components(s) else {
        return String::new();
    };
    let parts: Vec<String> = entries
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                k.to_string_lossy(),
                horizon_client::snbt::to_snbt(v)
            )
        })
        .collect();
    format!("[{}]", parts.join(","))
}

fn recipe(id: String, kind: RecipeKind) -> Recipe {
    Recipe {
        id,
        kind,
        category: String::new(),
        group: String::new(),
        width: 0,
        height: 0,
        slots: Vec::new(),
        results: Vec::new(),
        cooking_time: 0,
        experience: 0.0,
        book_id: -1,
    }
}

impl Game<'_> {
    /// `RecipeManager.getRecipeFor(CRAFTING, CraftingInput.of(width, height, items), level)`,
    /// then the recipe's `assemble` and `getRemainingItems`: what a `width` by `height` grid
    /// (each 1 to 3, `items` row by row) makes, through the crafting table's matcher, with the
    /// world's data packs' recipes and the special recipes. None when no recipe matches or it
    /// makes nothing, for a grid of another size, and off the server.
    pub(super) fn level_craft(
        &self,
        level: LevelRef,
        width: u32,
        height: u32,
        items: &[Option<ItemStack>],
    ) -> Option<CraftResult> {
        self.native_level_world(level)?;
        let (w, h) = (width as usize, height as usize);
        if !(1..=3).contains(&w) || !(1..=3).contains(&h) || items.len() != w * h {
            return None;
        }
        // The matcher reads a square grid and trims it to what is filled, as `CraftingInput`
        // does, so the grid sits in the top-left corner of one.
        let size = w.max(h);
        let at = |i: usize| (i / w) * size + i % w;
        let mut grid = vec![None; size * size];
        for (i, stack) in items.iter().enumerate() {
            grid[at(i)] = stack.clone().filter(|s| s.count > 0);
        }
        let scale = |id: i32| self.maps.data.get(&id).map(|m| m.scale);
        let recipes = self.recipes();
        let (recipe, output) = recipes
            .find_with(&self.loop_context, &grid, size, &scale)
            .filter(|(_, output)| output.count > 0)?;
        let mut placed = recipes.remainders(recipe, &grid, size);
        let remainders: Vec<_> = (0..w * h).map(|i| placed[at(i)].take()).collect();
        Some(CraftResult::new(
            output.try_into_api().ok()?,
            remainders.try_into_api().ok()?,
        ))
    }
    pub(super) fn registered_recipe_book_category(
        &mut self,
        kind: &str,
    ) -> Option<mod_api::recipe_book::RecipeBookCategory> {
        if self.recipe_tables.categories.is_none() {
            let mut declarations = Vec::new();
            self.native_recipe_book_categories(&mut declarations);
            self.recipe_tables.categories = Some(horizon_client::recipe_book::mod_categories(
                declarations,
                self.recipes(),
                self.items(),
            ));
        }
        self.recipe_tables.categories.as_ref()?.get(kind).cloned()
    }
    /// `give`: `count` of `item`, refused outside a world, unless
    /// creative and for an item the game doesn't have.
    pub(super) fn recipe_give(&mut self, item: &str, count: u32) -> Result<(), String> {
        if self.player.is_none() || self.res.rest.get().is_none() {
            return Err("not in a world".into());
        }
        if !self.creative {
            return Err("only in creative mode".into());
        }
        if self.res.items().get(item).is_none() {
            return Err(format!("no item {item}"));
        }
        self.mod_give(item, count.clamp(1, 99), "");
        Ok(())
    }
    /// `place_recipe`: refused as `recipes.place` refuses it.
    pub(super) fn recipe_place(&mut self, book_id: i32, all: bool) -> Result<(), String> {
        if !self.mod_recipe_screen() {
            return Err("no screen that takes recipes is open".into());
        }
        if !self.known_displays().contains(&book_id) {
            return Err(format!(
                "recipe {book_id} is not in the player's recipe book"
            ));
        }
        self.mod_place_recipe(book_id, all);
        Ok(())
    }
    /// `recipes.give`: a creative player takes the stack as from the
    /// creative inventory; a server hears it as `SetCreativeModeSlot`
    /// through the inventory's sync, and refuses it unless creative.
    pub(super) fn mod_give(&mut self, item: &str, count: u32, components: &str) {
        let Some(max) = self.items().get(item).map(|i| i.max_stack) else {
            return;
        };
        if !self.creative {
            return;
        }
        let (damage, extra) = horizon_client::item_data::command_components(components)
            .map_or((0, None), |tag| {
                horizon_client::item_data::decode_components(Some(&tag))
            });
        let _ = self.add_stack_to_inventory(ItemStack {
            item: item.into(),
            // One stack at most, as the creative inventory hands out.
            count: count.min(max),
            damage,
            extra,
        });
    }
    /// The tables, made on the first call; none while the world resources
    /// they come from are still loading behind the title on a slow start.
    pub(super) fn made_recipes(&mut self) -> Option<Arc<Recipes>> {
        self.res.rest.get()?;
        if self.recipe_tables.tables.is_none() {
            self.recipe_tables.tables = Some(Arc::new(self.make_mod_recipes()));
        }
        self.recipe_tables.tables.clone()
    }
    /// The `book_id`s the player's recipe book holds now.
    pub(super) fn known_displays(&mut self) -> Arc<[i32]> {
        if self.res.rest.get().is_none() {
            return Arc::from([]);
        }
        let (version, known) = self.recipe_book_displays();
        if self.recipe_tables.known.0 != Some(version) {
            self.recipe_tables.known = (Some(version), known.into());
        }
        self.recipe_tables.known.1.clone()
    }

    fn make_mod_recipes(&mut self) -> Recipes {
        let names = &self.pack.names;
        let items = self.items();
        let mut tags: HashMap<&str, Vec<String>> = HashMap::new();
        for (tag, members) in &items.item_tags {
            for m in members {
                tags.entry(m).or_default().push(tag.clone());
            }
        }
        let table: Vec<RecipeItem> = items
            .registry_order
            .iter()
            .map(|id| RecipeItem {
                id: id.to_string(),
                name: names.display_item(id),
                tags: tags.remove(&**id).map_or_else(Vec::new, |mut t| {
                    t.sort();
                    t
                }),
            })
            .collect();
        let index = Index(
            items
                .registry_order
                .iter()
                .enumerate()
                .map(|(i, id)| (id.clone(), i as u32))
                .collect(),
        );
        let mut out = Vec::new();
        let cooking: HashMap<String, (u32, f32)> = self
            .recipes()
            .cooking
            .iter()
            .map(|c| (qualified(&c.id), (c.time, c.experience)))
            .collect();
        for e in self.recipe_book_entries() {
            let furnace = |c: Category| match c.name().split('_').next() {
                Some("blast") => RecipeKind::Blasting,
                Some("smoker") => RecipeKind::Smoking,
                _ => RecipeKind::Smelting,
            };
            let kind = match &e.display {
                Display::Shaped { .. } => RecipeKind::CraftingShaped,
                Display::Shapeless(_) => RecipeKind::CraftingShapeless,
                Display::Furnace(_) => furnace(e.category),
            };
            let mut r = recipe(e.id.clone(), kind);
            r.category = e.category.name().to_owned();
            r.group = e.group.clone().unwrap_or_default();
            r.book_id = e.display_id;
            r.slots = match &e.display {
                Display::Shaped {
                    width,
                    height,
                    cells,
                } => {
                    (r.width, r.height) = (*width as u32, *height as u32);
                    cells
                        .iter()
                        .map(|c| c.as_ref().map_or_else(Vec::new, |i| index.any(i)))
                        .collect()
                }
                Display::Shapeless(list) => list.iter().map(|i| index.any(i)).collect(),
                Display::Furnace(i) => vec![index.any(i)],
            };
            // Requirements shown as other stacks (an arrow's potions): the
            // requirement's index counts the filled slots only.
            let filled: Vec<usize> = (0..r.slots.len())
                .filter(
                    |&k| !matches!(&e.display, Display::Shaped { cells, .. } if cells[k].is_none()),
                )
                .collect();
            for (req, stacks) in &e.stacks {
                if let Some(&k) = filled.get(*req) {
                    r.slots[k] = stacks.iter().filter_map(|s| index.of(s)).collect();
                }
            }
            r.results = e.results().iter().filter_map(|s| index.of(s)).collect();
            if let Some(&(time, xp)) = cooking.get(&e.id) {
                (r.cooking_time, r.experience) = (time, xp);
            }
            out.push(r);
        }
        for c in &self.recipes().cooking {
            if c.kind == "minecraft:campfire_cooking" {
                let mut r = recipe(qualified(&c.id), RecipeKind::Campfire);
                r.group = c.group.clone();
                r.slots = vec![index.any(&c.ingredient)];
                r.results = index.of(&c.result).into_iter().collect();
                (r.cooking_time, r.experience) = (c.time, c.experience);
                out.push(r);
            }
        }
        let stations = self.stations();
        for s in &stations.recipes.stonecutting {
            let mut r = recipe(s.id.clone(), RecipeKind::Stonecutting);
            r.slots = vec![index.any(&s.input)];
            r.results = index.of(&s.result).into_iter().collect();
            out.push(r);
        }
        for s in &stations.recipes.smithing {
            let (id, slots, results) = match s {
                Smithing::Transform {
                    id,
                    template,
                    base,
                    addition,
                    result,
                } => (
                    id,
                    vec![
                        template.as_ref().map(|i| index.any(i)).unwrap_or_default(),
                        index.any(base),
                        addition.as_ref().map(|i| index.any(i)).unwrap_or_default(),
                    ],
                    result
                        .item
                        .as_deref()
                        .and_then(|item| index.stack(item, result.count, String::new()))
                        .into_iter()
                        .collect(),
                ),
                Smithing::Trim {
                    id,
                    template,
                    base,
                    addition,
                    ..
                } => (
                    id,
                    vec![index.any(template), index.any(base), index.any(addition)],
                    index.any(base),
                ),
            };
            let mut r = recipe(id.clone(), RecipeKind::Smithing);
            r.slots = slots;
            r.results = results;
            out.push(r);
        }
        for b in &stations.brewing.recipes {
            let potion = |p: &horizon_client::brewing::PotionIngredient| -> Vec<RecipeStack> {
                let mut item_ids: Vec<&String> = p.items.iter().collect();
                item_ids.sort();
                let mut potions: Vec<&String> = p.potions.iter().flatten().collect();
                potions.sort();
                item_ids
                    .into_iter()
                    .flat_map(|item| {
                        let one = |c: String| index.stack(item, 1, c);
                        if potions.is_empty() {
                            one(String::new()).into_iter().collect::<Vec<_>>()
                        } else {
                            potions
                                .iter()
                                .filter_map(|p| {
                                    one(format!("[minecraft:potion_contents={{potion:\"{p}\"}}]"))
                                })
                                .collect()
                        }
                    })
                    .collect()
            };
            let mut r = recipe(b.id.clone(), RecipeKind::Brewing);
            r.slots = vec![potion(&b.input), potion(&b.reagent)];
            r.results = index.of(&b.output).into_iter().collect();
            out.push(r);
        }
        Recipes {
            items: table,
            recipes: out,
        }
    }
}

impl Game<'_> {
    /// The server's recipes a client was sent, over its own; None on a
    /// server and until a server sends any.
    pub(super) fn synced_recipes(&self) -> Option<&horizon_client::crafting::Recipes> {
        self.recipe_tables.synced.as_ref()
    }
    /// A new play session (`Login`): what the last server sent is gone.
    pub(super) fn forget_synced_recipes(&mut self) {
        self.recipe_tables.synced = None;
        self.recipe_tables.incoming.clear();
        self.recipe_tables.received = None;
        self.recipe_tables.tables = None;
        self.forget_book();
    }
    /// `native_recipes_received` for the types that came, once the window
    /// has its player: the recipes of a join come before it, and the helpers
    /// answer nothing until then. Once a frame.
    pub(super) fn recipes_received_now(&mut self) {
        if self.player.is_some()
            && let Some(types) = self.recipe_tables.received.take()
        {
            self.native_recipes_received(&types);
        }
    }
    /// `OnDatapackSyncEvent.sendRecipes`: the recipe types the native mods
    /// want their clients to have, from this load's tables, become what the
    /// server sends its Horizon clients at a join and after `/reload`. Called
    /// once a load's tables are built, before any player hears of them.
    pub(super) fn sync_recipe_content(&mut self) {
        use horizon_client::lan::recipes;
        let mut types = Vec::new();
        self.native_datapack_sync(&mut types);
        types.sort();
        types.dedup();
        let sources = if types.is_empty() {
            Vec::new()
        } else {
            horizon_items::recipe_sources::sources(&self.options.resources, &self.pack_files())
                .unwrap_or_else(|e| {
                    log::warn!("Horizon: the recipes for the clients were not read: {e}");
                    Vec::new()
                })
        };
        recipes::set_content(recipes::encode_content(
            &types,
            &sources,
            recipes::sent_before(),
        ));
    }
    /// A `horizon:recipe_content` part the server sent the window. On the
    /// last, the window's tables take the server's recipes of those types in
    /// place of its own, and the mods hear of it (`RecipesReceivedEvent`).
    pub(super) fn remote_recipe_content(&mut self, data: &[u8]) {
        let part: horizon_client::lan::recipes::ContentPart = match serde_json::from_slice(data) {
            Ok(part) => part,
            Err(e) => {
                log::warn!("Horizon: bad recipe content from the server: {e}");
                return;
            }
        };
        self.recipe_tables.incoming.extend(part.recipes);
        // The server is trusted to end its parts; one that never does is
        // dropped, not kept growing.
        if self.recipe_tables.incoming.len() > MAX_SYNCED_RECIPES {
            log::warn!("Horizon: the server sent more than {MAX_SYNCED_RECIPES} recipes; dropped");
            self.recipe_tables.incoming = Vec::new();
            return;
        }
        let Some(types) = part.last else {
            return;
        };
        let received = std::mem::take(&mut self.recipe_tables.incoming);
        // No type any more: back on the window's own recipes.
        if types.is_empty() {
            if self.recipe_tables.synced.take().is_some() {
                self.recipe_tables.tables = None;
                self.forget_book();
                self.recipe_tables.received = Some(types);
                self.recipes_received_now();
            }
            return;
        }
        let root = &self.options.resources;
        let overlay = horizon_world::loot::PackOverlay::NONE;
        let built = horizon_items::recipe_sources::sources(root, &overlay).and_then(|own| {
            let of = |v: &serde_json::Value| types.iter().any(|t| v["type"].as_str() == Some(t));
            let mut all: Vec<_> = own.into_iter().filter(|(_, v)| !of(v)).collect();
            all.extend(received);
            horizon_client::crafting::Recipes::from_sources(
                root,
                self.items(),
                &overlay,
                all,
                self.res.core.item_data.context(),
            )
        });
        match built {
            Ok(recipes) => {
                self.recipe_tables.synced = Some(recipes);
                // What was made of the window's own recipes is made again.
                self.recipe_tables.tables = None;
                self.forget_book();
                self.recipe_tables.received = Some(types);
                self.recipes_received_now();
            }
            Err(e) => log::warn!("Horizon: the server's recipes were not taken: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use mod_api::ItemStack;

    fn stack(item: &str) -> Option<ItemStack> {
        ItemStack::new(
            mod_api::__private::item_key(&format!("minecraft:{item}")).unwrap(),
            1,
        )
    }

    /// `level.craft` against vanilla's own recipes: a shaped recipe that hands back buckets, the
    /// same shaped recipe from a smaller grid and from a corner of a larger one, a special
    /// recipe, and grids that make nothing.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn craft_matches_the_grid_and_returns_what_each_slot_keeps() {
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let cake = [
                stack("milk_bucket"),
                stack("milk_bucket"),
                stack("milk_bucket"),
                stack("sugar"),
                stack("egg"),
                stack("sugar"),
                stack("wheat"),
                stack("wheat"),
                stack("wheat"),
            ];
            let made = level.craft(game, 3, 3, &cake).unwrap();
            assert_eq!(
                (made.output.item(), made.output.count()),
                (mod_api::__private::item_key("minecraft:cake").unwrap(), 1)
            );
            let bucket = stack("bucket");
            let mut kept = vec![bucket.clone(), bucket.clone(), bucket];
            kept.extend([None, None, None, None, None, None]);
            assert_eq!(made.remainders, kept);

            let planks = [stack("oak_planks"), stack("oak_planks")];
            let made = level.craft(game, 1, 2, &planks).unwrap();
            assert_eq!(
                (made.output.item(), made.output.count()),
                (mod_api::__private::item_key("minecraft:stick").unwrap(), 4)
            );
            assert_eq!(made.remainders, [None, None]);
            let mut corner = vec![None; 9];
            corner[5] = stack("birch_planks");
            corner[8] = stack("birch_planks");
            let made = level.craft(game, 3, 3, &corner).unwrap();
            assert_eq!(
                (made.output.item(), made.output.count()),
                (mod_api::__private::item_key("minecraft:stick").unwrap(), 4)
            );
            assert_eq!(made.remainders, vec![None; 9]);

            let rocket = [stack("paper"), stack("gunpowder")];
            let made = level.craft(game, 2, 1, &rocket).unwrap();
            assert_eq!(
                (made.output.item(), made.output.count()),
                (
                    mod_api::__private::item_key("minecraft:firework_rocket").unwrap(),
                    3
                )
            );

            assert!(level.craft(game, 1, 1, &[stack("dirt")]).is_none());
            assert!(level.craft(game, 1, 1, &[None]).is_none());
            assert!(level.craft(game, 4, 1, &[None, None, None, None]).is_none());
            assert!(level.craft(game, 1, 2, &planks[..1]).is_none());
        });
    }

    /// What one `level.craft` costs, for benchmarks/craft-match-2026-11-04: a grid that matches
    /// early, one that matches late, a special recipe and a grid no recipe takes (the whole list).
    #[test]
    #[ignore = "a benchmark: run it in release with HORIZON_TEST_RESOURCES"]
    fn craft_cost() {
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let mut cake = vec![stack("milk_bucket"); 3];
            cake.extend([stack("sugar"), stack("egg"), stack("sugar")]);
            cake.extend(vec![stack("wheat"); 3]);
            let grids: [(&str, u32, u32, Vec<Option<ItemStack>>); 4] = [
                ("cake (3x3)", 3, 3, cake),
                ("sticks (1x2)", 1, 2, vec![stack("oak_planks"); 2]),
                (
                    "firework rocket (2x1, special)",
                    2,
                    1,
                    vec![stack("paper"), stack("gunpowder")],
                ),
                ("dirt (1x1, no match)", 1, 1, vec![stack("dirt")]),
            ];
            println!("{} crafting recipes", game.recipes().crafting.len());
            const CALLS: u32 = 10_000;
            for (name, width, height, grid) in grids {
                let start = std::time::Instant::now();
                for _ in 0..CALLS {
                    std::hint::black_box(level.craft(game, width, height, &grid));
                }
                println!(
                    "{name}: {:.1} µs a call",
                    start.elapsed().as_secs_f64() * 1e6 / f64::from(CALLS)
                );
            }
        });
    }
}
