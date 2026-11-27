#![cfg(test)]
#[cfg(test)]
#[path = "minecarts/hit.rs"]
pub(crate) mod hit;
#[cfg(test)]
#[path = "minecarts/lifetime_tests.rs"]
pub(crate) mod lifetime_tests;

#[cfg(test)]
#[path = "minecarts/minecart_tnt_tests.rs"]
pub(crate) mod minecart_tnt_tests;

#[cfg(test)]
#[path = "minecarts/tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
pub(crate) mod tnt_impact_tests {
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::GameGameExt1;
    use {
        sunrise_gameplay_common::game::minecarts::CartKind,
        sunrise_gameplay_common::game::minecarts::Minecart,
    };

    /// Ticks a moving TNT cart until it stops or blows up; true when it
    /// blew up.
    fn runs_into_the_wall(rails: bool, at: Vec3, velocity: Vec3) -> bool {
        let mut result = false;
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let rail = registry.default_state("minecraft:rail").unwrap();
            let mut east_west = registry.state(rail).unwrap().clone();
            east_west
                .properties
                .insert("shape".into(), "east_west".into());
            let rail = registry.id(&east_west).unwrap();
            for x in 0..16 {
                for z in 0..16 {
                    game.world.set(x, 63, z, stone).unwrap();
                }
            }
            for z in 0..16 {
                game.world.set(9, 64, z, stone).unwrap();
                game.world.set(9, 65, z, stone).unwrap();
            }
            if rails {
                for x in 1..9 {
                    game.world.set(x, 64, 8, rail).unwrap();
                }
            }
            let mut cart = Minecart::new(1, CartKind::Tnt, at);
            cart.velocity = velocity;
            for _ in 0..40 {
                if !game.tick_cart(
                    &mut cart,
                    [0.0; 2],
                    0,
                    &mut sunrise_gameplay::game::server_side::minecarts_server::CartIndex::default(
                    ),
                ) {
                    result = true;
                    break;
                }
            }
        });
        result
    }

    #[test]
    #[ignore = "daily"]
    fn a_powered_rail_pushes_a_resting_cart_away_from_soul_sand() {
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        let registry = game.pack.assets.registry;
        let rail = registry.default_state("minecraft:powered_rail").unwrap();
        let mut state = registry.state(rail).unwrap().clone();
        state.properties.insert("shape".into(), "east_west".into());
        state.properties.insert("powered".into(), "true".into());
        let rail = registry.id(&state).unwrap();
        game.world.set(5, 64, 8, rail).unwrap();
        for (block, pushed) in [
            ("minecraft:soul_sand", true),
            ("minecraft:stone", true),
            ("minecraft:oak_slab", false),
        ] {
            let id = registry.default_state(block).unwrap();
            game.world.set(4, 64, 8, id).unwrap();
            let push = game.redstone_direction(
                [5, 64, 8],
                sunrise_gameplay_common::game::minecarts::Shape::EastWest,
            );
            assert_eq!(push.x == 1.0, pushed, "{block}");
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_tnt_cart_stopped_head_on_by_a_wall_does_not_blow_up() {
        // OldMinecartBehavior.moveAlongTrack: the move zeroes the blocked x
        // and the track's re-pointing scales what is left, so MinecartTNT
        // reads no speed at the wall.
        assert!(!runs_into_the_wall(
            true,
            Vec3::new(3.5, 64.0, 8.5),
            Vec3::new(0.4, 0.0, 0.0)
        ));
    }

    #[test]
    #[ignore = "daily"]
    fn a_tnt_cart_off_its_rails_glancing_off_a_wall_blows_up() {
        // comeOffTrack keeps the delta the move resolved: the wall zeroes x,
        // z goes on at over a tenth of a block a tick.
        assert!(runs_into_the_wall(
            false,
            Vec3::new(8.3, 64.0, 4.5),
            Vec3::new(0.4, 0.0, 0.4)
        ));
    }

    #[test]
    #[ignore = "daily"]
    fn a_cart_on_a_slope_presses_a_plate_at_its_temporary_position() {
        // OldMinecartBehavior.moveAlongTrack places the cart at the rail's top
        // height, moves it horizontally (Entity.move records the step) and settles
        // it on the slope; the tick's applyEffectsFromBlocks still sweeps the step.
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let plate = registry
            .default_state("minecraft:oak_pressure_plate")
            .unwrap();
        let rail = registry.default_state("minecraft:rail").unwrap();
        let mut slope = registry.state(rail).unwrap().clone();
        slope
            .properties
            .insert("shape".into(), "ascending_east".into());
        let rail = registry.id(&slope).unwrap();
        for x in 0..16 {
            for z in 0..16 {
                game.world.set(x, 63, z, stone).unwrap();
            }
        }
        game.world.set(5, 64, 8, rail).unwrap();
        game.world.set(6, 64, 8, stone).unwrap();
        game.world.set(6, 65, 8, plate).unwrap();
        let mut cart = Minecart::new(1, CartKind::Rideable, Vec3::new(5.5, 64.5, 8.5));
        cart.velocity = Vec3::new(0.4, 0.0, 0.0);
        game.carts.push(cart.clone());
        assert!(game.tick_cart(
            &mut cart,
            [0.0; 2],
            0,
            &mut sunrise_gameplay::game::server_side::minecarts_server::CartIndex::default()
        ));
        let powered = registry
            .state(game.world.block(6, 65, 8))
            .unwrap()
            .properties
            .get("powered")
            .cloned();
        assert_eq!(
            powered.as_deref(),
            Some("true"),
            "{:?}",
            registry.state(game.world.block(6, 65, 8))
        );
    }

    /// A cart blocked by a fence on a slope sits below the plate on the fence,
    /// but `AbstractMinecart.move` sweeps its blocks while it is still at the
    /// rail's top height, so the plate clocks on and off.
    #[test]
    #[ignore = "daily"]
    fn a_cart_resting_against_a_fence_keeps_toggling_the_plate_above_it() {
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let fence = registry.default_state("minecraft:oak_fence").unwrap();
        let plate = registry
            .default_state("minecraft:oak_pressure_plate")
            .unwrap();
        let rail = registry.default_state("minecraft:rail").unwrap();
        let mut slope = registry.state(rail).unwrap().clone();
        slope
            .properties
            .insert("shape".into(), "ascending_east".into());
        let slope = registry.id(&slope).unwrap();
        for x in 0..16 {
            for z in 0..16 {
                game.world.set(x, 62, z, stone).unwrap();
            }
        }
        game.world.set(0, 64, 8, fence).unwrap();
        game.world.set(0, 65, 8, plate).unwrap();
        game.world.set(1, 63, 8, stone).unwrap();
        game.world.set(1, 64, 8, slope).unwrap();
        game.world.set(2, 64, 8, stone).unwrap();
        game.world.set(2, 65, 8, rail).unwrap();
        game.carts.push(Minecart::new(
            1,
            CartKind::Rideable,
            Vec3::new(1.9, 64.9, 8.5),
        ));
        // Scheduled ticks and carts only run in lit, surrounded chunks near a
        // player.
        game.server.settings.simulation_distance = 4;
        game.server.settings.render_distance = 12;
        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 70.0, 12.5))
                .unwrap(),
        );
        for cx in -2..=2 {
            for cz in -2..=2 {
                if (cx, cz) != (0, 0) {
                    let mut chunk = sunrise_level::chunk::Chunk::empty(cx, cz);
                    chunk.lit = true;
                    game.world
                        .chunks
                        .insert((cx, cz), std::sync::Arc::new(chunk));
                    game.block_ticks
                        .add_empty_chunk(sunrise_core::position::ChunkPos::new(cx, cz))
                        .unwrap();
                }
            }
        }
        let mut center = (**game.world.chunks.get(&(0, 0)).unwrap()).clone();
        center.lit = true;
        game.world
            .chunks
            .insert((0, 0), std::sync::Arc::new(center));
        game.update_chunk_tickets();
        let mut edges = 0;
        let mut last = false;
        for _ in 0..200 {
            game.world.game_time += 1;
            game.tick_carts_now();
            game.tick_blocks().unwrap();
            let on = registry
                .state(game.world.block(0, 65, 8))
                .unwrap()
                .properties
                .get("powered")
                .is_some_and(|v| v == "true");
            if on != last {
                edges += 1;
                last = on;
            }
        }
        assert!(edges >= 4, "the plate changed {edges} times in 200 ticks");
    }
}

