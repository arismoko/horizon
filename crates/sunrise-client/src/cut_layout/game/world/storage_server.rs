#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {
    /// `ChestBlockEntity`'s opener counter plays the block's own hinge
    /// sound: a copper chest's, by weather state, not the wooden chest's.
    #[test]
    #[ignore = "daily"]
    fn a_player_opens_and_closes_a_chest_with_its_own_hinge_sound() {
        let mut resources = None;
        let mut game = super::super::game_tests::tests::server_game(&mut resources);
        game.server.audio_capture_active = true;
        for (block, sound) in [
            ("minecraft:chest", "minecraft:block.chest"),
            (
                "minecraft:exposed_copper_chest",
                "minecraft:block.copper_chest",
            ),
            (
                "minecraft:waxed_oxidized_copper_chest",
                "minecraft:block.copper_chest_oxidized",
            ),
        ] {
            let state = game.pack.assets.registry.default_state(block).unwrap();
            game.world.set(8, 64, 8, state).unwrap();
            game.server.pending_sounds.clear();
            game.container_edge_sound((8, 64, 8), true);
            game.container_edge_sound((8, 64, 8), false);
            let mut heard: Vec<_> = game
                .server
                .pending_sounds
                .iter()
                .map(|s| s.event.clone())
                .collect();
            heard.sort();
            assert_eq!(
                heard,
                [format!("{sound}.close"), format!("{sound}.open")],
                "{block}"
            );
        }
    }
}

#[cfg(test)]
pub(crate) mod reach_tests {
    use super::super::game_tests::tests::server_game;
    use sunrise_core::geometry::Vec3;
    use sunrise_level::containers::BlockEntity;
    use sunrise_sim::motion::PlayerMotion;

    /// `Container.stillValidBlockEntity` for a double chest (`CompoundContainer`
    /// asks both halves): breaking the half the player did not click ends the
    /// six-row menu instead of leaving a three-row one.
    #[test]
    #[ignore = "daily"]
    fn breaking_the_other_half_closes_the_open_double_chest() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.player = Some(
            PlayerMotion::new(game.motion, Vec3::new(8.5, 64.0, 8.5)).expect("a player position"),
        );
        let registry = game.pack.assets.registry;
        let half = |kind: &str| {
            let mut state = registry
                .state(registry.default_state("minecraft:chest").unwrap())
                .unwrap()
                .clone();
            state.properties.insert("type".into(), kind.into());
            state.properties.insert("facing".into(), "north".into());
            registry.id(&state).unwrap()
        };
        let left = (8, 64, 8);
        let right = (9, 64, 8);
        game.world
            .set(left.0, left.1, left.2, half("left"))
            .unwrap();
        game.world
            .set(right.0, right.1, right.2, half("right"))
            .unwrap();
        for key in [left, right] {
            game.world
                .block_entities
                .insert(key, BlockEntity::Chest(vec![None; 27]));
        }
        game.open_block_effect_menu([8, 64, 8]);
        assert_eq!(game.container, Some(left), "the double chest opened");

        game.destroy_block_dropping([9, 64, 8])
            .expect("the other half breaks");
        game.tick_container_reach();
        assert_eq!(
            game.container, None,
            "the open double chest kept a menu after its other half broke"
        );
    }
}

#[cfg(test)]
pub(crate) mod lock_tests {
    use super::super::game_tests::tests::server_game;
    use sunrise_core::nbt::Tag;
    use sunrise_items::items::ItemStack;
    use sunrise_level::containers::BlockEntity;
    use sunrise_protocol::packets::play::Clientbound;

    fn has_overlay(game: &sunrise_gameplay::game::ServerGame<'_>) -> bool {
        game.server
            .guest_packets
            .iter()
            .any(|packet| matches!(packet, Clientbound::SystemChat(chat) if chat.overlay))
    }

    /// `BaseContainerBlockEntity.canOpen` (`LockCode`): a `lock` whose item
    /// predicate the opener's main hand does not match refuses the menu at
    /// `createMenu`, so the block is never set as the open container. A hand
    /// holding the lock's item opens it.
    #[test]
    #[ignore = "daily"]
    fn a_locked_container_refuses_the_local_players_open() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.options.dedicated = None;
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:chest")
            .unwrap();
        let key = (8, 64, 8);
        game.world.set(key.0, key.1, key.2, state).unwrap();
        game.world
            .block_entities
            .insert(key, BlockEntity::Chest(vec![None; 27]));
        let lock = Tag::compound([(
            "lock",
            Tag::compound([("items", Tag::String("minecraft:gold_ingot".into()))]),
        )]);
        game.world.set_block_lock(key, &lock);

        // An empty hand does not match `lock:{items:"minecraft:gold_ingot"}`.
        game.server.guest_packets.clear();
        game.open_block_effect_menu([8, 64, 8]);
        assert_eq!(game.container, None, "a locked chest opened");
        assert!(has_overlay(&game), "no `container.isLocked` action bar");

        // `LockCode.canUnlock` reads the main hand: the key in the off hand
        // does not open it.
        game.inventory[0] = None;
        game.offhand = Some(ItemStack::new("minecraft:gold_ingot", 1));
        game.server.guest_packets.clear();
        game.open_block_effect_menu([8, 64, 8]);
        assert_eq!(
            game.container, None,
            "an off-hand key opened a locked chest"
        );

        // The lock's own item in the main hand opens it.
        game.inventory[0] = Some(ItemStack::new("minecraft:gold_ingot", 1));
        game.offhand = None;
        game.selected = 0;
        game.server.guest_packets.clear();
        game.open_block_effect_menu([8, 64, 8]);
        assert_eq!(game.container, Some(key), "the lock's item did not open it");
        assert!(!has_overlay(&game));
    }

    /// `ChestBlock.MENU_PROVIDER_COMBINER.createMenu`: a double chest asks
    /// `canOpen` of both halves, so a lock on the half the player did not
    /// click still refuses the six-row menu.
    #[test]
    #[ignore = "daily"]
    fn a_lock_on_either_half_refuses_a_double_chest() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.options.dedicated = None;
        let registry = game.pack.assets.registry;
        let half = |kind: &str| {
            let mut state = registry
                .state(registry.default_state("minecraft:chest").unwrap())
                .unwrap()
                .clone();
            state.properties.insert("type".into(), kind.into());
            state.properties.insert("facing".into(), "north".into());
            registry.id(&state).unwrap()
        };
        let left_key = (8, 64, 8);
        let right_key = (9, 64, 8);
        game.world
            .set(left_key.0, left_key.1, left_key.2, half("left"))
            .unwrap();
        game.world
            .set(right_key.0, right_key.1, right_key.2, half("right"))
            .unwrap();
        for key in [left_key, right_key] {
            game.world
                .block_entities
                .insert(key, BlockEntity::Chest(vec![None; 27]));
        }
        // The right half holds the lock; the player clicks the left one.
        let lock = Tag::compound([(
            "lock",
            Tag::compound([("items", Tag::String("minecraft:gold_ingot".into()))]),
        )]);
        game.world.set_block_lock(right_key, &lock);

        game.server.guest_packets.clear();
        game.open_block_effect_menu([8, 64, 8]);
        assert_eq!(game.container, None, "a half-locked double chest opened");
        assert!(has_overlay(&game));
    }
}
