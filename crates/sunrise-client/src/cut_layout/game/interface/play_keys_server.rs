#![cfg(test)]
#[cfg(test)]
#[cfg(any())]
pub(crate) mod swap_hands_tests {

    use sunrise_client_gameplay::game::client_side::play_keys::swap_hands_tests::Torch;

    use crate::game::native_entity_refs::tests::with_game;

    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    use sunrise_items::items::ItemStack;

    /// The server's own swap contract: a cancel keeps both hands, and a listener's stack lands in the off hand.
    #[test]
    #[ignore = "daily"]
    fn the_own_player_swap_asks_the_listener_for_each_hand() {
        let (cancel, calls) = (
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicUsize::new(0)),
        );
        let torch = Torch(Arc::clone(&cancel), Arc::clone(&calls));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("torch", Box::new(torch))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let selected = game.selected;
                game.inventory[selected] = Some(ItemStack::new("minecraft:diamond_sword", 1));
                game.offhand = None;
                game.set_mode_id(3);
                game.own_swap_hands();
                assert_eq!(calls.load(Ordering::SeqCst), 0);
                assert_eq!(
                    game.inventory[selected].as_ref().map(|s| &*s.item),
                    Some("minecraft:diamond_sword")
                );
                assert!(game.offhand.is_none());
                game.set_mode_id(0);
                game.own_swap_hands();
                assert_eq!(
                    game.inventory[selected].as_ref().map(|s| &*s.item),
                    Some("minecraft:diamond_sword")
                );
                assert!(game.offhand.is_none());
                cancel.store(false, Ordering::SeqCst);
                game.own_swap_hands();
                assert!(game.inventory[selected].is_none());
                assert_eq!(
                    game.offhand.as_ref().map(|s| &*s.item),
                    Some("minecraft:torch")
                );
            });
        });
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
