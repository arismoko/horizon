//! `ItemsHost::hover_name`: `ItemStack.getHoverName()` as a component, on the server too.
use mod_api::prelude::{ItemsHost, TextHost};
use mod_api::{ItemStack, text::TextContent};
use sunrise_game_data::lang::{Language, scoped};
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::api_values::TryIntoApi;

#[test]
#[ignore = "daily"]
fn a_hover_name_is_a_component_each_viewer_renders_in_their_language() {
    with_game(|game| {
        let named: ItemStack = sunrise_items::item_data::parse_stack(
            r#"{id:"minecraft:diamond_pickaxe",count:1,components:{"minecraft:custom_name":"Hammer"}}"#,
        )
        .unwrap()
        .unwrap()
        .try_into_api()
        .unwrap();
        let plain: ItemStack = sunrise_items::items::ItemStack::new("minecraft:stone", 1)
            .try_into_api()
            .unwrap();

        let name = ItemsHost::hover_name(&*game, &named).unwrap();
        assert!(matches!(&name.content, TextContent::Literal(text) if text == "Hammer"));
        let stone = ItemsHost::hover_name(&*game, &plain).unwrap();
        assert!(
            matches!(&stone.content, TextContent::Translatable { key, .. } if key == "block.minecraft.stone"),
            "{:?}",
            stone.content
        );

        // The server's language renders it: the component itself names no language.
        let spanish = Language::of(
            "es_es",
            [("block.minecraft.stone".to_owned(), "Piedra".to_owned())],
        );
        scoped(spanish, || {
            assert_eq!(TextHost::text_string(&*game, &stone), "Piedra");
            assert_eq!(TextHost::text_string(&*game, &name), "Hammer");
        });
    });
}
