//! `RecipesClientHost::panel_recipes`: a native mod's page shows in the panel, cut as the
//! renderer cuts one, and `None` brings its items back.
use crate::game::native_hooks::NativeRecipeLayout;
use crate::game::native_panel::{Owner, Panel};
use crate::game::native_panel_recipes::RecipePage;
use mod_api::{FluidStack, RecipeLayout, RecipeResult, RecipeView, Stack};
use sunrise_gameplay_common::game::native_hooks::panel_recipes::page_views;

fn keys() {
    mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
    mod_api::__private::resolve_items([
        "minecraft:oak_log",
        "minecraft:oak_planks",
        "minecraft:stone",
        "minecraft:dirt",
        "minecraft:crafting_table",
        "minecraft:furnace",
    ])
    .unwrap();
    mod_api::__private::resolve_fluids(
        &["minecraft:water", "minecraft:lava"],
        &sunrise_world::fluid::FluidKind::ALL.map(|kind| kind.name()),
    )
    .unwrap();
}
fn stack(item: &str) -> Stack {
    Stack {
        item: mod_api::__private::item_key(item).unwrap(),
        count: 1,
        components: String::new(),
    }
}
fn fluid(id: &str, amount: u32) -> FluidStack {
    FluidStack::new(mod_api::__private::fluid_key(id).unwrap(), amount)
}

/// What `Game::panel_recipes` does with a page: `native_panel_recipes`' `RecipePage::new`, then
/// `Panel::show_recipes`.
fn show(panel: &mut Panel, mod_id: &str, page: Option<mod_api::RecipePage>) {
    let (title, tabs, tab, views) = page_views(page);
    panel.show_recipes(mod_id, RecipePage::new(&title, tabs, tab, views));
}

#[test]
fn a_native_mods_recipe_page_shows_in_the_panel_and_none_restores_its_items() {
    keys();
    let mut panel = Panel::default();
    let items = vec![stack("minecraft:stone"), stack("minecraft:dirt")];
    panel.show_items("viewer", items.clone());

    // Past every limit: a 5 x 1 grid, 30 slots of 70 stacks, 6 results, 10 fluids in.
    let crafting = RecipeView::grid(5, 1)
        .with_slots(vec![vec![stack("minecraft:oak_log"); 70]; 30])
        .with_results(
            (0..6)
                .map(|i| {
                    let result = RecipeResult::new(vec![stack("minecraft:oak_planks")]);
                    if i == 1 {
                        result.with_chance(250.0)
                    } else {
                        result
                    }
                })
                .collect(),
        )
        .with_fluids(
            vec![fluid("minecraft:water", 1000); 10],
            vec![fluid("minecraft:lava", 250)],
        )
        .with_action(true);
    let smelting = RecipeView::new(RecipeLayout::Cooking)
        .with_slots(vec![vec![stack("minecraft:oak_log")]])
        .with_results(vec![RecipeResult::new(vec![stack("minecraft:stone")])])
        .with_note("10 s");
    let tabs = vec![stack("minecraft:crafting_table"); 20];
    let mut recipes = vec![crafting, smelting];
    recipes.extend(std::iter::repeat_n(RecipeView::grid(1, 1), 300));
    let page = mod_api::RecipePage::new("Oak Planks", recipes).with_tabs(tabs, 1);
    show(&mut panel, "viewer", Some(page));

    let shown = panel.shown(|_| true).expect("a panel");
    assert_eq!(shown.owner, Owner::Native("viewer".into()));
    assert_eq!(shown.items, items, "the items stay for when the page goes");
    let page = shown.recipes.as_ref().expect("the mod's recipe page");
    assert_eq!((page.title.as_str(), page.tab), ("Oak Planks", 1));
    assert_eq!((page.tabs.len(), page.recipes.len()), (16, 256));
    let r = &page.recipes[0];
    assert_eq!(r.layout, NativeRecipeLayout::Grid);
    assert_eq!((r.width, r.height, r.slots.len()), (3, 1, 27));
    assert!(r.slots.iter().all(|slot| slot.len() == 64));
    let results = r.results.iter().map(|x| (x.stacks.len(), x.chance));
    assert_eq!(
        results.collect::<Vec<_>>(),
        [(1, None), (1, Some(100.0)), (1, None), (1, None)]
    );
    assert_eq!((r.fluid_inputs.len(), r.fluid_outputs.len()), (8, 1));
    assert_eq!(r.action, Some(true));
    let cooking = &page.recipes[1];
    assert_eq!(cooking.layout, NativeRecipeLayout::Cooking);
    assert_eq!((cooking.note.as_str(), cooking.action), ("10 s", None));

    // `None` shows the items again, still the mod's panel.
    show(&mut panel, "viewer", None);
    let shown = panel.shown(|_| true).expect("a panel");
    assert_eq!(shown.owner, Owner::Native("viewer".into()));
    assert_eq!((&shown.items, shown.recipes.is_none()), (&items, true));

    // A page with no recipes does the same.
    show(&mut panel, "viewer", Some(page_of_one()));
    assert!(panel.shown(|_| true).unwrap().recipes.is_some());
    show(
        &mut panel,
        "viewer",
        Some(mod_api::RecipePage::new("", Vec::new())),
    );
    assert!(panel.shown(|_| true).unwrap().recipes.is_none());
}

fn page_of_one() -> mod_api::RecipePage {
    mod_api::RecipePage::new("Stone", vec![RecipeView::grid(1, 1)])
}
