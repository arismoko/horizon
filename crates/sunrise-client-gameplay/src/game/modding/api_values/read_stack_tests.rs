use sunrise_gameplay_common::game::api_values::{ApiStack, JavaString, Tag, stack_value};
use sunrise_gameplay_common::game::api_values::{TryIntoApi, TryIntoEngine};

#[test]
fn read_encoding_keeps_nonresident_stack_limits() {
    let items = sunrise_gameplay_common::game::api_values::stack_limit_tests::catalog();
    let key = mod_api::__private::item_key("minecraft:iron_sword").unwrap();
    let mut stackable = ApiStack::of(key);
    stackable
        .set_components([(JavaString::from("minecraft:max_stack_size"), Tag::Int(2))].into())
        .unwrap();
    let mut oversized_damage = ApiStack::of(key);
    oversized_damage.set_damage(u32::MAX);
    let mut excess = ApiStack::of(mod_api::__private::item_key("minecraft:stick").unwrap());
    excess.set_count(65).unwrap();
    for api in [stackable, oversized_damage, excess] {
        assert!(api.try_into_engine(&items).is_err(), "write remains strict");
        let encoded = stack_value(&api).unwrap();
        assert_eq!(encoded.count, api.count());
        assert_eq!(encoded.damage, api.damage());
        assert_eq!(encoded.try_into_api().unwrap(), api);
    }
}

#[test]
#[ignore = "daily"]
fn draw_consumers_render_nonresident_stacks() {
    use crate::game::host::draw::{GameBlockEntityDrawContext, GameDrawContext};
    use mod_api::{BlockEntityDrawContext, DrawContext, ItemDisplayContext};
    use sunrise_client_render::hud::HudBatch;
    use sunrise_gameplay_common::game::tables::GameItems as _;
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        let mut excess = ApiStack::of(mod_api::__private::item_key("minecraft:stick").unwrap());
        excess.set_count(65).unwrap();
        let mut stackable =
            ApiStack::of(mod_api::__private::item_key("minecraft:iron_sword").unwrap());
        stackable
            .set_components([(JavaString::from("minecraft:max_stack_size"), Tag::Int(2))].into())
            .unwrap();
        let mut oversized_count =
            ApiStack::of(mod_api::__private::item_key("minecraft:iron_sword").unwrap());
        oversized_count.set_count(2).unwrap();
        for api in [excess, stackable, oversized_count] {
            assert!(
                api.try_into_engine(game.items()).is_err(),
                "not a resident write"
            );
            let mut gui = HudBatch::scratch([320, 240]);
            {
                let mut context = GameDrawContext::new(game, &mut gui);
                assert!(context.item_stack(&api, [16.0, 32.0], 16.0));
            }
            assert!(!gui.vertices.is_empty(), "item_stack renders geometry");
            let mut decorated = HudBatch::scratch([320, 240]);
            GameDrawContext::new(game, &mut decorated).gui_draw_item_with(&api, [16.0, 32.0], None);
            assert!(
                !decorated.vertices.is_empty(),
                "gui_draw_item_with renders geometry"
            );
            let mut world = crate::game::native_block_entity_draw::BlockEntityDraw::posed(
                0.0,
                0,
                0,
                [0, 64, 0],
                glam::Mat4::IDENTITY,
                (glam::Vec3::X, glam::Vec3::Y),
                Vec::new(),
            );
            assert!(GameBlockEntityDrawContext::new(game, &mut world).item(
                &api,
                ItemDisplayContext::Fixed,
                None,
                0
            ));
            assert!(
                !world.into_vertices().is_empty(),
                "block item renders geometry"
            );
        }
    });
}
