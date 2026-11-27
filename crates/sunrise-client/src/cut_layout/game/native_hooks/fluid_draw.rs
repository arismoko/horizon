#![cfg(test)]
#[cfg(feature = "client")]
use sunrise_client::hud::rgba;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::native_hooks::{FluidStack, Text, mod_api};

#[cfg(all(test, feature = "client"))]
mod tests {
    use super::*;
    use mod_api::{ClientHost, FluidTooltipEvent, Hooks, ModEntry, Mods};

    struct Relabel;
    #[mod_api::__subscriptions]
    impl Hooks for Relabel {
        fn fluid_tooltip(&self, _: &dyn ClientHost, event: &mut FluidTooltipEvent<'_>) {
            event.lines.remove(0);
            event
                .lines
                .push((mod_api::Text::literal("Tank 3"), 0xff55ffff));
        }
    }

    /// A mod rewrites a fluid's tooltip, with no item and no item tooltip hook: the lines the
    /// game draws are its own, a mod's edit applied.
    #[test]
    #[ignore = "daily"]
    fn a_mod_edits_a_fluid_tooltip() {
        // Load the real registries on the client side before resolving the fluid.
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            let water = mod_api::__private::fluid_key("minecraft:water").unwrap();
            let stack = FluidStack::new(water, 750);
            let plain = |lines: Vec<(Text, u32)>| -> Vec<(String, u32)> {
                lines.into_iter().map(|(t, c)| (t.plain(), c)).collect()
            };
            assert_eq!(
                plain(game.fluid_tooltip_lines(&stack)),
                [
                    ("Water".to_owned(), u32::MAX),
                    ("750 mB".to_owned(), rgba(170, 170, 170, 255))
                ]
            );
            let mods: Mods = vec![ModEntry::new("fluid-tips", Box::new(Relabel))].into();
            mod_platform::testing::with_mods(mods, || {
                assert_eq!(
                    plain(game.fluid_tooltip_lines(&stack)),
                    [
                        ("750 mB".to_owned(), rgba(170, 170, 170, 255)),
                        ("Tank 3".to_owned(), 0xff55ffff)
                    ]
                );
            });
        });
    }
}
