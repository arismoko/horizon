use sunrise_gameplay_common::game::native_hooks::enchantments::EnchantmentsGameExt1;
#[test]
#[ignore = "daily"]
fn enchantment_registry_returns_records_and_unknown_cost() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::native_hooks::enchantment_and_lang_tests::enchantment_registry_returns_records_and_unknown_cost",
        || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let all = game.enchantments();
                let facts = |id| {
                    let e = all.iter().find(|e| e.id == id).unwrap();
                    (e.max_level, e.weight, e.anvil_cost, e.curse)
                };
                // data/minecraft/enchantment/*.json of 26.3.
                assert_eq!(facts("minecraft:sharpness"), (5, 10, 1, false));
                assert_eq!(facts("minecraft:binding_curse"), (1, 1, 8, true));
                assert_eq!(facts("minecraft:mending"), (1, 2, 4, false));
                // `Enchantment.description`: vanilla's are translatable keys.
                let sharpness = all.iter().find(|e| e.id == "minecraft:sharpness").unwrap();
                assert!(matches!(
                    &sharpness.description.content,
                    mod_api::text::TextContent::Translatable { key, .. }
                        if key == "enchantment.minecraft.sharpness"
                ));
                assert!(
                    game.enchantment_min_cost("minecraft:sharpness", 1)
                        .is_some()
                );
                assert_eq!(game.enchantment_min_cost("missing:unknown", 1), None);
            });
        },
    );
}

/// On a server, the reads answer from the registry it synced
/// (`ClientboundRegistryDataPacket`), which holds a data pack's enchantments.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_servers_synced_enchantment_reaches_the_registry_reads() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::native_hooks::enchantment_and_lang_tests::a_servers_synced_enchantment_reaches_the_registry_reads",
        || {
            sunrise_client_gameplay::game::client_side::native_entity_refs::tests::with_client_game(
                |game| {
                    let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                    game.connect_local(client, "SyncedEnchantment").unwrap();
                    let definition = serde_json::json!({
                        "description": "Drill", "supported_items": "minecraft:diamond_pickaxe",
                        "weight": 7, "max_level": 3, "min_cost": {"base": 4, "per_level_above_first": 0},
                        "max_cost": {"base": 9, "per_level_above_first": 0}, "anvil_cost": 6,
                        "slots": ["mainhand"], "effects": {}
                    });
                    let synced = game
                        .stations()
                        .enchantments
                        .with_entries([("fixture:drill", &definition)])
                        .unwrap()
                        .unwrap();
                    game.client.remote.as_mut().unwrap().enchantments =
                        Some(std::sync::Arc::new(synced));

                    let drill = game
                        .enchantments()
                        .into_iter()
                        .find(|e| e.id == "fixture:drill")
                        .expect("the synced enchantment is listed");
                    assert_eq!(
                        (drill.max_level, drill.weight, drill.anvil_cost, drill.curse),
                        (3, 7, 6, false)
                    );
                    assert_eq!(game.enchantment_min_cost("fixture:drill", 1), Some(4));
                    // A data pack's literal description stays literal.
                    assert_eq!(drill.description, mod_api::Text::literal("Drill"));
                },
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn translate_gives_the_text_of_a_defined_key_and_none_otherwise() {
    use sunrise_game_data::lang::{Language, ResourceSource, scoped};
    struct Empty;
    impl ResourceSource for Empty {
        fn read(&self, _: &str) -> Option<Vec<u8>> {
            None
        }
    }
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        let (got, _) = scoped(Language::load(&[&Empty], "en_us"), || {
            (
                game.translate("sunrise.options.controller"),
                game.translate("missing.key"),
            )
        });
        assert_eq!(got, (Some("Controller".to_owned()), None));
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_trait_mod_measures_text_through_its_draw_context() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use mod_api::DrawContext;
    use sunrise_client_gameplay::game::host::draw::GameDrawContext;

    with_client_game(|game| {
        let mut batch = sunrise_client::hud::HudBatch::scratch([320, 240]);
        let draw = GameDrawContext::new(game, &mut batch);
        // A headless client's HUD atlas still measures glyph advances: an upper-case A
        // advances 6 GUI pixels.
        assert_eq!(draw.text_width("AA"), 12.0);
        assert_eq!(draw.text_width(""), 0.0);
    });
}

/// On a server, a mod reads an enchantment's definition through its `Host` (#5392): the server's
/// own registry answers the maximum level, the minimum cost and the supported items.
#[test]
#[ignore = "daily"]
fn a_server_host_reads_enchantment_definitions() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::native_hooks::enchantment_and_lang_tests::a_server_host_reads_enchantment_definitions",
        || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                let host: &dyn mod_api::ServerHost = game;
                let sharpness = host
                    .enchantments()
                    .into_iter()
                    .find(|e| e.id == "minecraft:sharpness")
                    .unwrap();
                // data/minecraft/enchantment/sharpness.json of 26.3: max level 5, and its
                // min_cost {base 1, per_level_above_first 11} gives 12 at level 2.
                assert_eq!(sharpness.max_level, 5);
                assert_eq!(
                    host.enchantment_min_cost("minecraft:sharpness", 2),
                    Some(12)
                );
                assert_eq!(host.enchantment_min_cost("missing:unknown", 1), None);
                let key = |id: &str| {
                    host.item_key(&mod_api::Identifier::parse(id).unwrap())
                        .unwrap()
                };
                assert!(
                    host.enchantment_supports(
                        "minecraft:sharpness",
                        key("minecraft:diamond_sword")
                    )
                );
                assert!(!host.enchantment_supports("minecraft:sharpness", key("minecraft:stick")));
                assert!(
                    !host.enchantment_supports("missing:unknown", key("minecraft:diamond_sword"))
                );
            });
        },
    );
}

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::tables::GameItems as _;