#[cfg(test)]
pub(crate) mod cart_push_tests {
    use sunrise_core::geometry::Vec3;
    use sunrise_items::items::ItemStack;
    use sunrise_sim::save::PendingContainerLoot;
    use {
        sunrise_gameplay_common::game::minecarts::CartKind,
        sunrise_gameplay_common::game::minecarts::Minecart,
    };

    /// Two chest carts pushed apart (`OldMinecartBehavior.pushAndPickupEntities`):
    /// the push runs on copies of the ticked cart, and the one still holding
    /// its mineshaft roll keeps the roll, so it is saved as its loot table.
    #[test]
    #[ignore = "daily"]
    fn a_pushed_chest_cart_keeps_its_pending_loot() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            for x in 4..13 {
                for z in 4..13 {
                    game.world.set(x, 63, z, stone).unwrap();
                }
            }
            let items = vec![Some(ItemStack::new("minecraft:rail", 12)); 27];
            let loot = PendingContainerLoot {
                table: sunrise_core::nbt::Tag::String(
                    "minecraft:chests/abandoned_mineshaft".into(),
                ),
                seed: Some(sunrise_core::nbt::Tag::Long(42)),
            };
            let mut rolled = Minecart::new(1, CartKind::Chest, Vec3::new(8.3, 64.0, 8.5));
            rolled.items = items;
            rolled.loot = Some(loot.clone());
            game.carts.push(rolled);
            // Facing along X (yaw 0) and 0.4 apart on X: the old behaviour pushes.
            game.carts
                .push(Minecart::new(2, CartKind::Chest, Vec3::new(8.7, 64.0, 8.5)));
            for _ in 0..5 {
                game.tick_carts();
            }
            let cart = |id| game.carts.iter().find(|c| c.id == id).unwrap();
            assert!(
                cart(2).position.x - cart(1).position.x > 0.4,
                "the carts were pushed apart: {} {}",
                cart(1).position.x,
                cart(2).position.x
            );
            assert_eq!(cart(1).loot.as_ref(), Some(&loot));
            assert_eq!(cart(2).loot, None);
        });
    }

    /// `OldMinecartBehavior.pushAndPickupEntities` for carts: a moving rideable
    /// cart only pushes what `EntitySelector.pushableBy(cart)` allows, by its
    /// own team's collision rule and the target's; a still cart keeps vanilla's
    /// unfiltered minecart push.
    #[test]
    #[ignore = "daily"]
    fn a_moving_carts_team_rules_which_carts_it_pushes() {
        use sunrise_game_data::scoreboard::CollisionRule;
        use sunrise_gameplay_common::game::commands_common::Target;

        /// Puts a cart on a team of its own, as `/team join` would.
        fn on_a_team(
            game: &mut sunrise_gameplay::game::ServerGame<'_>,
            id: u32,
            rule: CollisionRule,
        ) {
            let name = game.holder_name(Target::Cart(id)).expect("a cart's UUID");
            let team = format!("cart-{id}");
            game.score.board.add_team(&team).collision_rule = rule;
            game.score.board.add_to_team(&name, &team);
        }
        /// Pushes with cart `pusher` moving at `velocity`, and returns the
        /// target's velocity afterwards.
        fn push_apart(
            game: &mut sunrise_gameplay::game::ServerGame<'_>,
            pusher: usize,
            target: usize,
            velocity: Vec3,
        ) -> Vec3 {
            game.carts.update(target, |cart| cart.velocity = Vec3::ZERO);
            game.carts.update(pusher, |cart| cart.velocity = velocity);
            let mut index =
                sunrise_gameplay::game::server_side::minecarts_server::CartIndex::default();
            game.push_from_cart(pusher, &mut index);
            game.carts[target].velocity
        }

        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        // Cart 2 moves east, into cart 1's box; yaw 0 faces east, along the push.
        game.carts.push(Minecart::new(
            1,
            CartKind::Rideable,
            Vec3::new(8.5, 64.0, 8.5),
        ));
        game.carts.push(Minecart::new(
            2,
            CartKind::Rideable,
            Vec3::new(8.1, 64.0, 8.5),
        ));
        let east = Vec3::new(0.2, 0.0, 0.0);
        // No teams: the moving cart pushes the still one.
        assert_ne!(
            push_apart(&mut game, 1, 0, east),
            Vec3::ZERO,
            "the moving cart pushes the still one"
        );
        // The target's `never` rule holds the push back.
        on_a_team(&mut game, 1, CollisionRule::Never);
        assert_eq!(
            push_apart(&mut game, 1, 0, east),
            Vec3::ZERO,
            "the target's collisionRule never keeps it off"
        );
        // The moving cart's own rule does too.
        game.score.board.remove_team("cart-1");
        on_a_team(&mut game, 2, CollisionRule::Never);
        assert_eq!(
            push_apart(&mut game, 1, 0, east),
            Vec3::ZERO,
            "the moving cart's collisionRule never keeps it off"
        );
        // Allies with `pushOwnTeam` do not push each other.
        game.score.board.remove_team("cart-2");
        on_a_team(&mut game, 1, CollisionRule::PushOwnTeam);
        let name = game.holder_name(Target::Cart(2)).expect("a cart's UUID");
        game.score.board.add_to_team(&name, "cart-1");
        assert_eq!(
            push_apart(&mut game, 1, 0, east),
            Vec3::ZERO,
            "pushOwnTeam keeps allies apart"
        );
        // A still cart is vanilla's else branch: no team filter at all.
        assert_ne!(
            push_apart(&mut game, 1, 0, Vec3::ZERO),
            Vec3::ZERO,
            "a still cart pushes the minecart in its box whatever the teams"
        );
    }

    /// `EntitySelector.pushableBy(cart)` for mobs: a moving rideable cart on a
    /// `never` team neither boards nor pushes the cow in its path.
    #[test]
    #[ignore = "daily"]
    fn a_moving_carts_team_keeps_it_from_boarding_a_cow() {
        use sunrise_game_data::scoreboard::CollisionRule;
        use sunrise_gameplay_common::game::commands_common::Target;
        use sunrise_sim::mobs::Mob;

        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        let kind = game.res.mob_types().get("minecraft:cow").cloned().unwrap();
        let mut cart = Minecart::new(2, CartKind::Rideable, Vec3::new(8.5, 64.0, 8.5));
        cart.velocity = Vec3::new(0.2, 0.0, 0.0);
        game.carts.push(cart);
        let at = Vec3::new(8.5, 64.0, 8.5);
        game.mobs.push(Mob::new(7, kind, at, 0.0));
        let mut index = sunrise_gameplay::game::server_side::minecarts_server::CartIndex::default();
        game.push_from_cart(0, &mut index);
        assert_eq!(
            game.mobs[0].extra.mount,
            Some(2),
            "the cow boards the empty moving cart"
        );
        game.mobs[0].extra.mount = None;
        game.mobs[0].velocity = Vec3::ZERO;
        // The cart's own `never` rule: no boarding and no push.
        let name = game.holder_name(Target::Cart(2)).expect("a cart's UUID");
        game.score.board.add_team("cart-2").collision_rule = CollisionRule::Never;
        game.score.board.add_to_team(&name, "cart-2");
        game.carts
            .update(0, |cart| cart.velocity = Vec3::new(0.2, 0.0, 0.0));
        let mut index = sunrise_gameplay::game::server_side::minecarts_server::CartIndex::default();
        game.push_from_cart(0, &mut index);
        assert_eq!(
            game.mobs[0].extra.mount, None,
            "the cart's collisionRule never keeps the cow off"
        );
        assert_eq!(
            game.mobs[0].velocity,
            Vec3::ZERO,
            "and the cow is not pushed either"
        );
    }
}

