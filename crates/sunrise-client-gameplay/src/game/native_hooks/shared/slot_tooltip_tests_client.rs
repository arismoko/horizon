use super::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, AtomicUsize, Ordering},
};
use sunrise_gameplay_common::game::native_hooks::slot_tooltip_tests::Listener;
use sunrise_gameplay_common::game::native_hooks::slot_tooltip_tests::MENU;
use sunrise_gameplay_common::game::native_hooks::slot_tooltip_tests::ObservedComponent;

#[test]
#[ignore = "daily"]
fn filled_slot_tooltip_replaces_lines_but_keeps_item_image_and_visibility() {
    for enabled in [false, true] {
        let painted = Arc::new(AtomicUsize::new(0));
        let mode = Arc::new(AtomicU8::new(1));
        let calls = Arc::new(AtomicUsize::new(0));
        let drawn = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "tips",
            Box::new(Listener {
                enabled,
                painted: painted.clone(),
                mode: mode.clone(),
                calls: calls.clone(),
                drawn: drawn.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            let mut factories =
                sunrise_client::tooltip_components::TooltipComponentFactories::default();
            {
                crate::game::client_side::native_hooks_client::register_client_tooltip_component_factories::native_register_client_tooltip_component_factories(
                    &<mod_api::RegistrationContext as mod_api::__private::game::RegistrationContextNew>::new(),
                    &mut factories,
                );
            }
            sunrise_client::tooltip_components::with_test_factories(factories, || {
                mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
                crate::game::native_entity_refs::tests::with_client_game(|game| {
                    let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
                    game.connect_local(pipe, "tips").unwrap();
                    sunrise_level::containers::with_mod_menu_types(
                        [(
                            MENU.into(),
                            "tips".into(),
                            27,
                            0,
                            vec![],
                            vec![],
                            None,
                            None,
                            true,
                            None,
                        )],
                        || {
                            seed_native_menu(game, 1, MENU);
                            let key = (i32::MIN, 0, 0);
                            game.container = Some(key);
                            game.world
                                .block_entities
                                .insert(key, BlockEntity::Chest(vec![None; 27]));
                            assert!(game.open_native_screen(key, None, 27, None));
                            game.set_screen(Screen::Inventory);
                            assert!(game.native_screen_shown());
                            let mut stack = ItemStack::new("minecraft:stone", 1);
                            stack.data_mut().custom_name =
                                Some(sunrise_core::nbt::Tag::compound([
                                    ("text", sunrise_core::nbt::Tag::String("Named stone".into())),
                                    ("color", sunrise_core::nbt::Tag::String("gold".into())),
                                    ("italic", sunrise_core::nbt::Tag::Byte(1)),
                                ]));
                            game.inventory[9] = Some(stack.clone());
                            let mut batch = HudBatch::scratch([512, 512]);
                            let (ox, oy) = game.inventory_origin(&batch);
                            let (x, y) = game.slot_position(9);
                            game.client.cursor = (ox + x + 8.0, oy + y + 8.0);
                            assert_eq!(
                                game.slot_at(&batch, game.client.cursor.0, game.client.cursor.1),
                                Some(9)
                            );
                            game.draw_stack_tooltip(
                                &mut batch,
                                &stack,
                                game.client.cursor.0,
                                game.client.cursor.1,
                            );
                            let original = drawn.lock().unwrap().pop().unwrap();
                            assert!(
                                original
                                    .iter()
                                    .any(|element| matches!(element, ObservedComponent::Custom))
                            );
                            let before_image_draw = painted.load(Ordering::Relaxed);
                            batch.vertices.clear();
                            game.draw_slot_tooltip(&mut batch);
                            assert!(
                                painted.load(Ordering::Relaxed) > before_image_draw,
                                "the retained image is drawn"
                            );
                            assert!(
                                batch
                                    .vertices
                                    .iter()
                                    .any(|v| v.color == rgba(0x12, 0x34, 0x56, 0xFF)),
                                "the stack-aware box style is drawn"
                            );
                            let actual = drawn.lock().unwrap().pop().unwrap();
                            if enabled {
                                assert!(
                                    matches!(&actual[0], ObservedComponent::Text(text, _) if *text == mod_api::Text::literal("replacement"))
                                );
                                let images = |elements: &[ObservedComponent]| {
                                    elements
                                        .iter()
                                        .filter(|element| {
                                            matches!(element, ObservedComponent::Custom)
                                        })
                                        .cloned()
                                        .collect::<Vec<_>>()
                                };
                                assert!(images(&actual) == images(&original));
                                assert_eq!(actual.len(), 2);
                                mode.store(0, Ordering::Relaxed);
                                game.draw_slot_tooltip(&mut batch);
                                assert!(
                                    drawn.lock().unwrap().pop().unwrap() == original,
                                    "passing preserves styled item lines"
                                );
                                mode.store(3, Ordering::Relaxed);
                                game.draw_slot_tooltip(&mut batch);
                                let appended = drawn.lock().unwrap().pop().unwrap();
                                assert!(
                                    appended[..original.len()] == original,
                                    "adding a hint preserves every styled source entry"
                                );
                                assert!(
                                    matches!(appended.last().unwrap(), ObservedComponent::Text(text, _) if *text == mod_api::Text::literal("slot hint"))
                                );
                                mode.store(2, Ordering::Relaxed);
                                game.draw_slot_tooltip(&mut batch);
                                assert!(drawn.lock().unwrap().pop().unwrap() == images(&original));
                            } else {
                                assert!(
                                    actual == original,
                                    "without a slot listener, use the item's tooltip"
                                );
                            }
                            let before = calls.load(Ordering::Relaxed);
                            game.carried = Some(stack);
                            game.draw_slot_tooltip(&mut batch);
                            assert!(drawn.lock().unwrap().is_empty());
                            assert_eq!(calls.load(Ordering::Relaxed), before);
                            game.carried = None;
                            if enabled {
                                mode.store(4, Ordering::Relaxed);
                                game.draw_slot_tooltip(&mut batch);
                                assert!(drawn.lock().unwrap().pop().unwrap() == original);
                                assert_eq!(calls.load(Ordering::Relaxed), before + 1);
                                mode.store(1, Ordering::Relaxed);
                                game.draw_slot_tooltip(&mut batch);
                                assert!(drawn.lock().unwrap().pop().unwrap() == original);
                                assert_eq!(calls.load(Ordering::Relaxed), before + 1);
                            }
                        },
                    );
                });
            });
        });
    }
}
