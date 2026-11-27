#![cfg(test)]
use mod_api::TooltipElement;
#[cfg(feature = "client")]
use sunrise_client::hud::HudBatch;
use sunrise_items::items::ItemStack;
#[cfg(test)]
#[cfg(feature = "client")]
pub(crate) mod tests {

    use super::*;
    use sunrise_client_gameplay::game::native_hooks::TryIntoEngine;
    use sunrise_gameplay_common::game::TryIntoApi;

    #[test]
    #[ignore = "daily"]
    fn rendered_image_callback_receives_whole_box() {
        use mod_api::{
            ClientHost, ClientTooltipComponent, ClientTooltipComponentFactoriesRegistrar,
            DrawContext, Hooks, ItemPart, ModEntry, Mods, Registrar,
            RenderTooltipGatherComponentsEvent, TooltipComponent, TooltipComponentData,
            TooltipFont, TooltipImageEvent,
        };
        use std::sync::atomic::{AtomicBool, Ordering};
        use sunrise_client::tooltip_components::{TooltipComponentFactories, with_test_factories};

        static TEXT_DRAWN: AtomicBool = AtomicBool::new(false);
        static IMAGE_DRAWN: AtomicBool = AtomicBool::new(false);

        struct BoxData;
        impl TooltipComponentData for BoxData {}

        struct BoxClient;
        impl ClientTooltipComponent for BoxClient {
            fn height(&self, _: &dyn TooltipFont) -> i32 {
                16
            }
            fn width(&self, _: &dyn TooltipFont) -> i32 {
                70
            }
            fn draw_text(&self, _: &dyn TooltipFont, x: i32, y: i32, _: &mut dyn DrawContext) {
                assert_eq!((x, y), (22, 20));
                assert!(!IMAGE_DRAWN.load(Ordering::Relaxed));
                TEXT_DRAWN.store(true, Ordering::Relaxed);
            }
            fn draw_image(
                &self,
                _: &dyn TooltipFont,
                x: i32,
                y: i32,
                width: i32,
                height: i32,
                _: &mut dyn DrawContext,
            ) {
                assert!(TEXT_DRAWN.load(Ordering::Relaxed));
                assert_eq!((x, y), (22, 20));
                assert_eq!((width, height), (70, 46));
                IMAGE_DRAWN.store(true, Ordering::Relaxed);
            }
        }

        #[derive(Clone)]
        struct Image;
        #[mod_api::sunrise_part]
        impl ItemPart for Image {
            fn tooltip_image(
                &self,
                _: &dyn ClientHost,
                _: &mut TooltipImageEvent<'_>,
            ) -> Option<TooltipComponent> {
                Some(TooltipComponent::custom(BoxData))
            }
        }

        struct Mod;
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let key = r.item_key("minecraft:stick");
                r.item_part(key, Image);
            }
            fn render_tooltip_gather_components(
                &self,
                _: &dyn ClientHost,
                event: &mut RenderTooltipGatherComponentsEvent<'_>,
            ) -> mod_api::Flow {
                assert!(matches!(event.elements[1], TooltipElement::Component(_)));
                let component = event.elements[1].clone();
                *event.elements = vec![
                    TooltipElement::Text(mod_api::Text::literal("A"), u32::MAX),
                    component,
                    TooltipElement::Text(mod_api::Text::literal("B"), u32::MAX),
                    TooltipElement::Text(mod_api::Text::literal("C"), u32::MAX),
                ];
                mod_api::Flow::Continue
            }
        }

        let mut factories = TooltipComponentFactories::default();
        let registrar: &mut dyn ClientTooltipComponentFactoriesRegistrar = &mut factories;
        registrar.register::<BoxData, BoxClient>(|_| BoxClient);

        let mods: Mods = vec![ModEntry::new("image", Box::new(Mod))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
            with_test_factories(factories, || {
                crate::game::native_entity_refs::tests::with_client_game(|game| {
                    let draw = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                                stack: &ItemStack| {
                        TEXT_DRAWN.store(false, Ordering::Relaxed);
                        IMAGE_DRAWN.store(false, Ordering::Relaxed);
                        game.draw_stack_tooltip(
                            &mut HudBatch::scratch([320, 240]),
                            stack,
                            10.0,
                            20.0,
                        );
                        assert!(TEXT_DRAWN.load(Ordering::Relaxed));
                        assert!(IMAGE_DRAWN.load(Ordering::Relaxed));
                    };
                    draw(game, &ItemStack::new("minecraft:stick", 1));
                    // The component also draws for view-only stacks that cannot enter storage.
                    let mut stack = ItemStack::new("minecraft:stick", 65)
                        .try_into_api()
                        .unwrap();
                    for remove_damage in [false, true] {
                        stack
                            .set_components(
                                [(
                                    sunrise_core::nbt::JavaString::from(if remove_damage {
                                        "!minecraft:max_damage"
                                    } else {
                                        "minecraft:max_damage"
                                    }),
                                    if remove_damage {
                                        sunrise_core::nbt::Tag::Compound(Default::default())
                                    } else {
                                        sunrise_core::nbt::Tag::Int(250)
                                    },
                                )]
                                .into(),
                            )
                            .unwrap();
                        stack.set_damage(251);
                        assert!(
                            stack
                                .try_into_engine(
                                    sunrise_gameplay_common::game::tables::GameItems::items(game)
                                )
                                .is_err()
                        );
                        draw(
                            game,
                            &sunrise_gameplay_common::game::api_values::stack_value(&stack)
                                .unwrap(),
                        );
                    }
                });
            });
        });
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "tooltip_bundle_tests.rs"]
mod bundle_tests;

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "tooltip_component_tests.rs"]
mod component_tests;
