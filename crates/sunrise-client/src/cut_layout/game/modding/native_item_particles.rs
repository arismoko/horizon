#![cfg(test)]
use sunrise_items::items::ItemStack;
#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_gameplay_common::game::api_values::TryIntoApi;

    #[test]
    #[ignore = "daily"]
    fn host_resolves_stack_particle_sprites() {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            // The game fixture resolves the particle providers, as content_root does.
            let stack = ItemStack::new("minecraft:apple", 1);
            let api_stack = stack.try_into_api().unwrap();
            let host: &dyn mod_api::ClientHost = game;
            let options = host.item_particle_options(&api_stack).unwrap();
            assert_eq!(options.item, Some(vec!["item/apple".to_owned()]));
            assert_eq!(options, game.item_particle(&stack).try_into_api().unwrap());
        });
    }
}
