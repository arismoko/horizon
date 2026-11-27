#![cfg(test)]

#[cfg(test)]
mod tests {

    use std::sync::atomic::Ordering;
    use sunrise_core::{geometry::Vec3, position::BlockPos};
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_surface::NativeSurfaceGameExt1;
    use sunrise_gameplay_common::game::native_surface::TryIntoApi;
    use sunrise_gameplay_common::game::side_ops::world::WorldSideGameExt;
    use sunrise_gameplay_common::game::tables::GameItems;
    use sunrise_gameplay_common::game::tables::WorldTablesExt;
    use sunrise_gameplay_common::game_side;
    use sunrise_sim::entities::{ArmorStand, StandMedium};
    use sunrise_sim::motion::{MotionWorld, PlayerMotion};
    use sunrise_world::living_air::{LivingAirWorld, travel_in_air};

    fn item_and_orb(game: &sunrise_gameplay::game::ServerGame<'_>) -> (f64, f64) {
        use sunrise_sim::entities::{ExperienceOrb, ItemEntity};
        let at = Vec3::new(5.5, 65.0, 5.5);
        let mut item = ItemEntity {
            size_answer: None,
            attachment_lifetime: Default::default(),
            rotation: [0.0; 2],
            join: Default::default(),
            health: 5,
            fire_ticks: 0,
            id: 8,
            stack: sunrise_items::items::ItemStack {
                item: "minecraft:stone".into(),
                count: 1,
                damage: 0,
                extra: None,
            },
            thrower: None,
            target: None,
            position: at,
            previous: at,
            velocity: Vec3::new(0.2, -0.04, 0.0),
            ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
            fall_distance: 0.0,
            age: 0,
            stack_revision: 0,
            lifespan: ItemEntity::LIFETIME,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 0,
            needs_sync: false,
            stuck: Vec3::ZERO,
            was_touching_water: false,
            no_gravity: false,
            vertical_collision: false,
            random: sunrise_core::random::LegacyRandom::new(0),
        };
        let step = item.tick_move(
            &game.world,
            game.res.facts(),
            game.collision,
            false,
            (0.0, 0.0),
            &[],
            None,
            None,
            &game.loop_context,
        );
        assert!(item.ground.on_ground);
        assert!(item.tick_settle(
            &game.world,
            game.pack.assets.registry,
            game.res.facts(),
            step,
            &game.loop_context,
            game,
        ));
        let mut orb = ExperienceOrb::new(9, at, Vec3::new(0.2, 0.0, 0.0), 1);
        orb.ground.on_ground = true;
        let (alive, _) = orb.tick_among(
            &game.world,
            game.res.facts(),
            game.collision,
            None,
            false,
            &mut || 0.0,
            None,
            &game.motion.fluids,
            &game.loop_context,
            Some(&game.motion.blocks),
            game,
            &mut |_, _| {},
            &[],
        );
        assert!(alive);
        assert!(orb.ground.on_ground);
        (item.velocity.x, orb.velocity.x)
    }

