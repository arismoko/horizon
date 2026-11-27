use sunrise_gameplay_common::game::remote::predict_clicks;

/// Q over a slot of the inventory on a server (#3357): the predicted THROW
/// drops the stack, so the player's hand swings, as `LivingEntity.drop`
/// does for `Prediction.PREDICTED`; a plain pick up drops nothing.
#[test]
#[ignore = "daily"]
fn a_predicted_throw_from_a_slot_swings_the_hand() {
    use sunrise_items::{
        enchant::Enchantments,
        items::{ItemStack, Items},
    };
    use sunrise_level::brewing::Brewing;
    use sunrise_sim::item_names::Names;
    use sunrise_sim::{
        banner::Banners,
        crafting::Recipes,
        menu::{Env, Input, Kind, Menu},
        stations::StationRecipes,
    };
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
    );
    let none = &sunrise_world::loot::PackOverlay::NONE;
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    let items = Items::catalog(&root, none, &defaults).unwrap();
    let recipes = Recipes::load(
        &root,
        &items,
        none,
        &sunrise_gameplay_common::game::test_server_load(),
    )
    .unwrap();
    let brewing = Brewing::load(&root, &items, none);
    let banners = Banners::load(&root, &defaults);
    let station_recipes = StationRecipes::load(
        &root,
        &items,
        none,
        &sunrise_gameplay_common::game::test_server_load(),
    )
    .unwrap();
    let enchantments = Enchantments::default();
    let names = Names::default();
    let env = Env {
        crafting_remainder: None,
        player: None,
        player_entity_id: None,
        loop_context: &sunrise_gameplay_common::game::test_server_load()
            .loop_context(sunrise_world::native_load_context::LoopSide::Client, []),
        items: &items,
        recipes: &recipes,
        brewing: &brewing,
        banners: &banners,
        station_recipes: &station_recipes,
        enchantments: &enchantments,
        names: &names,
        trade: None,
        maps: None,
        limited_crafting: false,
        can_equip: None,
        known_recipes: None,
        slot_rules: None,
        stacked: None,
    };
    let mut menu = Menu::new(Kind::Inventory);
    let cobblestone = ItemStack {
        item: "minecraft:cobblestone".into(),
        count: 16,
        damage: 0,
        extra: None,
    };
    // The first hotbar slot of InventoryMenu.
    menu.set_slot(&env, 36, Some(cobblestone));
    let (sent, threw) = predict_clicks(&mut menu, &env, &[(36, 0, Input::Throw)]).unwrap();
    assert!(threw, "Q over a slot drops one and swings");
    assert_eq!(menu.slot(36).map(|s| s.count), Some(15));
    assert_eq!(sent.len(), 1);
    let (_, threw) = predict_clicks(&mut menu, &env, &[(36, 0, Input::Pickup)]).unwrap();
    assert!(!threw, "picking the stack up drops nothing");
    let (_, threw) = predict_clicks(&mut menu, &env, &[(-999, 1, Input::Pickup)]).unwrap();
    assert!(threw, "one of the carried stack outside the window");
}