#[cfg(test)]
pub(crate) mod container_drop_tests {
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;
    use sunrise_core::random::{LegacyRandom, RandomSource};
    use sunrise_gameplay::game::server_side::minecarts_server::CartHit;
    use sunrise_gameplay_common::game::boats_common::Boat;
    use sunrise_items::items::ItemStack;
    use {
        sunrise_gameplay_common::game::minecarts::CartKind,
        sunrise_gameplay_common::game::minecarts::Minecart,
    };

    /// Advances `r` past the draws a spill takes from the level random, as
    /// vanilla's `Containers.dropContents` takes them: for each slot
    /// (`Containers.dropItemStack`) its spot, then for each pile the split
    /// size and the triangle motion. Every count here is at most 10, so a
    /// slot with items is a single pile.
    fn vanilla_spill_draws(r: &mut LegacyRandom, counts: &[u32]) {
        for &count in counts {
            for _ in 0..3 {
                r.next_double();
            }
            if count > 0 {
                let _ = r.next_int_bound(21);
                for _ in 0..3 {
                    r.triangle(0.0, 0.114_850_001_711_398_36);
                }
            }
        }
    }

    /// A chest boat or chest minecart broken with entity_drops on spills its
    /// slots in `remove` and then the emptied container again in
    /// `chestVehicleDestroyed`; the hull item takes the item entity's own
    /// random. The level random must end where vanilla's does.
    #[test]
    #[ignore = "daily"]
    fn a_broken_chest_vehicle_draws_the_level_random_like_vanilla() {
        let mut slots = vec![None; 27];
        slots[0] = Some(ItemStack::new("minecraft:diamond", 5));
        slots[4] = Some(ItemStack::new("minecraft:dirt", 8));
        let mut counts = vec![0; 27];
        counts[0] = 5;
        counts[4] = 8;
        for boat in [true, false] {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                assert!(
                    game.ui
                        .rules
                        .bool_at(sunrise_game_data::gamerules::vanilla::ENTITY_DROPS)
                );
                let at = Vec3::new(8.5, 64.0, 8.5);
                if boat {
                    let mut b = Boat::new(1, Arc::from("oak"), at, 0.0);
                    b.chest = true;
                    b.items = slots.clone();
                    game.boats.push(b);
                } else {
                    let mut c = Minecart::new(1, CartKind::Chest, at);
                    c.items = slots.clone();
                    game.carts.push(c);
                }
                game.world.random.set_seed(1234);
                if boat {
                    assert_eq!(game.hurt_boat_result(0, 100.0, false), Some(true));
                    assert!(game.boats.is_empty());
                } else {
                    assert!(game.hurt_cart_result(0, 100.0, false, false, CartHit::default()));
                    assert!(game.carts.is_empty());
                }
                let mut expected = LegacyRandom::new(1234);
                vanilla_spill_draws(&mut expected, &counts);
                vanilla_spill_draws(&mut expected, &[0; 27]);
                assert_eq!(
                    game.world.random.next_long(),
                    expected.next_long(),
                    "boat: {boat}"
                );
                // The two piles and the hull item.
                assert_eq!(game.dropped.len(), 3, "boat: {boat}");
            });
        }
    }
}

