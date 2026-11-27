#![cfg(test)]
#[cfg(test)]
#[path = "trident/impact_tests.rs"]
pub(crate) mod impact_tests;

#[cfg(test)]
pub(crate) mod tests {

    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1;
    use sunrise_items::items::ItemStack;
    use sunrise_sim::entities::arrows::Pickup;

    /// A stack can reach its final durability point after use began. The release
    /// guard is independent of admission, and short charges still do nothing.
    #[test]
    #[ignore = "daily"]
    fn trident_release_rechecks_stack_durability_after_charge() {
        use sunrise_core::nbt::Tag;
        use {
            crate::game::Settings, crate::game::load_resources,
            sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::PlayerMotion,
            sunrise_gameplay_common::game::Progress,
        };
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                .join("trident-release"),
        );
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 200.0, 4.5)).unwrap());
        let mut stack = ItemStack::new("minecraft:trident", 1);
        stack.damage = 8;
        stack
            .data_mut()
            .set_other("minecraft:max_damage", Some(Tag::Int(10)));
        *game.held_mut() = Some(stack.clone());
        assert!(game.use_trident(&stack));
        assert!(game.using.is_some());
        game.held_mut().as_mut().unwrap().damage = 9;
        game.release_trident(10);
        assert!(game.tridents.is_empty());
        assert_eq!(game.held().unwrap().damage, 9);
        game.using = None;

        stack.damage = 249;
        stack
            .data_mut()
            .set_other("minecraft:max_damage", Some(Tag::Int(500)));
        *game.held_mut() = Some(stack.clone());
        assert!(game.use_trident(&stack));
        assert!(game.using.is_some());
        game.release_trident(9);
        assert!(game.tridents.is_empty());
        assert_eq!(game.held().unwrap().damage, 249);
        game.release_trident(10);
        assert_eq!(game.tridents.len(), 1);
        assert_eq!(game.tridents[0].stack.damage, 250);
        assert!(game.held().is_none());
        assert_eq!(game.tridents[0].pickup, Pickup::Allowed);
        assert_eq!(game.tridents[0].owner_uuid, game.level.player_uuid);
        assert!(!game.tridents[0].has_been_shot);
        assert!(!game.tridents[0].left_owner);

        game.creative = true;
        game.using = None;
        let stack = ItemStack::new("minecraft:trident", 1);
        *game.held_mut() = Some(stack.clone());
        assert!(game.use_trident(&stack));
        game.release_trident(10);
        assert_eq!(game.tridents[1].pickup, Pickup::CreativeOnly);
        assert!(game.held().is_some());
    }
    /// `TridentItem.releaseUsing` launches a Riptide III player along the float look vector
    /// scaled to 3.0 by its float length, added to the velocity it had. The expected components
    /// are computed apart from the code, with every step rounded to f32 as the look is.
    #[test]
    #[ignore = "daily"]
    fn riptide_launch_preserves_float_push() {
        use {
            crate::game::Settings, crate::game::load_resources,
            sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::PlayerMotion,
            sunrise_gameplay_common::game::Progress,
        };
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                .join("trident-riptide"),
        );
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
        );
        let water = game
            .pack
            .assets
            .registry
            .default_state("minecraft:water")
            .unwrap();
        game.world.set(4, 200, 4, water).unwrap();
        let mut player = PlayerMotion::new(game.motion, Vec3::new(4.5, 200.0, 4.5)).unwrap();
        player.velocity = Vec3::new(0.1, 0.2, 0.3);
        game.player = Some(player);
        game.yaw = 30.0;
        game.pitch = 20.0;
        let mut stack = ItemStack::new("minecraft:trident", 1);
        stack
            .data_mut()
            .enchantments
            .push(("minecraft:riptide".into(), 3));
        *game.held_mut() = Some(stack.clone());
        assert!(game.use_trident(&stack));
        game.release_trident(10);
        let velocity = game.player.as_ref().unwrap().velocity;
        assert_eq!(velocity.x, f64::from_bits(0xbff4_f3bb_2666_6666));
        assert_eq!(velocity.y, f64::from_bits(0xbfea_6d1e_9999_999a));
        assert_eq!(velocity.z, f64::from_bits(0x4005_ee9e_c666_6666));
    }
    /// TridentItem.releaseUsing on the ground: the riptide's `move(SELF, (0, 1.1999999, 0))` is the player's own
    /// move, whose end writes the ground state, so the player is in the air for the rest of the tick (and travels
    /// with air friction) as vanilla's is.
    #[test]
    #[ignore = "daily"]
    fn a_riptide_from_the_ground_leaves_the_player_in_the_air() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            use std::sync::Arc;
            use sunrise_level::chunk::Chunk;
            use sunrise_sim::motion::PlayerMotion;
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let water = registry.default_state("minecraft:water").unwrap();
            game.world.set(0, 63, 0, stone).unwrap();
            game.world.set(0, 64, 0, water).unwrap();
            let mut player = PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap();
            player.ground.on_ground = true;
            game.player = Some(player);
            let mut stack = ItemStack::new("minecraft:trident", 1);
            stack.enchant("minecraft:riptide", 3);
            *game.held_mut() = Some(stack.clone());
            assert!(game.use_trident(&stack));
            game.release_trident(20);
            let p = game.player.as_ref().unwrap();
            assert!(!p.ground.on_ground);
            assert!(
                (p.position.y - (64.0 + f64::from(1.1999999_f32))).abs() < 1e-9,
                "{}",
                p.position.y
            );
        });
    }
}
