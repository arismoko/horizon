//! `AbstractFurnaceBlockEntity`'s cooking over the furnace data the chunk
//! holds (`containers::Furnace`): it needs the recipes, so it lives above them.
use crate::{
    containers::{Furnace, recipe_used_key},
    crafting::Recipes,
    items::{ItemStack, Items},
};
use sunrise_items::item_data::cooking_fuel::{self, CookingFuel};
use sunrise_world::native_load_context::LoopContext;

/// The furnace methods that read the recipes.
pub trait FurnaceCooking {
    /// `AbstractFurnaceBlockEntity.setItem` on the input slot: anything but
    /// the same item with the same components restarts cooking, with the
    /// total time of the new input's recipe (200 ticks without one).
    fn set_input(&mut self, context: &LoopContext, recipes: &Recipes, stack: Option<ItemStack>);
    /// `recipesUsed`, each recipe with its `experience` (0 for one this data pack does
    /// not have), in the order `getRecipesToAwardAndPopExperience` pays them.
    fn recipe_experience(&self, recipes: &Recipes) -> Vec<(u32, f32)>;
    /// A menu click wrote its slots into `items`, and `before` was the input
    /// slot until then: the menu called `setItem` on it only if it changed
    /// (`Slot.setByPlayer`; partial takes and merges keep the stack).
    fn input_clicked(
        &mut self,
        context: &LoopContext,
        recipes: &Recipes,
        before: Option<ItemStack>,
    );
    /// Count down before building the loot context, which can read this block entity.
    fn count_down_burn(&mut self) -> bool;
    fn prepare_tick<'r>(
        &self,
        context: &LoopContext,
        recipes: &'r Recipes,
        items: &Items,
        was_lit: bool,
        resolve: impl FnOnce(&CookingFuel) -> (i32, f32),
    ) -> FurnaceTick<'r>;
    fn idle(&self) -> bool;
}
impl FurnaceCooking for Furnace {
    fn set_input(&mut self, context: &LoopContext, recipes: &Recipes, stack: Option<ItemStack>) {
        let same = match (&self.items[0], &stack) {
            (Some(old), Some(new)) => old.same_item_same_components(new),
            _ => false,
        };
        self.items[0] = stack;
        if !same {
            self.cooking_total = self.items[0]
                .as_ref()
                .and_then(|input| recipes.cook(context, self.kind.recipe_type(), input))
                .map_or(200, |recipe| total_time(recipe.time, self.speed));
            self.cooking_timer = 0;
        }
    }
    fn recipe_experience(&self, recipes: &Recipes) -> Vec<(u32, f32)> {
        self.recipes_used
            .iter()
            .map(|(id, count)| (*count, recipes.cooking_experience(id)))
            .collect()
    }
    fn input_clicked(
        &mut self,
        context: &LoopContext,
        recipes: &Recipes,
        before: Option<ItemStack>,
    ) {
        if self.items[0] != before {
            let after = std::mem::replace(&mut self.items[0], before);
            self.set_input(context, recipes, after);
        }
    }
    fn count_down_burn(&mut self) -> bool {
        let was_lit = self.lit_remaining > 0;
        if was_lit {
            self.lit_remaining -= 1;
        }
        was_lit
    }

    fn prepare_tick<'r>(
        &self,
        context: &LoopContext,
        recipes: &'r Recipes,
        items: &Items,
        was_lit: bool,
        resolve: impl FnOnce(&CookingFuel) -> (i32, f32),
    ) -> FurnaceTick<'r> {
        let lit = self.lit_remaining > 0;
        let action = match (&self.items[0], &self.items[1]) {
            (None, _) if lit => CookAction::Reset,
            (Some(input), fuel) if lit || fuel.is_some() => {
                match recipes.cook(context, self.kind.recipe_type(), input) {
                    None => CookAction::None,
                    Some(recipe) => {
                        let max = items.get(&recipe.result.item).map_or(64, |i| i.max_stack);
                        if self.items[2].as_ref().is_none_or(|out| {
                            out.same_item_same_components(&recipe.result)
                                && out.count + recipe.result.count <= max
                        }) {
                            let ignition = (!lit).then(|| {
                                fuel.as_ref()
                                    .and_then(|stack| cooking_fuel::for_stack(items, stack))
                                    .map_or((0, 1.0), resolve)
                            });
                            CookAction::Cook(recipe, ignition)
                        } else {
                            CookAction::Reset
                        }
                    }
                }
            }
            _ => CookAction::Decay,
        };
        FurnaceTick {
            action,
            was_lit,
            lit,
        }
    }

    fn idle(&self) -> bool {
        self.lit_remaining <= 0
            && self.cooking_timer <= 0
            && !(self.items[0].is_some() && self.items[1].is_some())
    }
}
/// The read phase of `AbstractFurnaceBlockEntity.serverTick` has finished;
/// apply its decision to the same furnace before processing another tick.
pub struct FurnaceTick<'r> {
    action: CookAction<'r>,
    was_lit: bool,
    lit: bool,
}

