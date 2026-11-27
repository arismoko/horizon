use sunrise_gameplay::game::native_entity_refs::tests::with_game;

#[test]
#[ignore = "daily"]
fn biome_tag_forwarder_distinguishes_unknown_tags_from_unknown_biomes() {
    with_game(|game| {
        game.install_live_features();
        let level = game.level(game.dim.id()).unwrap();
        assert_eq!(
            game.level_biome_in_tag(level, "minecraft:plains", "carry:undefined"),
            Some(false)
        );
        assert_eq!(
            game.level_biome_in_tag(level, "carry:undefined", "carry:undefined"),
            None
        );
        assert_eq!(
            game.level_biome_in_tag(level, "minecraft:forest", "minecraft:is_forest"),
            Some(true)
        );
        assert_eq!(
            game.level_biome_in_tag(level, "minecraft:forest", "#minecraft:is_forest"),
            Some(true)
        );
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
