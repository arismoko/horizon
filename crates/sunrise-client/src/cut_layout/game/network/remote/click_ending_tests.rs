use sunrise_gameplay_common::game::remote::use_ends_click;

/// `FireworkRocketItem.use` is SUCCESS while gliding: the main hand's
/// rocket ends the click and the off hand's shield is not raised (#3424).
/// On foot it passes, so the off hand still gets its turn.
#[test]
#[ignore = "daily"]
fn a_rocket_used_while_gliding_ends_the_click() {
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
    );
    let registry = sunrise_world::level::data::load_registry(&root).unwrap();
    let facts = sunrise_items::blocks::BlockFacts::load(&root, &registry).unwrap();
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    let items = sunrise_items::items::Items::load(
        &root,
        &registry,
        &facts,
        &sunrise_world::loot::PackOverlay::NONE,
        &defaults,
    )
    .unwrap();
    let rocket = items.get("minecraft:firework_rocket").unwrap();
    assert!(use_ends_click(rocket, true), "gliding");
    assert!(!use_ends_click(rocket, false), "on foot");
    // Other items answer the same whether or not the player glides.
    let shield = items.get("minecraft:shield").unwrap();
    let stick = items.get("minecraft:stick").unwrap();
    for gliding in [false, true] {
        assert!(use_ends_click(shield, gliding));
        assert!(!use_ends_click(stick, gliding));
        // FoodOnAStickItem.use passes on a client: no swing while the boost cools down.
        for rod in [
            "minecraft:carrot_on_a_stick",
            "minecraft:warped_fungus_on_a_stick",
        ] {
            let rod = items.get(rod).unwrap();
            assert!(!use_ends_click(rod, gliding), "{}", rod.id);
        }
        // A carved pumpkin has an equip_slot (head) but is not swappable, so it has no Item.use.
        let pumpkin = items.get("minecraft:carved_pumpkin").unwrap();
        assert!(!use_ends_click(pumpkin, gliding));
    }
}