enum CookAction<'r> {
    None,
    Reset,
    Decay,
    Cook(&'r crate::crafting::Cooking, Option<(i32, f32)>),
}

pub struct FurnaceTickResult {
    /// `Some(lit)` when the block's LIT property must change.
    pub lit: Option<bool>,
    /// The tick actually consumed a fuel, not merely wrote a duration.
    pub consumed_fuel: bool,
}

impl FurnaceTick<'_> {
    pub fn apply(
        self,
        furnace: &mut Furnace,
        drops: &mut Vec<ItemStack>,
        remainder: impl FnOnce(&ItemStack) -> Option<ItemStack>,
    ) -> FurnaceTickResult {
        let mut lit = self.lit;
        let mut consumed_fuel = false;
        match self.action {
            CookAction::None => {}
            CookAction::Reset => furnace.cooking_timer = 0,
            CookAction::Decay if furnace.cooking_timer > 0 => {
                // Mth.clamp is min(max(v, 0), max), also for negative totals.
                furnace.cooking_timer = (furnace.cooking_timer - 2)
                    .max(0)
                    .min(furnace.cooking_total);
            }
            CookAction::Decay => {}
            CookAction::Cook(recipe, ignition) => {
                if let Some((time, speed)) = ignition {
                    furnace.lit_remaining = time;
                    furnace.lit_total = time;
                    furnace.speed = speed;
                    if furnace.cooking_total > 0 && furnace.cooking_timer < furnace.cooking_total {
                        let ratio = furnace.cooking_timer as f32 / furnace.cooking_total as f32;
                        furnace.cooking_total = total_time(recipe.time, speed);
                        furnace.cooking_timer =
                            (ratio * furnace.cooking_total as f32).ceil() as i32;
                    }
                    if time > 0 {
                        // consumeFuel: leave the remainder in the emptied slot, or drop it.
                        if let Some(fuel) = furnace.items[1].as_mut() {
                            let remainder = remainder(fuel);
                            fuel.count -= 1;
                            if fuel.count == 0 {
                                furnace.items[1] = remainder;
                            } else if let Some(remainder) = remainder {
                                drops.push(remainder);
                            }
                            consumed_fuel = true;
                        }
                        lit = true;
                    }
                }
                if lit {
                    furnace.cooking_timer += 1;
                    if furnace.cooking_timer >= furnace.cooking_total {
                        furnace.cooking_timer = 0;
                        furnace.cooking_total = total_time(recipe.time, furnace.speed);
                        match &mut furnace.items[2] {
                            Some(out) => out.count += recipe.result.count,
                            None => furnace.items[2] = Some(recipe.result.clone()),
                        }
                        *furnace
                            .recipes_used
                            .entry(recipe_used_key(&recipe.id))
                            .or_insert(0) += 1;
                        // burn: drying a wet sponge fills a bucket in the fuel slot.
                        if furnace.items[0]
                            .as_ref()
                            .is_some_and(|s| &*s.item == "minecraft:wet_sponge")
                            && furnace.items[1]
                                .as_ref()
                                .is_some_and(|s| &*s.item == "minecraft:bucket")
                        {
                            furnace.items[1] = Some(ItemStack::new("minecraft:water_bucket", 1));
                        }
                        if let Some(input) = furnace.items[0].as_mut() {
                            input.count -= 1;
                            if input.count == 0 {
                                furnace.items[0] = None;
                            }
                        }
                    }
                } else {
                    furnace.cooking_timer = 0;
                }
            }
        }
        FurnaceTickResult {
            lit: (self.was_lit != lit).then_some(lit),
            consumed_fuel,
        }
    }
}

