#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {
    use crate::game::server_side::disabled_block_tests::tests::DISPENSER;
    use crate::game::server_side::disabled_block_tests::tests::DISPENSER_SLOT;
    use crate::game::server_side::disabled_block_tests::tests::place_dispenser;
    use crate::game::server_side::disabled_block_tests::tests::with_inert_block;
    use std::sync::atomic::Ordering;
    use sunrise_items::items::ItemStack;

    #[test]
    #[ignore = "daily"]
    fn inert_dispense_keeps_a_legacy_over_max_stack_without_events() {
        with_inert_block(
            "cut_layout::game::client_side::disabled_block_tests_client::tests::inert_dispense_keeps_a_legacy_over_max_stack_without_events",
            |game, _, probe| {
                assert!(mod_api::__private::set_mod_enabled("enabled_prefix", false));
                let dispenser = place_dispenser(game);
                game.world
                    .block_entities
                    .get_mut(&DISPENSER_SLOT)
                    .unwrap()
                    .slots_mut()[0] = Some(ItemStack::new("minecraft:stick", 65));
                game.start_audio_capture();
                let particles = game.server.pending_particles.len();
                let notices = probe.dispenser_changes.load(Ordering::Relaxed);
                for (enabled, count, writes) in [(false, 65, 0), (true, 64, 1)] {
                    assert!(mod_api::__private::set_mod_enabled("inert_test", enabled));
                    game.dispense_from(DISPENSER, dispenser).unwrap();
                    assert_eq!(
                        game.world
                            .block_entities
                            .get(&DISPENSER_SLOT)
                            .unwrap()
                            .slots()[0],
                        Some(ItemStack::new("minecraft:stick", count))
                    );
                    assert_eq!(
                        probe.dispenser_changes.load(Ordering::Relaxed),
                        notices + writes
                    );
                    assert!(game.dropped.is_empty());
                    assert!(game.server.pending_sounds.is_empty());
                    assert_eq!(game.server.pending_particles.len(), particles);
                }
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn rejected_dispense_reply_preserves_the_disabled_owners_fallback() {
        with_inert_block(
            "cut_layout::game::client_side::disabled_block_tests_client::tests::rejected_dispense_reply_preserves_the_disabled_owners_fallback",
            |game, _, probe| {
                let dispenser = place_dispenser(game);
                game.start_audio_capture();
                let particles = game.server.pending_particles.len();
                for enabled in [false, true] {
                    assert!(mod_api::__private::set_mod_enabled("inert_test", enabled));
                    for (input, remaining, writes) in [(65, 65, 0), (4, 2, 1)] {
                        let notices = probe.dispenser_changes.load(Ordering::Relaxed);
                        game.world
                            .block_entities
                            .get_mut(&DISPENSER_SLOT)
                            .unwrap()
                            .slots_mut()[0] = Some(ItemStack::new("minecraft:stick", input));
                        game.dispense_from(DISPENSER, dispenser).unwrap();
                        assert_eq!(
                            game.world
                                .block_entities
                                .get(&DISPENSER_SLOT)
                                .unwrap()
                                .slots()[0],
                            Some(ItemStack::new("minecraft:stick", remaining))
                        );
                        assert_eq!(
                            probe.dispenser_changes.load(Ordering::Relaxed),
                            notices + writes
                        );
                        assert!(
                            game.dropped.is_empty(),
                            "vanilla must not run a second behaviour"
                        );
                        assert!(game.server.pending_sounds.is_empty());
                        assert_eq!(game.server.pending_particles.len(), particles);
                    }
                }
                assert_eq!(probe.earlier_dispenses.load(Ordering::Relaxed), 4);
            },
        );
    }
}