#[cfg(test)]
pub(crate) mod rejected_passenger_tests {
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay_common::game::minecarts::{
        CartKind, HEIGHT, InteractTarget, Minecart, Vec3,
    };
    use {
        sunrise_gameplay::game::native_entity_refs::tests::with_game,
        sunrise_gameplay_common::game::commands_common::Target,
    };

    fn pig(uuid: i32, children: Vec<Tag>) -> Tag {
        Tag::compound([
            ("id", Tag::String("minecraft:pig".into())),
            ("UUID", Tag::IntArray(vec![0, 2175, 0, uuid])),
            (
                "Pos",
                Tag::List(vec![
                    Tag::Double(8.0f64.to_bits()),
                    Tag::Double(64.0f64.to_bits()),
                    Tag::Double(8.0f64.to_bits()),
                ]),
            ),
            ("Passengers", Tag::List(children)),
        ])
    }

    #[test]
    #[ignore = "daily"]
    fn rejected_cart_passenger_occupies_capacity_physics_pickup_and_interaction() {
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let cart = 5000;
            game.carts.push(Minecart::new(
                cart,
                CartKind::Rideable,
                Vec3::new(8.0, 64.0, 8.0),
            ));
            game.load_mob(pig(1, vec![]), None).unwrap();
            let ghost = game
                .load_mob(pig(1, vec![pig(2, vec![])]), Some(cart))
                .unwrap();
            assert!(game.persist.off_level.contains_key(&ghost));
            let candidate = game.load_mob(pig(3, vec![]), None).unwrap();
            assert!(game.has_seated_passengers(cart));
            let mut copy = game.carts[0].clone();
            copy.velocity = Vec3::new(1.0, 0.0, 0.0);
            assert_eq!(
                game.natural_slowdown(&mut copy, game.has_seated_passengers(cart), 0.96)
                    .x,
                0.997
            );
            assert!(!game.start_riding_target(Target::Mob(candidate), Target::Cart(cart), false));
            game.carts
                .update(0, |c| c.velocity = Vec3::new(0.2, 0.0, 0.0));
            game.pickup_from_cart(
                0,
                &mut sunrise_gameplay::game::server_side::minecarts_server::CartIndex::default(),
            );
            assert_eq!(game.mob_object(candidate).unwrap().extra.mount, None);
            assert!(!game.interact_cart(Some(InteractTarget {
                id: cart as i32,
                location: [0.0; 3]
            })));
            assert_eq!(game.riding, None);
            let mut camel = pig(4, vec![]);
            if let Tag::Compound(c) = &mut camel {
                c.insert("id".into(), Tag::String("minecraft:camel".into()));
            }
            let camel = game.load_mob(camel, None).unwrap();
            game.load_mob(pig(1, vec![]), Some(camel)).unwrap();
            game.load_mob(pig(1, vec![]), Some(camel)).unwrap();
            game.load_mob(pig(1, vec![]), Some(camel)).unwrap();
            assert_eq!(game.seated_passengers(camel).len(), 3);
            assert!(!game.start_riding_target(Target::Mob(candidate), Target::Mob(camel), false));
            let mut active = game.carts[0].clone();
            active.position = Vec3::new(40.0, 70.0, 40.0);
            active.yaw = 90.0;
            active.flipped = true;
            game.activate_cart(&mut active, true);
            let rider = game.mob_object(ghost).unwrap();
            assert_eq!(rider.extra.mount, None);
            assert_eq!(rider.position, Vec3::new(40.0, 70.0 + HEIGHT, 40.0));
            assert_eq!(rider.previous, rider.position);
            let leaf = game.mob_passenger_ids(ghost)[0];
            assert_eq!(game.mob_object(leaf).unwrap().position.x, 40.0);
            assert!(game.mob_object(leaf).unwrap().position.y >= 70.0);
            assert_eq!(game.carts[0].position, Vec3::new(8.0, 64.0, 8.0));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn cart_destruction_ejects_rejected_rider_and_snaps_frozen_descendant() {
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.carts.push(Minecart::new(
                5000,
                CartKind::Rideable,
                Vec3::new(8.0, 70.0, 8.0),
            ));
            game.load_mob(pig(1, vec![]), None).unwrap();
            let ghost = game
                .load_mob(pig(1, vec![pig(2, vec![])]), Some(5000))
                .unwrap();
            let leaf = game.mob_passenger_ids(ghost)[0];
            assert!(game.mobs_below_off_level().contains(&leaf));
            assert!(game.kill_vehicle_object(sunrise_gameplay::game::commands::Target::Cart(5000)));
            let rider = game.mob_object(ghost).unwrap();
            assert_eq!(rider.extra.mount, None);
            assert!(!rider.extra.passenger);
            assert_eq!(rider.position.y, 70.0);
            assert_eq!(rider.position, rider.previous);
            assert_eq!(game.mob_object(leaf).unwrap().extra.mount, Some(ghost));
            assert!(game.mob_object(leaf).unwrap().position.y >= 70.0);
        });
    }
}