/// Loot conditions in `BaseContainerBlockEntity.getLootContext`: the live
/// furnace's block state and container, its centre as origin, no entity or tool.
pub struct FurnaceConditions<'a> {
    pub world: &'a dyn crate::advancements::World,
    pub pos: [i32; 3],
}

impl<'v> crate::providers::ConditionContext<'v> for FurnaceConditions<'v> {
    fn named(&self, id: &str) -> Option<std::borrow::Cow<'v, serde_json::Value>> {
        self.world
            .predicate(&crate::advancements::full(id))
            .map(std::borrow::Cow::Borrowed)
    }

    fn valid(&self, condition: &serde_json::Value) -> bool {
        crate::advancements::references_valid(condition, self.world)
    }

    fn leaf(&mut self, condition: &serde_json::Value) -> bool {
        use crate::advancements as adv;
        let context = adv::Entities {
            this: None,
            target: None,
            interacting: None,
            tool: None,
            event_block: self
                .world
                .block_state(self.pos)
                .map(|state| adv::BlockEvent {
                    state,
                    block_entity: Some(self.pos),
                }),
            origin: self.pos.map(|axis| f64::from(axis) + 0.5),
        };
        condition.is_null() || adv::condition_leaf(condition, &context, self.world)
    }
}

fn total_time(time: u32, speed: f32) -> i32 {
    if speed > 0.0 {
        (time as f32 / speed).ceil() as i32
    } else {
        time as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        containers::FurnaceKind,
        items::Items,
        providers::{Evaluation, Providers},
    };
    use sunrise_core::random::{LegacyRandom, LevelRandom};

    pub(super) fn tick(
        furnace: &mut Furnace,
        context: &LoopContext,
        recipes: &Recipes,
        items: &Items,
        providers: &Providers,
        drops: &mut Vec<ItemStack>,
        remainder: &dyn Fn(&ItemStack) -> Option<ItemStack>,
    ) -> Option<bool> {
        let was_lit = furnace.count_down_burn();
        let plan = furnace.prepare_tick(context, recipes, items, was_lit, |fuel| {
            Evaluation::new(providers, &mut LevelRandom::for_level(LegacyRandom::new(0)))
                .cooking_fuel(fuel, &mut |_: &serde_json::Value| false)
        });
        plan.apply(furnace, drops, remainder).lit
    }
    use std::{collections::HashMap, fs};
    use sunrise_world::loot::PackOverlay;

    #[test]
    fn a_furnace_is_idle_only_while_its_tick_changes_nothing() {
        let stack = |id: &str| Some(ItemStack::new(id, 1));
        let furnace = |input, fuel, lit_remaining, cooking_timer| Furnace {
            kind: FurnaceKind::Furnace,
            items: [input, fuel, stack("minecraft:iron_ingot")],
            lit_remaining,
            lit_total: 0,
            cooking_timer,
            cooking_total: 200,
            speed: 1.0,
            recipes_used: HashMap::new(),
        };
        assert!(furnace(None, None, 0, 0).idle());
        assert!(furnace(stack("minecraft:iron_ore"), None, 0, 0).idle());
        assert!(furnace(None, stack("minecraft:coal"), 0, 0).idle());
        assert!(!furnace(stack("minecraft:iron_ore"), stack("minecraft:coal"), 0, 0).idle());
        assert!(!furnace(None, None, 1, 0).idle());
        assert!(!furnace(stack("minecraft:iron_ore"), None, 0, 5).idle());
    }

    /// A data root holding one recipe, `test:ore` into `test:ingot`, and an item table
    /// with two fuels: `test:coal` (no remainder) and `test:cell` (a content mod's item
    /// that leaves `test:husk`). The folder is gone when this returns.
    pub(super) fn data(name: &str) -> (Items, Recipes, Providers) {
        let root =
            std::env::temp_dir().join(format!("sunrise-furnace-{name}-{}", std::process::id(),));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        fs::write(
            root.join("data/minecraft/recipe/test_ingot.json"),
            br#"{"type":"minecraft:smelting","ingredient":"test:ore","result":{"id":"test:ingot"}}"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        let fuel_component =
            r#"{"minecraft:cooking_fuel": {"burn_time": 200, "speed_multiplier": 1.0}}"#;
        fs::write(
            root.join("items.json"),
            format!(
                r#"{{"test:ore": {{}}, "test:ingot": {{}}, "test:coal": {fuel_component}, "test:cell": {fuel_component}, "test:husk": {{}}}}"#
            ),
        )
        .unwrap();
        fs::write(
            root.join(sunrise_items::items::MOD_CRAFT_REMAINDERS),
            r#"{"test:cell": "test:husk"}"#,
        )
        .unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items = Items::catalog(&root, &PackOverlay::NONE, &defaults).unwrap();
        let recipes = Recipes::load(
            &root,
            &items,
            &PackOverlay::NONE,
            &crate::test_resources::server_load(),
        )
        .unwrap();
        let providers = Providers::load(&root);
        let _ = fs::remove_dir_all(&root);
        (items, recipes, providers)
    }

    #[test]
    fn loaded_totals_survive_until_actual_completion() {
        let (items, recipes, providers) = data("loaded-total");
        let context = crate::test_resources::server_loop_context();
        assert_eq!(
            recipes
                .cook(
                    context,
                    FurnaceKind::Furnace.recipe_type(),
                    &ItemStack::new("test:ore", 3)
                )
                .unwrap()
                .time,
            200
        );
        // Fresh Java's five-case Game fixture independently admits the loaded
        // zero (including a missing saved field), five and normal totals.
        for (total, timer, completion) in [(0, 0, 1), (5, 0, 5), (200, 0, 200), (5, 1, 4)] {
            let mut f = Furnace {
                kind: FurnaceKind::Furnace,
                items: [Some(ItemStack::new("test:ore", 3)), None, None],
                lit_remaining: 20,
                lit_total: 20,
                cooking_timer: timer,
                cooking_total: total,
                speed: 1.0,
                recipes_used: HashMap::new(),
            };
            for elapsed in 1..=6 {
                let mut drops = Vec::new();
                assert!(!f.idle());
                assert_eq!(
                    tick(
                        &mut f,
                        context,
                        &recipes,
                        &items,
                        &providers,
                        &mut drops,
                        &|s| { crate::crafting::CraftingRemainder::crafting_remainder(&items, s) }
                    ),
                    None
                );
                assert!(drops.is_empty());
                assert_eq!((f.lit_remaining, f.lit_total), (20 - elapsed, 20));
                assert!(f.items[1].is_none());
                let complete = elapsed >= completion;
                assert_eq!(
                    f.items[0].as_ref().unwrap().count,
                    if complete { 2 } else { 3 },
                    "loaded {total}/{timer}, tick {elapsed}"
                );
                assert_eq!(
                    f.items[2].as_ref().map(|s| (&*s.item, s.count)),
                    if complete {
                        Some(("test:ingot", 1))
                    } else {
                        None
                    }
                );
                assert_eq!(f.cooking_total, if complete { 200 } else { total });
                assert_eq!(
                    f.cooking_timer,
                    if complete {
                        elapsed - completion
                    } else {
                        timer + elapsed
                    }
                );
            }
            // A changed input still goes through setItem's independent reset.
            f.set_input(context, &recipes, Some(ItemStack::new("test:ingot", 1)));
            assert_eq!((f.cooking_timer, f.cooking_total), (0, 200));
        }
    }

    #[test]
    fn cooking_requires_output_components_and_capacity() {
        let (items, recipes, providers) = data("output-components");
        let context = crate::test_resources::server_loop_context();
        let mut different = ItemStack::new("test:ingot", 1);
        different.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("Cold".into()));
        for (output, eligible) in [
            (Some(different), false),
            (None, true),
            (Some(ItemStack::new("test:ingot", 1)), true),
            (Some(ItemStack::new("test:ingot", 64)), false),
            (Some(ItemStack::new("test:ingot", 63)), true),
        ] {
            let before = output.clone();
            let mut f = Furnace {
                kind: FurnaceKind::Furnace,
                items: [
                    Some(ItemStack::new("test:ore", 3)),
                    Some(ItemStack::new("test:coal", 1)),
                    output,
                ],
                lit_remaining: 0,
                lit_total: 0,
                cooking_timer: 0,
                cooking_total: 200,
                speed: 1.0,
                recipes_used: HashMap::new(),
            };
            let recipe = recipes
                .cook(context, f.kind.recipe_type(), f.items[0].as_ref().unwrap())
                .unwrap();
            assert_eq!(
                (&*recipe.result.item, recipe.result.count, recipe.time),
                ("test:ingot", 1, 200)
            );
            let mut drops = Vec::new();
            assert_eq!(
                tick(
                    &mut f,
                    context,
                    &recipes,
                    &items,
                    &providers,
                    &mut drops,
                    &|s| { crate::crafting::CraftingRemainder::crafting_remainder(&items, s) }
                ),
                eligible.then_some(true),
                "output {before:?}"
            );
            assert!(drops.is_empty());
            if eligible {
                assert!(f.items[1].is_none());
                for _ in 1..200 {
                    tick(
                        &mut f,
                        context,
                        &recipes,
                        &items,
                        &providers,
                        &mut drops,
                        &|s| crate::crafting::CraftingRemainder::crafting_remainder(&items, s),
                    );
                }
                assert_eq!(f.items[0].as_ref().unwrap().count, 2);
                assert_eq!(
                    f.items[2].as_ref().unwrap().count,
                    before.as_ref().map_or(0, |s| s.count) + 1
                );
            } else {
                assert_eq!(f.items[0].as_ref().unwrap().count, 3);
                assert_eq!(f.items[1].as_ref().unwrap().count, 1);
                assert_eq!(f.items[2], before);
                assert_eq!((f.lit_remaining, f.cooking_timer), (0, 0));
            }
        }
    }

    /// The furnace sleeps while `idle`, so its tick must change nothing then: run the
    /// real tick on every idle case and compare, and check a furnace that can smelt
    /// isn't idle and does change.
    #[test]
    fn an_idle_furnace_tick_changes_nothing() {
        let (items, recipes, providers) = data("idle");
        let stack = |id: &str| Some(ItemStack::new(id, 1));
        let furnace = |input, fuel, output| Furnace {
            kind: FurnaceKind::Furnace,
            items: [input, fuel, output],
            lit_remaining: 0,
            lit_total: 0,
            cooking_timer: 0,
            cooking_total: 200,
            speed: 1.0,
            recipes_used: HashMap::new(),
        };
        let idle = [
            furnace(None, None, None),
            furnace(stack("test:ore"), None, None),
            furnace(None, stack("test:coal"), None),
            furnace(None, None, stack("test:ingot")),
            furnace(stack("test:ingot"), None, stack("test:ingot")),
        ];
        for before in idle {
            assert!(before.idle(), "{before:?}");
            let mut after = before.clone();
            let mut drops = Vec::new();
            let lit = tick(
                &mut after,
                crate::test_resources::server_loop_context(),
                &recipes,
                &items,
                &providers,
                &mut drops,
                &|s| crate::crafting::CraftingRemainder::crafting_remainder(&items, s),
            );
            assert_eq!(lit, None, "{before:?}");
            assert!(drops.is_empty(), "{before:?}");
            assert_eq!(format!("{after:?}"), format!("{before:?}"));
        }
        let mut busy = furnace(stack("test:ore"), stack("test:coal"), None);
        assert!(!busy.idle());
        let before = format!("{busy:?}");
        assert_eq!(
            tick(
                &mut busy,
                crate::test_resources::server_loop_context(),
                &recipes,
                &items,
                &providers,
                &mut Vec::new(),
                &|s| crate::crafting::CraftingRemainder::crafting_remainder(&items, s),
            ),
            Some(true)
        );
        assert_ne!(format!("{busy:?}"), before);
    }

    /// A furnace with `fuel` x `count` burning `test:ore` into `test:ingot` ([`data`]).
    fn burn(fuel: &str, count: u32) -> (Furnace, Vec<ItemStack>) {
        let (items, recipes, providers) =
            data(&format!("remainder-{}-{count}", fuel.replace(':', "-")));
        let mut furnace = Furnace {
            kind: FurnaceKind::Furnace,
            items: [
                Some(ItemStack::new("test:ore", 1)),
                Some(ItemStack::new(fuel, count)),
                None,
            ],
            lit_remaining: 0,
            lit_total: 0,
            cooking_timer: 0,
            cooking_total: 0,
            speed: 1.0,
            recipes_used: HashMap::new(),
        };
        let mut drops = Vec::new();
        let lit = tick(
            &mut furnace,
            crate::test_resources::server_loop_context(),
            &recipes,
            &items,
            &providers,
            &mut drops,
            &|s| crate::crafting::CraftingRemainder::crafting_remainder(&items, s),
        );
        assert_eq!(lit, Some(true), "the fuel lit the furnace");
        (furnace, drops)
    }

    fn fuel_slot(furnace: &Furnace) -> Option<(&str, u32)> {
        furnace.items[1].as_ref().map(|s| (&*s.item, s.count))
    }

    /// `AbstractFurnaceBlockEntity.consumeFuel`: the last of a fuel with a remainder
    /// leaves it in the fuel slot.
    #[test]
    fn the_last_fuel_leaves_its_remainder_in_the_slot() {
        let (furnace, drops) = burn("test:cell", 1);
        assert_eq!(fuel_slot(&furnace), Some(("test:husk", 1)));
        assert!(drops.is_empty());
    }

    /// With more of the fuel left, the remainder is dropped at the furnace.
    #[test]
    fn a_fuel_still_in_the_stack_drops_its_remainder() {
        let (furnace, drops) = burn("test:cell", 3);
        assert_eq!(fuel_slot(&furnace), Some(("test:cell", 2)));
        assert_eq!(drops, vec![ItemStack::new("test:husk", 1)]);
    }

    /// A fuel without a remainder leaves nothing.
    #[test]
    fn a_fuel_without_a_remainder_leaves_nothing() {
        let (furnace, drops) = burn("test:coal", 1);
        assert_eq!(fuel_slot(&furnace), None);
        assert!(drops.is_empty());
        let (furnace, drops) = burn("test:coal", 2);
        assert_eq!(fuel_slot(&furnace), Some(("test:coal", 1)));
        assert!(drops.is_empty());
    }
}

