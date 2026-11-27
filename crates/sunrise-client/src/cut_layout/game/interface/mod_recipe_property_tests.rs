//! A menu part reads the same authoritative generation as Host queries.
#![cfg(test)]

#[cfg(test)]
mod tests {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use mod_api::{Hooks, MenuPart, MenuSlotRulesEvent, ModEntry, Mods, Registrar};
    use sunrise_core::vanilla_bytes::Writer;
    use sunrise_gameplay_common::game::tables::WorldTablesExt;
    use sunrise_items::items::ItemStack;
    use sunrise_protocol::{
        buf::{Codec, Reader},
        connection::Pipe,
        packets::recipes::UpdateRecipes,
    };
    struct Input;
    #[mod_api::sunrise_part]
    impl MenuPart for Input {
        fn menu_slot_rules(&self, host: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
            if event.slot == 0 {
                event.may_place &=
                    host.mod_recipe_property_contains("minecraft:custom_input", event.stack.item());
            }
        }
    }
    struct Pot;
    #[mod_api::__subscriptions]
    impl Hooks for Pot {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            r.menu_part("pot:menu", Input);
        }
    }
    #[test]
    #[ignore = "daily"]
    fn host_and_slot_prediction_use_replacement_property_generation() {
        let mods: Mods = vec![ModEntry::new("pot", Box::new(Pot))].into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        crate::entry::install_client_hooks();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "PropertyPrediction").unwrap();
                let item = mod_api::__private::item_key("minecraft:emerald").unwrap();
                let id = game
                    .client
                    .remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:item")
                    .unwrap()
                    .id("minecraft:emerald")
                    .unwrap();
                let packet = UpdateRecipes {
                    properties: vec![
                        ("minecraft:custom_input".into(), vec![id]),
                        ("minecraft:furnace_input".into(), vec![id]),
                    ],
                    stonecutter: vec![],
                };
                let mut wire = Writer::new();
                UpdateRecipes::write(&packet, &mut wire);
                game.recipe_tables
                    .server_properties
                    .replace(UpdateRecipes::read(&mut Reader::new(&wire.data)).unwrap());
                game.refresh_recipe_property_context();
                assert!(game.recipe_tables.server_properties.sets.as_ref().unwrap()
                    ["minecraft:furnace_input"].contains(&id));
                assert!(
                    !sunrise_gameplay_common::game::tables::WorldTablesExt::mod_recipe_property_contains(
                        game,
                        "minecraft:furnace_input",
                        item
                    )
                );
                assert!(
                    mod_api::prelude::RecipePropertiesHost::mod_recipe_property_contains(
                        game,
                        "minecraft:custom_input",
                        item
                    )
                );
                let stack = ItemStack::new("minecraft:emerald", 1);
                let accepts = |game: &sunrise_client_gameplay::game::ClientGame<'_>,
                               stack: &ItemStack| {
                    sunrise_sim::menu::apply_menu_slot_rules(
                        Some(game as &dyn sunrise_sim::menu::SlotRules),
                        "pot:menu",
                        0,
                        stack,
                        sunrise_sim::menu::SlotContext::default(),
                        (true, true, 64),
                        64,
                    )
                    .0
                };
                assert!(accepts(game, &stack));
                assert!(!accepts(game, &ItemStack::new("minecraft:diamond", 1)));
                game.recipe_tables
                    .server_properties
                    .replace(UpdateRecipes::default());
                game.refresh_recipe_property_context();
                assert!(!accepts(game, &stack));
                assert!(
                    !mod_api::prelude::RecipePropertiesHost::mod_recipe_property_contains(
                        game,
                        "minecraft:custom_input",
                        item
                    )
                );
            })
        });
    }
}
