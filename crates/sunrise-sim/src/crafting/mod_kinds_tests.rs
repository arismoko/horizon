//! Literal registered-kind fixtures used by crafting menu tests.
use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use sunrise_world::native_load_context as engine;
type Calls = Arc<Mutex<Vec<Vec<u32>>>>;
struct Kind {
    shape: Option<(u32, u32)>,
    input: mod_api::ItemKey,
    output: mod_api::ItemKey,
    rest: mod_api::ItemKey,
    byproducts: mod_api::TagKey,
    calls: Calls,
    decodes: Arc<AtomicUsize>,
}
impl mod_api::CraftingRecipeKind for Kind {
    type Value = u32;
    fn decode(&self, json: &str) -> Result<u32, String> {
        self.decodes.fetch_add(1, Ordering::Relaxed);
        let value: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        value["rest"]
            .as_u64()
            .filter(|n| *n <= 3)
            .map(|n| n as u32)
            .ok_or("rest must be 0, 1, 2 or 3".into())
    }
    fn write(&self, value: &u32) -> Result<String, String> {
        Ok(format!("{{\"rest\":{value}}}"))
    }
    fn matches(
        &self,
        _context: &dyn mod_api::LoopContext,
        value: &u32,
        grid: CraftingGrid,
    ) -> bool {
        if self
            .shape
            .is_some_and(|shape| shape != (grid.width, grid.height))
        {
            return false;
        }
        grid.cells.iter().enumerate().all(|(i, cell)| {
            if i == 0 || (*value == 3 && i == 4) {
                cell.as_ref().is_some_and(|s| s.item() == self.input)
            } else {
                cell.is_none()
            }
        })
    }
    fn assemble(
        &self,
        _context: &dyn mod_api::LoopContext,
        _: &u32,
        _: CraftingGrid,
    ) -> Option<mod_api::ItemStack> {
        mod_api::ItemStack::new(self.output, 3)
    }
    fn remaining_items(
        &self,
        context: &dyn mod_api::CraftingRemainders,
        value: &u32,
        grid: CraftingGrid,
    ) -> Option<Vec<Option<mod_api::ItemStack>>> {
        self.calls.lock().unwrap().push(
            grid.cells
                .iter()
                .map(|s| s.as_ref().map_or(0, |s| s.count()))
                .collect(),
        );
        let mut out = vec![None; grid.cells.len()];
        if *value == 3 {
            out[4] = context.default_remainder(4);
            let &key = context.items_in_tag(self.byproducts.typed())?.first()?;
            let mut custom = mod_api::ItemStack::new(key, 2)?;
            custom.set_damage(23);
            out[0] = Some(custom);
            assert!(context.default_remainder(5).is_none());
            assert!(context.default_remainder(6).is_none());
            return Some(out);
        }
        out[0] = match value {
            1 => mod_api::ItemStack::new(self.input, 1),
            2 => mod_api::ItemStack::new(self.rest, 1),
            _ => None,
        };
        Some(out)
    }
}
struct Mod {
    shape: Option<(u32, u32)>,
    calls: Calls,
    decodes: Arc<AtomicUsize>,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for Mod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let kind = Kind {
            shape: self.shape,
            input: r.item_key("test:input"),
            output: r.item_key("test:output"),
            rest: r.item_key("test:rest"),
            byproducts: r.tag_key("test:byproducts"),
            calls: self.calls.clone(),
            decodes: self.decodes.clone(),
        };
        r.register_crafting_recipe_serializer("literal", kind);
    }
}
pub(crate) fn with_recipe(rest: u32, f: impl FnOnce(Recipes, Calls)) {
    with_shape_recipe(rest, None, f);
}
pub(crate) fn with_shape_recipe(
    rest: u32,
    shape: Option<(u32, u32)>,
    f: impl FnOnce(Recipes, Calls),
) {
    with_literal_kind(shape, |calls, _| {
        let mut recipes = Recipes::empty();
        recipes.mod_crafting = true;
        recipes.crafting.push((
            "test:literal".into(),
            load(
                "test:literal",
                &serde_json::json!({"type":"test:literal", "rest":rest}),
            )
            .unwrap(),
        ));
        f(recipes, calls);
    });
}