#[cfg(test)]
mod crafting_remainder_tests {
    use super::tests::tick;
    use super::*;
    // A five-use tool: using it at damage four breaks it.
    fn worn(stack: &ItemStack) -> Option<ItemStack> {
        (stack.damage < 4).then(|| ItemStack {
            count: 1,
            damage: stack.damage + 1,
            ..stack.clone()
        })
    }
    #[test]
    fn crafting_remainder_preserves_cell_and_uses_current_damage() {
        let (_, recipes, _) = super::tests::data("stack-remainder");
        for (size, cell) in [(3, 4), (2, 3)] {
            let mut grid = vec![None; size * size];
            grid[cell] = Some(ItemStack {
                damage: 2,
                ..ItemStack::new("test:cell", 1)
            });
            let first = recipes.remainders(
                crate::test_resources::server_loop_context(),
                "missing",
                &grid,
                size,
                &worn,
            );
            assert_eq!(
                first[cell].as_ref().map(|s| (s.damage, s.count)),
                Some((3, 1))
            );
            assert_eq!(first.iter().flatten().count(), 1);
            let second = recipes.remainders(
                crate::test_resources::server_loop_context(),
                "missing",
                &first,
                size,
                &worn,
            );
            assert_eq!(
                second[cell].as_ref().map(|s| (s.damage, s.count)),
                Some((4, 1))
            );
            assert!(
                recipes
                    .remainders(
                        crate::test_resources::server_loop_context(),
                        "missing",
                        &second,
                        size,
                        &worn
                    )
                    .iter()
                    .all(Option::is_none)
            );
        }
    }
    #[test]
    fn crafting_remainder_fuel_and_crafting_agree() {
        let (items, recipes, providers) = super::tests::data("stack-remainder-fuel");
        let stack = ItemStack {
            damage: 2,
            ..ItemStack::new("test:cell", 1)
        };
        let grid = [Some(stack.clone()), None, None, None];
        let crafted = recipes.remainders(
            crate::test_resources::server_loop_context(),
            "missing",
            &grid,
            2,
            &worn,
        );
        let mut furnace = crate::containers::Furnace {
            kind: crate::containers::FurnaceKind::Furnace,
            items: [Some(ItemStack::new("test:ore", 1)), Some(stack), None],
            lit_remaining: 0,
            lit_total: 0,
            cooking_timer: 0,
            cooking_total: 0,
            speed: 1.0,
            recipes_used: Default::default(),
        };
        let mut drops = Vec::new();
        tick(
            &mut furnace,
            crate::test_resources::server_loop_context(),
            &recipes,
            &items,
            &providers,
            &mut drops,
            &worn,
        );
        assert_eq!(furnace.items[1], crafted[0]);
        assert_eq!(
            furnace.items[1].as_ref().map(|s| (s.damage, s.count)),
            Some((3, 1))
        );
        assert!(drops.is_empty());
    }
}
