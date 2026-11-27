#![cfg(test)]

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt1;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt2;
    use sunrise_items::items::ItemStack;

    #[test]
    #[ignore = "daily"]
    fn a_stack_names_its_cooldown_group() {
        with_game(|game| {
            let pearl = ItemStack::new("minecraft:ender_pearl", 1);
            assert_eq!(game.item_cooldown_group(&pearl), "minecraft:ender_pearl");
            let mut wand = ItemStack::new("minecraft:stick", 1);
            wand.data_mut().set_other(
                "minecraft:use_cooldown",
                Some(Tag::compound([
                    ("seconds", Tag::Float(1.0f32.to_bits())),
                    ("cooldown_group", Tag::String("mymod:wands".into())),
                ])),
            );
            assert_eq!(game.item_cooldown_group(&wand), "mymod:wands");
            let unknown = ItemStack::new("mymod:not_an_item", 1);
            assert_eq!(game.item_cooldown_group(&unknown), "mymod:not_an_item");
        });
    }

    /// `player`'s handle from another world epoch, which names no one.
    fn stale(player: mod_api::PlayerRef) -> mod_api::PlayerRef {
        let entity = player.entity();
        mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(
                mod_api::WorldEpoch::new(entity.epoch().get() + 1),
                entity.id(),
            ),
            player.uuid(),
        )
    }

    #[test]
    #[ignore = "daily"]
    fn exp_cooldowns_reach_their_player() {
        super::super::native_guest_turn_tests::with_guests(|game| {
            let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
            assert!(game.player_set_exp_cooldown(a, 5));
            assert_eq!(game.player_exp_cooldown(a), Some(5));
            assert_eq!(game.player_exp_cooldown(b), Some(0));
            // In A's turn, A's own delay is in place and B's is still B's.
            game.as_guest(12, |game| {
                assert_eq!(game.player_exp_cooldown(a), Some(5));
                assert_eq!(game.player_exp_cooldown(b), Some(0));
                assert!(game.player_set_exp_cooldown(a, 9));
                assert!(game.player_set_exp_cooldown(b, 3));
                // A's state is displaced while B's turn runs inside A's.
                game.as_guest(13, |game| {
                    assert_eq!(game.player_exp_cooldown(a), None);
                    assert!(!game.player_set_exp_cooldown(a, 1));
                })
                .unwrap();
            })
            .unwrap();
            assert_eq!(game.player_exp_cooldown(a), Some(9));
            assert_eq!(game.player_exp_cooldown(b), Some(3));

            assert_eq!(game.player_exp_cooldown(stale(a)), None);
            assert!(!game.player_set_exp_cooldown(stale(a), 1));
            assert_eq!(game.player_exp_cooldown(a), Some(9));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_warden_warning_cooldown_is_its_trackers_owners() {
        use sunrise_level::sculk::WardenSpawnTracker;
        let tracker = |warning_level: i32| {
            Some(Tag::compound([(
                "warden_spawn_tracker",
                Tag::compound([
                    ("ticks_since_last_warning", Tag::Int(0)),
                    ("warning_level", Tag::Int(warning_level)),
                    ("cooldown_ticks", Tag::Int(0)),
                ]),
            )]))
        };
        let warns = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
            WardenSpawnTracker::try_warn(&mut [game.server.sculk.tracker], false)
        };
        super::super::native_guest_turn_tests::with_guests(|game| {
            // Guest 12 owns the world: their data is the level's, and 13 has
            // their own saved tracker (as `lan_guest_joined` loads them).
            game.server.lan.as_mut().unwrap().owner =
                Some(sunrise_gameplay::game::server_side::lan::Owner {
                    uuid: 12,
                    saved: None,
                });
            game.level.player_extra = tracker(2);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update_all(|guest| {
                    guest.saved = if guest.uuid == 12 {
                        tracker(2)
                    } else {
                        tracker(4)
                    };
                });
            let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());

            // Anyone else's write is refused and loads nothing, in or out of
            // their turn; so is the owner's while another turn is in place.
            assert!(!game.player_set_warden_warning_cooldown(b, 40));
            game.as_guest(13, |game| {
                assert!(!game.player_set_warden_warning_cooldown(b, 40));
                assert!(!game.player_set_warden_warning_cooldown(a, 40));
            })
            .unwrap();
            game.as_guest(12, |game| {
                game.as_guest(13, |game| {
                    assert!(!game.player_set_warden_warning_cooldown(a, 40));
                })
                .unwrap();
            })
            .unwrap();
            assert!(!game.player_set_warden_warning_cooldown(stale(a), 40));
            assert!(!game.server.sculk.tracker_loaded);
            assert_eq!(game.server.sculk.tracker, WardenSpawnTracker::default());
            game.tick_sculk_entities();
            assert_eq!(game.server.sculk.tracker.warning_level, 2);

            // The owner's write keeps the shrieker from warning until it has
            // run out, over the level's saved tracker.
            game.server.sculk.tracker = WardenSpawnTracker::default();
            game.server.sculk.tracker_loaded = false;
            assert!(game.player_set_warden_warning_cooldown(a, 40));
            for _ in 0..39 {
                game.tick_sculk_entities();
            }
            assert_eq!(warns(game), None);
            game.tick_sculk_entities();
            assert_eq!(warns(game), Some(3));
            // Also from the owner's own turn.
            game.as_guest(12, |game| {
                assert!(game.player_set_warden_warning_cooldown(a, 5));
            })
            .unwrap();
            assert_eq!(game.server.sculk.tracker.cooldown_ticks, 5);
        });
    }
}