fn with_literal_kind(shape: Option<(u32, u32)>, f: impl FnOnce(Calls, &AtomicUsize)) {
    let calls: Calls = Default::default();
    let decodes = Arc::new(AtomicUsize::new(0));
    let entry = || {
        mod_api::ModEntry::new(
            "test",
            Box::new(Mod {
                shape,
                calls: calls.clone(),
                decodes: decodes.clone(),
            }),
        )
    };
    let mods = vec![entry()];
    let entries = [entry()];
    mod_platform::testing::with_mods(mods.into(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&entries).unwrap();
        mod_api::__private::resolve_items(["test:input", "test:output", "test:rest"]).unwrap();
        f(calls, &decodes);
    });
}

#[test]
fn recipe_sync_panic_disables_just_that_writer_across_loads() {
    struct Writer;
    impl mod_api::RecipeSerializer for Writer {
        type Value = String;
        fn decode(&self, json: &str) -> Result<String, String> {
            Ok(json.to_owned())
        }
        fn write(&self, json: &String) -> Result<String, String> {
            match serde_json::from_str::<Value>(json).unwrap()["mode"].as_str() {
                Some("panic") => panic!("recipe writer regression panic"),
                Some("reject") => Err("recipe writer rejected input".into()),
                _ => Ok(json.clone()),
            }
        }
    }
    struct Writers;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Writers {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.register_recipe_type("sync");
            for writer in ["panicking", "healthy"] {
                r.register_read_only_recipe_serializer(writer, "writer_owner:sync", Writer);
            }
        }
    }
    let entry = || mod_api::ModEntry::new("writer_owner", Box::new(Writers));
    crate::test_log::install();
    mod_platform::testing::with_mods(vec![entry()].into(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&[entry()]).unwrap();
        let data = crate::test_resources::server_load();
        let items = Items::empty();
        let root = tempfile::tempdir().unwrap();
        let types = ["writer_owner:sync".to_owned()];
        let load = |first_mode| {
            Recipes::from_sources(
                root.path(),
                &items,
                &PackOverlay::NONE,
                [
                    ("fixture:first", "writer_owner:panicking", first_mode),
                    ("fixture:rejected", "writer_owner:healthy", "reject"),
                    ("fixture:sent", "writer_owner:healthy", "keep"),
                ]
                .map(|(id, serializer, mode)| {
                    (
                        id.into(),
                        serde_json::json!({"type":serializer, "mode":mode}),
                    )
                })
                .into(),
                &data,
            )
            .unwrap()
        };
        crate::test_log::clear();
        let expected = [(
            "fixture:sent".to_owned(),
            serde_json::json!({"type":"writer_owner:healthy", "mode":"keep"}),
        )];
        assert_eq!(load("panic").recipe_content(&types), expected);
        assert_eq!(load("keep").recipe_content(&types), expected);
        let logs = crate::test_log::records();
        assert_eq!(
            logs.iter()
                .filter(|(level, text)| *level == log::Level::Error
                    && text.contains("writer_owner")
                    && text.contains("recipe writer regression panic"))
                .count(),
            1
        );
        assert_eq!(
            logs.iter()
                .filter(|(level, text)| *level == log::Level::Error
                    && text.contains("fixture:rejected")
                    && text.contains("recipe writer rejected input"))
                .count(),
            2
        );
        assert_eq!(
            mod_api::__private::take_hook_panics(),
            [mod_api::__private::HookPanicReport {
                mod_id: "writer_owner",
                hook: "recipe_serializer_write",
            }]
        );
    });
}

