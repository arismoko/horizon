#![cfg(test)]
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::entity::EntityHandle;
#[cfg(test)]
pub(crate) mod order_tests {
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;
    use sunrise_core::{geometry::Direction, position::BlockPos};
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_gameplay_common::game::piston_push::PistonPushGameExt1;
    use sunrise_level::chunk::Chunk;
    use sunrise_world::level::pistons::MovingPiston;
    use {
        crate::game::load_resources, sunrise_client_render::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    #[test]
    #[ignore = "daily"]
    fn a_piston_pushes_a_resident_armor_stand_with_zero_health() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            let registry = &game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let moving = registry.default_state("minecraft:moving_piston").unwrap();
            let mut chunk = Chunk::empty(0, 0);
            chunk.set(5, 200, 5, moving.0);
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            assert!(game.run_command("summon armor_stand 5.5 200 5.5 {Health:0f}"));
            assert_eq!(game.armor_stands[0].health, 0.0);

            let pos = BlockPos::new(5, 200, 5);
            let piston = MovingPiston {
                moved_state: stone,
                direction: Direction::East,
                extending: true,
                source: false,
                progress: 0.0,
                progress_o: 0.0,
                last_ticked: 0,
                death_ticks: 0,
                id: 1,
            };
            game.block_ticks.moving_pistons = vec![(
                pos,
                MovingPiston {
                    progress: 0.5,
                    ..piston
                },
            )];
            game.sync_moving_pistons();
            game.push_piston_entities(vec![(pos, piston, 0.5)]);
            assert_eq!(game.armor_stands[0].position.x, 5.5 + (0.25 + 0.01));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_pushed_entity_meets_the_other_moving_blocks_as_they_are_at_its_push() {
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
            std::env::temp_dir().join(format!("sunrise-piston-order-unit-{}", std::process::id())),
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
        game.build_registries();
        game.set_allow_commands(true);
        let registry = &game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let moving = registry.default_state("minecraft:moving_piston").unwrap();
        let mut chunk = Chunk::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                chunk.set(x, 199, z, stone.0);
            }
        }
        chunk.set(5, 200, 5, moving.0);
        chunk.set(6, 200, 4, moving.0);
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        let piston = |id, direction, progress| MovingPiston {
            moved_state: stone,
            direction,
            extending: true,
            source: false,
            progress,
            progress_o: progress,
            last_ticked: 0,
            death_ticks: 0,
            id,
        };
        let mut moved = Vec::new();
        for far_first in [true, false] {
            let near = (BlockPos::new(5, 200, 5), piston(1, Direction::East, 0.0));
            let far = (BlockPos::new(6, 200, 4), piston(2, Direction::North, 0.5));
            game.block_ticks.moving_pistons = if far_first {
                vec![far, near]
            } else {
                vec![near, far]
            };
            game.sync_moving_pistons();
            game.tnts.clear();
            assert!(game.run_command("summon tnt 5.5 200 5.5 {fuse:80,Motion:[0.0d,0.0d,0.0d]}"));
            let before = game.tnts[0].position.x;
            let pushes: Vec<_> = game
                .block_ticks
                .moving_pistons
                .iter()
                .map(|&(pos, old)| (pos, old, old.progress + 0.5))
                .collect();
            for (_, piston) in &mut game.block_ticks.moving_pistons {
                piston.progress += 0.5;
            }
            game.push_piston_entities(pushes);
            moved.push(game.tnts[0].position.x - before);
        }
        let expected = f64::from(0.98_f32) * 0.5 + 0.01;
        assert!(
            (moved[0] - expected).abs() < 1e-9,
            "far stone gone first: {}",
            moved[0]
        );
        assert!(moved[1] < 0.02, "far stone still in the way: {}", moved[1]);
    }

    #[test]
    #[ignore = "daily"]
    fn a_cancelled_push_spends_none_of_the_piston_budget() {
        use mod_api::{EntityMoveEvent, Flow, Hooks, LoopContext};
        struct Gate(std::sync::Arc<std::sync::atomic::AtomicBool>);
        #[mod_api::__subscriptions]
        impl Hooks for Gate {
            fn entity_move(&self, _: &dyn LoopContext, _: &mut EntityMoveEvent<'_>) -> Flow {
                if self.0.load(std::sync::atomic::Ordering::Relaxed) {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
        }
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "cancel_gate",
            Box::new(Gate(cancel.clone())),
        )]
        .into();
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
            std::env::temp_dir().join(format!("sunrise-piston-budget-unit-{}", std::process::id())),
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
        game.build_registries();
        game.set_allow_commands(true);
        let registry = &game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let moving = registry.default_state("minecraft:moving_piston").unwrap();
        let mut chunk = Chunk::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                chunk.set(x, 199, z, stone.0);
            }
        }
        chunk.set(5, 200, 5, moving.0);
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        let piston = |id, direction, progress| MovingPiston {
            moved_state: stone,
            direction,
            extending: true,
            source: false,
            progress,
            progress_o: progress,
            last_ticked: 0,
            death_ticks: 0,
            id,
        };
        mod_platform::testing::with_mods(mods, || {
            let mut travelled = Vec::new();
            for cancelled in [true, false] {
                cancel.store(cancelled, std::sync::atomic::Ordering::Relaxed);
                game.block_ticks.moving_pistons =
                    vec![(BlockPos::new(5, 200, 5), piston(1, Direction::East, 0.0))];
                game.sync_moving_pistons();
                game.tnts.clear();
                assert!(
                    game.run_command("summon tnt 5.5 200 5.5 {fuse:80,Motion:[0.0d,0.0d,0.0d]}")
                );
                let before = game.tnts[0].position.x;
                let pushes: Vec<_> = game
                    .block_ticks
                    .moving_pistons
                    .iter()
                    .map(|&(pos, old)| (pos, old, old.progress + 0.5))
                    .collect();
                for (_, piston) in &mut game.block_ticks.moving_pistons {
                    piston.progress += 0.5;
                }
                game.push_piston_entities(pushes);
                travelled.push(game.tnts[0].position.x - before);
            }
            assert!(
                travelled[0].abs() < 1e-9,
                "the cancelled push moved the TNT: {}",
                travelled[0]
            );
            assert!(
                travelled[1] > 0.4,
                "the second push of the same tick lost the budget: {}",
                travelled[1]
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_pushed_tnt_loses_velocity_into_a_wall() {
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
        options.game_dir = Some(std::env::temp_dir().join(format!(
            "sunrise-piston-velocity-unit-{}",
            std::process::id()
        )));
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
        game.build_registries();
        game.set_allow_commands(true);
        let registry = &game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let moving = registry.default_state("minecraft:moving_piston").unwrap();
        let mut chunk = Chunk::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                chunk.set(x, 199, z, stone.0);
            }
        }
        chunk.set(6, 200, 5, stone.0);
        chunk.set(5, 200, 5, moving.0);
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        let piston = MovingPiston {
            moved_state: stone,
            direction: Direction::East,
            extending: true,
            source: false,
            progress: 0.0,
            progress_o: 0.0,
            last_ticked: 0,
            death_ticks: 0,
            id: 1,
        };
        game.block_ticks.moving_pistons = vec![(BlockPos::new(5, 200, 5), piston)];
        game.sync_moving_pistons();
        game.tnts.clear();
        assert!(game.run_command("summon tnt 5.5 200 5.5 {fuse:80,Motion:[0.4d,0.0d,0.0d]}"));
        assert!(
            (game.tnts[0].velocity.x - 0.4).abs() < 1.0e-9,
            "TNT starts with vx 0.4, got {:?}",
            game.tnts[0].velocity
        );
        let pushes: Vec<_> = game
            .block_ticks
            .moving_pistons
            .iter()
            .map(|&(pos, old)| (pos, old, old.progress + 0.5))
            .collect();
        for (_, piston) in &mut game.block_ticks.moving_pistons {
            piston.progress += 0.5;
        }
        game.push_piston_entities(pushes);
        assert!(
            game.tnts[0].velocity.x.abs() < 1.0e-9,
            "TNT velocity.x should be zeroed by the wall, got {:?}",
            game.tnts[0].velocity
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_pushed_tnt_bounces_on_slime() {
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
            std::env::temp_dir().join(format!("sunrise-piston-slime-unit-{}", std::process::id())),
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
        game.build_registries();
        game.set_allow_commands(true);
        let registry = &game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let slime = registry.default_state("minecraft:slime_block").unwrap();
        let moving = registry.default_state("minecraft:moving_piston").unwrap();
        let mut chunk = Chunk::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                chunk.set(x, 199, z, stone.0);
            }
        }
        chunk.set(5, 199, 5, slime.0);
        chunk.set(5, 200, 5, moving.0);
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        let piston = MovingPiston {
            moved_state: stone,
            direction: Direction::Down,
            extending: true,
            source: false,
            progress: 0.0,
            progress_o: 0.0,
            last_ticked: 0,
            death_ticks: 0,
            id: 1,
        };
        for (floor, y, vy, final_y, final_vy) in [
            (slime, 200.5, -0.4, 200.0, 0.3510000138133766_f64),
            (stone, 200.5, -0.4, 200.0, 0.0),
            (slime, 200.0, -0.4, 200.0, 0.3200000047683716),
            (slime, 200.00001, -0.4, 200.00001, 0.32000064476813395),
            (slime, 200.5, 0.3, 200.0, -0.0),
        ] {
            game.world.set(5, 199, 5, floor).unwrap();
            game.block_ticks.moving_pistons = vec![(BlockPos::new(5, 200, 5), piston)];
            game.sync_moving_pistons();
            game.tnts.clear();
            assert!(game.run_command(&format!(
                "summon tnt 5.5 {y} 5.5 {{fuse:80,Motion:[0.0d,{vy}d,0.0d]}}"
            )));
            assert_eq!(game.tnts[0].velocity.y, vy);
            let pushes: Vec<_> = game
                .block_ticks
                .moving_pistons
                .iter()
                .map(|&(pos, old)| (pos, old, old.progress + 0.5))
                .collect();
            for (_, piston) in &mut game.block_ticks.moving_pistons {
                piston.progress += 0.5;
            }
            game.push_piston_entities(pushes);
            assert_eq!(game.tnts[0].position, Vec3::new(5.5, final_y, 5.5));
            assert_eq!(game.tnts[0].velocity.y.to_bits(), final_vy.to_bits());
            assert!(game.tnts[0].ground.on_ground);
        }
    }
}