#[cfg(test)]
pub(crate) mod hook_mutation_regressions {
    use std::sync::{Arc, Mutex};
    use sunrise_sim::mobs::Mob;
    use {
        crate::game::minecarts::*,
        sunrise_gameplay_common::game::minecarts::{CartKind, Minecart, Vec3},
    };

    struct MutateRider {
        pub discard: bool,
        heard: Arc<Mutex<Vec<(u32, bool)>>>,
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for MutateRider {
        fn entity_mount(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            self.heard
                .lock()
                .unwrap()
                .push((event.entity, event.mounting));
            if !event.mounting && event.entity == 72 {
                if self.discard {
                    assert!(host.entity(72).unwrap().discard(host));
                } else {
                    let later = host.entity(73).unwrap();
                    let replacement = host.entity(74).unwrap();
                    assert!(later.start_riding(host, replacement, true).is_ok());
                }
            }
            mod_api::Flow::Continue
        }
    }
    #[test]
    #[ignore = "daily"]
    fn activator_preserves_a_later_passengers_callback_reparenting() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "mutate_rider",
            Box::new(MutateRider {
                discard: false,
                heard: heard.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_resident_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.carts
                    .push(Minecart::new(71, CartKind::Rideable, Vec3::ZERO));
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                for id in [72, 73, 74] {
                    let mut mob = Mob::new(id, kind.clone(), Vec3::ZERO, 0.0);
                    mob.extra.mount = (id != 74).then_some(71);
                    mob.extra.vehicle_order = i64::from(id);
                    game.mobs.push(mob);
                }
                let mut active = game.carts[0].clone();
                game.activate_cart(&mut active, true);
                assert_eq!(game.mob_object(73).unwrap().extra.mount, Some(74));
                assert_eq!(
                    heard
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|&&(id, mounting)| id == 73 && !mounting)
                        .count(),
                    1,
                    "only the callback's deliberate reparenting dismounts this rider"
                );
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn activator_does_not_snap_a_rider_discarded_by_its_callback() {
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "mutate_rider",
            Box::new(MutateRider {
                discard: true,
                heard: Arc::default(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_resident_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.carts
                    .push(Minecart::new(71, CartKind::Rideable, Vec3::ZERO));
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                let original = Vec3::new(2.0, 60.0, 3.0);
                let mut rider = Mob::new(72, kind.clone(), original, 0.0);
                rider.extra.mount = Some(71);
                game.mobs.push(rider);
                let mut child = Mob::new(73, kind, original, 0.0);
                child.extra.mount = Some(72);
                game.mobs.push(child);
                let mut active = game.carts[0].clone();
                active.position = Vec3::new(40.0, 70.0, 40.0);
                game.activate_cart(&mut active, true);
                assert!(game.mob_object(72).unwrap().removed);
                assert_eq!(game.mob_object(72).unwrap().position, original);
                assert_eq!(game.mob_object(73).unwrap().position, original);
            });
        });
    }
    struct DiscardDamagedVehicle(Arc<Mutex<usize>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for DiscardDamagedVehicle {
        fn vehicle_damage(
            &self,
            host: &mut dyn mod_api::ServerHost,
            _: &mut mod_api::VehicleDamageEvent<'_>,
        ) -> mod_api::Flow {
            assert!(host.entity(71).unwrap().discard(host));
            mod_api::Flow::Continue
        }
        fn vehicle_destroy(
            &self,
            _: &mut dyn mod_api::ServerHost,
            _: &mut mod_api::VehicleDestroyEvent<'_>,
        ) {
            *self.0.lock().unwrap() += 1;
        }
    }
    #[test]
    #[ignore = "daily"]
    fn a_damage_callback_discard_does_not_also_destroy_or_drop_the_vehicle() {
        for cart in [false, true] {
            let destroyed = Arc::new(Mutex::new(0));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "discard_damaged",
                Box::new(DiscardDamagedVehicle(destroyed.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                sunrise_gameplay::game::native_entity_refs::tests::with_resident_game(|game| {
                    game.options.dedicated = Some(Vec::new());
                    game.ui.rules.set(
                        "entity_drops",
                        sunrise_game_data::gamerules::Value::Bool(true),
                    );
                    if cart {
                        game.carts
                            .push(Minecart::new(71, CartKind::Rideable, Vec3::ZERO));
                        game.hurt_cart(
                            0,
                            5.0,
                            false,
                            false,
                            sunrise_gameplay::game::server_side::minecarts_server::CartHit::default(
                            ),
                        );
                    } else {
                        game.boats
                            .push(sunrise_gameplay_common::game::boats_common::Boat::new(
                                71,
                                "oak".into(),
                                Vec3::ZERO,
                                0.0,
                            ));
                        game.hurt_boat(0, 5.0, false);
                    }
                    assert_eq!(*destroyed.lock().unwrap(), 0);
                    assert!(
                        game.dropped.is_empty(),
                        "discard must not create the vehicle item"
                    );
                    game.native_removals_flush();
                    assert_eq!(*destroyed.lock().unwrap(), 0);
                    assert!(game.dropped.is_empty());
                    assert!(game.entity(71).is_none());
                });
            });
        }
    }
}

#[cfg(test)]
pub(crate) mod cart_cost_bench {
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay::game::game_tests::tests::server_game;
    use sunrise_gameplay_common::game::GameGameExt1;
    use {
        sunrise_gameplay_common::game::minecarts::CartKind,
        sunrise_gameplay_common::game::minecarts::Minecart,
    };