#[test]
fn custom_recipes_decode_once_per_source_generation() {
    with_literal_kind(None, |_, decodes| {
        let id = "test:literal";
        let load = crate::test_resources::server_load();
        let context = load.captured_loop_context(engine::LoopSide::Server);
        let root = tempfile::tempdir().unwrap();
        let items = Items::empty();
        let resolve = |rest| {
            ResolvedSources::resolve(
                vec![
                    (
                        "before".into(),
                        serde_json::json!({"type":"minecraft:crafting_special_repairitem"}),
                    ),
                    (
                        id.into(),
                        serde_json::json!({"type":"test:literal", "rest":rest}),
                    ),
                ],
                &items,
                &load,
            )
            .unwrap()
        };
        let project = |sources: &ResolvedSources| {
            Recipes::from_resolved(root.path(), &items, &PackOverlay::NONE, sources).unwrap()
        };
        let written = |recipes: &Recipes| {
            let Some(Recipe::Mod(recipe)) = recipes.recipe(id) else {
                panic!("missing registered recipe");
            };
            recipe.written().unwrap().unwrap()
        };
        let mut grid = [const { None }; 9];
        grid[0] = Some(ItemStack::new("test:input", 1));
        let expected = Some(ItemStack::new("test:output", 3));

        let sources = resolve(1);
        assert_eq!(decodes.load(Ordering::Relaxed), 0, "no decode in resolve");
        let first = project(&sources);
        let second = project(&sources);
        assert_eq!(decodes.load(Ordering::Relaxed), 1);
        for recipes in [&first, &second] {
            assert_eq!(recipes.craft(&context, &grid, 3), expected);
            assert_eq!(written(recipes), r#"{"rest":1}"#);
        }

        let next_sources = resolve(3);
        assert_eq!(decodes.load(Ordering::Relaxed), 1);
        let next = project(&next_sources);
        assert_eq!(decodes.load(Ordering::Relaxed), 2);
        assert_eq!(written(&next), r#"{"rest":3}"#);
        assert_eq!(next.craft(&context, &grid, 3), None);
        assert_eq!(first.craft(&context, &grid, 3), expected);
        assert_eq!(written(&first), r#"{"rest":1}"#);
        grid[4] = Some(ItemStack::new("test:input", 2));
        assert_eq!(next.craft(&context, &grid, 3), expected);

        let refused = resolve(99);
        assert!(project(&refused).recipe(id).is_none());
        assert!(project(&refused).recipe(id).is_none());
        assert_eq!(decodes.load(Ordering::Relaxed), 3, "refusal was cached");
    });
}

#[test]
fn rectangular_kind_mixes_component_aware_defaults_custom_and_empty_cells() {
    with_shape_recipe(3, Some((2, 3)), |recipes, calls| {
        let load = engine::LoadContext::new(
            engine::DataLoadOrigin::ServerDataLoad,
            Arc::new(engine::LoadInputs::new(
                Default::default(),
                Default::default(),
            )),
            [],
        );
        let mut context = load.captured_loop_context(engine::LoopSide::Server);
        context.set_item_tags(mod_api::__private::BoundItemTags::bind(
            &[(
                "test:byproducts".into(),
                vec!["test:rest".into(), "test:input".into()],
            )]
            .into(),
            &["test:input", "test:output", "test:rest"]
                .into_iter()
                .collect(),
        ));
        let mut grid = vec![None; 9];
        grid[0] = Some(ItemStack::new("test:input", 7));
        let mut input = ItemStack::new("test:input", 3);
        input.damage = 11;
        let patch = Tag::Compound(
            [(
                "minecraft:custom_data".into(),
                Tag::Compound([("marker".into(), Tag::Int(41))].into()),
            )]
            .into(),
        );
        input.extra = crate::item_data::decode_components(Some(&patch)).1;
        grid[6] = Some(input.clone());
        let mut expected = input;
        expected.item = "test:rest".into();
        expected.count = 1;
        let default_calls = std::cell::Cell::new(0);
        let result =
            recipes.mod_remainders_rect(&context, "test:literal", &grid, 3, 2, 3, &|stack| {
                assert_eq!(stack, grid[6].as_ref().unwrap());
                default_calls.set(default_calls.get() + 1);
                Some(expected.clone())
            });
        assert_eq!(default_calls.get(), 1);
        assert_eq!(*calls.lock().unwrap(), [vec![7, 0, 0, 0, 3, 0]]);
        assert_eq!(result[6], Some(expected));
        let mut custom = ItemStack::new("test:rest", 2);
        custom.damage = 23;
        assert_eq!(result[0], Some(custom));
        assert!(
            result
                .iter()
                .enumerate()
                .all(|(i, s)| i == 0 || i == 6 || s.is_none())
        );
        assert_eq!(
            grid[6].as_ref().unwrap().count,
            3,
            "pre-take input is untouched"
        );
    });
}

#[test]
fn serializer_refusal_skips_known_recipe_and_unknown_refuses_load() {
    with_recipe(0, |_, _| {
        let mut errors = Vec::new();
        assert!(
            load_reported(
                "test:bad.json",
                &serde_json::json!({"type":"test:literal", "rest":99}),
                &mut |e| errors.push(e)
            )
            .is_none()
        );
        assert_eq!(
            errors,
            ["recipe test:bad.json, crafting kind test:literal: rest must be 0, 1, 2 or 3"]
        );
        let error = recipe_type("test:machine", "test:machine").unwrap_err();
        assert_eq!(
            error,
            "recipe data/test/recipe/machine.json, unregistered recipe serializer test:machine"
        );
    });
}

#[test]
fn registered_kind_written_value_syncs_only_to_sunrise_clients() {
    with_recipe(2, |recipes, _| {
        let Recipe::Mod(recipe) = &recipes.crafting[0].1 else {
            panic!("registered kind");
        };
        assert_eq!(recipe.written().unwrap().as_deref(), Some(r#"{"rest":2}"#));
        let sources = vec![(
            "test:literal".into(),
            serde_json::json!({"type":"test:literal", "rest":2}),
        )];
        let mut sunrise = crate::lan::recipes::PlayerRecipeSync::default();
        sunrise.replace(true, vec!["minecraft:crafting".into()], &sources);
        let received: Vec<_> = sunrise
            .payloads
            .iter()
            .flat_map(|part| {
                crate::lan::recipes::ContentPart::decode(part)
                    .unwrap()
                    .recipes
            })
            .collect();
        assert_eq!(received, sources);
        let mut vanilla = crate::lan::recipes::PlayerRecipeSync::default();
        vanilla.replace(false, vec!["minecraft:crafting".into()], &sources);
        assert!(vanilla.payloads.is_empty());
    });
}

struct EmptyKind(mod_api::ItemKey);
impl mod_api::CraftingRecipeKind for EmptyKind {
    type Value = ();
    fn decode(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn write(&self, _: &()) -> Result<String, String> {
        Ok("{}".into())
    }
    fn matches(&self, _context: &dyn mod_api::LoopContext, _: &(), grid: CraftingGrid) -> bool {
        grid.width == 2 && grid.height == 2 && grid.cells.iter().all(Option::is_none)
    }
    fn assemble(
        &self,
        _context: &dyn mod_api::LoopContext,
        _: &(),
        _: CraftingGrid,
    ) -> Option<mod_api::ItemStack> {
        mod_api::ItemStack::new(self.0, 3)
    }
}
struct EmptyMod;
#[mod_api::__subscriptions]
impl mod_api::Hooks for EmptyMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let key = r.item_key("test:output");
        r.register_crafting_recipe_serializer("empty", EmptyKind(key));
    }
}
#[test]
fn registered_kind_receives_the_full_empty_grid_at_every_match_entry() {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("test", Box::new(EmptyMod))].into(),
        || {
            mod_platform::__private::part_tests::install_parts_fixture(&[mod_api::ModEntry::new(
                "test",
                Box::new(EmptyMod),
            )])
            .unwrap();
            mod_api::__private::resolve_items(["test:output"]).unwrap();
            let mut recipes = Recipes::empty();
            recipes.mod_crafting = true;
            recipes.crafting.push((
                "test:empty".into(),
                load("test:empty", &serde_json::json!({"type":"test:empty"})).unwrap(),
            ));
            let grid = vec![None; 4];
            let context = crate::test_resources::server_loop_context();
            let (id, output) = recipes.find(context, &grid, 2).unwrap();
            assert_eq!(id, "test:empty");
            assert_eq!(output, ItemStack::new("test:output", 3));
            assert!(recipes.recipe_matches(context, "test:empty", &grid, 2));
            assert_eq!(
                recipes
                    .find_with_dimensions(context, &grid, 2, 2, 2, &|_| None)
                    .unwrap()
                    .1,
                ItemStack::new("test:output", 3)
            );
            assert_eq!(
                recipes.remainders(context, "test:empty", &grid, 2, &|_| None),
                [None, None, None, None]
            );
        },
    );
}

struct SizedKind {
    input: mod_api::ItemKey,
    output: mod_api::ItemKey,
    rest: mod_api::ItemKey,
}
impl mod_api::CraftingRecipeKind for SizedKind {
    type Value = ();
    fn decode(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn write(&self, _: &()) -> Result<String, String> {
        Ok("{}".into())
    }
    fn matches(&self, _context: &dyn mod_api::LoopContext, _: &(), grid: CraftingGrid) -> bool {
        match (grid.width, grid.height) {
            (0, 0) | (0, u32::MAX) | (u32::MAX, 0) => grid.cells.is_empty(),
            (4, 4) => {
                grid.cells.len() == 16
                    && grid.cells[15]
                        .as_ref()
                        .is_some_and(|s| s.item() == self.input)
            }
            _ => false,
        }
    }
    fn assemble(
        &self,
        _context: &dyn mod_api::LoopContext,
        _: &(),
        grid: CraftingGrid,
    ) -> Option<mod_api::ItemStack> {
        let count = if grid.cells.is_empty() { 1 } else { 4 };
        mod_api::ItemStack::new(self.output, count)
    }
    fn remaining_items(
        &self,
        context: &dyn mod_api::CraftingRemainders,
        _: &(),
        grid: CraftingGrid,
    ) -> Option<Vec<Option<mod_api::ItemStack>>> {
        if grid.cells.is_empty() {
            assert!(context.default_remainder(0).is_none());
            return Some(Vec::new());
        }
        let mut out = vec![None; grid.cells.len()];
        out[15] = mod_api::ItemStack::new(self.rest, 1);
        Some(out)
    }
}
struct SizedMod;
#[mod_api::__subscriptions]
impl mod_api::Hooks for SizedMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let kind = SizedKind {
            input: r.item_key("test:input"),
            output: r.item_key("test:output"),
            rest: r.item_key("test:rest"),
        };
        r.register_crafting_recipe_serializer("sized", kind);
    }
}
#[test]
fn registered_kind_matches_and_returns_remainders_for_empty_and_4x4_grids() {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("test", Box::new(SizedMod))].into(),
        || {
            mod_platform::__private::part_tests::install_parts_fixture(&[mod_api::ModEntry::new(
                "test",
                Box::new(SizedMod),
            )])
            .unwrap();
            mod_api::__private::resolve_items(["test:input", "test:output", "test:rest"]).unwrap();
            let mut recipes = Recipes::empty();
            recipes.mod_crafting = true;
            recipes.crafting.push((
                "test:sized".into(),
                load("test:sized", &serde_json::json!({"type":"test:sized"})).unwrap(),
            ));
            let context = crate::test_resources::server_loop_context();

            let empty_grid: Vec<Option<ItemStack>> = Vec::new();
            for (w, h) in [(0, 0), (0, u32::MAX as usize), (u32::MAX as usize, 0)] {
                let (id, output) = recipes
                    .find_with_dimensions(context, &empty_grid, w, w, h, &|_| None)
                    .unwrap();
                assert_eq!(id, "test:sized");
                assert_eq!(output, ItemStack::new("test:output", 1));
                assert!(
                    recipes
                        .mod_remainders_rect(context, id, &empty_grid, w, w, h, &|_| None)
                        .is_empty()
                );
            }

            let mut large_grid = vec![None; 16];
            large_grid[15] = Some(ItemStack::new("test:input", 1));
            let (id, output) = recipes
                .find_with_dimensions(context, &large_grid, 4, 4, 4, &|_| None)
                .unwrap();
            assert_eq!(id, "test:sized");
            assert_eq!(output, ItemStack::new("test:output", 4));
            let mut expected_remainders = vec![None; 16];
            expected_remainders[15] = Some(ItemStack::new("test:rest", 1));
            assert_eq!(
                recipes.mod_remainders_rect(context, "test:sized", &large_grid, 4, 4, 4, &|_| None),
                expected_remainders
            );
        },
    );
}