    #[test]
    #[ignore = "daily"]
    fn a_block_part_answers_the_friction_an_entity_reads() {
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::native_surface::tests::a_block_part_answers_the_friction_an_entity_reads",
            "SUNRISE_FRICTION_TRAVEL_CHILD",
            &["travel"],
        ) {
            return;
        }
        // Glass asks for its material's friction for players. White wool answers 0.98;
        // red wool answers NaN, blue 3.0, green infinity, black -0.5 and orange 1.5.
        let calls = mod_platform::__private::part_tests::friction_game_thread();
        // The client library asks the mods through the executable's callbacks.
        crate::entry::install_client_hooks();
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let floor = BlockPos::new(5, 64, 5);
            // The player's ground travel reads the block through its motion world.
            let player = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                let mut player = PlayerMotion::new(game.motion, Vec3::new(5.5, 65.0, 5.5)).unwrap();
                player.set_walking_speed_base(0.1).unwrap();
                player.ground.on_ground = true;
                player.velocity = Vec3::new(0.2, 0.0, 0.0);
                let (player_uuid, level, session_lock) = game.side_game_side_meta();
                let side = game_side!(
                    game.common,
                    (player_uuid, level, session_lock),
                    game.common.level.enabled_features.as_arc(),
                    session_lock.then(|| game.common.res.items())
                );
                let mut world = MotionWorld {
                    side: &side,
                    entities: &[],
                    player: &mut player,
                    scene: &mut game.common.world,
                    assets: &game.common.pack.assets,
                    collision_binding: game.common.collision,
                    resources: game.common.motion,
                    health: &game.common.health_resources,
                    tag_items: Some(game.common.res.items()),
                    loop_context: &game.common.loop_context,
                    damage_hooks: &game.common.damage_hooks,
                    block_registries: Some(&game.common.ui.registries),
                };
                let coefficient = LivingAirWorld::block_friction(&mut world, floor).unwrap();
                calls.store(0, Ordering::SeqCst);
                travel_in_air(
                    &mut world,
                    &mut sunrise_world::move_gate::MoveGate::new(),
                    Vec3::new(1.0, 0.0, 0.0).into(),
                )
                .unwrap();
                let reads = calls.load(Ordering::SeqCst);
                (
                    coefficient,
                    world.player.position.x - 5.5,
                    world.player.velocity.x,
                    reads,
                )
            };
            // An armor stand moving 0.2 along x on the ground, after one travel: 0.2 × friction × 0.91.
            let travel_stand = |game: &sunrise_gameplay::game::ServerGame<'_>,
                                stand: &mut ArmorStand| {
                let scene = sunrise_level::collisions::SceneCollisions {
                    world: &game.world,
                    binding: game.collision,
                    subject: None,
                };
                let step = stand
                    .tick_move(
                        &scene,
                        game.res.facts(),
                        Some(&game.motion.blocks),
                        &game.motion.climbing,
                        false,
                        None,
                        &[],
                        &game.loop_context,
                        game,
                    )
                    .unwrap();
                stand.tick_settle(StandMedium::Air, step);
            };
            let stand = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                let mut stand = ArmorStand::new(7, Vec3::new(5.5, 65.0, 5.5), 0.0);
                stand.ground.on_ground = true;
                stand.velocity = Vec3::new(0.2, 0.0, 0.0);
                travel_stand(game, &mut stand);
                stand.velocity.x
            };
            let registry = game.pack.assets.registry;
            let material = registry.default_state("minecraft:white_wool").unwrap();
            game.world
                .set(floor.x, floor.y, floor.z + 1, material)
                .unwrap();
            let level = game.level_ref(game.native_lifecycle.epoch().unwrap(), game.dim.key());
            // The player's ground travel moves 0.2 along x with input (1, 0, 0) and a walking speed
            // of 0.1F. Travel clamps the coefficient to 0..=1 (`computeModifiedFriction`) before
            // it uses it, so the speed is scaled by 0.21600002F / friction³ above 0.6 of that
            // clamped value, the distance is 0.2 + that speed, and the velocity after is the
            // distance times (friction × 0.91F). The coefficient the part answered is read as given.
            let mut ice_travel = None;
            for (block, player_friction, expected_distance, expected_velocity, answers) in [
                // No part: the block's own value.
                (
                    "minecraft:stone",
                    0.6_f32,
                    0.3000000014901163_f64,
                    0.16380001983940612_f64,
                    0,
                ),
                (
                    "minecraft:ice",
                    0.98_f32,
                    0.2229496210813524_f64,
                    0.19882648232869793_f64,
                    0,
                ),
                // Glass queries the material at its own position, only for the player.
                (
                    "minecraft:glass",
                    0.98_f32,
                    0.2229496210813524_f64,
                    0.19882648232869793_f64,
                    2,
                ),
                (
                    "minecraft:white_wool",
                    0.98_f32,
                    0.2229496210813524_f64,
                    0.19882648232869793_f64,
                    1,
                ),
                // A finite answer is read as given, whatever its size or sign; travel then clamps it to
                // 0..=1.
                (
                    "minecraft:orange_wool",
                    1.5_f32,
                    0.221600002422929_f64,
                    0.2016560080165566_f64,
                    1,
                ),
                (
                    "minecraft:blue_wool",
                    3.0_f32,
                    0.221600002422929_f64,
                    0.2016560080165566_f64,
                    1,
                ),
                (
                    "minecraft:black_wool",
                    -0.5_f32,
                    0.3000000014901163_f64,
                    0.0_f64,
                    1,
                ),
                // A non-finite answer is dropped for the block's own.
                (
                    "minecraft:red_wool",
                    0.6_f32,
                    0.3000000014901163_f64,
                    0.16380001983940612_f64,
                    1,
                ),
                (
                    "minecraft:green_wool",
                    0.6_f32,
                    0.3000000014901163_f64,
                    0.16380001983940612_f64,
                    1,
                ),
            ] {
                let state = registry.default_state(block).unwrap();
                game.world.set(floor.x, floor.y, floor.z, state).unwrap();
                let (coefficient, distance, velocity, reads) = player(game);
                assert_eq!(coefficient, player_friction, "{block}");
                assert_eq!(distance.to_bits(), expected_distance.to_bits(), "{block}");
                assert_eq!(velocity.to_bits(), expected_velocity.to_bits(), "{block}");
                assert_eq!(reads, answers, "part answers per travel tick: {block}");
                if block == "minecraft:ice" {
                    ice_travel = Some((distance, velocity));
                } else if block == "minecraft:glass" {
                    assert_eq!(Some((distance, velocity)), ice_travel);
                    // LevelRef asks the material at glass's position even without an entity.
                    assert_eq!(
                        level.friction(
                            game,
                            [floor.x, floor.y, floor.z],
                            material.try_into_api().unwrap(),
                            None,
                        ),
                        Ok(Some(0.98)),
                    );
                }
            }
            // An armor stand's velocity after a tick is 0.2 × (friction × 0.91f32), and an
            // item's or orb's 0.2 × (0.98f32 × friction), as the entities scale them. A finite
            // answer is used as given, whatever its size or sign.
            for (block, stand_x, item_x) in [
                (
                    "minecraft:stone",
                    0.10920001268386842_f64,
                    0.11760001182556153_f64,
                ),
                ("minecraft:ice", 0.17836000919342043, 0.19208000898361208),
                // Glass answers for the player only.
                ("minecraft:glass", 0.10920001268386842, 0.11760001182556153),
                (
                    "minecraft:white_wool",
                    0.17836000919342043,
                    0.19208000898361208,
                ),
                (
                    "minecraft:orange_wool",
                    0.27300000190734863,
                    0.29400000572204593,
                ),
                (
                    "minecraft:blue_wool",
                    0.5460000038146973,
                    0.5880000114440919,
                ),
                (
                    "minecraft:black_wool",
                    -0.09100000262260438,
                    -0.09800000190734864,
                ),
                // A non-finite answer is dropped for the block's own.
                (
                    "minecraft:red_wool",
                    0.10920001268386842,
                    0.11760001182556153,
                ),
                (
                    "minecraft:green_wool",
                    0.10920001268386842,
                    0.11760001182556153,
                ),
            ] {
                let state = registry.default_state(block).unwrap();
                game.world.set(floor.x, floor.y, floor.z, state).unwrap();
                assert_eq!(stand(game).to_bits(), stand_x.to_bits(), "{block}");
                let (item_got, orb_got) = item_and_orb(game);
                assert_eq!(item_got.to_bits(), item_x.to_bits(), "item {block}");
                assert_eq!(orb_got.to_bits(), item_x.to_bits(), "orb {block}");
            }

            // Its feet are over the lower path but the edge of its box lands on wool.
            // The next travel reads the wool's Part rather than the path's own friction.
            let path = registry.default_state("minecraft:dirt_path").unwrap();
            game.world.set(floor.x, floor.y, floor.z, material).unwrap();
            game.world.set(floor.x + 1, floor.y, floor.z, path).unwrap();
            let mut edge = ArmorStand::new(7, Vec3::new(6.125, 65.0, 5.5), 0.0);
            edge.velocity = Vec3::new(0.0, -0.08, 0.0);
            travel_stand(game, &mut edge);
            assert_eq!(edge.ground.supporting_block, Some(floor));
            edge.velocity = Vec3::new(0.2, -0.08, 0.0);
            calls.store(0, Ordering::SeqCst);
            travel_stand(game, &mut edge);
            assert_eq!(edge.velocity.x, 0.2 * f64::from(0.98_f32 * 0.91));
            assert_eq!(calls.load(Ordering::SeqCst), 1, "wool's friction Part");
        });
    }
    #[test]
    #[ignore = "daily"]
    fn nested_friction_queries_stop_at_the_depth_bound_and_unknown_states_are_refused() {
        let calls = mod_platform::__private::part_tests::friction_game_thread();
        crate::entry::install_client_hooks();
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let registry = game.pack.assets.registry;
            let lime = registry.default_state("minecraft:lime_wool").unwrap();
            let level = game.level_ref(game.native_lifecycle.epoch().unwrap(), game.dim.key());
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            super::super::native_entity_refs::tests::bind_local_player(game, 100, 100);
            let player = game.entity(100).unwrap();
            calls.store(0, Ordering::SeqCst);
            // Each nested ask names another position, so no query repeats: only the bound stops
            // the chain. The innermost part is refused and answers 0.5, which every part above passes on.
            assert_eq!(
                level.friction(game, [5, 64, 5], lime.try_into_api().unwrap(), Some(player),),
                Ok(Some(0.5)),
            );
            assert_eq!(calls.load(Ordering::SeqCst), 16);
            // Two parts on one block: the second sees the first's coefficient, but not its
            // changes to the position, state or entity.
            let pink = registry.default_state("minecraft:pink_wool").unwrap();
            calls.store(0, Ordering::SeqCst);
            assert_eq!(
                level.friction(game, [5, 64, 5], pink.try_into_api().unwrap(), Some(player),),
                Ok(Some(0.9)),
            );
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            let stale = mod_api::EntityRef::new(
                mod_api::WorldEpoch::new(player.epoch().get() + 1),
                player.id(),
            );
            assert_eq!(
                level.friction(game, [5, 64, 5], lime.try_into_api().unwrap(), Some(stale)),
                Err(mod_api::LevelReadError::Stale),
            );
            assert_eq!(
                calls.load(Ordering::SeqCst),
                2,
                "a stale entity does not dispatch"
            );
            let unknown = sunrise_world::registry::BlockStateId(u32::MAX);
            assert_eq!(
                game.level_friction(level, [5, 64, 5], unknown, None),
                Err(mod_api::LevelReadError::Unavailable),
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_server_friction_part_reads_its_block_entitys_saved_material() {
        use mod_api::{
            Block, BlockEntityEvent, BlockEntityPart, BlockPart, BlockUpdateFlags, BlocksEvent,
            Compound, FrictionEvent, Hooks, LevelSide, ModEntry, Mods, Registrar,
            RegistrationContext, Tag,
        };
        use std::sync::Arc;
        use sunrise_gameplay_common::game::native_surface::FrictionLevel;
        use sunrise_level::chunk::Chunk;
        use {
            sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
            sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
            sunrise_gameplay::game::native_entity_refs::tests::with_game_at,
        };

        if run_fresh_cases(
            "cut_layout::game::native_surface::tests::a_server_friction_part_reads_its_block_entitys_saved_material",
            "SUNRISE_FRICTION_BLOCK_ENTITY_CHILD",
            &["server"],
        ) {
            return;
        }

        const PANEL: &str = "friction_data_test:panel";
        struct Panel;
        struct PanelEntity;
        #[mod_api::__subscriptions]
        impl Hooks for Panel {
            fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
                let panel = registrar.block_key(PANEL);
                registrar.block_part(panel, Panel);
                let kind = registrar
                    .block_entity_type("panel")
                    .blocks(&[PANEL])
                    .build();
                registrar.block_entity_part(&kind, PanelEntity).unwrap();
            }
            fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
                event
                    .register(Block::new(PANEL, "minecraft:stone"))
                    .unwrap();
            }
        }
        #[mod_api::sunrise_part]
        impl BlockPart for PanelEntity {}
        #[mod_api::sunrise_part]
        impl BlockEntityPart for PanelEntity {
            fn block_entity(
                &self,
                _: LevelSide<'_>,
                _: &mut BlockEntityEvent<'_>,
            ) -> Option<Compound> {
                Some(Compound::from([(
                    "Material".into(),
                    Tag::String("minecraft:blue_ice".into()),
                )]))
            }
        }
        #[mod_api::sunrise_part]
        impl BlockPart for Panel {
            fn friction(&self, side: LevelSide<'_>, event: &mut FrictionEvent) {
                let LevelSide::Server(level) = side else {
                    return;
                };
                let pos = [event.pos.x, event.pos.y, event.pos.z];
                let entity = level
                    .block_entity(pos)
                    .expect("the panel has a block entity");
                let Some(data) = level.block_entity_data(entity) else {
                    return;
                };
                let Some(Tag::String(material)) = data.get(&"Material".into()) else {
                    panic!("the saved panel has a material");
                };
                let state = level.block_state_id(&material.to_string_lossy()).unwrap();
                event.friction = level.friction(pos, state, None).unwrap();
            }
        }

        let mods: Mods = vec![ModEntry::new("friction_data_test", Box::new(Panel))].into();
        mod_platform::__private::install(mods.clone());
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        let root =
            sunrise_gameplay_common::game::mods::content_root(&fixture_root(), &Default::default())
                .unwrap();
        with_game_at(root.prepared().to_path_buf(), |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let registry = game.pack.assets.registry;
            let state = registry.default_state(PANEL).unwrap();
            assert_eq!(
                game.level_set_block(level, [3, 64, 3], state, BlockUpdateFlags::NONE),
                Ok(true)
            );
            let facts = game.res.facts();
            let side = game_side!(game, game.side_game_side_meta());
            let view = FrictionLevel {
                world: &game.world,
                registry,
                context: &game.loop_context,
                side: &side,
                coefficient: &|id| facts.states.get(id.0 as usize).map(|facts| facts.friction),
            };
            let mut value = facts.get(state).friction;
            assert!(sunrise_gameplay_common::game::native_surface::friction(
                view,
                None,
                BlockPos::new(3, 64, 3),
                state,
                &mut value
            ));
            assert_eq!(value, 0.989);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_friction_view_reads_tags_and_resident_block_entities_of_its_side() {
        use mod_api::{
            __private::game::{Fields, block_entity_ref},
            LevelHost, Tag,
        };
        use sunrise_gameplay::game::native_entity_refs::tests::with_game;
        use {
            sunrise_gameplay_common::game::native_surface::FrictionLevel,
            sunrise_gameplay_common::game::native_surface::FrictionView,
        };
        let _parts = mod_platform::__private::part_tests::friction_game_thread();
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let dir = tempfile::tempdir().unwrap();
            game.test_acquire_session_lock(dir.path());
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.set(3, 64, 3, chest).unwrap();
            game.world.block_entities.insert(
                (3, 64, 3),
                sunrise_level::containers::BlockEntity::Chest(vec![None; 27]),
            );
            let side = game_side!(game, game.side_game_side_meta());
            let coefficient = |_: sunrise_world::registry::BlockStateId| None;
            let view = FrictionView(
                FrictionLevel {
                    world: &game.world,
                    registry: game.pack.assets.registry,
                    context: &game.loop_context,
                    side: &side,
                    coefficient: &coefficient,
                },
                None,
            );
            // A tagged block, an untagged one and a fluid tag answer as the game's own host does.
            let stone = mod_platform::__private::part_tests::block_key("stone");
            let water = mod_api::__private::fluid_key("minecraft:water").unwrap();
            let base_stone = view
                .tags()
                .key(&mod_api::Identifier::parse("base_stone_overworld").unwrap())
                .unwrap()
                .typed();
            let climbable = view
                .tags()
                .key(&mod_api::Identifier::parse("minecraft:climbable").unwrap())
                .unwrap()
                .typed();
            let water_tag = view
                .tags()
                .key(&mod_api::Identifier::parse("minecraft:water").unwrap())
                .unwrap()
                .typed();
            assert_eq!(stone.is_in_tag(view.tags(), base_stone), Some(true));
            assert_eq!(
                stone.is_in_tag(view.tags(), base_stone),
                stone.is_in_tag(&*game, base_stone)
            );
            assert_eq!(stone.is_in_tag(view.tags(), climbable), Some(false));
            assert_eq!(water.is_in_tag(view.tags(), water_tag), Some(true));
            let stone_state = view.block_state_default(stone).unwrap();
            let view_states = view.block_states();
            assert_eq!(
                stone_state.outline(&view_states),
                Some(&[[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]][..])
            );
            assert_eq!(
                water.pickup_sound(&view_states),
                Some("minecraft:item.bucket.fill")
            );
            // A resident block entity has the handle of this side's level; an empty position none.
            let level = game.level_ref(game.native_lifecycle.epoch().unwrap(), game.dim.key());
            let found = view.block_entity([3, 64, 3]).unwrap();
            assert_eq!(Some(found), game.block_entity(level, [3, 64, 3]));
            let saved = view.block_entity_data(found).unwrap();
            assert_eq!(saved.get(&"Items".into()), Some(&Tag::List(Vec::new())));
            let fields = found.fields();
            let other_level = game.level_ref(level.epoch(), mod_api::DimensionKey::NETHER);
            let other_entity = block_entity_ref(other_level, fields.position, fields.generation);
            assert_eq!(view.block_entity_data(other_entity), None);
            assert_eq!(view.block_entity([4, 64, 3]), None);
            assert_eq!(
                view.enabled_features().iter().collect::<Vec<_>>(),
                game.enabled_feature_ids()
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn mob_travel_and_both_leash_sites_use_the_parts_answer_once() {
        use sunrise_gameplay_common::game::boats_common::Boat;
        use sunrise_sim::mobs::{Mob, leash};
        let calls = mod_platform::__private::part_tests::friction_game_thread();
        crate::entry::install_client_hooks();
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let at = Vec3::new(5.5, 65.0, 5.5);
            let horse = game.res.mob_types().get("minecraft:horse").unwrap().clone();
            let ghast = game.res.mob_types().get("minecraft:ghast").unwrap().clone();
            game.mobs.push(Mob::new(99, ghast, at, 0.0));
            let mut damage = sunrise_sim::health::TestLevel::new(0);
            for (block, drag, spin, listens) in [
                ("minecraft:stone", 0.10920001, 1.0920001, 0),
                ("minecraft:white_wool", 0.17836001, 1.7836001, 1),
            ] {
                let state = game.pack.assets.registry.default_state(block).unwrap();
                game.world.set(5, 64, 5, state).unwrap();
                let snapshot: Vec<_> = game
                    .mobs
                    .iter()
                    .map(|m| m.seen(None, game.items()))
                    .collect();
                {
                    let world = game.mob_world_base(&snapshot);
                    let mut mob = Mob::new(10, horse.clone(), at, 0.0);
                    mob.ground.on_ground = true;
                    mob.velocity = Vec3::new(0.2, 0.0, 0.0);
                    calls.store(0, Ordering::SeqCst);
                    mob.travel(&world, &mut damage.level(), Vec3::ZERO, &mut Vec::new());
                    assert!(
                        (mob.velocity.x - drag).abs() < 1.0e-7,
                        "mob {block}: {}",
                        mob.velocity.x
                    );
                    assert_eq!(
                        calls.load(Ordering::SeqCst),
                        listens,
                        "one answer per mob travel tick"
                    );
                    mob.position = at;
                    mob.ground.on_ground = true;
                    mob.velocity = Vec3::ZERO;
                    mob.extra.leash = Some(leash::Holder::Mob(99));
                    mob.extra.leash_spin = 2.0;
                    calls.store(0, Ordering::SeqCst);
                    leash::tick(&mut mob, &world, &mut Vec::new());
                    assert!(
                        (mob.extra.leash_spin - spin).abs() < 1.0e-6,
                        "mob lead {block}: {}",
                        mob.extra.leash_spin
                    );
                    assert_eq!(calls.load(Ordering::SeqCst), listens);
                }
                let mut boat = Boat::new(11, "oak".into(), at, 0.0);
                boat.ground.on_ground = true;
                boat.leash_spin = 2.0;
                boat.leash = Some(leash::Holder::Mob(99));
                calls.store(0, Ordering::SeqCst);
                game.server_tick_boat_leash(&mut boat);
                assert!(
                    (boat.leash_spin - spin).abs() < 1.0e-6,
                    "boat lead {block}: {}",
                    boat.leash_spin
                );
                assert_eq!(calls.load(Ordering::SeqCst), listens);
            }

            let registry = game.pack.assets.registry;
            let wool = registry.default_state("minecraft:white_wool").unwrap();
            let path = registry.default_state("minecraft:dirt_path").unwrap();
            game.world.set(5, 64, 5, wool).unwrap();
            game.world.set(6, 64, 5, path).unwrap();
            let world = game.mob_world_base(&[]);
            let mut edge = Mob::new(12, horse, Vec3::new(6.25, 65.0, 5.5), 0.0);
            edge.velocity = Vec3::new(0.0, -0.08, 0.0);
            edge.travel(&world, &mut damage.level(), Vec3::ZERO, &mut Vec::new());
            assert_eq!(edge.ground.supporting_block, Some(BlockPos::new(5, 64, 5)));
            edge.velocity = Vec3::new(0.2, -0.08, 0.0);
            calls.store(0, Ordering::SeqCst);
            edge.travel(&world, &mut damage.level(), Vec3::ZERO, &mut Vec::new());
            assert_eq!(edge.velocity.x, 0.2 * f64::from(0.98_f32 * 0.91));
            assert_eq!(calls.load(Ordering::SeqCst), 1, "wool's friction Part");
        });
    }
}
