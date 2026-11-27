#[cfg(test)]
#[cfg(feature = "client")]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::TryIntoApi;

    #[test]
    #[ignore = "daily"]
    fn resource_tooltip_uses_inventory_lines_and_explicit_flags() {
        use mod_api::{
            AddAttributeTooltipsEvent, ClientHost, Hooks, ItemResource, ItemTooltipEvent, ModEntry,
            Mods, TooltipElement,
        };
        use std::sync::atomic::{AtomicBool, Ordering};
        use sunrise_core::nbt::Tag;

        const BASE_NAME: &str = "base item line";
        const REENTERED: &str = "reentered tooltip listener";
        struct Listener(AtomicBool);
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn add_attribute_tooltips(
                &self,
                _: &dyn ClientHost,
                event: &mut AddAttributeTooltipsEvent<'_>,
            ) {
                event
                    .lines
                    .push((mod_api::Text::literal("attribute listener"), u32::MAX));
            }

            fn item_tooltip(&self, host: &dyn ClientHost, event: &mut ItemTooltipEvent<'_>) {
                if self.0.swap(true, Ordering::Relaxed) {
                    event
                        .lines
                        .push(mod_api::Text::literal(REENTERED), u32::MAX)
                        .unwrap();
                    return;
                }
                // Asking for the same stack must return base lines, not enter another listener.
                let nested = ItemResource::of(&event.stack.project().unwrap())
                    .tooltip(host, event.advanced, event.creative)
                    .unwrap();
                self.0.store(false, Ordering::Relaxed);
                assert!(matches!(
                    nested.first(),
                    Some(TooltipElement::Text(text, _)) if *text == mod_api::Text::literal(BASE_NAME)
                ));
                assert!(!nested.contains(&TooltipElement::Text(
                    mod_api::Text::literal(REENTERED),
                    u32::MAX,
                )));
                event
                    .lines
                    .push(mod_api::Text::literal("listener"), u32::MAX)
                    .unwrap();
            }
        }
        let mods: Mods = vec![ModEntry::new(
            "listener",
            Box::new(Listener(AtomicBool::new(false))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
                let name = Tag::compound([("text", Tag::String(BASE_NAME.into()))]);
                let mut stack = sunrise_items::items::ItemStack::new("minecraft:iron_sword", 1);
                stack
                    .data_mut()
                    .set_other("minecraft:item_name", Some(name.clone()));
                let resource = ItemResource::of(
                    &sunrise_gameplay_common::game::api_values::test_snapshot(&stack),
                );
                let inventory: Vec<_> = game
                    .stack_tooltip_lines(&stack, false, false)
                    .into_iter()
                    .map(|(text, color)| TooltipElement::Text(text.try_into_api().unwrap(), color))
                    .collect();
                let advanced = game.options_advanced_tooltips();
                assert!(resource.tooltip(game, advanced, false).unwrap() == inventory);
                let attribute_line =
                    TooltipElement::Text(mod_api::Text::literal("attribute listener"), u32::MAX);
                assert!(inventory.contains(&attribute_line));
                let mut hidden = stack.clone();
                hidden.data_mut().set_other(
                    "minecraft:tooltip_display",
                    Some(Tag::compound([("hide_tooltip", Tag::Byte(1))])),
                );
                let hidden = ItemResource::of(
                    &sunrise_gameplay_common::game::api_values::test_snapshot(&hidden),
                );
                assert!(
                    !hidden
                        .tooltip(game, false, false)
                        .unwrap()
                        .contains(&attribute_line)
                );
                let mut attributes_hidden = stack.clone();
                attributes_hidden.data_mut().set_other(
                    "minecraft:tooltip_display",
                    Some(Tag::compound([(
                        "hidden_components",
                        Tag::List(vec![Tag::String("minecraft:attribute_modifiers".into())]),
                    )])),
                );
                let attributes_hidden = ItemResource::of(
                    &sunrise_gameplay_common::game::api_values::test_snapshot(&attributes_hidden),
                );
                let lines = attributes_hidden.tooltip(game, false, false).unwrap();
                assert!(lines.contains(&attribute_line));
                assert!(lines.contains(&TooltipElement::Text(
                    mod_api::Text::literal("listener"),
                    u32::MAX,
                )));

                let mut crossbow = sunrise_items::items::ItemStack::new("minecraft:crossbow", 1);
                crossbow
                    .data_mut()
                    .set_other("minecraft:item_name", Some(name));
                crossbow
                    .data_mut()
                    .charged
                    .push(sunrise_items::items::ItemStack::new("minecraft:arrow", 1));
                let lines = game.stack_tooltip_lines(&crossbow, false, false);
                assert_eq!(
                    lines
                        .iter()
                        .filter(|(text, _)| text.plain().trim() == "listener")
                        .count(),
                    1,
                    "only the crossbow gets the final tooltip event",
                );

                for advanced in [false, true] {
                    let lines = resource.tooltip(game, advanced, false).unwrap();
                    assert!(lines.last().is_some_and(|line| {
                        *line == TooltipElement::Text(mod_api::Text::literal("listener"), u32::MAX)
                    }));
                    assert_eq!(
                        lines.iter().any(|line| matches!(
                            line,
                            TooltipElement::Text(text, _) if *text == mod_api::Text::literal("minecraft:iron_sword")
                        )),
                        advanced
                    );
                }
            });
        });
    }
    /// A stick's part adds its line where `Item.appendHoverText` does, after the name and before
    /// the advanced id line; `item_tooltip` (`ItemTooltipEvent`) then gets the whole tooltip, with
    /// its flags, and its changes are what shows. A stick whose tooltip is hidden asks neither.
    #[test]
    #[ignore = "daily"]
    fn tooltip_part_lines_come_after_the_name_and_never_for_a_hidden_tooltip() {
        use mod_api::{
            self, ClientHost, Hooks, ItemPart, ItemTooltipEvent, ModEntry, Mods, Registrar,
            TooltipEvent,
        };
        use {
            crate::game::server_side::native_entity_refs::tests::with_client_game,
            sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
        };

        if run_fresh_cases(
            "cut_layout::game::client_side::stations::tests::tooltip_part_lines_come_after_the_name_and_never_for_a_hidden_tooltip",
            "SUNRISE_TOOLTIP_PART_CHILD",
            &["tooltip"],
        ) {
            return;
        }

        #[derive(Clone)]
        struct Lore;
        #[mod_api::sunrise_part]
        impl ItemPart for Lore {
            fn tooltip(&self, _: &dyn ClientHost, event: &mut TooltipEvent<'_, '_>) {
                assert!(event.lines.is_empty());
                event
                    .lines
                    .push((mod_api::Text::literal("part"), u32::MAX))
                    .unwrap();
            }
        }
        struct Mod;
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let item = r.item_key("minecraft:stick");
                r.item_part(item, Lore);
            }
            fn item_tooltip(&self, _: &dyn ClientHost, event: &mut ItemTooltipEvent<'_>) {
                assert_eq!(
                    event
                        .lines
                        .iter()
                        .take(3)
                        .map(|line| line.unwrap().0)
                        .collect::<Vec<_>>(),
                    [
                        mod_api::Text::translate("item.minecraft.stick", Vec::new()),
                        mod_api::Text::literal("part"),
                        mod_api::Text::literal("minecraft:stick"),
                    ]
                );
                assert!(event.advanced);
                // Only the hidden stack is asked from a creative slot, where its
                // display hides every component's lines.
                assert_eq!(event.display.shows("minecraft:lore"), !event.creative);
                // Every line is the mod's to change: drop the count, add one at the end.
                event.lines.pop();
                event
                    .lines
                    .push(mod_api::Text::literal("event"), u32::MAX)
                    .unwrap();
            }
        }
        let mods: Mods = vec![ModEntry::new("lore", Box::new(Mod))].into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                game.client.settings.file.set(
                    "advancedItemTooltips",
                    sunrise_client::options_file::V::Bool(true),
                );
                let stick = sunrise_items::items::ItemStack::new("minecraft:stick", 1);
                let lines: Vec<String> = game
                    .stack_tooltip_lines(&stick, false, false)
                    .iter()
                    .map(|(text, _)| text.plain())
                    .collect();
                assert_eq!(lines, ["Stick", "part", "minecraft:stick", "event"]);
                // `hide_tooltip`: no name, Part line, or tooltip event line.
                let mut hidden = stick.clone();
                hidden.data_mut().set_other(
                    "minecraft:tooltip_display",
                    Some(sunrise_core::nbt::Tag::compound([(
                        "hide_tooltip",
                        sunrise_core::nbt::Tag::Byte(1),
                    )])),
                );
                let lines: Vec<String> = game
                    .stack_tooltip_lines(&hidden, false, false)
                    .iter()
                    .map(|(text, _)| text.plain())
                    .collect();
                assert!(
                    !lines
                        .iter()
                        .any(|l| l == "Stick" || l == "part" || l == "event"),
                    "{lines:?}"
                );
                // The creative screen's inventory and hotbar slots aren't creative
                // tab slots: still hidden there.
                game.set_screen(sunrise_gameplay_common::game::Screen::Creative);
                let inventory: Vec<String> = game
                    .stack_tooltip_lines(&hidden, false, false)
                    .iter()
                    .map(|(text, _)| text.plain())
                    .collect();
                assert!(
                    !inventory
                        .iter()
                        .any(|l| l == "Stick" || l == "part" || l == "event"),
                    "{inventory:?}"
                );
                // A creative tab's own slot (`TooltipFlag.isCreative`) shows it anyway.
                let creative: Vec<String> = game
                    .stack_tooltip_lines(&hidden, false, true)
                    .iter()
                    .map(|(text, _)| text.plain())
                    .collect();
                assert_eq!(creative[..2], ["Stick", "part"]);
                assert_eq!(creative.last().map(String::as_str), Some("event"));
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn item_tooltip_invalid_edit_keeps_other_listeners_lines() {
        use mod_api::{ClientHost, Hooks, ItemTooltipEvent, ModEntry, Mods, Text, TextFormError};
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        use {
            crate::game::server_side::native_entity_refs::tests::with_client_game,
            sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
        };

        if run_fresh_cases(
            "cut_layout::game::client_side::stations::tests::item_tooltip_invalid_edit_keeps_other_listeners_lines",
            "SUNRISE_ITEM_TOOLTIP_INVALID_EDIT_CHILD",
            &["malformed"],
        ) {
            return;
        }

        struct ValidListener;
        #[mod_api::__subscriptions]
        impl Hooks for ValidListener {
            fn item_tooltip(&self, host: &dyn ClientHost, event: &mut ItemTooltipEvent<'_>) {
                assert_eq!(event.stack.item().id(host), Some("minecraft:white_banner"));
                assert_eq!(event.stack.count(), 3);
                assert!(event.display.name_visible(false));
                assert!(!event.display.shows("minecraft:banner_patterns"));
                event
                    .lines
                    .push(Text::literal("listener one"), u32::MAX)
                    .unwrap();
            }
        }

        struct InvalidListener(Arc<AtomicBool>);
        #[mod_api::__subscriptions]
        impl Hooks for InvalidListener {
            fn item_tooltip(&self, _: &dyn ClientHost, event: &mut ItemTooltipEvent<'_>) {
                let mut malformed = Text::literal("invalid");
                malformed.metadata = mod_api::Tag::Int(7);
                assert_eq!(
                    event.lines.push(malformed.clone(), u32::MAX),
                    Err(TextFormError::Malformed)
                );
                assert_eq!(
                    event.lines.set(0, malformed, u32::MAX),
                    Err(TextFormError::Malformed)
                );
                self.0.store(true, Ordering::Relaxed);
            }
        }

        let ran_second = Arc::new(AtomicBool::new(false));
        let mods: Mods = vec![
            ModEntry::new("first", Box::new(ValidListener)),
            ModEntry::new("second", Box::new(InvalidListener(ran_second.clone()))),
        ]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                game.client.settings.file.set(
                    "advancedItemTooltips",
                    sunrise_client::options_file::V::Bool(false),
                );
                let mut banner = sunrise_sim::mobs::raider::ominous_banner();
                banner.count = 3;
                let lines: Vec<String> = game
                    .stack_tooltip_lines(&banner, false, false)
                    .iter()
                    .map(|(text, _)| text.plain())
                    .collect();
                assert!(ran_second.load(Ordering::Relaxed));
                assert_eq!(lines, ["Ominous Banner", "listener one"]);
            });
        });
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "tests/brewing_changed_tests.rs"]
pub(crate) mod brewing_changed_tests;

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "stations/brewing_tick_tests.rs"]
pub(crate) mod brewing_tick_tests;

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "stations/stonecutter_scroll_tests.rs"]
pub(crate) mod stonecutter_scroll_tests;
#[path = "stations/tooltip.rs"]
pub(crate) mod tooltip;

#[path = "stations/anvil.rs"]
pub(crate) mod anvil;