#[test]
fn extreme_aspect_ratio_rectangular_grids_avoid_square_padding() {
    let load = crate::test_resources::server_load();
    let context = load.captured_loop_context(engine::LoopSide::Server);
    let root = tempfile::tempdir().unwrap();
    let items = Items::empty();
    let sources = ResolvedSources::resolve(
        vec![(
            "test:planks".into(),
            serde_json::json!({
                "type": "minecraft:crafting_shapeless",
                "ingredients": ["minecraft:oak_log"],
                "result": {"id": "minecraft:oak_planks", "count": 4}
            }),
        )],
        &items,
        &load,
    )
    .unwrap();
    let recipes =
        Recipes::from_resolved(root.path(), &items, &PackOverlay::NONE, &sources).unwrap();
    let log = ItemStack::new("minecraft:oak_log", 1);
    let remainder = ItemStack::new("minecraft:stick", 1);

    for (w, h, slot) in [(1, 50_000, 49_999), (50_000, 1, 49_999)] {
        let mut grid = vec![None; w * h];
        grid[slot] = Some(log.clone());
        let (id, output) = recipes
            .find_with_dimensions(&context, &grid, w, w, h, &|_| None)
            .unwrap();
        assert_eq!(id, "test:planks");
        assert_eq!(output, ItemStack::new("minecraft:oak_planks", 4));
        let remainders = recipes.remainders_rect(&context, id, &grid, w, w, h, &|s| {
            (&*s.item == "minecraft:oak_log").then(|| remainder.clone())
        });
        assert_eq!(remainders.len(), w * h);
        assert_eq!(remainders[slot], Some(remainder.clone()));
        assert_eq!(remainders[0], None);
    }
}
