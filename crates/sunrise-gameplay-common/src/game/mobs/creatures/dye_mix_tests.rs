use super::*;

/// `DyeColor.getMixedColor` for the offspring a sheep, wolf or cat breed
/// gets: the colour of the crafting recipe that mixes its parents' two
/// dyes (vanilla's `crafting_shapeless` rows: red + yellow makes orange,
/// blue + green cyan, blue + red purple), and a pair no recipe mixes
/// keeps one of the two.
#[test]
fn two_dyes_mix_by_their_crafting_recipe() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/sunrise-client")
        .join(sunrise_core::VANILLA_VERSION);
    if !root.join("data/minecraft/recipe").is_dir() {
        return;
    }
    let registry = sunrise_world::level::data::load_registry(&root).unwrap();
    let facts = sunrise_items::blocks::BlockFacts::load(&root, &registry).unwrap();
    let overlay = sunrise_world::loot::PackOverlay::NONE;
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    let items =
        sunrise_items::items::Items::load(&root, &registry, &facts, &overlay, &defaults).unwrap();
    let recipes = Recipes::load(&root, &items, &overlay, &crate::game::test_server_load()).unwrap();
    let dye = |name: &str| {
        u8::try_from(
            sunrise_level::dyes::DYE_NAMES
                .iter()
                .position(|n| *n == name)
                .unwrap_or_else(|| panic!("{name} is not a dye colour")),
        )
        .unwrap()
    };
    let lc = crate::game::test_server_load()
        .loop_context(sunrise_world::native_load_context::LoopSide::Server, []);
    let mut random = sunrise_core::random::LegacyRandom::new(1);
    for (first, second, mixed) in [
        ("red", "yellow", "orange"),
        ("blue", "green", "cyan"),
        ("blue", "red", "purple"),
        ("green", "white", "lime"),
    ] {
        let (a, b, made) = (dye(first), dye(second), dye(mixed));
        assert_eq!(
            dye_color_mix_in_recipes(&lc, &recipes, a, b),
            Some(made),
            "{first} + {second} makes {mixed}"
        );
        // The recipe answers, so the coin never has to.
        assert_eq!(dye_mix(&lc, &recipes, &mut random, a, b), made);
    }
    // Two dyes that no recipe mixes keep one of the two colours.
    let (orange, cyan) = (dye("orange"), dye("cyan"));
    assert_eq!(dye_color_mix_in_recipes(&lc, &recipes, orange, cyan), None);
    let kept = dye_mix(&lc, &recipes, &mut random, orange, cyan);
    assert!(kept == orange || kept == cyan, "kept {kept}");
}