    /// Times `tick_carts` with many parked carts and some moving ones:
    /// `CARTS_PARKED` (default 600) carts each on its own rail piece, and
    /// `CARTS_MOVING` (default 100) on powered tracks. Prints the ms per tick.
    #[test]
    #[ignore = "a timing, needs prepared resources in SUNRISE_TEST_RESOURCES"]
    fn tick_carts_with_many_parked_carts() {
        let n = |key: &str, default: usize| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };
        let (parked, moving) = (n("CARTS_PARKED", 600), n("CARTS_MOVING", 100));
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.server.settings.simulation_distance = 6;
        game.server.settings.render_distance = 12;
        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 80.0, 12.5))
                .unwrap(),
        );
        for cx in -3..=3 {
            for cz in -3..=3 {
                if (cx, cz) != (0, 0) {
                    let mut chunk = sunrise_level::chunk::Chunk::empty(cx, cz);
                    chunk.lit = true;
                    game.world
                        .chunks
                        .insert((cx, cz), std::sync::Arc::new(chunk));
                    game.block_ticks
                        .add_empty_chunk(sunrise_core::position::ChunkPos::new(cx, cz))
                        .unwrap();
                }
            }
        }
        game.update_chunk_tickets();
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let rail = registry.default_state("minecraft:rail").unwrap();
        let powered = registry.default_state("minecraft:powered_rail").unwrap();
        let with = |base: sunrise_world::registry::BlockStateId, shape: &str, on: bool| {
            let mut s = registry.state(base).unwrap().clone();
            s.properties.insert("shape".into(), shape.into());
            if on {
                s.properties.insert("powered".into(), "true".into());
            }
            registry.id(&s).unwrap()
        };
        let (rail_ns, rail_ew) = (
            with(rail, "north_south", false),
            with(powered, "east_west", true),
        );
        for x in -30..46 {
            for z in -30..46 {
                game.world.set(x, 63, z, stone).unwrap();
                game.world.set(x, 69, z, stone).unwrap();
            }
        }
        let mut id = 1;
        'park: for x in (-28..44).step_by(2) {
            for z in (-28..44).step_by(2) {
                if id as usize > parked {
                    break 'park;
                }
                game.world.set(x, 64, z, rail_ns).unwrap();
                let at = Vec3::new(f64::from(x) + 0.5, 64.0, f64::from(z) + 0.5);
                game.carts.push(Minecart::new(id, CartKind::Rideable, at));
                id += 1;
            }
        }
        for lane in 0..10 {
            let z = -28 + lane * 6;
            for x in -30..46 {
                game.world.set(x, 70, z, rail_ew).unwrap();
            }
            for k in 0..moving.div_ceil(10) {
                if id as usize > parked + moving {
                    break;
                }
                let at = Vec3::new(-29.5 + f64::from(k as i32) * 3.0, 70.0, f64::from(z) + 0.5);
                let mut c = Minecart::new(id, CartKind::Rideable, at);
                c.velocity = Vec3::new(0.3, 0.0, 0.0);
                game.carts.push(c);
                id += 1;
            }
        }
        let chicken = game
            .res
            .mob_types()
            .get("minecraft:chicken")
            .unwrap()
            .clone();
        for k in 0..n("MOBS", 3000) {
            let (x, z) = (
                -30.0 + (k % 76) as f64 + 0.5,
                -30.0 + ((k / 76) % 76) as f64 + 0.5,
            );
            game.mobs.push(sunrise_sim::mobs::Mob::new(
                1000 + k as u32,
                chicken.clone(),
                Vec3::new(x, 75.0, z),
                0.0,
            ));
        }
        let total = game.carts.len();
        let (warm, ticks) = (5, n("TICKS", 40));
        let mut spent = std::time::Duration::ZERO;
        for t in 0..warm + ticks {
            game.world.game_time += 1;
            // Mobs move every tick on a live server: the list is written.
            if !game.mobs.is_empty() {
                let _ = &mut game.mobs[0];
            }
            let start = std::time::Instant::now();
            game.tick_carts();
            if t >= warm {
                spent += start.elapsed();
            }
        }
        eprintln!(
            "tick_carts: {total} carts ({parked} parked, {moving} moving): {:.3} ms/tick",
            spent.as_secs_f64() * 1e3 / ticks as f64
        );
    }
}
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