#[cfg(test)]
pub(crate) mod h255_mob_piston_ground_tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_core::geometry::Direction;
    use sunrise_core::{geometry::Vec3, position::ChunkPos, random::RandomSource};
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_gameplay_common::game::move_gate::MoveWho;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::motion::PlayerMotion;
    use {
        crate::game::load_resources, sunrise_client_render::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    pub(crate) fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        with_role_game(
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            test,
        );
    }

    pub(crate) fn with_role_game(
        role: sunrise_gameplay_common::game::execution_role::ExecutionRole,
        test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
    ) {
        mod_platform::testing::with_mods(Default::default(), || {
            let root =
                std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
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
                    .join(format!("h255-{:?}", std::thread::current().id())),
            );
            options.execution_role = role;
            let mut game = sunrise_gameplay::game::ServerGame::new(
                &loaded.resources,
                loaded.pack,
                options,
                settings.sim_settings(),
                None,
                crate::game::profile_resolver,
            )
            .unwrap();
            game.native_lifecycle
                .begin_open(Some(mod_api::Authority::IntegratedServer));
            game.build_registries();
            game.set_allow_commands(true);

            let mut center = Chunk::empty(0, 0);
            center.lit = true;
            game.world.chunks.insert((0, 0), Arc::new(center));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            assert!(game.run_command("difficulty normal"));
            assert!(game.run_command("forceload add 0 0"));
            for x in -1..=1 {
                for z in -1..=1 {
                    if x == 0 && z == 0 {
                        continue;
                    }
                    let mut chunk = Chunk::empty(x, z);
                    chunk.lit = true;
                    game.world.chunks.insert((x, z), Arc::new(chunk));
                    game.block_ticks
                        .add_empty_chunk(ChunkPos::new(x, z))
                        .unwrap();
                }
            }
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
            game.load_ticking_neighbourhood(0, 0);
            game.creative = true;
            assert!(game.run_command("fill 0 199 0 31 199 31 stone"));
            game.world.random.set_seed(255);
            game.update_chunk_tickets();
            assert!(crate::game::ticking_chunk(&game.world, 0, 0));
            assert!(game.server.tickets.0.levels().block_ticking(0, 0));
            test(&mut game);
        });
    }

    pub(crate) fn mob_index(game: &sunrise_gameplay::game::ServerGame<'_>, n: i32) -> usize {
        let id = game
            .persist
            .uuids
            .iter()
            .find_map(|(id, u)| (*u == [255, 1, 0, n]).then_some(*id))
            .unwrap();
        game.mobs.iter().position(|m| m.id == id).unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn h255_real_honey_piston_lifecycle_clears_mob_ground_before_second_drag() {
        with_game(|game| {
            game.world.game_time = 0;
            for cmd in [
                "gamerule spawn_mobs false",
                "gamerule random_tick_speed 0",
                "gamerule advance_time false",
                "setblock 3 201 4 piston[facing=east]",
                "setblock 4 201 4 honey_block",
                "setblock 4 201 12 honey_block",
                "setblock 12 200 12 piston[facing=up]",
                "setblock 12 201 12 stone",
                "summon husk 4.5 202 4.5 {UUID:[I;255,1,0,1],Health:10f,NoAI:1b,NoGravity:1b,OnGround:1b,Motion:[0d,0d,0d]}",
                "summon husk 4.5 202 12.5 {UUID:[I;255,1,0,2],Health:10f,NoAI:1b,NoGravity:1b,OnGround:1b,Motion:[0d,0d,0d]}",
                "summon husk 12.5 202 12.5 {UUID:[I;255,1,0,3],Health:10f,NoAI:1b,NoGravity:1b,OnGround:0b,Motion:[0d,0d,0d]}",
            ] {
                assert!(game.run_command(cmd), "{cmd}");
            }
            let main = mob_index(game, 1);
            let idle = mob_index(game, 2);
            let vertical = mob_index(game, 3);
            let id = game.mobs[main].id;
            let age = game.mobs[main].age;
            assert!(game.mobs[main].ground.on_ground);
            assert!(game.mobs[idle].ground.on_ground);
            assert!(!game.mobs[vertical].ground.on_ground);
            assert_eq!(game.mobs[main].position, Vec3::new(4.5, 202.0, 4.5));
            assert_eq!(
                game.res
                    .facts()
                    .get(game.world.block(4, 201, 4))
                    .name
                    .as_ref(),
                "minecraft:honey_block"
            );
            assert!(game.run_command("setblock 3 201 5 redstone_block"));
            assert!(game.run_command("setblock 13 200 12 redstone_block"));
            let mut failures = Vec::new();
            for tick in 1..=2 {
                game.world.game_time += 1;
                game.update_chunk_tickets();
                assert!(game.entity_ticking_area()(4, 4));
                game.tick_blocks().unwrap();
                game.tick_creature_entities().unwrap();
                game.tick_moving_pistons().unwrap();
                let p = game
                    .block_ticks
                    .moving_pistons
                    .iter()
                    .find(|(p, _)| *p == BlockPos::new(5, 201, 4))
                    .expect("real moving honey block");
                assert_eq!(p.1.progress, 0.5 * tick as f32);
                assert_eq!(
                    p.1.moved_state,
                    game.pack
                        .assets
                        .registry
                        .default_state("minecraft:honey_block")
                        .unwrap()
                );
                assert_eq!(game.mobs[main].id, id);
                assert_eq!(game.uuid_for(id), [255, 1, 0, 1]);
                assert_eq!(game.mobs[main].age, age + tick);
                assert!(game.mobs[main].no_ai);
                assert_eq!(game.mobs[main].health, 10.0);
                assert_eq!(game.mobs[main].velocity, Vec3::ZERO);
                if game.mobs[main].ground.on_ground {
                    failures.push(format!("tick{tick}: actual Mob still grounded"));
                }
                if game.mobs[main].position != Vec3::new(5.0, 202.0, 4.5) {
                    failures.push(format!(
                        "tick{tick}: position {:?}",
                        game.mobs[main].position
                    ));
                }
                if game.mob_tree_nbt(main).get("OnGround") != Some(&sunrise_core::nbt::Tag::Byte(0))
                {
                    failures.push(format!("tick{tick}: saved ground is stale"));
                }
                assert_eq!(game.mobs[idle].position, Vec3::new(4.5, 202.0, 12.5));
                assert!(game.mobs[idle].ground.on_ground);
                assert_eq!(
                    game.mobs[vertical].position,
                    Vec3::new(12.5, if tick == 1 { 202.51 } else { 203.01 }, 12.5)
                );
                assert!(!game.mobs[vertical].ground.on_ground);
            }
            assert!(
                failures.is_empty(),
                "actual lifecycle failures: {failures:#?}"
            );
        });
    }

    pub(crate) fn added_mob(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        n: i32,
        kind: &str,
        no_ai: bool,
        x: f64,
    ) -> usize {
        let tame = if kind == "horse" { ",Tame:1b" } else { "" };
        assert!(game.run_command(&format!(
            "summon {kind} {x} 202 4.5 {{UUID:[I;255,1,0,{n}],Health:10f,NoAI:{}b,NoGravity:1b,OnGround:1b,Motion:[0d,0d,0d]{tame}}}",
            i32::from(no_ai)
        )));
        let i = mob_index(game, n);
        assert_eq!(game.mobs[i].no_ai, no_ai);
        assert!(game.mobs[i].ground.on_ground);
        assert_eq!(game.uuid_for(game.mobs[i].id), [255, 1, 0, n]);
        i
    }

    pub(crate) fn added_push(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        i: usize,
        movement: Vec3,
    ) {
        let m = &game.mobs[i];
        let (bounds, at, step, ground) = (
            m.bounds(),
            [m.position.x, m.position.z],
            m.kind.step_height as f32,
            m.ground.on_ground,
        );
        game.move_pushed(
            MoveWho::Entity(EntityHandle::Mob(i)),
            bounds,
            at,
            step,
            ground,
            movement,
            Direction::East,
            &mut std::collections::HashMap::new(),
            &mut Vec::new(),
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_stuck_arrow_whose_push_a_wall_stops_falls_when_nothing_holds_it() {
        with_game(|game| {
            assert!(game.run_command("setblock 6 202 4 minecraft:stone"));
            game.creatures.projectiles.clear();
            assert!(game.run_command("summon minecraft:arrow 5.75 202.5 4.5 {NoGravity:0b}"));
            let i = game
                .creatures
                .projectiles
                .iter()
                .position(|p| p.arrow.is_some())
                .expect("the arrow");
            game.creatures.projectiles.update(i, |p| {
                p.velocity = Vec3::ZERO;
                p.stuck = true;
                p.arrow.as_mut().unwrap().in_ground = true;
            });
            let p = &game.creatures.projectiles[i];
            let (bounds, at) = (p.bounds(), [p.position.x, p.position.z]);
            let before = p.position;
            game.move_pushed(
                MoveWho::Entity(EntityHandle::Projectile(i)),
                bounds,
                at,
                0.0,
                false,
                Vec3::new(0.3, 0.0, 0.0),
                Direction::East,
                &mut std::collections::HashMap::new(),
                &mut Vec::new(),
            );
            let p = &game.creatures.projectiles[i];
            assert_eq!(p.position, before, "the stone stops the push");
            assert!(
                !p.arrow.as_ref().unwrap().in_ground,
                "nothing holds it, so it falls"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn h255_added_tnt_authority_and_exhausted_budget() {
        use sunrise_gameplay_common::game::execution_role::ExecutionRole;
        for role in [
            ExecutionRole::IntegratedServer,
            ExecutionRole::DedicatedServer,
        ] {
            with_role_game(role, |game| {
                assert!(game.run_command("setblock 5 199 5 slime_block"));
                assert!(
                    game.run_command("summon tnt 5.5 200.5 5.5 {fuse:80,Motion:[0d,-0.4d,0d]}")
                );
                let mut budget = std::collections::HashMap::new();
                let push = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                            budget: &mut std::collections::HashMap<_, _>,
                            movement| {
                    let tnt = &game.tnts[0];
                    game.move_pushed(
                        MoveWho::Entity(EntityHandle::Tnt(0)),
                        tnt.bounds(),
                        [tnt.position.x, tnt.position.z],
                        0.0,
                        tnt.ground.on_ground,
                        movement,
                        Direction::Down,
                        budget,
                        &mut Vec::new(),
                    );
                };
                push(game, &mut budget, Vec3::ZERO);
                assert_eq!(game.tnts[0].position.y, 200.5);
                assert_eq!(game.tnts[0].velocity.y, -0.4);
                push(game, &mut budget, Vec3::new(0.0, -0.51, 0.0));
                let expected = 0.3510000138133766_f64;
                assert_eq!(game.tnts[0].velocity.y.to_bits(), expected.to_bits());
                game.tnts[0].position.y = 200.5;
                game.tnts[0].velocity.y = -0.4;
                push(game, &mut budget, Vec3::new(0.0, -0.51, 0.0));
                assert_eq!(game.tnts[0].position.y, 200.5);
                assert_eq!(game.tnts[0].velocity.y, -0.4);
            });
        }
    }

    #[test]
    #[ignore = "daily"]
    fn h255_added_no_request_and_exhausted_budget_keep_state() {
        with_game(|game| {
            let i = added_mob(game, 20, "horse", true, 8.5);
            added_push(game, i, Vec3::ZERO);
            assert!(game.mobs[i].ground.on_ground);
            assert_eq!(game.mobs[i].position, Vec3::new(8.5, 202.0, 4.5));
            game.mobs[i].set_saddle(Some(sunrise_items::items::ItemStack::new(
                "minecraft:saddle",
                1,
            )));
            assert!(game.run_command("ride @s mount 000000ff-0000-0001-0000-000000000014"));
            assert_eq!(game.riding, Some(game.mobs[i].id));
            added_push(game, i, Vec3::new(0.51, 0.0, 0.0));
            assert!(
                game.mobs[i].ground.on_ground,
                "occupied path remains excluded"
            );
            assert_eq!(game.mobs[i].position, Vec3::new(9.01, 202.0, 4.5));
            assert!(game.run_command("setblock 9 201 4 slime_block"));
            game.mobs[i].position.y = 202.25;
            game.mobs[i].velocity = Vec3::new(0.0, -1.0, 0.0);
            game.mobs[i].sync_position = false;
            added_push(game, i, Vec3::new(0.0, -0.5, 0.0));
            assert_eq!(
                game.mobs[i].velocity.y, -1.0,
                "the controlling rider's client owns restitution"
            );
            assert_eq!(game.mobs[i].position.y, 202.0);
            assert!(
                game.mobs[i].ground.on_ground,
                "even the observer records vertical collision"
            );
            assert!(!game.mobs[i].sync_position);
            assert!(game.run_command("ride @s dismount"));
            assert!(game.riding.is_none());
            assert!(game.mobs[i].rider.is_none());
            assert!(game.mobs[i].extra.rider.is_none());
            assert!(game.mobs[i].extra.mount.is_none());
            assert!(!game.mobs[i].extra.passenger);
            let before = game.mobs[i].position;
            added_push(game, i, Vec3::new(0.25, 0.0, 0.0));
            assert_eq!(
                game.mobs[i].position, before,
                "real same-tick piston budget is exhausted"
            );
            assert!(
                game.mobs[i].ground.on_ground,
                "limitedZERO precedes the ground update"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn h255_added_blocked_request_updates_ground_before_tiny_return() {
        with_game(|game| {
            assert!(game.run_command("fill 9 202 4 9 204 4 stone"));
            let width = game.res.mob_types().get("minecraft:husk").unwrap().width as f32;
            let x = 9.0 - f64::from(width) * 0.5;
            let i = added_mob(game, 21, "husk", true, x);
            assert_eq!(game.mobs[i].bounds().max_position().x, 9.0);
            let before = game.mobs[i].position;
            added_push(game, i, Vec3::new(0.25, 0.0, 0.0));
            assert_eq!(
                game.mobs[i].position, before,
                "actual loaded wall blocks the nonzero request"
            );
            assert!(
                !game.mobs[i].ground.on_ground,
                "Entity.move ground ordering, source/unit-only coverage"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn h255_added_client_role_ai_and_vertical_ground_state() {
        use sunrise_gameplay_common::game::execution_role::ExecutionRole as Role;
        for (role, no_ai, movement, on_ground) in [
            (
                Role::IntegratedServer,
                false,
                Vec3::new(0.25, 0.0, 0.0),
                false,
            ),
            (
                Role::IntegratedServer,
                true,
                Vec3::new(0.0, 0.25, 0.0),
                false,
            ),
        ] {
            with_role_game(role, |game| {
                let i = added_mob(game, 22, "husk", no_ai, 8.5);
                let before = game.mobs[i].position;
                added_push(game, i, movement);
                assert_eq!(game.mobs[i].position, before.add_vector(movement));
                assert_eq!(
                    game.mobs[i].ground.on_ground, on_ground,
                    "{role:?}, NoAI={no_ai}"
                );
            });
        }
    }

    #[test]
    #[ignore = "daily"]
    fn h255_added_partial_push_reads_slime_after_the_position_changes() {
        with_game(|game| {
            assert!(game.run_command("setblock 8 201 4 slime_block"));
            let i = added_mob(game, 23, "iron_golem", true, 8.5);
            game.mobs[i].position.y = 202.25;
            game.mobs[i].velocity = Vec3::new(0.0, -1.0, 0.0);
            game.mobs[i].sync_position = false;
            added_push(game, i, Vec3::new(0.0, -0.5, 0.0));
            assert_eq!(game.mobs[i].position, Vec3::new(8.5, 202.0, 4.5));
            assert!(game.mobs[i].velocity.y > 0.9);
            assert!(game.mobs[i].sync_position);
        });
    }
}

#[cfg(test)]
pub(crate) mod lid_push_tests {
    use super::h255_mob_piston_ground_tests::with_game;
    use sunrise_gameplay_common::game::creatures::ModGameExt1;
    use {
        sunrise_gameplay_common::game::entity::EntityHandle,
        sunrise_gameplay_common::game::piston_push::Lid,
    };

    use sunrise_core::geometry::{Aabb, Vec3};
    use sunrise_sim::mobs::{MobEvent, Species};

    fn shulker(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> u32 {
        assert!(game.run_command("summon minecraft:shulker 24 200 24 {NoAI:1b}"));
        game.mobs
            .iter()
            .find(|m| m.species() == Species::Shulker)
            .expect("the shulker")
            .id
    }

    fn area(x0: f64, z0: f64, x1: f64, z1: f64) -> Aabb {
        Aabb::new(Vec3::new(x0, 200.0, z0), Vec3::new(x1, 201.0, z1))
    }

    fn item_y(game: &sunrise_gameplay::game::ServerGame<'_>) -> f64 {
        game.dropped.iter().next().expect("the item").position.y
    }

    fn arrow_y(game: &sunrise_gameplay::game::ServerGame<'_>) -> f64 {
        let arrow = game
            .creatures
            .projectiles
            .iter()
            .find(|p| p.arrow.is_some());
        arrow.expect("the arrow").position.y
    }

    #[test]
    #[ignore = "daily"]
    fn forced_living_moves_land_booted_bodies_on_powder_snow() {
        use sunrise_core::position::BlockPos;
        use sunrise_world::entity_move::MoverType;

        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::piston_push_server::lid_push_tests::forced_living_moves_land_booted_bodies_on_powder_snow",
            || {
                with_game(|game| {
                    assert!(game.run_command("fill 8 200 8 15 200 8 powder_snow"));
                    for (i, cause) in [MoverType::Piston, MoverType::ShulkerBox]
                        .into_iter()
                        .enumerate()
                    {
                        let x = 8 + 4 * i as i32;
                        for (kind, x) in [("armor_stand", x), ("zombie", x + 2)] {
                            assert!(game.run_command(&format!(
                                r#"summon {kind} {x}.5 201.25 8.5 {{NoAI:1b,equipment:{{feet:{{id:"minecraft:leather_boots",count:1}}}}}}"#,
                            )));
                        }
                        for (handle, x) in [
                            (EntityHandle::ArmorStand(i), x),
                            (EntityHandle::Mob(i), x + 2),
                        ] {
                            let moved =
                                game.move_listed_entity(handle, cause, Vec3::new(0.0, -0.5, 0.0));
                            assert_eq!(moved.y, -0.25, "{cause:?}");
                            let ground = match handle {
                                EntityHandle::ArmorStand(i) => &game.armor_stands[i].ground,
                                EntityHandle::Mob(i) => &game.mobs[i].ground,
                                _ => unreachable!(),
                            };
                            assert!(ground.on_ground);
                            assert_eq!(ground.supporting_block, Some(BlockPos::new(x, 200, 8)));
                        }
                    }
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn shulker_push_updates_stand_support_in_both_join_orders() {
        use sunrise_core::position::BlockPos;

        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::piston_push_server::lid_push_tests::shulker_push_updates_stand_support_in_both_join_orders",
            || {
                with_game(|game| {
                    assert!(game.run_command("summon armor_stand 10.5 200.25 10.5"));
                    let shulker = shulker(game);
                    assert!(game.run_command("summon armor_stand 13.5 200.25 10.5"));
                    let shulker_join = game.mobs.iter().find(|m| m.id == shulker).unwrap().join;
                    assert!(game.armor_stands[0].join < shulker_join);
                    assert!(game.armor_stands[1].join > shulker_join);
                    let swept = area(9.0, 9.0, 15.0, 12.0);
                    game.handle_mob_events(vec![MobEvent::LidPush {
                        shulker,
                        area: swept,
                        delta: [0.0, -0.5, 0.0],
                    }])
                    .unwrap();
                    for (stand, x) in game.armor_stands.iter().zip([10, 13]) {
                        assert_eq!(stand.position.y, 200.0);
                        assert!(stand.ground.on_ground);
                        assert_eq!(
                            stand.ground.supporting_block,
                            Some(BlockPos::new(x, 199, 10))
                        );
                    }
                    game.handle_mob_events(vec![MobEvent::LidPush {
                        shulker,
                        area: swept,
                        delta: [0.25, 0.0, 0.0],
                    }])
                    .unwrap();
                    for (stand, x) in game.armor_stands.iter().zip([10.75, 13.75]) {
                        assert_eq!(stand.position.x, x);
                        assert!(!stand.ground.on_ground);
                        assert_eq!(stand.ground.supporting_block, None);
                    }
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_shulker_lid_pushes_an_item_a_boat_and_a_projectile() {
        with_game(|game| {
            let shulker = shulker(game);
            assert!(game.run_command(
                "summon minecraft:item 10.5 200 10.5 {Item:{id:\"minecraft:stone\",count:1}}"
            ));
            assert!(game.run_command("summon minecraft:oak_boat 13.5 200 10.5"));
            assert!(game.run_command("summon minecraft:arrow 16.5 200 10.5 {NoGravity:1b}"));
            game.handle_mob_events(vec![MobEvent::LidPush {
                shulker,
                area: area(9.0, 9.0, 18.0, 12.0),
                delta: [0.0, 0.25, 0.0],
            }])
            .unwrap();
            assert_eq!(item_y(game), 200.25, "the item rises with the lid");
            assert_eq!(
                game.boats[0].position.y, 200.25,
                "the boat rises with the lid"
            );
            assert_eq!(arrow_y(game), 200.25, "the arrow rises with the lid");
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_wall_stops_what_a_shulker_lid_pushes() {
        with_game(|game| {
            let shulker = shulker(game);
            assert!(game.run_command("setblock 10 201 10 minecraft:stone"));
            assert!(game.run_command("setblock 13 201 13 minecraft:stone"));
            assert!(game.run_command("setblock 2 202 2 minecraft:stone"));
            assert!(game.run_command(
                "summon minecraft:item 10.5 200 10.5 {Item:{id:\"minecraft:stone\",count:1}}"
            ));
            assert!(game.run_command("summon minecraft:pig 13.5 200 13.5 {NoAI:1b}"));
            let lid = Lid::Shulker(shulker);
            game.lid_push(lid, area(10.0, 10.0, 11.0, 11.0), Vec3::new(0.0, 1.0, 0.0));
            game.lid_push(lid, area(13.0, 13.0, 14.0, 14.0), Vec3::new(0.0, 0.5, 0.0));
            game.lid_push(lid, area(2.0, 2.0, 3.0, 3.0), Vec3::new(0.0, 0.5, 0.0));
            assert_eq!(item_y(game), 200.75, "the stone stops the item");
            let pig = game.mobs.iter().find(|m| m.species() == Species::Pig);
            let pig = pig.expect("the pig").position.y;
            let pig_height = f64::from(0.9_f32);
            assert!(
                (pig - (201.0 - pig_height)).abs() < 1.0e-9,
                "the stone stops the pig: {pig}"
            );
            let player = game.player.as_ref().unwrap().position.y;
            let player_height = f64::from(1.8_f32);
            assert!(
                (player - (202.0 - player_height)).abs() < 1.0e-9,
                "the stone stops the player: {player}"
            );
        });
    }

    fn dropped(game: &sunrise_gameplay::game::ServerGame<'_>, item: &str) -> usize {
        game.dropped
            .iter()
            .filter(|d| &*d.stack.item == item)
            .count()
    }

    #[test]
    #[ignore = "daily"]
    fn a_shulker_lid_breaks_the_hanging_entities_it_meets() {
        with_game(|game| {
            let shulker = shulker(game);
            assert!(game.run_command(
                "summon minecraft:painting 10.5 200.5 10.5 {block_pos:[I;10,200,10],facing:2b,variant:\"minecraft:kebab\"}"
            ));
            assert!(game.run_command(
                "summon minecraft:item_frame 13.5 200.5 10.5 {Facing:2b,Item:{id:\"minecraft:stick\",count:1}}"
            ));
            assert!(game.run_command(
                "summon minecraft:item_frame 16.5 200.5 10.5 {Facing:2b,Fixed:1b,Item:{id:\"minecraft:stick\",count:1}}"
            ));
            assert!(game.run_command("summon minecraft:cushion 19.5 200 10.5"));
            assert_eq!(game.paintings.len(), 1);
            assert_eq!(game.item_frames.len(), 2);
            assert_eq!(game.cushions.len(), 1);
            game.handle_mob_events(vec![MobEvent::LidPush {
                shulker,
                area: area(9.0, 9.0, 21.0, 12.0),
                delta: [0.0, 0.25, 0.0],
            }])
            .unwrap();
            assert!(game.paintings.is_empty(), "the painting broke");
            assert_eq!(dropped(game, "minecraft:painting"), 1);
            assert_eq!(game.item_frames.len(), 1, "the loose frame broke");
            assert!(game.item_frames[0].fixed, "the fixed frame stays");
            assert!(game.item_frames[0].item.is_some(), "with its stick");
            assert_eq!(dropped(game, "minecraft:item_frame"), 1);
            assert_eq!(dropped(game, "minecraft:stick"), 1);
            assert!(game.cushions.is_empty(), "the cushion broke");
            assert_eq!(dropped(game, "minecraft:white_cushion"), 1);
        });
    }

    /// `isPassengerOfSameVehicle` on the server walk: a shulker riding a boat
    /// leaves the boat and the player riding it where they are.
    #[test]
    #[ignore = "daily"]
    fn a_shulker_lid_spares_what_shares_its_vehicle() {
        with_game(|game| {
            let shulker = shulker(game);
            assert!(game.run_command("summon minecraft:oak_boat 2.5 200 6.5"));
            let boat = game.boats[0].id;
            let i = game.mobs.iter().position(|m| m.id == shulker).unwrap();
            game.mobs[i].extra.mount = Some(boat);
            game.riding = Some(boat);
            let (around, lift) = (area(1.0, 1.0, 4.0, 8.0), Vec3::new(0.0, 0.25, 0.0));
            game.lid_push(Lid::Shulker(shulker), around, lift);
            assert_eq!(
                game.boats[0].position.y, 200.0,
                "the shulker's vehicle stays"
            );
            let player_y = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.player.as_ref().unwrap().position.y
            };
            assert_eq!(player_y(game), 200.0, "its fellow passenger stays");
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_shulker_lid_leaves_no_physics_bodies_and_shulkers_be() {
        with_game(|game| {
            let shulker = shulker(game);
            assert!(game.run_command("summon minecraft:armor_stand 10.5 200 10.5 {NoGravity:1b}"));
            assert!(game.run_command("summon minecraft:shulker_bullet 13.5 200 10.5"));
            assert!(game.run_command("summon minecraft:shulker 16 200 10 {NoAI:1b}"));
            let lid = area(9.0, 9.0, 18.0, 12.0);
            let lift = Vec3::new(0.0, 0.25, 0.0);
            game.lid_push(Lid::Shulker(shulker), lid, lift);
            assert_eq!(
                game.armor_stands[0].position.y, 200.0,
                "the stand has no physics"
            );
            let bullet = game
                .creatures
                .projectiles
                .iter()
                .next()
                .expect("the bullet");
            assert_eq!(bullet.position.y, 200.0, "the bullet has no physics");
            let other = game
                .mobs
                .iter()
                .find(|m| m.id != shulker)
                .expect("the other shulker");
            assert_eq!(other.position.y, 200.0, "a shulker is not pushed");
            game.lid_push(Lid::Box, lid, lift);
            assert_eq!(
                game.armor_stands[0].position.y, 200.25,
                "a box's lid moves the stand"
            );
        });
    }
}
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
