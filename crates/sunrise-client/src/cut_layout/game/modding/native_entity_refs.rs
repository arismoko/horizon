#[cfg(test)]
#[path = "native_entity_refs/level_uuid_tests.rs"]
pub(crate) mod level_uuid_tests;
#[cfg(test)]
#[path = "native_entity_refs/roll_loot_tests.rs"]
pub(crate) mod roll_loot_tests;

#[cfg(test)]
pub(crate) mod general_handle_tests {
    use super::tests::with_game;
    use mod_api::{EntityRef, WorldEpoch};
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::commands_common::Target;
    use sunrise_gameplay_common::game::general::GeneralRow;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5;

    fn row(id: u32, uuid: [i32; 4], fall: f64) -> GeneralRow {
        GeneralRow {
            id,
            uuid,
            join: sunrise_sim::join_seq::JoinSeq::UNJOINED,
            type_slot: 0,
            position: Vec3::new(0.5, 64.0, 0.5),
            previous: Vec3::new(0.5, 64.0, 0.5),
            velocity: Vec3::ZERO,
            rotation: [0.0, 0.0],
            previous_rotation: [0.0, 0.0],
            owner: None,
            fall_distance: fall,
            custom_name: None,
            custom_name_visible: false,
            base: Default::default(),
            on_ground: false,
            tick_count: 0,
            no_physics: false,
            horizontal_collision: false,
            vertical_collision: false,
            vertical_collision_below: false,
            fluids: Default::default(),
            under_water: false,
            payload_version: 1,
            payload_bytes: Vec::new(),
            mount: None,
            vehicle_order: 0,
            boarding_cooldown: 0,
            attachment_lifetime: Default::default(),
        }
    }

    /// #792's stale-handle row, as the id counter really behaves: a removed
    /// row's handle refuses, and the next row never reuses the id — ids come
    /// from the game's one monotonic counter (`take_entity_id`,
    /// game/network/lan_entities.rs:25: `next_entity += 1`, never decreased,
    /// and `EntityRef`'s contract says ids are never reused while the world
    /// is open), so the old handle has nothing to latch onto. Red without
    /// the resident lookup refusing a gone id: the old handle would resolve.
    #[test]
    #[ignore = "daily"]
    fn removed_general_row_refuses_its_handle_and_the_next_row_takes_a_new_id() {
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let uuid = [21, 22, 23, 24];
            game.resident_entities.general.rows.push(row(42, uuid, 7.0));
            let handle = game.entity(42).expect("the row became a handle");
            assert_eq!(handle.id(), 42);
            assert!(game.native_entity_target(handle).is_some());

            // The row leaves; the handle refuses.
            game.resident_entities
                .general
                .rows
                .retain(|row| row.id != 42);
            assert!(game.entity(42).is_none());
            assert!(game.native_entity_target(handle).is_none());

            // The next row is id 43 — the counter moved on, the same UUID
            // notwithstanding — and the old handle still refuses.
            game.resident_entities.general.rows.push(row(43, uuid, 7.0));
            let fresh = game.entity(43).expect("the next row became a handle");
            assert_ne!(fresh.id(), handle.id());
            assert!(
                game.native_entity_target(handle).is_none(),
                "the old handle resolved the next row"
            );
            let target = game
                .native_entity_target(fresh)
                .expect("the next row's handle resolves");
            assert!(matches!(target, Target::General(43)));
            let next = game
                .resident_entities
                .general
                .rows
                .iter()
                .find(|row| row.id == 43)
                .expect("the next row is in its list");
            assert_eq!(next.fall_distance, 7.0);
        });
    }

    /// A handle from an earlier epoch of the world — what a reload's restore
    /// of the same UUID leaves in a mod's hands — refuses by the epoch check
    /// alone, before any row is consulted.
    #[test]
    #[ignore = "daily"]
    fn general_handle_of_an_earlier_epoch_refuses_by_the_epoch_check() {
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            game.resident_entities
                .general
                .rows
                .push(row(42, [21, 22, 23, 24], 7.0));
            let handle = game.entity(42).expect("the row became a handle");
            assert!(game.native_entity_target(handle).is_some());
            // A reload opens a new epoch; a handle minted against the old one
            // refuses even though a row of the same UUID and id is resident.
            let stale = EntityRef::new(WorldEpoch::new(handle.epoch().get() + 1), handle.id());
            assert!(game.native_entity_target(stale).is_none());
        });
    }
}

#[cfg(test)]
pub(crate) mod tests {

    pub use sunrise_gameplay::game::native_entity_refs::tests::{
        bind_local_player, collect_all_entities, collect_entities_in, collect_game_entities_in,
        lever_on, piston_pulls_above, prepare_entity_ticking_neighborhood,
    };
    #[cfg(not(feature = "client"))]
    pub use sunrise_gameplay::game::native_entity_refs::tests::{
        with_game, with_game_at, with_game_role, with_resident_game,
    };
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2;
    use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1;
    use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5;
    use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1;
    use sunrise_gameplay_common::game::native_player_food::NativePlayerFoodGameExt1;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt1;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt3;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1;
    #[cfg(feature = "client")]
    pub use sunrise_gameplay_common::game::test_fixtures::{fixture_facts_mut, resolve_parts};
    pub use sunrise_gameplay_common::game::test_fixtures::{
        fixture_root, record_key, run_fresh_cases,
    };
    use sunrise_gameplay_common::{game_side, world_biomes};

    #[allow(unused_imports)]
    use core::ops::ControlFlow;
    #[allow(unused_imports)]
    use mod_api::__private::game::*;
    #[allow(unused_imports)]
    use mod_api::{EntityRef, LevelReadError, LevelRef};
    #[allow(unused_imports)]
    use sunrise_gameplay_common::game::api_values::TryIntoApi;

    // Keep the handle and direct Game routes separate when collecting for assertions.
    /// A child run with a name no test has fails the parent instead of passing vacuously.
    #[test]
    #[ignore = "daily"]
    #[should_panic(expected = "matched no test of that name")]
    fn a_fresh_case_whose_name_matches_no_test_fails() {
        run_fresh_cases(
            "game::server_side::native_entity_refs::tests::no_test_has_this_name",
            "SUNRISE_NO_MATCH_CHILD",
            &["missing"],
        );
    }

    /// The parent runs and checks an isolated child; the child loads the Part fixtures' types.
    pub(crate) fn declared_part_root(name: &str) -> Option<std::path::PathBuf> {
        if run_fresh_cases(name, "SUNRISE_DECLARED_PART_GAME_CHILD", &["valid"]) {
            return None;
        }
        let _mods = mod_platform::__private::part_tests::game_thread();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root = sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
            .unwrap()
            .prepared()
            .to_path_buf();
        // game_thread installs a small synthetic key table for API-only tests.
        // This fixture loads a real resource graph: resolve its complete keys first.
        let (registry, _) = sunrise_world::level::data::load_registry_with_flags(&root).unwrap();
        let ids: std::collections::HashSet<_> = (0..registry.len())
            .map(|i| {
                registry
                    .state(sunrise_world::registry::BlockStateId(i as u32))
                    .unwrap()
                    .name
                    .clone()
            })
            .collect();
        crate::game::api_values::register_game_keys(ids);
        Some(root)
    }

    #[cfg(feature = "client")]
    pub(crate) fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        with_game_at(fixture_root(), test);
    }

    /// Boots a server with the origin's entity-ticking neighbourhood loaded.
    #[cfg(feature = "client")]
    pub(crate) fn with_resident_game(
        test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
    ) {
        with_game(|game| {
            prepare_entity_ticking_neighborhood(game, sunrise_core::position::ChunkPos::new(0, 0));
            test(game);
        });
    }

    #[cfg(feature = "client")]
    pub(crate) fn with_game_at(
        root: std::path::PathBuf,
        test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
    ) {
        with_game_role(
            root,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            test,
        );
    }

    #[cfg(feature = "client")]
    pub(crate) fn with_client_game(
        test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
    ) {
        with_game_role_client(
            fixture_root(),
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
            test,
        );
    }

    #[cfg(feature = "client")]
    pub(crate) fn with_uncached_client_game(
        test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
    ) {
        with_game_role_and_facts_with_client(
            fixture_root(),
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
            None,
            false,
            test,
        );
    }

    #[cfg(feature = "client")]
    pub(crate) fn with_game_role(
        root: std::path::PathBuf,
        role: sunrise_gameplay_common::game::execution_role::ExecutionRole,
        test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
    ) {
        with_game_role_and_facts(root, role, None, test);
    }

    #[cfg(feature = "client")]
    pub(crate) fn with_game_role_client(
        root: std::path::PathBuf,
        role: sunrise_gameplay_common::game::execution_role::ExecutionRole,
        test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
    ) {
        with_game_role_and_facts_client(root, role, None, test);
    }

    /// Configure the fixture's owned gameplay facts before constructing the game.
    #[cfg(feature = "client")]
    pub(crate) fn with_game_role_and_facts(
        root: std::path::PathBuf,
        role: sunrise_gameplay_common::game::execution_role::ExecutionRole,
        configure: Option<
            fn(&sunrise_world::registry::BlockRegistry, &mut sunrise_items::blocks::BlockFacts),
        >,
        test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
    ) {
        with_game_role_and_facts_with(root, role, configure, configure.is_none(), test);
    }

    #[cfg(feature = "client")]
    pub(crate) fn with_game_role_and_facts_client(
        root: std::path::PathBuf,
        role: sunrise_gameplay_common::game::execution_role::ExecutionRole,
        configure: Option<
            fn(&sunrise_world::registry::BlockRegistry, &mut sunrise_items::blocks::BlockFacts),
        >,
        test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
    ) {
        with_game_role_and_facts_with_client(root, role, configure, configure.is_none(), test);
    }

    #[cfg(feature = "client")]
    fn with_game_role_and_facts_with(
        root: std::path::PathBuf,
        role: sunrise_gameplay_common::game::execution_role::ExecutionRole,
        configure: Option<
            fn(&sunrise_world::registry::BlockRegistry, &mut sunrise_items::blocks::BlockFacts),
        >,
        cached: bool,
        test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
    ) {
        use crate::game::{load_resources, load_resources_uncached};
        use crate::settings::Settings;
        use mod_api::Authority;
        use sunrise_gameplay_common::game::{Options, Progress};

        sunrise_gameplay_common::game::test_fixtures::prepare_game_boot(
            &root,
            mod_platform::__private::part_tests::register_game_keys,
        );
        let saves = tempfile::tempdir().unwrap();
        let settings = Settings::default();
        let mut loaded = if cached {
            load_resources(
                sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
                Vec::new(),
                None,
                &root,
                &settings,
                std::time::Instant::now(),
                false,
                role,
                &Progress::new(),
                &mut |_, _| unreachable!(),
            )
            .unwrap()
        } else {
            load_resources_uncached(
                sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
                Vec::new(),
                None,
                &root,
                &settings,
                std::time::Instant::now(),
                false,
                role,
                &Progress::new(),
                &mut |_, _| unreachable!(),
            )
            .unwrap()
        };
        if let Some(configure) = configure {
            configure(
                loaded.pack.assets.registry,
                fixture_facts_mut(&mut loaded.resources),
            );
        }
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = role;
        options.game_dir = Some(saves.path().to_path_buf());
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.reset_entity_residency();
        game.native_lifecycle
            .begin_open(Some(Authority::IntegratedServer));

        // A game that opened its world resolves its parts; the harness stops
        // short of the open, so do it here for the by-state part table.
        resolve_parts(&game).unwrap();
        game.install_root_block_transformers();
        // The title's save folder is under the working directory: give each test its own, so a
        // test that saves leaves the checkout clean and parallel tests don't share files.
        game.server.save.0 = std::sync::Arc::new(sunrise_sim::save::SaveDir::new(
            saves.path().to_path_buf(),
            sunrise_sim::save::LevelShape::OVERWORLD,
            Some(Default::default()),
        ));
        test(&mut game);
    }

    #[cfg(feature = "client")]
    fn with_game_role_and_facts_with_client(
        root: std::path::PathBuf,
        role: sunrise_gameplay_common::game::execution_role::ExecutionRole,
        configure: Option<
            fn(&sunrise_world::registry::BlockRegistry, &mut sunrise_items::blocks::BlockFacts),
        >,
        cached: bool,
        test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
    ) {
        use crate::audio::Audio;
        use crate::game::{load_resources, load_resources_uncached};
        use crate::settings::Settings;
        use mod_api::Authority;
        use sunrise_gameplay_common::game::execution_role::ExecutionRole;
        use sunrise_gameplay_common::game::{Options, Progress};

        assert_eq!(role, ExecutionRole::Client);
        sunrise_gameplay_common::game::test_fixtures::prepare_game_boot(
            &root,
            mod_platform::__private::part_tests::register_game_keys,
        );
        let saves = tempfile::tempdir().unwrap();
        let settings = Settings::default();
        let mut loaded = if cached {
            load_resources(
                sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
                Vec::new(),
                None,
                &root,
                &settings,
                std::time::Instant::now(),
                false,
                role,
                &Progress::new(),
                &mut |_, _| unreachable!(),
            )
            .unwrap()
        } else {
            load_resources_uncached(
                sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
                Vec::new(),
                None,
                &root,
                &settings,
                std::time::Instant::now(),
                false,
                role,
                &Progress::new(),
                &mut |_, _| unreachable!(),
            )
            .unwrap()
        };
        if let Some(configure) = configure {
            configure(
                loaded.pack.assets.registry,
                fixture_facts_mut(&mut loaded.resources),
            );
        }
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = role;
        options.game_dir = Some(saves.path().to_path_buf());
        let mut game = sunrise_client_gameplay::game::ClientGame::new(
            &loaded.resources,
            loaded.client_res.unwrap(),
            loaded.pack,
            loaded.client_pack.unwrap(),
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
            sunrise_gameplay::game::server_side::integrated::serve,
            sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
            crate::game::session_verifier(),
            crate::game::profile_resolver,
        )
        .unwrap();

        game.native_lifecycle
            .begin_open(Some(Authority::IntegratedServer));

        // A game that opened its world resolves its parts; the harness stops
        // short of the open, so do it here for the by-state part table.
        resolve_parts(&game).unwrap();
        // The title's save folder is under the working directory: give each test its own, so a
        // test that saves leaves the checkout clean and parallel tests don't share files.

        test(&mut game);
    }

    #[test]
    #[ignore = "daily"]
    fn typed_uuid_and_nbt_reads_follow_the_entity_level() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            // with_game prepares lifecycle, not world-open command registries.
            // execute in and summon must resolve real dimension and entity keys.
            game.build_registries();
            let words = [7, 0, 0, 3];
            let uuid = (7u128 << 96) | 3;
            // Harness commands use the console boundary; setting dedicated
            // options alone does not grant the command parser operator rights.
            assert!(game.as_console(|game| game.run_command(
                "execute in minecraft:the_nether run summon cow 8.5 70 8.5 {UUID:[I;7,0,0,3],NoAI:1b}"
            )));
            let original = game.dim;
            game.enter_command_level(sunrise_gameplay_common::game::Dim::Nether)
                .unwrap();
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let id = *game
                .persist
                .uuids
                .iter()
                .find(|(_, value)| **value == words)
                .unwrap()
                .0;
            let entity = game.entity(id).unwrap();
            assert_eq!(game.entity_uuid(entity), Some(uuid));
            let nbt = game.entity_nbt(entity).unwrap();
            assert_eq!(
                nbt.get(&"NoAI".into()),
                Some(&sunrise_core::nbt::Tag::Byte(1))
            );
            assert!(!nbt.contains_key(&"id".into()));
            game.enter_command_level(original).unwrap();
            assert_eq!(game.entity_nbt(entity), None);
            game.enter_command_level(sunrise_gameplay_common::game::Dim::Nether)
                .unwrap();
            assert_eq!(game.entity_uuid(entity), Some(uuid));
            assert_eq!(game.entity_nbt(entity), Some(nbt));
            game.enter_command_level(original).unwrap();
            assert_eq!(game.dim, original);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_part_allows_weak_power_in_real_game_level() {
        use std::sync::Arc;
        use sunrise_core::position::BlockPos;
        use sunrise_level::chunk::Chunk;

        mod_platform::__private::part_tests::weak_power_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let glass = registry.default_state("minecraft:glass").unwrap();
            let source = registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:lever".into(),
                    properties: [("face", "wall"), ("facing", "east"), ("powered", "true")]
                        .into_iter()
                        .map(|(key, value)| (key.into(), value.into()))
                        .collect(),
                })
                .unwrap();
            let pos = BlockPos::new(5, 64, 5);
            game.world.set(5, 64, 5, glass).unwrap();
            game.world.set(6, 64, 5, source).unwrap();
            let signal = game.with_level(|b, level, ticks| {
                b.best_neighbor_signal(level, ticks, BlockPos::new(pos.x - 1, pos.y, pos.z))
            });
            assert_eq!(signal, 15);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_placement_view_reads_redstone_and_collision_shapes() {
        use mod_api::{Direction, LevelHost};
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;

        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let glass = registry.default_state("minecraft:glass").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let lever = registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:lever".into(),
                    properties: [("face", "wall"), ("facing", "east"), ("powered", "true")]
                        .into_iter()
                        .map(|(key, value)| (key.into(), value.into()))
                        .collect(),
                })
                .unwrap();
            game.world.set(5, 64, 5, glass).unwrap();
            game.world.set(6, 64, 5, lever).unwrap();
            game.world.set(9, 64, 5, stone).unwrap();
            game.with_level(|_, level, ticks| {
                let view = level.view(ticks);
                // The glass has a powered lever on its east side; the stone nothing.
                assert_eq!(view.has_neighbor_signal([5, 64, 5]), Some(true));
                assert_eq!(view.best_neighbor_signal([5, 64, 5]), Some(15));
                assert_eq!(view.signal([5, 64, 5], Direction::East), Some(0));
                assert_eq!(view.direct_signal_to([5, 64, 5]), Some(15));
                assert_eq!(view.has_neighbor_signal([9, 64, 5]), Some(false));
                // The same reads from the level itself have no ticks to ask.
                assert_eq!(LevelHost::has_neighbor_signal(&*level, [5, 64, 5]), None);
                // A full block's collision box, none for air, nothing for an unloaded position.
                for host in [&view as &dyn LevelHost, &*level] {
                    assert_eq!(
                        host.collision_shape([9, 64, 5])
                            .map(|shape| shape.boxes().collect::<Vec<_>>()),
                        Some(vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]])
                    );
                    assert!(host.collision_shape([9, 70, 5]).unwrap().is_empty());
                    assert!(host.collision_shape([160, 64, 5]).is_none());
                }
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_soil_part_answers_for_the_plants_above_it_in_a_real_game_level() {
        use std::sync::Arc;
        use sunrise_core::position::BlockPos;
        use sunrise_level::chunk::Chunk;

        // The fixture's parts: glass sustains any plant, grass sustains none.
        mod_platform::__private::part_tests::sustain_plant_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let state = |name: &str| registry.default_state(name).unwrap();
            let dandelion = state("minecraft:dandelion");
            let survives = |game: &mut sunrise_gameplay::game::ServerGame<'_>, soil: &str| {
                game.world.set(5, 64, 5, state(soil)).unwrap();
                game.with_level(|b, level, ticks| {
                    b.can_survive(level, ticks, BlockPos::new(5, 65, 5), dandelion)
                })
            };
            // Allow: vanilla refuses a dandelion on glass; the soil's part lets it stand.
            assert!(survives(game, "minecraft:glass"));
            // Deny: vanilla lets a dandelion stand on grass; the soil's part refuses it.
            assert!(!survives(game, "minecraft:grass_block"));
            // Default: soils with no part answer as vanilla's plant rule does.
            assert!(survives(game, "minecraft:dirt"));
            assert!(!survives(game, "minecraft:stone"));
            // A non-authoritative copy of the level asks the part too (the client side itself is
            // covered by `client_placement_prediction_asks_placement_and_survival_parts`).
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
            game.options.dedicated = None;
            assert!(survives(game, "minecraft:glass"));
            assert!(!survives(game, "minecraft:grass_block"));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_block_part_decides_whether_the_tool_harvests() {
        use std::sync::Arc;
        use sunrise_core::{geometry::Vec3, position::ChunkPos};
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::motion::PlayerMotion;

        // The fixture's parts: stone is harvested by any tool, dirt by none.
        mod_platform::__private::part_tests::harvest_game_thread();
        with_game(|game| {
            let scratch = std::env::temp_dir().join(format!(
                "sunrise-harvest-part-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            std::fs::create_dir_all(&scratch).unwrap();
            game.server.world_root = scratch.clone();
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 62.0, 4.5)).unwrap());
            game.player.as_mut().unwrap().ground.on_ground = true;
            game.creative = false;
            game.build_registries();
            game.set_allow_commands(true);
            assert!(game.run_command("gamemode survival"));
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            let registry = game.pack.assets.registry;
            let pos = [5, 63, 4];
            // The dig's progress and the drops of a break by bare hand: (block, its hardness, what
            // vanilla drops by hand, and what the drop is). Vanilla's own rule: a block that
            // requires a tool drops nothing by hand, and the dig is 3.33 times slower for it.
            for (block, hardness, part, by_hand, drop) in [
                // Stone needs a pickaxe; the part harvests it anyway.
                (
                    "minecraft:stone",
                    1.5_f32,
                    Some(true),
                    false,
                    "minecraft:cobblestone",
                ),
                // Dirt needs none; the part refuses it.
                ("minecraft:dirt", 0.5, Some(false), true, "minecraft:dirt"),
                // No part: vanilla's rule alone, both ways.
                (
                    "minecraft:cobblestone",
                    2.0,
                    None,
                    false,
                    "minecraft:cobblestone",
                ),
                ("minecraft:grass_block", 0.6, None, true, "minecraft:dirt"),
            ] {
                let state = registry.default_state(block).unwrap();
                let harvest = !part.unwrap_or(by_hand);
                game.world.set(pos[0], pos[1], pos[2], state).unwrap();
                let divisor = if harvest { 30.0 } else { 100.0 };
                assert_eq!(
                    game.destroy_progress(pos),
                    1.0 / hardness / divisor,
                    "{block}"
                );
                game.dropped.clear();
                game.break_block(pos).unwrap();
                let dropped = game.dropped.iter().any(|item| &*item.stack.item == drop);
                assert_eq!(dropped, harvest, "{block}: drops {drop}");
            }
            std::fs::remove_dir_all(scratch).unwrap();
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_level_handle_reads_redstone_as_signal_getter_does() {
        use sunrise_core::geometry::Direction::{East, North, West};

        with_game(|game| {
            lever_on(game, "minecraft:stone");
            let level = game.level(game.dim.id()).unwrap();
            let (stone, lever, beside, far) = ([5, 64, 5], [6, 64, 5], [4, 64, 5], [5, 64, 9]);
            // A powered lever: weak 15 every way, strong 15 only into the block it hangs on,
            // which lies the way it faces from that block (LeverBlock.getDirectSignal).
            assert_eq!(level.signal(game, lever, North), Some(15));
            assert_eq!(level.direct_signal(game, lever, East), Some(15));
            assert_eq!(level.direct_signal(game, lever, North), Some(0));
            // Stone conducts the strong power to the block west of it.
            assert_eq!(level.direct_signal_to(game, stone), Some(15));
            assert_eq!(level.signal(game, stone, West), Some(15));
            assert_eq!(level.best_neighbor_signal(game, beside), Some(15));
            assert!(level.has_neighbor_signal(game, beside));
            // Nothing gives strong power into `beside`, and nothing reaches `far`.
            assert_eq!(level.direct_signal_to(game, beside), Some(0));
            assert_eq!(level.best_neighbor_signal(game, far), Some(0));
            assert!(!level.has_neighbor_signal(game, far));
        });
        // Glass is no conductor: the lever's power stops at it.
        with_game(|game| {
            lever_on(game, "minecraft:glass");
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(level.best_neighbor_signal(game, [4, 64, 5]), Some(0));
            assert_eq!(level.signal(game, [5, 64, 5], West), Some(0));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_level_bone_meals_a_block_and_sets_a_biome_cell() {
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;

        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let farmland = registry.default_state("minecraft:farmland").unwrap();
            let wheat = registry.default_state("minecraft:wheat").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(0, 63, 0, farmland).unwrap();
            game.world.set(0, 64, 0, wheat).unwrap();
            game.world.set(5, 64, 5, stone).unwrap();
            let level = game.level(game.dim.id()).unwrap();
            assert!(game.level_bone_meal(level, [0, 64, 0]));
            assert_ne!(game.world.block(0, 64, 0), wheat, "the crop grew");
            assert!(!game.level_bone_meal(level, [5, 64, 5]), "stone");
            assert!(!game.level_bone_meal(level, [100_000, 64, 0]), "unloaded");

            let current = usize::from(game.world.chunks[&(0, 0)].biome(0, 64, 0));
            let target = usize::from(current == 0);
            let name = world_biomes!(game).list[target].name.clone();
            assert!(game.level_set_biome(level, [0, 64, 0], &name));
            assert_eq!(
                usize::from(game.world.chunks[&(0, 0)].biome(3, 67, 3)),
                target,
                "the whole 4x4x4 cell changed"
            );
            assert!(game.world.modified.contains(&(0, 0)));
            assert!(
                !game.level_set_biome(level, [0, 64, 0], &name),
                "already set"
            );
            assert!(!game.level_set_biome(level, [0, 64, 0], "minecraft:nope"));
            assert!(!game.level_set_biome(level, [100_000, 64, 0], &name));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_level_handle_updates_neighbours_as_vanillas_writes_do() {
        use mod_api::NeighbourUpdate::{Neighbours, NeighboursExcept, OutputSignal};
        use sunrise_core::geometry::Direction::East;

        let lit = |game: &sunrise_gameplay::game::ServerGame<'_>, [x, y, z]: [i32; 3]| {
            let state = game.world.block(x, y, z);
            game.pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("lit"))
                .map(String::as_str)
                == Some("true")
        };
        let set =
            |game: &mut sunrise_gameplay::game::ServerGame<'_>, [x, y, z]: [i32; 3], name: &str| {
                let state = game.pack.assets.registry.default_state(name).unwrap();
                game.world.set(x, y, z, state).unwrap();
            };
        // The powered lever at (6, 64, 5) on stone; unlit lamps beside it, east and above,
        // placed without updates, so they don't know it powers them yet.
        let (lever, east, above) = ([6, 64, 5], [7, 64, 5], [6, 65, 5]);
        with_game(|game| {
            lever_on(game, "minecraft:stone");
            let level = game.level(game.dim.id()).unwrap();
            let state = game.world.block(6, 64, 5);
            set(game, east, "minecraft:redstone_lamp");
            set(game, above, "minecraft:redstone_lamp");
            // `updateNeighborsAtExceptFromFacing(lever, East)`: every neighbour but the east one.
            assert_eq!(
                level.update_neighbors(
                    game,
                    lever,
                    state.try_into_api().unwrap(),
                    NeighboursExcept(East)
                ),
                Ok(true)
            );
            assert!(lit(game, above) && !lit(game, east));
            // `updateNeighborsAt(lever)`: the east lamp hears it too.
            assert_eq!(
                level.update_neighbors(game, lever, state.try_into_api().unwrap(), Neighbours),
                Ok(true)
            );
            assert!(lit(game, east));
        });
        // `Level.neighborChanged(east, lever)`: only that lamp hears.
        with_game(|game| {
            lever_on(game, "minecraft:stone");
            let level = game.level(game.dim.id()).unwrap();
            let state = game.world.block(6, 64, 5);
            set(game, east, "minecraft:redstone_lamp");
            set(game, above, "minecraft:redstone_lamp");
            assert_eq!(
                level.neighbor_changed(game, east, state.try_into_api().unwrap()),
                Ok(true)
            );
            assert!(lit(game, east) && !lit(game, above));
        });
        // A repeater in front of the lever, reading it, schedules its own tick when told (a
        // diode's `checkTickOnNeighbor`), through the level's queue; told again while that tick
        // is pending, it schedules no second one (`willTickThisTick` / `hasScheduledTick`).
        // Hooks that run inside the update get level hosts, not a `ServerHost`, so a mod
        // can't call `update_neighbors` from within one: re-entry goes through the queue.
        with_game(|game| {
            lever_on(game, "minecraft:stone");
            game.block_ticks
                .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                .unwrap();
            let level = game.level(game.dim.id()).unwrap();
            set(game, [7, 63, 5], "minecraft:stone");
            let repeater = game
                .pack
                .assets
                .registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:repeater".into(),
                    properties: [
                        ("facing", "west"),
                        ("delay", "1"),
                        ("locked", "false"),
                        ("powered", "false"),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
                })
                .unwrap();
            game.world.set(7, 64, 5, repeater).unwrap();
            let block = game.res.behaviours().block(repeater);
            let at = sunrise_core::position::BlockPos::new(7, 64, 5);
            let state = game.world.block(6, 64, 5);
            assert!(!game.block_ticks.has_block_tick(at, block));
            assert_eq!(
                level.update_neighbors(game, lever, state.try_into_api().unwrap(), Neighbours),
                Ok(true)
            );
            assert!(game.block_ticks.has_block_tick(at, block));
            assert_eq!(
                level.update_neighbors(game, lever, state.try_into_api().unwrap(), Neighbours),
                Ok(true)
            );
            assert!(game.block_ticks.has_block_tick(at, block));
            assert_eq!(
                game.world.block(7, 64, 5),
                repeater,
                "unpowered until its tick runs"
            );
        });
        // `updateNeighbourForOutputSignal(cake)`: the comparator reading the cake, which
        // outputs 14 when whole while the comparator still reads 0, schedules its tick.
        with_game(|game| {
            lever_on(game, "minecraft:stone");
            game.block_ticks
                .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                .unwrap();
            let level = game.level(game.dim.id()).unwrap();
            // Both stand on stone, as they must to survive a neighbour update.
            set(game, [10, 63, 5], "minecraft:stone");
            set(game, [11, 63, 5], "minecraft:stone");
            set(game, [10, 64, 5], "minecraft:cake");
            let comparator = game
                .pack
                .assets
                .registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:comparator".into(),
                    properties: [
                        ("facing", "west"),
                        ("mode", "compare"),
                        ("powered", "false"),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
                })
                .unwrap();
            game.world.set(11, 64, 5, comparator).unwrap();
            let block = game.res.behaviours().block(comparator);
            let at = sunrise_core::position::BlockPos::new(11, 64, 5);
            assert!(!game.block_ticks.has_block_tick(at, block));
            let cake = game.world.block(10, 64, 5);
            assert_eq!(
                level.update_neighbors(
                    game,
                    [10, 64, 5],
                    cake.try_into_api().unwrap(),
                    OutputSignal
                ),
                Ok(true)
            );
            assert!(game.block_ticks.has_block_tick(at, block));
            // Off the level in place: refused, nothing done.
            assert_eq!(
                level.update_neighbors(
                    game,
                    [10, 64, 500_000],
                    cake.try_into_api().unwrap(),
                    OutputSignal
                ),
                Ok(false)
            );
            // A source that is no block state: refused, not a panic.
            let known: mod_api::BlockState = cake.try_into_api().unwrap();
            let unknown: mod_api::BlockState =
                mod_api::__private::block_state_from_ordinal(known.block(), u16::MAX);
            assert_eq!(
                level.update_neighbors(game, [10, 64, 5], unknown, Neighbours),
                Err(mod_api::LevelWriteError::InvalidState(
                    mod_api::BlockStateError::UnknownState
                ))
            );
            assert_eq!(
                level.neighbor_changed(game, [11, 64, 5], unknown),
                Err(mod_api::LevelWriteError::InvalidState(
                    mod_api::BlockStateError::UnknownState
                ))
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn neighbor_changed_reads_its_inputs_and_its_tick_fires_at_its_delay() {
        use mod_platform::__private::part_tests::{
            SignalRead, signal_reader_game_thread, signal_reads,
        };

        // The fixture's part on glass reads its inputs in `neighbor_changed` and schedules
        // its own tick 2 ticks later, as a diode does.
        signal_reader_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let registry = game.pack.assets.registry;
            let glass = registry.default_state("minecraft:glass").unwrap();
            let lever = |powered: &str| {
                registry
                    .resolve(&sunrise_world::chunk::BlockState {
                        name: "minecraft:lever".into(),
                        properties: [("face", "wall"), ("facing", "east"), ("powered", powered)]
                            .into_iter()
                            .map(|(key, value)| (key.into(), value.into()))
                            .collect(),
                    })
                    .unwrap()
            };
            // A block-ticking chunk: lit, its neighbours loaded, with its tick list, as
            // loading gives one.
            for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
                let mut chunk = sunrise_level::chunk::Chunk::empty(x, z);
                chunk.lit = true;
                game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
            }
            game.block_ticks
                .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                .unwrap();
            // A lever on the east face of stone, and glass north of the lever: the lever's
            // weak power reaches it from the south.
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(5, 64, 5, stone).unwrap();
            game.world.set(6, 64, 4, glass).unwrap();
            game.world.set(6, 64, 5, lever("false")).unwrap();
            let dimension = game.dim.id().to_owned();
            let start = game.world.game_time;
            assert!(signal_reads().is_empty());

            // Pulling the lever updates its neighbours (`Block.UPDATE_ALL`), the glass among them.
            assert!(game.native_set_block(&dimension, [6, 64, 5], lever("true"), 3));
            assert_eq!(
                signal_reads(),
                [SignalRead::Changed([6, 64, 4], true, 15, true)]
            );
            // The tick it scheduled runs through the level's own queue, 2 ticks later, and
            // reads the lever south of it from `tick_block` (`LevelWriteHost: LevelTicksHost`).
            for (elapsed, reads) in [(1, vec![]), (2, vec![SignalRead::Ticked([6, 64, 4], 15)])] {
                game.world.game_time = start + elapsed;
                game.tick_scheduled().unwrap();
                assert_eq!(signal_reads(), reads, "after {elapsed} ticks");
            }
        });
    }

    /// `RedstoneLampBlock.neighborChanged` sets its own block in the call
    /// (`level.setBlock(pos, state.cycle(LIT), 2)`): a part's write in `neighbor_changed`
    /// lands within the lever's update, and its own neighbour updates run next, depth first,
    /// before the lever's next neighbour hears (`CollectingNeighborUpdater`).
    #[test]
    #[ignore = "daily"]
    fn neighbor_changed_writes_the_level_and_its_updates_run_next() {
        use mod_platform::__private::part_tests::{neighbor_writer_game_thread, neighbor_writes};

        // The fixture installs its mod for the process: run it in a child of its own.
        if run_fresh_cases(
            "cut_layout::game::server_side::native_entity_refs::tests::neighbor_changed_writes_the_level_and_its_updates_run_next",
            "SUNRISE_NEIGHBOR_WRITER_CHILD",
            &["lever"],
        ) {
            return;
        }
        neighbor_writer_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let registry = game.pack.assets.registry;
            let glass = registry.default_state("minecraft:glass").unwrap();
            let lit = registry
                .default_state("minecraft:white_stained_glass")
                .unwrap();
            let lever = |powered: &str| {
                registry
                    .resolve(&sunrise_world::chunk::BlockState {
                        name: "minecraft:lever".into(),
                        properties: [("face", "wall"), ("facing", "east"), ("powered", powered)]
                            .into_iter()
                            .map(|(key, value)| (key.into(), value.into()))
                            .collect(),
                    })
                    .unwrap()
            };
            for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
                let mut chunk = sunrise_level::chunk::Chunk::empty(x, z);
                chunk.lit = true;
                game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
            }
            // A lever on the east face of stone. `NeighborUpdater.UPDATE_ORDER` is west, east,
            // down, up, north, south: glass `a` east of the lever hears second (west is the
            // stone), glass `b` above it fourth. Glass `c` east of `a` hears only `a`'s write.
            let (a, b, c) = ([7, 64, 5], [6, 65, 5], [8, 64, 5]);
            game.world
                .set(5, 64, 5, registry.default_state("minecraft:stone").unwrap())
                .unwrap();
            game.world.set(6, 64, 5, lever("false")).unwrap();
            for [x, y, z] in [a, b, c] {
                game.world.set(x, y, z, glass).unwrap();
            }
            let dimension = game.dim.id().to_owned();
            let key = |state: sunrise_world::registry::BlockStateId| {
                use sunrise_gameplay_common::game::api_values::TryIntoApi;
                let state: mod_api::BlockState = state.try_into_api().unwrap();
                state.block()
            };
            assert!(neighbor_writes().is_empty());

            assert!(game.native_set_block(&dimension, [6, 64, 5], lever("true"), 3));
            assert_eq!(
                neighbor_writes(),
                [
                    (a, key(lever("true"))),
                    // `Level.setBlock` names the old block as the source (`oldState.getBlock()`).
                    (c, key(glass)),
                    (b, key(lever("true"))),
                ]
            );
            // Both glasses the lever reached changed in the same call, with no tick run.
            for [x, y, z] in [a, b] {
                assert_eq!(game.world.block(x, y, z), lit);
            }
            assert_eq!(game.world.block(c[0], c[1], c[2]), glass);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_level_handle_reads_redstone_through_should_check_weak_power() {
        use sunrise_core::geometry::Direction::West;

        // The fixture's part lets glass conduct (`should_check_weak_power`).
        mod_platform::__private::part_tests::weak_power_game_thread();
        with_game(|game| {
            lever_on(game, "minecraft:glass");
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(level.signal(game, [5, 64, 5], West), Some(15));
            assert_eq!(level.best_neighbor_signal(game, [4, 64, 5]), Some(15));
            // Strong power into the glass is its own; the part changes only what it conducts.
            assert_eq!(level.direct_signal_to(game, [5, 64, 5]), Some(15));
            assert_eq!(level.direct_signal_to(game, [4, 64, 5]), Some(0));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_part_answers_is_pathfindable_once_per_state_and_type() {
        use mod_api::entity::mobs::PathComputationType::{self, Air, Land, Water};

        // Before any part: every state answers as the vanilla export says.
        with_game(|game| {
            let data = &game.res.rest().mob_data;
            for (i, vanilla) in data.states.iter().enumerate() {
                for (kind, bit) in [(Land, 1), (Water, 2), (Air, 4)] {
                    let state = sunrise_world::registry::BlockStateId(i as u32);
                    assert_eq!(data.pathfindable(state, kind), vanilla.flags & bit != 0);
                }
            }
        });
        mod_platform::__private::part_tests::pathfindable_game_thread();
        with_game(|game| {
            let registry = game.pack.assets.registry;
            let asks = |name: &str, kind: PathComputationType| {
                let state = registry.default_state(name).unwrap();
                game.res.rest().mob_data.pathfindable(state, kind)
            };
            // Glass: land and air answered, water left to the block (vanilla: none of them).
            assert!(asks("minecraft:glass", Land));
            assert!(asks("minecraft:glass", Air));
            assert!(!asks("minecraft:glass", Water));
            // Air: land refused, water allowed, air left to the block.
            assert!(!asks("minecraft:air", Land));
            assert!(asks("minecraft:air", Water));
            assert!(asks("minecraft:air", Air));
            // A block with no part answers as before.
            assert!(!asks("minecraft:stone", Land));
            // A reload without the mod's answers takes them back.
            game.res.mob_data().resolve_parts(
                game.res.facts(),
                &sunrise_sim::mobs::data::PartAnswers::default(),
            );
            assert!(!asks("minecraft:glass", Land));
            assert!(asks("minecraft:air", Land));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_part_asks_neighbours_to_update_when_its_block_is_removed() {
        use std::sync::Arc;
        use sunrise_core::position::BlockPos;
        use sunrise_level::chunk::Chunk;

        mod_platform::__private::part_tests::removal_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let glass = registry.default_state("minecraft:glass").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let lamp_state = |lit: &str| {
                registry
                    .resolve(&sunrise_world::chunk::BlockState {
                        name: "minecraft:redstone_lamp".into(),
                        properties: [("lit".into(), lit.into())].into_iter().collect(),
                    })
                    .unwrap()
            };
            let power = registry.default_state("minecraft:redstone_block").unwrap();
            // An unlit lamp beside a redstone block lights when told a neighbour changed, and
            // set into the world it hears nothing: the proof the update reached it. Moved by a
            // piston, a removal tells no neighbour itself, so only the part's request can.
            let lamp = BlockPos::new(6, 64, 5);
            for (state, heard) in [(stone, false), (glass, true)] {
                game.world.set(5, 64, 5, state).unwrap();
                game.world.set(6, 64, 5, lamp_state("false")).unwrap();
                game.world.set(7, 64, 5, power).unwrap();
                let lit = game.with_level(|b, level, ticks| {
                    // Flags 82: how a piston pushes a block out (`PistonBaseBlock.moveBlocks`).
                    use sunrise_world::level::flags;
                    let flags = flags::UPDATE_MOVE_BY_PISTON
                        | flags::UPDATE_KNOWN_SHAPE
                        | flags::UPDATE_CLIENTS;
                    b.set_block(
                        level,
                        ticks,
                        BlockPos::new(5, 64, 5),
                        registry.default_state("minecraft:air").unwrap(),
                        flags,
                    );
                    sunrise_world::level::LevelAccess::block_state(level, lamp)
                        == lamp_state("true")
                });
                assert_eq!(lit, heard);
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_parts_decide_what_a_piston_pulls_along() {
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;

        mod_platform::__private::part_tests::sticky_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            // The block below, the block on top of it, and whether the top one moves.
            for ((block, above, pulled), y) in [
                // No part: vanilla's slime and honey, and nothing else sticks.
                ("minecraft:slime_block", "minecraft:stone", true),
                ("minecraft:honey_block", "minecraft:stone", true),
                ("minecraft:slime_block", "minecraft:honey_block", false),
                ("minecraft:stone", "minecraft:stone", false),
                // A sticky part pulls whatever its `can_stick_to` allows: diamond all of it,
                // gold only gold.
                ("minecraft:diamond_block", "minecraft:stone", true),
                ("minecraft:diamond_block", "minecraft:honey_block", true),
                ("minecraft:gold_block", "minecraft:gold_block", true),
                ("minecraft:gold_block", "minecraft:stone", false),
                ("minecraft:gold_block", "minecraft:honey_block", false),
                // Either side may refuse: honey is held back by gold's rule though it is sticky.
                ("minecraft:honey_block", "minecraft:gold_block", false),
                // A slime part is kept apart from honey as slime is.
                ("minecraft:emerald_block", "minecraft:honey_block", false),
                ("minecraft:emerald_block", "minecraft:stone", true),
            ]
            .into_iter()
            .zip((8..).step_by(4))
            {
                assert_eq!(
                    piston_pulls_above(game, y, block, above),
                    pulled,
                    "{above} on {block}"
                );
            }
        });
    }

    /// The restitution an entity landing on a block reads is asked of the block's parts in the
    /// entity's level, on the side that ticks it, with the entity and the block's position; a
    /// block without a part is not asked, and a non-finite answer is ignored.
    #[test]
    #[ignore = "daily"]
    fn a_block_part_changes_the_bounce_restitution_an_entity_reads() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::native_entity_refs::tests::a_block_part_changes_the_bounce_restitution_an_entity_reads",
            block_part_bounce_restitution_in_own_process,
        );
    }

    pub(crate) fn block_part_bounce_restitution_in_own_process() {
        use sunrise_core::position::BlockPos;
        use sunrise_sim::block_contact::{Contact, bounce_restitution};

        let heard = mod_platform::__private::part_tests::restitution_game_thread();
        sunrise_gameplay_common::game::native_hooks::install_block_contact_hooks();
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let registry = game.pack.assets.registry;
            let state = |name: &str| registry.default_state(name).unwrap();
            let (glass, stone, slime) = (
                state("minecraft:glass"),
                state("minecraft:stone"),
                state("minecraft:slime_block"),
            );
            let pos = BlockPos::new(3, 64, 5);
            let mut read = |block, own, client_side| {
                game.world.set(pos.x, pos.y, pos.z, block).unwrap();
                let contact = Contact {
                    entity: 7,
                    kind: "minecraft:pig",
                    state: block,
                    position: pos,
                };
                use sunrise_world::native_load_context::{
                    DataLoadOrigin, LoadContext, LoadInputs, LoopSide,
                };
                let context = LoadContext::new(
                    DataLoadOrigin::ServerDataLoad,
                    std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
                    [],
                )
                .captured_loop_context(if client_side {
                    LoopSide::Client
                } else {
                    LoopSide::Server
                });
                bounce_restitution(
                    sunrise_sim::motion::FrictionLevel {
                        world: &game.world,
                        registry,
                        context: &context,
                        side: &game_side!(game),
                        coefficient: &|id| {
                            game.res
                                .facts()
                                .states
                                .get(id.0 as usize)
                                .map(|p| p.friction)
                        },
                    },
                    contact,
                    own,
                )
            };
            // Glass's part answers whatever the block's own value: 0.5 on a server, 0.25 on a
            // client predicting its own bounce.
            assert_eq!(read(glass, 0.0, false), 0.5);
            assert_eq!(read(glass, 1.0, true), 0.25);
            // A block with no part keeps its own: slime bounces fully.
            assert_eq!(read(slime, 1.0, false), 1.0);
            // Stone's part answers not a number, which is ignored.
            assert_eq!(read(stone, 0.0, false), 0.0);
            let pig = |client| (client, 7, "minecraft:pig".to_owned(), pos, true);
            assert_eq!(*heard.lock().unwrap(), [pig(false), pig(true)]);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_part_connects_wire_in_a_real_game_level() {
        use std::sync::Arc;
        use sunrise_core::{geometry::Direction, position::BlockPos};
        use sunrise_level::chunk::Chunk;

        mod_platform::__private::part_tests::redstone_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let wire = registry.default_state("minecraft:redstone_wire").unwrap();
            let glass = registry.default_state("minecraft:glass").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let pos = BlockPos::new(5, 64, 5);
            let neighbour = BlockPos::new(6, 64, 5);
            game.world.set(5, 63, 5, stone).unwrap();
            game.world.set(6, 64, 5, glass).unwrap();
            let connected = game.with_level(|b, level, ticks| {
                b.update_shape(level, ticks, wire, pos, Direction::East, neighbour, glass)
            });
            assert_eq!(
                registry.state(connected).unwrap().properties["east"],
                "side"
            );
            game.world.set(6, 64, 5, stone).unwrap();
            let vanilla = game.with_level(|b, level, ticks| {
                b.update_shape(level, ticks, wire, pos, Direction::East, neighbour, stone)
            });
            assert_eq!(registry.state(vanilla).unwrap().properties["east"], "none");
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_part_flips_fence_wall_and_pane_connections_in_a_real_game_level() {
        use std::sync::Arc;
        use sunrise_core::{geometry::Direction, position::BlockPos};
        use sunrise_level::chunk::Chunk;

        mod_platform::__private::part_tests::connection_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let state = |name: &str| registry.default_state(name).unwrap();
            let stone = state("minecraft:stone");
            let torch = state("minecraft:torch");
            let pos = BlockPos::new(5, 64, 5);
            let neighbour = BlockPos::new(6, 64, 5);
            let side = |game: &mut sunrise_gameplay::game::ServerGame<'_>, block, beside| {
                game.world.set(6, 64, 5, beside).unwrap();
                let shaped = game.with_level(|b, level, ticks| {
                    b.update_shape(level, ticks, block, pos, Direction::East, neighbour, beside)
                });
                registry.state(shaped).unwrap().properties["east"].clone()
            };
            game.world.set(5, 63, 5, stone).unwrap();
            // Vanilla joins each to a sturdy face and not to a torch. The part on
            // the oak fence, cobblestone wall and glass pane flips both answers;
            // the spruce fence, brick wall and iron bars have none.
            for (block, with_part, yes, no) in [
                ("oak_fence", true, "true", "false"),
                ("spruce_fence", false, "true", "false"),
                ("cobblestone_wall", true, "low", "none"),
                ("brick_wall", false, "low", "none"),
                ("glass_pane", true, "true", "false"),
                ("iron_bars", false, "true", "false"),
            ] {
                let block_state = state(&format!("minecraft:{block}"));
                let (sturdy, not) = if with_part { (no, yes) } else { (yes, no) };
                assert_eq!(side(game, block_state, stone), sturdy, "{block} by stone");
                assert_eq!(side(game, block_state, torch), not, "{block} by a torch");
            }
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn side_guards_agree_across_integrated_dedicated_and_client() {
        use std::sync::Arc;
        use sunrise_gameplay_common::game::execution_role::ExecutionRole;
        use sunrise_level::chunk::Chunk;
        let mut server_level = None;
        with_game(|game| {
            game.world
                .chunks
                .insert((-2, 3), Arc::new(Chunk::empty(-2, 3)));
            let level = game.level(game.dim.id()).unwrap();
            server_level = Some(level);
            // Server-only write guards do not exist on ClientGame.
            for (role, dedicated) in [
                (ExecutionRole::IntegratedServer, true),
                (ExecutionRole::DedicatedServer, true),
            ] {
                game.options.execution_role = role;
                game.options.dedicated = dedicated.then(Vec::new);
                assert!(game.native_simulates());
                assert!(game.level(game.dim.id()).is_some());
                assert!(game.level_in_place(level));
                assert!(game.native_level_write_allowed(level, [-32, 64, 48]));
            }
        });
        let level = server_level.unwrap();
        with_client_game(|client| {
            client
                .world
                .chunks
                .insert((-2, 3), Arc::new(Chunk::empty(-2, 3)));
            assert!(!client.is_logical_server());
            assert!(client.level(client.dim.id()).is_none());
            assert!(!client.level_in_place(level));
            assert!(client.native_current_level().is_none());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn chunk_handles_resolve_loaded_parked_and_stale_levels() {
        use mod_api::{Authority, ChunkStatus};
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            game.world
                .chunks
                .insert((-2, 3), Arc::new(Chunk::empty(-2, 3)));
            let chunk = game.chunk(level, [-2, 3]).unwrap();
            assert_eq!(chunk.pos(), [-2, 3]);
            assert_eq!(chunk.level(), level);
            assert!(chunk.loaded(game));
            assert_eq!(chunk.status(game), Some(ChunkStatus::Full));
            assert!(game.chunk(level, [99, 99]).is_none());
            assert!(
                game.enter_command_level(sunrise_gameplay_common::game::Dim::Nether)
                    .is_some()
            );
            assert_eq!(chunk.status(game), Some(ChunkStatus::Full));
            assert_eq!(game.chunk(level, [-2, 3]), Some(chunk));
            assert!(
                game.enter_command_level(sunrise_gameplay_common::game::Dim::Overworld)
                    .is_some()
            );
            game.world.chunks.remove(&(-2, 3));
            assert!(!chunk.loaded(game));
            assert_eq!(chunk.status(game), None);
            game.world
                .chunks
                .insert((-2, 3), Arc::new(Chunk::empty(-2, 3)));
            game.native_lifecycle
                .begin_open(Some(Authority::IntegratedServer));
            assert!(game.chunk(level, [-2, 3]).is_none());
            assert!(!chunk.loaded(game));
            assert_eq!(chunk.status(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn level_reads_and_writes_use_dimension_build_height() {
        use mod_api::BlockUpdateFlags;
        use std::sync::Arc;
        use sunrise_level::chunk::{Chunk, LevelShape};
        with_game(|game| {
            game.world.shape = LevelShape::new(0, 128, 128, true, false).unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 127, 0, stone).unwrap();
            game.world.set(0, 128, 0, stone).unwrap();
            assert!(game.level_is_loaded(level, [0, 127, 0]));
            assert_eq!(game.level_block_state(level, [0, 127, 0]), Some(stone));
            assert_eq!(
                game.level_block(level, [0, 127, 0])
                    .and_then(mod_api::__private::block_key_id),
                Some("minecraft:stone")
            );
            assert!(game.native_level_write_allowed(level, [0, 127, 0]));
            assert!(!game.level_is_loaded(level, [0, 128, 0]));
            assert_eq!(game.level_block_state(level, [0, 128, 0]), None);
            assert_eq!(game.level_block(level, [0, 128, 0]), None);
            assert!(!game.level_is_loaded(level, [0, -1, 0]));
            assert_eq!(game.level_block_state(level, [0, -1, 0]), None);
            assert_eq!(
                game.level_set_block(level, [0, 128, 0], stone, BlockUpdateFlags::NEIGHBORS),
                Ok(false)
            );
            assert!(
                game.level_destroy_block(level, [0, 128, 0], false, None, None, false)
                    .is_none()
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn equipping_mobs_by_hand_keeps_one_complete_stack() {
        use sunrise_core::geometry::Vec3;
        use sunrise_items::items::ItemStack;
        use sunrise_sim::mobs::Mob;
        with_game(|game| {
            game.creative = false;
            for (id, kind, item) in [
                (12, "minecraft:horse", "minecraft:iron_horse_armor"),
                (13, "minecraft:llama", "minecraft:red_carpet"),
                (14, "minecraft:happy_ghast", "minecraft:red_harness"),
                (15, "minecraft:nautilus", "minecraft:iron_nautilus_armor"),
            ] {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                let mut mob = Mob::new(id, kind, Vec3::ZERO, 0.0);
                mob.extra.tame = true;
                let mut held = ItemStack::new(item, 3);
                held.damage = 7;
                held.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("kept".into()));
                game.common.inventory[game.common.selected] = Some(held.clone());
                let index = game.mobs.len();
                game.mobs.push(mob);
                let used = match id {
                    12 => game.interact_mount(index, item),
                    13 => game.interact_llama(index, item),
                    14 => game.interact_happy_ghast(index, item),
                    _ => game.interact_nautilus(index, item),
                };
                assert!(used, "{item}");
                assert_eq!(game.inventory[game.selected].as_ref().unwrap().count, 2);
                held.count = 1;
                assert_eq!(game.mobs[index].equipment_stacks()[6], Some(held), "{item}");
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn sulfur_cube_absorbs_one_complete_stack_by_hand_and_pickup() {
        use sunrise_core::geometry::Vec3;
        use sunrise_items::items::ItemStack;
        use sunrise_sim::mobs::Mob;
        with_game(|game| {
            game.creative = false;
            let kind = game
                .res
                .mob_types()
                .get("minecraft:sulfur_cube")
                .unwrap()
                .clone();
            let item = kind.tempt.first().expect("a swallowable item").to_string();
            let mut held = ItemStack::new(&item, 3);
            held.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("kept".into()));
            game.common.inventory[game.common.selected] = Some(held.clone());
            game.mobs.push(Mob::new(12, kind.clone(), Vec3::ZERO, 0.0));
            assert!(game.interact_sulfur_cube(0, &item));
            assert_eq!(game.inventory[game.selected].as_ref().unwrap().count, 2);
            held.count = 1;
            assert_eq!(game.mobs[0].equipment_stacks()[6], Some(held.clone()));
            game.mobs.clear();
            game.mobs.push(Mob::new(13, kind, Vec3::ZERO, 0.0));
            game.creatures.ticked.insert(13);
            game.spawn_item(
                ItemStack {
                    count: 3,
                    ..held.clone()
                },
                [0.0, 0.0, 0.0],
                [0.0; 3],
                0,
            );
            game.sulfur_cube_pickups();
            assert_eq!(game.mobs[0].equipment_stacks()[6], Some(held));
            assert_eq!(game.dropped[0].stack.count, 2);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn shearing_llama_and_happy_ghast_returns_full_body_stacks() {
        use sunrise_core::geometry::Vec3;
        use sunrise_items::items::ItemStack;
        use sunrise_sim::mobs::Mob;
        with_game(|game| {
            for (id, kind, item) in [
                (12, "minecraft:llama", "minecraft:red_carpet"),
                (13, "minecraft:happy_ghast", "minecraft:red_harness"),
            ] {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                let mut mob = Mob::new(id, kind, Vec3::ZERO, 0.0);
                mob.extra.tame = true;
                let mut stack = ItemStack::new(item, 3);
                stack.damage = 7;
                stack.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("kept".into()));
                mob.extra.set_body_stack(Some(stack.clone()));
                let index = game.mobs.len();
                game.mobs.push(mob);
                game.common.inventory[game.common.selected] =
                    Some(ItemStack::new("minecraft:shears", 1));
                let used = if item.ends_with("carpet") {
                    game.interact_llama(index, "minecraft:shears")
                } else {
                    game.interact_happy_ghast(index, "minecraft:shears")
                };
                assert!(used);
                assert!(game.dropped.iter().any(|drop| drop.stack == stack));
                assert!(game.mobs[index].extra.body_id().is_none());
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn acting_guest_handles_resolve_during_guest_turns() {
        use mod_api::{EquipmentSlot, GameMode};
        use std::sync::{
            Arc,
            atomic::{AtomicI32, AtomicUsize},
        };
        use sunrise_gameplay::game::server_side::lan::LanSettings;
        use sunrise_items::items::ItemStack;
        use sunrise_protocol::packets::configuration::ClientInformation;
        use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};

        with_game(|game| {
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: game.dim.id().to_owned(),
                allow_flight: true,
                seed: 0,
                max_players: 2,
                integrated: true,
                levels: game.server.level_shapes.clone(),
                level_seeds: game.server.levels.0.runtime.seeds(),
            };
            game.lan_host(settings, net, config).unwrap();
            let host = game.server.lan.as_mut().unwrap();
            for id in [12, 13] {
                let (tx, _rx) = std::sync::mpsc::channel();
                let mut guest = Guest::new(
                    Joined {
                        conn: id,
                        uuid: id as u128,
                        name: format!("guest{id}"),
                        properties: Vec::new(),
                        info: ClientInformation::new(8),
                        brand: String::new(),
                        sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                        latency: Arc::new(AtomicI32::new(0)),
                        address: "127.0.0.1:1".parse().unwrap(),
                        expected: false,
                    },
                    id as i32,
                    [3.0, 64.0, 5.0],
                    10,
                );
                guest.placed = true;
                if id == 12 {
                    // A player loaded from a file saved with a walking speed.
                    guest.abilities.walking_speed = 0.25;
                    guest.loaded_body = Some(sunrise_sim::lan::guest::SavedBody {
                        fire_ticks: 0,
                        air: 300,
                        fall_distance: 0.0,
                        velocity: [0.0; 3],
                        food_exhaustion: 0.0,
                        food_tick_timer: 0,
                        ticks_frozen: 0,
                        invulnerable: false,
                        invulnerable_time: 0,
                    });
                }
                guest.inventory[0] = Some(ItemStack::new("minecraft:stone", 1));
                host.hub.guests.push(guest);
            }
            let motion = sunrise_sim::motion::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::new(3.0, 64.0, 5.0),
            )
            .unwrap();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| guest.motion = Some(Box::new(motion)))
                .unwrap();
            let player = game.player(12).unwrap();
            player
                .with_inventory(game, |view| {
                    let mut slots = view.iter();
                    let (slot, stack) = slots.next().unwrap();
                    let stone = mod_api::__private::item_key("minecraft:stone").unwrap();
                    assert_eq!((slot, stack.item(), stack.count()), (0, stone, 1));
                    assert!(slots.next().is_none());
                })
                .unwrap();
            let guest_living = game.living(player.fields().entity).unwrap();
            let host: &dyn mod_api::Host = game;
            assert_eq!(
                guest_living.attribute_base(host, "minecraft:max_health"),
                Some(20.0)
            );
            assert_eq!(
                guest_living.attribute_value(host, "minecraft:max_health"),
                Some(20.0)
            );
            assert_eq!(guest_living.max_health(host), Some(20.0));
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| guest.motion = None)
                .unwrap();
            // Body and saddle are stored on a guest like on a mob.
            let living = game.living(player.fields().entity).unwrap();
            let saddle = ItemStack::new("minecraft:saddle", 1);
            assert!(game.living_set_equipment(living, EquipmentSlot::Saddle, Some(saddle.clone())));
            assert!(game.living_set_equipment(
                living,
                EquipmentSlot::Body,
                Some(ItemStack::new("minecraft:iron_horse_armor", 1))
            ));
            assert_eq!(
                game.living_equipment_stack(living, EquipmentSlot::Saddle)
                    .map(|stack| stack.project())
                    .as_deref(),
                Some(&saddle)
            );
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .equipment
                    .get(sunrise_items::equipment::BODY)
                    .map(|s| &*s.item),
                Some("minecraft:iron_horse_armor")
            );
            assert!(game.living_set_equipment(living, EquipmentSlot::Saddle, None));
            assert!(
                game.living_equipment_stack(living, EquipmentSlot::Saddle)
                    .is_none()
            );
            // The host's own previous mode, which no guest's change may touch.
            game.previous_mode = Some(sunrise_command::args::GameMode::Adventure);
            assert_eq!(game.player_previous_game_mode(player), Some(None));
            assert!(!game.player_set_game_mode(player, GameMode::Survival));
            assert_eq!(game.player_previous_game_mode(player), Some(None));
            assert!(game.player_set_game_mode(player, GameMode::Creative));
            assert_eq!(game.player_game_mode(player), Some(GameMode::Creative));
            assert_eq!(
                game.player_previous_game_mode(player),
                Some(Some(GameMode::Survival))
            );
            let abilities = game.player_abilities(player).unwrap();
            assert!(abilities.invulnerable && abilities.may_fly && abilities.instabuild);
            assert!(game.player_set_game_mode(player, GameMode::Survival));
            assert_eq!(game.player_game_mode(player), Some(GameMode::Survival));
            let abilities = game.player_abilities(player).unwrap();
            assert!(!abilities.invulnerable && !abilities.may_fly && !abilities.instabuild);
            // A mode change updates the flags while preserving both supplied speeds.
            let mut written = abilities;
            written.may_build = false;
            written.walk_speed = 0.2;
            written.fly_speed = 0.1;
            assert!(game.player_set_abilities(player, written));
            assert_eq!(game.player_abilities(player), Some(written));
            assert_eq!(game.player_may_build(player), Some(false));
            written.walk_speed = f32::NAN;
            written.fly_speed = f32::INFINITY;
            assert!(game.player_set_abilities(player, written));
            let stored = game.player_abilities(player).unwrap();
            assert!(stored.walk_speed.is_nan());
            assert_eq!(stored.fly_speed, f32::INFINITY);
            assert!(game.player_set_game_mode(player, GameMode::Creative));
            let abilities = game.player_abilities(player).unwrap();
            assert!(abilities.may_build && abilities.instabuild && abilities.may_fly);
            assert!(abilities.walk_speed.is_nan());
            assert_eq!(abilities.fly_speed, f32::INFINITY);
            assert!(game.player_set_game_mode(player, GameMode::Survival));
            let stale = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(mod_api::WorldEpoch::new(0), player.fields().entity.id()),
                player.uuid(),
            );
            assert!(!game.player_set_game_mode(stale, GameMode::Creative));
            let other = game
                .native_lifecycle
                .epoch()
                .map(|epoch| mod_api::EntityRef::new(epoch, 13))
                .unwrap();
            game.as_guest(12, |game| {
                let player = game.player(12).unwrap();
                let entity = game.entity(12).unwrap();
                assert_eq!(player.fields().entity, entity);
                assert_eq!(entity.position(game), Some([3.0, 64.0, 5.0]));
                assert_eq!(entity.uuid(game), Some(12));
                let living = game.living(entity).unwrap();
                assert_eq!(
                    living
                        .with_equipment(game, EquipmentSlot::MainHand, |stack| stack
                            .item()
                            .id(game))
                        .unwrap(),
                    Some("minecraft:stone")
                );
                assert_eq!(game.player.as_ref().unwrap().health.health(), 20.0);
                // The loaded walking speed is the movement speed's base.
                assert_eq!(
                    game.player
                        .as_mut()
                        .unwrap()
                        .attribute("minecraft:movement_speed")
                        .unwrap(),
                    f64::from(0.25f32)
                );
                // In the player's own turn the write reaches the body as well.
                let mut a = game.player_abilities(player).unwrap();
                a.may_fly = true;
                a.walk_speed = 0.3;
                assert!(game.player_set_abilities(player, a));
                assert_eq!(game.player_abilities(player), Some(a));
                assert!(game.player.as_ref().unwrap().may_fly);
                assert_eq!(other.position(game), Some([3.0, 64.0, 5.0]));
                assert!(game.living(other).is_some());
            })
            .unwrap();
            // A mode change in a turn, then a write in the same turn: the write wins.
            game.as_guest(12, |game| {
                let player = game.player(12).unwrap();
                // The guest's own previous mode is swapped in for the turn.
                assert_eq!(
                    game.player_previous_game_mode(player),
                    Some(Some(GameMode::Creative))
                );
                assert_eq!(game.player_is_secondary_use_active(player), Some(false));
                game.player.as_mut().unwrap().shift_key_down = true;
                assert_eq!(game.player_is_secondary_use_active(player), Some(true));
                game.player.as_mut().unwrap().shift_key_down = false;
                assert!(game.player_set_game_mode(player, GameMode::Creative));
                assert_eq!(
                    game.player_previous_game_mode(player),
                    Some(Some(GameMode::Survival))
                );
                let mut a = game.player_abilities(player).unwrap();
                assert!(a.instabuild && a.may_fly);
                a.fly_speed = 0.4;
                a.walk_speed = 0.3;
                assert!(game.player_set_abilities(player, a));
            })
            .unwrap();
            let kept = game.server.lan.as_ref().unwrap().hub.guest(12).unwrap();
            assert_eq!(kept.game_mode, 1);
            assert_eq!(kept.previous_game_mode, Some(0));
            assert_eq!(
                game.previous_mode,
                Some(sunrise_command::args::GameMode::Adventure)
            );
            assert!(kept.abilities.instabuild);
            assert_eq!(kept.abilities.flying_speed, 0.4);
            assert_eq!(kept.abilities.walking_speed, 0.3);
            // The turn's write is stored in the guest's record.
            let stored = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(12)
                .unwrap()
                .abilities;
            assert!(stored.may_fly && stored.walking_speed == 0.3);
            game.with_guest_swap(12, game.dim, |game| {
                assert!(game.entity(12).is_some());
                let other = game.entity(13).unwrap();
                assert_eq!(other.position(game), Some([3.0, 64.0, 5.0]));
                assert_eq!(other.uuid(game), Some(13));
                assert_eq!(
                    other.kind(game).and_then(|kind| kind.id(game)).as_deref(),
                    Some("minecraft:player")
                );
                let living = game.living(other).unwrap();
                assert_eq!(
                    living
                        .with_equipment(game, EquipmentSlot::MainHand, |stack| stack
                            .item()
                            .id(game))
                        .unwrap(),
                    Some("minecraft:stone")
                );
                let player = game.player(13).unwrap();
                assert_eq!(player.fields().entity, other);
                assert_eq!(player.game_mode(game), Some(GameMode::Survival));
                assert!(game.player_set_game_mode(player, GameMode::Creative));
                assert_eq!(player.game_mode(game), Some(GameMode::Creative));
                assert_eq!(game.server.lan_acting, Some(12));
                let acting = game.player(12).unwrap();
                assert!(game.player_set_game_mode(acting, GameMode::Adventure));
                assert_eq!(game.player_game_mode(acting), Some(GameMode::Adventure));
                game.with_guest_swap(13, game.dim, |game| {
                    assert!(
                        game.with_guest_swap(12, game.dim, |_| panic!("active guest re-entered"))
                            .is_none()
                    );
                    assert_eq!(game.server.lan_acting, Some(13));
                    assert!(game.entity(12).is_none());
                    assert!(game.player(12).is_none());
                    assert!(
                        game.living(mod_api::EntityRef::new(other.epoch(), 12))
                            .is_none()
                    );
                    assert!(game.entity(13).is_some());
                })
                .unwrap();
                assert_eq!(game.server.lan_acting, Some(12));
            })
            .unwrap();
            assert!(game.server.lan_guest_swaps.is_empty());
            // A player saved in creative (game mode 1, mayfly and instabuild)
            // joins again: without force they stay creative; forced into the
            // default mode (survival) they get its abilities.
            let root = std::env::temp_dir().join(format!("abilities-force-{}", std::process::id()));
            game.server.world_root = root.clone();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |g| {
                    g.game_mode = 1;
                    g.abilities.update_for_mode(1);
                    assert!(g.abilities.may_fly && g.abilities.instabuild);
                    sunrise_sim::lan::playerdata::save(g, &root, &Default::default());
                })
                .unwrap();
            let joined = |game: &mut sunrise_gameplay::game::ServerGame<'_>, force| {
                let host = game.server.lan.as_mut().unwrap();
                host.hub.config.default_game_mode = 0;
                host.hub.config.force_game_mode = force;
                host.hub
                    .update_guest(13, |g| {
                        g.game_mode = 0;
                        g.abilities = Default::default();
                    })
                    .unwrap();
                game.lan_guest_joined(13, &[], [0.0, 64.0, 0.0], None);
                let g = game.server.lan.as_ref().unwrap().hub.guest(13).unwrap();
                (
                    g.game_mode,
                    g.abilities.may_fly,
                    g.abilities.instabuild,
                    g.abilities.may_build,
                )
            };
            assert_eq!(joined(game, false), (1, true, true, true));
            assert_eq!(joined(game, true), (0, false, false, true));
            // A remote guest can mount through the same handle path as the acting player.
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(sunrise_sim::mobs::Mob::new(
                21,
                kind,
                sunrise_core::geometry::Vec3::ZERO,
                0.0,
            ));
            let guest = game.entity(12).unwrap();
            let vehicle = game.entity(21).unwrap();
            assert!(game.entity_start_riding(guest, vehicle, true).is_ok());
            assert_eq!(game.command_guest(12).unwrap().vehicle, Some(21));
            assert!(game.entity_stop_riding(guest));
            assert_eq!(game.command_guest(12).unwrap().vehicle, None);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| guest.dimension = "minecraft:the_nether".into())
                .unwrap();
            assert!(game.entity_start_riding(guest, vehicle, true).is_err());
            assert!(game.entity_start_riding(vehicle, guest, true).is_err());
            let _ = std::fs::remove_dir_all(root);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn entities_in_returns_readable_handles_and_intersects_boxes() {
        use mod_api::{LevelRef, WorldEpoch};
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::entities::ArmorStand;
        with_resident_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            game.armor_stands
                .push(ArmorStand::new(0, Vec3::new(0.5, 0.0, 0.0), 0.0));
            assert!(game.entity(0).is_none());
            game.armor_stands
                .push(ArmorStand::new(12, Vec3::new(1.0, 0.0, 0.0), 0.0));
            let found =
                collect_game_entities_in(game, level, [0.0, 0.0, -1.0], [0.8, 1.0, 1.0]).unwrap();
            assert_eq!(found.len(), 1);
            assert_eq!(game.entity_position(found[0]), Some([1.0, 0.0, 0.0]));
            assert_eq!(
                game.entity_kind(found[0])
                    .and_then(|kind| kind.id(game))
                    .as_deref(),
                Some("minecraft:armor_stand")
            );
            assert!(
                collect_game_entities_in(game, level, [0.0, 0.0, -1.0], [0.75, 1.0, 1.0])
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(
                collect_game_entities_in(
                    game,
                    LevelRef::new(WorldEpoch::new(0), game.dim.key()),
                    [0.0; 3],
                    [2.0; 3]
                ),
                Err(LevelReadError::Stale)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn connectionless_guest_has_no_transport() {
        use mod_api::HostTransaction;
        use std::sync::{Arc, atomic::Ordering};
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::packets::{
            configuration::ClientInformation,
            play::{Clientbound, Distance},
        };
        use sunrise_sim::{
            lan::{
                Joined, Lan, NetConfig, PlayerSender,
                chunks::ChunkEncoder,
                conn::{Holding, flush_all},
                data::SyncData,
                guest::Guest,
                hub::{Hub, HubConfig},
            },
            motion::PlayerMotion,
        };
        with_game(|game| {
            let sync = Arc::new(SyncData::empty());
            let net = NetConfig {
                announce: false,
                ..NetConfig::lan(0, "test".into(), 2)
            };
            let lan = Lan::start_with(sync.clone(), net).unwrap();
            let real_id = lan.shared.next_id().unwrap();
            let fake_id = lan.shared.next_id().unwrap();
            assert_ne!(real_id, fake_id);
            let encoder = ChunkEncoder::new(
                [] as [(Arc<str>, bool); 0],
                &sunrise_protocol::registry::Builtin::default(),
                &[],
                &[],
                Default::default(),
            );
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 10,
                simulation_distance: 10,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 20,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            };
            let mut hub = Hub::new(
                lan,
                sync,
                encoder,
                config.clone(),
                std::sync::Arc::new(
                    sunrise_gameplay_common::game::test_server_load()
                        .loop_context(sunrise_world::native_load_context::LoopSide::Server, []),
                ),
            );
            let motion = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            let fake = Guest::connectionless(fake_id, 2, "fake".into(), 2, [0.0; 3], motion);
            assert!(fake.placed && fake.motion.is_some());
            assert!(fake.loaded && fake.client_loaded());
            assert!(!fake.participates_online() && !fake.participates_in_player_simulation());
            assert_eq!((fake.game_mode, fake.permission, fake.input), (0, 0, 0));
            assert!(fake.vehicle.is_none() && fake.bed.is_none() && fake.menu.is_none());
            let (tx, rx) = std::sync::mpsc::channel();
            let mut real = Guest::new(
                Joined {
                    conn: real_id,
                    uuid: 1,
                    name: "real".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::default(),
                    address: ([127, 0, 0, 1], 1).into(),
                    expected: false,
                },
                1,
                [0.0; 3],
                10,
            );
            real.placed = true;
            hub.guests.push(real);
            hub.guests.push(fake);
            assert!(hub.here(&hub.guests[0]));
            assert!(!hub.here(&hub.guests[1]));
            let packet = Clientbound::SetSimulationDistance(Distance { distance: 8 });
            let hold = Holding::start();
            hub.send_to(fake_id, &packet);
            hub.broadcast(&packet);
            hub.guests[1].sender.send_many([vec![1, 2, 3]]);
            flush_all(hub.guests.iter().map(|g| &g.sender));
            drop(hold);
            assert_eq!(hub.guests[1].sender.pending.load(Ordering::Relaxed), 0);
            assert!(rx.recv_timeout(std::time::Duration::from_secs(2)).is_ok());
            game.lan_host(
                sunrise_gameplay::game::server_side::lan::LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                },
                NetConfig {
                    announce: false,
                    ..NetConfig::lan(0, "swap".into(), 2)
                },
                config,
            )
            .unwrap();
            let host = game.server.lan.as_mut().unwrap();
            host.hub.guests = std::mem::take(&mut hub.guests);
            {
                let value = [1600.0, 64.0, 0.0];
                host.hub.guests.update(1, |guest| guest.position = value);
            }
            game.lan_update_streaming();
            assert!(
                game.server
                    .streaming
                    .extra
                    .iter()
                    .any(|(chunk, _)| *chunk == (0, 0))
            );
            assert!(
                !game
                    .server
                    .streaming
                    .extra
                    .iter()
                    .any(|(chunk, _)| *chunk == (100, 0))
            );
            game.server.next_entity = 100;
            game.server.lan.as_ref().unwrap().hub.lan.shared.next_id();
            game.server.lan.as_ref().unwrap().hub.lan.shared.next_id();
            let level = game.level(game.dim.id()).unwrap();
            let role = game.options.execution_role;
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
            assert!(game.fake_player(level, 1234, "machine").is_none());
            game.options.execution_role = role;
            let made = game.fake_player(level, 1234, "machine").unwrap();
            assert_eq!(game.fake_player(level, 1234, "machine"), Some(made));
            assert!(game.fake_player(level, 1234, "other").is_none());
            assert!(game.fake_player(level, 1, "real").is_none());
            assert_eq!(game.player_is_fake(made), Some(true));
            assert!(game.player(1234).is_none());
            let mut abilities = game.player_abilities(made).unwrap();
            abilities.may_fly = true;
            assert!(game.player_set_abilities(made, abilities));
            assert!(game.player_abilities(made).unwrap().may_fly);
            assert!(game.player_set_food_level(made, 10));
            assert!(game.player_feed(made, 3, 0.0));
            assert_eq!(game.player_food(made).unwrap().level, 13);
            assert!(game.player_set_experience_levels(made, 7));
            assert_eq!(game.player_experience(made).unwrap().level, 7);
            assert!(game.player_set_item_cooldown(made, "minecraft:stone", 20));
            assert_eq!(
                game.player_has_item_cooldown(made, "minecraft:stone"),
                Some(true)
            );
            assert!(game.player_remove_fake(made));
            assert!(!game.player_remove_fake(made));
            assert_eq!(game.player_is_fake(made), None);
            let replacement = game.fake_player(level, 1234, "machine").unwrap();
            assert_ne!(made, replacement);
            let saved_root = game.server.world_root.clone();
            let scratch = std::env::current_dir()
                .unwrap()
                .join("target/fake-player-save-test");
            let _ = std::fs::remove_dir_all(&scratch);
            game.server.world_root = scratch.clone();
            let host = game.server.lan.as_mut().unwrap();
            let i = host.hub.guests.iter().position(|g| g.uuid == 1234).unwrap();
            let mut actor = host.hub.guests.remove(i);
            game.save_guest(&mut actor);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .insert(i, actor);
            assert!(!scratch.exists());
            game.server.world_root = saved_root;
            let conn = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .find(|g| g.uuid == 1234)
                .unwrap()
                .conn;
            game.as_guest(conn, |game| {
                assert_eq!(game.player_permission_level(replacement), Some(0));
                assert!(!game.player_remove_fake(replacement));
                assert!(game.fake_player(level, 4321, "other").is_none());
            })
            .unwrap();
            assert!(game.player_remove_fake(replacement));
            let overworld_actor = game.fake_player(level, 2345, "parked").unwrap();
            let attachment = sunrise_mod_runtime::attachments::AttachmentType {
                namespace: "test",
                name: "identity",
                default: || 0i32,
                codec: None,
                copy_on_death: false,
                sync_codec: None,
            };
            let holder = sunrise_mod_runtime::attachments::Holder::Player(2345);
            game.options.dedicated = Some(Vec::new());
            let real_holder = sunrise_mod_runtime::attachments::Holder::Player(1);
            attachment
                .set(
                    game.attachment_map_quiet(real_holder, mod_api::AttachmentAccess::Existing)
                        .unwrap(),
                    sunrise_mod_runtime::attachments::Holder::Level,
                    17,
                )
                .unwrap();
            attachment
                .set(
                    game.attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
                        .unwrap(),
                    sunrise_mod_runtime::attachments::Holder::Level,
                    3,
                )
                .unwrap();
            assert!(
                game.enter_command_level(sunrise_gameplay_common::game::Dim::Nether)
                    .is_some()
            );
            let nether = game.level("minecraft:the_nether").unwrap();
            assert!(game.fake_player(level, 3456, "other").is_none());
            let nether_actor = game.fake_player(nether, 2345, "parked").unwrap();
            attachment
                .set(
                    game.attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
                        .unwrap(),
                    sunrise_mod_runtime::attachments::Holder::Level,
                    8,
                )
                .unwrap();
            assert_eq!(
                attachment
                    .get(
                        game.attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
                            .unwrap(),
                        sunrise_mod_runtime::attachments::Holder::Level
                    )
                    .unwrap(),
                8
            );
            assert_ne!(overworld_actor, nether_actor);
            let host: &mut dyn mod_api::ServerHost = game;
            let aborted: Result<(), mod_api::TransactionError<()>> = host.transaction(|host, _| {
                attachment.set(host, holder, 99).unwrap();
                Err(())
            });
            assert!(aborted.is_err());
            assert_eq!(
                attachment
                    .get(
                        game.attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
                            .unwrap(),
                        sunrise_mod_runtime::attachments::Holder::Level
                    )
                    .unwrap(),
                8
            );
            let place = mod_api::ItemPlace::PlayerSlot {
                player: 2345,
                slot: 0,
            };
            let sticks = mod_api::ItemResource::of(
                &sunrise_gameplay_common::game::api_values::test_snapshot(
                    &sunrise_items::items::ItemStack::new("minecraft:stick", 1),
                ),
            );
            let host: &mut dyn mod_api::ServerHost = game;
            let rolled_back: Result<(), mod_api::TransactionError<()>> =
                host.transaction(|host, tx| {
                    assert_eq!(place.insert(host, tx, &sticks, 1), Ok(1));
                    Err(())
                });
            assert!(rolled_back.is_err());
            assert!(place.with_stack(game, |_| ()).is_none());
            assert_eq!(
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guests
                    .iter()
                    .find(|g| g.entity_id as u32 == overworld_actor.entity().id())
                    .unwrap()
                    .inventory[0],
                None
            );
            let host: &mut dyn mod_api::ServerHost = game;
            assert_eq!(
                host.transaction(|host, tx| place.insert(host, tx, &sticks, 1)),
                Ok(1)
            );
            assert_eq!(place.with_stack(game, |stack| stack.count()).unwrap(), 1);
            assert_eq!(game.fake_player(nether, 2345, "parked"), Some(nether_actor));
            assert!(game.player_set_experience_levels(nether_actor, 8));
            assert!(game.player_set_experience_levels(overworld_actor, 3));
            assert_eq!(game.player_experience(nether_actor).unwrap().level, 8);
            assert_eq!(game.player_experience(overworld_actor).unwrap().level, 3);
            assert!(
                game.enter_command_level(sunrise_gameplay_common::game::Dim::Overworld)
                    .is_some()
            );
            assert_eq!(
                attachment
                    .get(
                        game.attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
                            .unwrap(),
                        sunrise_mod_runtime::attachments::Holder::Level
                    )
                    .unwrap(),
                3
            );
            assert!(game.player_remove_fake(overworld_actor));
            assert_eq!(
                attachment
                    .get(
                        game.attachment_map_quiet(real_holder, mod_api::AttachmentAccess::Existing)
                            .unwrap(),
                        sunrise_mod_runtime::attachments::Holder::Level
                    )
                    .unwrap(),
                17
            );
            game.enter_command_level(sunrise_gameplay_common::game::Dim::Nether)
                .unwrap();
            assert_eq!(
                attachment
                    .get(
                        game.attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
                            .unwrap(),
                        sunrise_mod_runtime::attachments::Holder::Level
                    )
                    .unwrap(),
                8
            );
            assert!(game.player_remove_fake(nether_actor));
            game.enter_command_level(sunrise_gameplay_common::game::Dim::Overworld)
                .unwrap();
            game.options.dedicated = None;
            let players = game.native_server_players();
            assert_eq!(players.len(), 1);
            assert_eq!(players[0].uuid(), 1);
            // A fake record in the Nether does not keep that level ticking.
            {
                let value = "minecraft:the_nether".into();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(1, |guest| guest.dimension = value);
            }
            let ticking = game.levels_to_tick();
            assert!(!ticking.contains(&sunrise_gameplay_common::game::Dim::Nether));
            {
                let value = "minecraft:the_nether".into();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.dimension = value);
            }
            assert!(
                game.levels_to_tick()
                    .contains(&sunrise_gameplay_common::game::Dim::Nether)
            );
            {
                let value = "minecraft:overworld".into();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.dimension = value);
            }
            game.server.lan_acting = Some(real_id);
            let selector = sunrise_command::args::Selector {
                kind: 'a',
                limit: usize::MAX,
                ..Default::default()
            };
            assert_eq!(game.select(&selector).len(), 1);
            let named = sunrise_command::args::Selector {
                player_name: Some("fake".into()),
                ..Default::default()
            };
            assert!(game.select(&named).is_empty());
            assert!(game.run_command_quietly("list"));
            assert!(!game.run_command_quietly("tp fake 0 64 0"));
            game.server.lan_acting = None;
            for (conn, learns) in [(fake_id, false), (real_id, true)] {
                game.as_guest(conn, |game| {
                    let mut book =
                        sunrise_items::items::ItemStack::new("minecraft:knowledge_book", 1);
                    book.data_mut().set_other(
                        "minecraft:recipes",
                        Some(sunrise_core::nbt::Tag::List(vec![
                            sunrise_core::nbt::Tag::String("minecraft:stick".into()),
                        ])),
                    );
                    let slot = game.held_slot();
                    *game.stack_in_mut(slot) = Some(book.clone());
                    assert!(game.use_knowledge_book(&book));
                    assert!(game.stack_in(slot).is_none());
                    assert_eq!(game.level.recipe_book.knows("minecraft:stick"), learns);
                })
                .unwrap();
            }
            game.server
                .guest_packets
                .push(Clientbound::SetSimulationDistance(Distance { distance: 4 }));
            game.as_guest(fake_id, |game| {
                assert!(game.server.lan_acting_fake);
                assert!(game.player.as_ref().is_some_and(|p| p.client_loaded));
                assert!(game.player_view().is_none());
                let mut visited = 0;
                game.for_each_player(|_| visited += 1);
                assert_eq!(visited, 0);
                let before = game.stats.get("custom", "minecraft:jump");
                game.award_stat("custom", "minecraft:jump", 1);
                assert_eq!(game.stats.get("custom", "minecraft:jump"), before);
                game.trigger("minecraft:tick", |_, _| panic!("fake criterion evaluated"));
                assert_eq!(game.award_recipes(&["minecraft:test".into()]), 0);
                let source = sunrise_command::args::Selector {
                    kind: 's',
                    limit: 1,
                    ..Default::default()
                };
                assert_eq!(
                    game.select(&source),
                    vec![sunrise_gameplay_common::game::commands_common::Target::Player]
                );
                let named = sunrise_command::args::Selector {
                    player_name: Some("fake".into()),
                    ..Default::default()
                };
                assert!(game.select(&named).is_empty());
                let uuid = sunrise_command::args::Selector {
                    uuid: Some(2),
                    ..Default::default()
                };
                assert!(game.select(&uuid).is_empty());
                game.server
                    .guest_packets
                    .push(Clientbound::SetSimulationDistance(Distance {
                        distance: 12,
                    }));
                game.server.guest_entity_events.push(3);
                game.server
                    .guest_cooldowns
                    .push(("minecraft:test".into(), 20));
            })
            .unwrap();
            assert_eq!(game.server.guest_packets.len(), 1);
            assert!(
                game.server.guest_entity_events.is_empty()
                    && game.server.guest_cooldowns.is_empty()
            );
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[1]
                    .sender
                    .pending
                    .load(Ordering::Relaxed),
                0
            );
            assert!(rx.try_recv().is_err());
            game.with_guest_swap(real_id, game.dim, |game| {
                assert!(!game.server.lan_acting_fake);
                game.server
                    .guest_cooldowns
                    .push(("minecraft:real".into(), 7));
                game.server
                    .guest_player_sounds
                    .push(("outer".into(), [0.0; 3], 1.0, 1.0));
                game.as_guest(fake_id, |game| {
                    assert!(game.server.lan_acting_fake);
                    game.server
                        .guest_cooldowns
                        .push(("minecraft:fake".into(), 20));
                })
                .unwrap();
                assert!(!game.server.lan_acting_fake);
                assert_eq!(game.server.guest_cooldowns.len(), 1);
                assert_eq!(&*game.server.guest_cooldowns[0].0, "minecraft:real");
                assert_eq!(game.server.guest_player_sounds[0].0, "outer");
                game.server.guest_player_sounds.clear();
            })
            .unwrap();
            let out = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
            let sunrise_sim::lan::Out::Packet(body) = out else {
                panic!("expected cooldown packet")
            };
            let mut reader = sunrise_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            let packet = Clientbound::decode(id, reader.rest()).unwrap();
            let Clientbound::Cooldown(cooldown) = packet else {
                panic!("expected cooldown")
            };
            assert_eq!(cooldown.group, "minecraft:real");
            assert_eq!(cooldown.duration, 7);
            assert!(rx.try_recv().is_err());

            // A real turn nested in a fake must not receive the fake's cooldown.
            game.with_guest_swap(fake_id, game.dim, |game| {
                assert!(game.server.lan_acting_fake);
                game.server
                    .guest_cooldowns
                    .push(("minecraft:fake_outer".into(), 8));
                game.server
                    .guest_player_sounds
                    .push(("fake_outer".into(), [0.0; 3], 1.0, 1.0));
                game.as_guest(real_id, |game| {
                    assert!(!game.server.lan_acting_fake);
                    let before = game.stats.get("custom", "minecraft:jump");
                    game.award_stat("custom", "minecraft:jump", 1);
                    assert_eq!(game.stats.get("custom", "minecraft:jump"), before + 1);
                    game.server
                        .guest_cooldowns
                        .push(("minecraft:real_inner".into(), 9));
                })
                .unwrap();
                assert!(game.server.lan_acting_fake);
                assert_eq!(&*game.server.guest_cooldowns[0].0, "minecraft:fake_outer");
                assert_eq!(game.server.guest_player_sounds[0].0, "fake_outer");
                game.server.guest_player_sounds.clear();
            })
            .unwrap();
            let sunrise_sim::lan::Out::Packet(body) =
                rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap()
            else {
                panic!("expected cooldown")
            };
            let mut reader = sunrise_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            let Clientbound::Cooldown(cooldown) = Clientbound::decode(id, reader.rest()).unwrap()
            else {
                panic!("expected cooldown")
            };
            assert_eq!(cooldown.group, "minecraft:real_inner");
            assert!(rx.try_recv().is_err());

            // Two real nested turns deliver each queue to its own connection.
            let (tx2, rx2) = std::sync::mpsc::channel();
            let mut second = Guest::new(
                Joined {
                    conn: 3,
                    uuid: 3,
                    name: "second".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx2, Arc::default()),
                    latency: Arc::default(),
                    address: ([127, 0, 0, 1], 3).into(),
                    expected: false,
                },
                3,
                [0.0; 3],
                10,
            );
            second.placed = true;
            game.server.lan.as_mut().unwrap().hub.guests.push(second);
            game.with_guest_swap(real_id, game.dim, |game| {
                game.server
                    .guest_cooldowns
                    .push(("minecraft:real_outer".into(), 10));
                game.server
                    .guest_player_sounds
                    .push(("real_outer".into(), [0.0; 3], 1.0, 1.0));
                game.as_guest(3, |game| {
                    assert_eq!(game.server.lan_acting, Some(3));
                    assert!(
                        game.as_guest(real_id, |_| panic!("active guest re-entered"))
                            .is_none()
                    );
                    assert_eq!(game.server.lan_acting, Some(3));
                    game.server
                        .guest_cooldowns
                        .push(("minecraft:real_second".into(), 11))
                })
                .unwrap();
                assert_eq!(&*game.server.guest_cooldowns[0].0, "minecraft:real_outer");
                assert_eq!(game.server.guest_player_sounds[0].0, "real_outer");
                game.server.guest_player_sounds.clear();
            })
            .unwrap();
            for (receiver, expected) in [
                (&rx, "minecraft:real_outer"),
                (&rx2, "minecraft:real_second"),
            ] {
                let sunrise_sim::lan::Out::Packet(body) = receiver
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap()
                else {
                    panic!("expected cooldown")
                };
                let mut reader = sunrise_protocol::buf::Reader::new(&body);
                let id = reader.varint().unwrap();
                let Clientbound::Cooldown(cooldown) =
                    Clientbound::decode(id, reader.rest()).unwrap()
                else {
                    panic!("expected cooldown")
                };
                assert_eq!(cooldown.group, expected);
                assert!(receiver.try_recv().is_err());
            }
            let fake = game.fake_player(level, 5678, "joiner").unwrap();
            game.enter_command_level(sunrise_gameplay_common::game::Dim::Nether)
                .unwrap();
            let nether = game.level("minecraft:the_nether").unwrap();
            let other_fake = game.fake_player(nether, 5678, "joiner").unwrap();
            game.enter_command_level(sunrise_gameplay_common::game::Dim::Overworld)
                .unwrap();
            let saved_root = game.server.world_root.clone();
            let scratch = std::env::current_dir()
                .unwrap()
                .join("target/fake-login-save-test");
            let _ = std::fs::remove_dir_all(&scratch);
            game.server.world_root = scratch.clone();
            let shared = &game.server.lan.as_ref().unwrap().hub.lan.shared;
            let conn = shared.next_id().unwrap();
            let (tx, _rx) = std::sync::mpsc::channel();
            shared
                .events
                .send(sunrise_sim::lan::NetEvent::Joined(
                    Box::new(Joined {
                        conn,
                        uuid: 5678,
                        name: "joiner".into(),
                        properties: Vec::new(),
                        info: ClientInformation::new(8),
                        brand: String::new(),
                        sender: PlayerSender::new(tx, Arc::default()),
                        latency: Arc::default(),
                        address: ([127, 0, 0, 1], 1).into(),
                        expected: false,
                    }),
                    None,
                ))
                .unwrap();
            use mod_api;
            struct Logout(std::sync::Arc<std::sync::atomic::AtomicUsize>);
            #[mod_api::__subscriptions]
            impl mod_api::Hooks for Logout {
                fn player_logged_out(
                    &self,
                    _: &mut dyn mod_api::ServerHost,
                    _: &mut mod_api::PlayerLoggedOutEvent<'_>,
                ) {
                    self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
            let logouts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "logout-test",
                Box::new(Logout(logouts.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                game.native_player_logged_out(0, "control");
                assert_eq!(logouts.swap(0, std::sync::atomic::Ordering::Relaxed), 1);
                game.tick_lan_packets();
                assert_eq!(logouts.load(std::sync::atomic::Ordering::Relaxed), 0);
            });
            assert_eq!(game.player_is_fake(fake), None);
            assert_eq!(game.player_is_fake(other_fake), None);
            assert!(
                !sunrise_sim::lan::playerdata::path(&scratch, 5678).exists(),
                "fake login replacement saved player data"
            );
            game.server.world_root = saved_root;
            let matching: Vec<_> = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .filter(|g| g.uuid == 5678)
                .collect();
            assert_eq!(matching.len(), 1);
            assert!(matching[0].participates_online());
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "logout-shutdown-test",
                Box::new(Logout(logouts.clone())),
            )]
            .into();
            let real_count = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .filter(|g| g.placed && g.participates_online())
                .count();
            mod_platform::testing::with_mods(mods, || game.native_players_leaving());
            assert_eq!(logouts.load(Ordering::Relaxed), real_count);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .play
                .entry(fake_id)
                .or_default();
            game.award_leave_game_to_all();
            let host = game.server.lan.as_mut().unwrap();
            assert!(!host.play.get(&fake_id).unwrap().left_game);
            assert!(host.play.get(&real_id).unwrap().left_game);
            host.hub.ticks = 2401;
            assert_eq!(game.ban_ip_target("fake"), None);
            assert_eq!(game.ban_ip_target("real").as_deref(), Some("127.0.0.1"));
            game.kick_idle(1);
            let host = game.server.lan.as_ref().unwrap();
            assert!(host.hub.guest(fake_id).unwrap().left_reason.is_empty());
            assert!(!host.hub.guest(real_id).unwrap().left_reason.is_empty());
            let old = game.fake_player(level, 6789, "reopen").unwrap();
            game.native_lifecycle_world_closed();
            game.native_lifecycle_open_started();
            let reopened = game.level(game.dim.id()).unwrap();
            assert_ne!(level.epoch(), reopened.epoch());
            assert_eq!(game.player_is_fake(old), None);
            let fresh = game.fake_player(reopened, 6789, "reopen").unwrap();
            assert_ne!(fresh, old);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn level_reads_whether_a_block_conducts_redstone() {
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            for (name, conducts) in [
                ("minecraft:stone", true),
                ("minecraft:glass", false),
                ("minecraft:stone_slab", false),
            ] {
                let state = game.pack.assets.registry.default_state(name).unwrap();
                game.world.set(0, 64, 0, state).unwrap();
                assert_eq!(
                    level.is_redstone_conductor(game, [0, 64, 0]),
                    Some(conducts),
                    "{name}"
                );
            }
            // An unloaded position is unknown, not a guess.
            assert_eq!(level.is_redstone_conductor(game, [160, 64, 0]), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn level_sturdy_uses_gameplay_support() {
        use mod_api::SupportType as Support;
        use mod_api::{BlockFace as Face, LevelRef, WorldEpoch};
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let slab = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone_slab")
                .unwrap();
            game.world.set(0, 64, 0, slab).unwrap();
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Down, Support::Full),
                Some(true)
            );
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::North, Support::Full),
                Some(false)
            );
            let wall = game
                .pack
                .assets
                .registry
                .default_state("minecraft:cobblestone_wall")
                .unwrap();
            game.world.set(0, 64, 0, wall).unwrap();
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Center),
                Some(true)
            );
            assert_eq!(
                level.block_face_sturdy(game, [16, 64, 0], Face::Down, Support::Full),
                None
            );
            assert_eq!(
                level.block_face_sturdy(game, [0, 10000, 0], Face::Down, Support::Full),
                None
            );
            assert!(
                game.enter_command_level(sunrise_gameplay_common::game::Dim::Nether)
                    .is_some()
            );
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Center),
                Some(true)
            );
            assert!(
                game.enter_command_level(sunrise_gameplay_common::game::Dim::Overworld)
                    .is_some()
            );
            let shulker = game
                .pack
                .assets
                .registry
                .default_state("minecraft:shulker_box")
                .unwrap();
            game.world.set(0, 64, 0, shulker).unwrap();
            use sunrise_level::shulker_box::{Shulker, ShulkerStatus};
            // The early guest packet pass changes opener status before entity
            // hooks run, and before tick_block_entities synchronizes animations.
            // Exercise that producer with both an absent and a closed snapshot.
            for had_snapshot in [false, true] {
                game.server.be_anims.shulkers.remove(&[0, 64, 0]);
                game.world.shulkers.remove(&[0, 64, 0]);
                if had_snapshot {
                    game.world.shulkers.insert([0, 64, 0], Shulker::default());
                }
                for support in [Support::Full, Support::Center, Support::Rigid] {
                    assert_eq!(
                        level.block_face_sturdy(game, [0, 64, 0], Face::Up, support),
                        Some(true),
                    );
                }
                assert!(game.container_openers_changed((0, 64, 0), true));
                assert_eq!(
                    game.server.be_anims.shulkers[&[0, 64, 0]].status,
                    ShulkerStatus::Opening
                );
                for parked in [false, true] {
                    if parked {
                        assert!(
                            game.enter_command_level(sunrise_gameplay_common::game::Dim::Nether)
                                .is_some()
                        );
                    }
                    for support in [Support::Full, Support::Center, Support::Rigid] {
                        assert_eq!(
                            level.block_face_sturdy(game, [0, 64, 0], Face::Up, support),
                            Some(false),
                        );
                        assert_eq!(
                            level.block_face_sturdy(game, [0, 64, 0], Face::Down, support),
                            Some(true),
                        );
                    }
                }
                assert!(
                    game.enter_command_level(sunrise_gameplay_common::game::Dim::Overworld)
                        .is_some()
                );
                assert!(game.container_openers_changed((0, 64, 0), false));
                assert_eq!(
                    game.world.shulkers[&[0, 64, 0]].status,
                    ShulkerStatus::Closing
                );
                for support in [Support::Full, Support::Center, Support::Rigid] {
                    assert_eq!(
                        level.block_face_sturdy(game, [0, 64, 0], Face::Up, support),
                        Some(false),
                    );
                }
            }
            game.server.be_anims.shulkers.remove(&[0, 64, 0]);
            for status in [
                ShulkerStatus::Closed,
                ShulkerStatus::Opening,
                ShulkerStatus::Opened,
                ShulkerStatus::Closing,
            ] {
                game.world.shulkers.insert(
                    [0, 64, 0],
                    Shulker {
                        status,
                        ..Default::default()
                    },
                );
                for support in [Support::Full, Support::Center, Support::Rigid] {
                    assert_eq!(
                        level.block_face_sturdy(game, [0, 64, 0], Face::Up, support),
                        Some(status == ShulkerStatus::Closed),
                    );
                    assert_eq!(
                        level.block_face_sturdy(game, [0, 64, 0], Face::Down, support),
                        Some(true),
                    );
                }
            }
            assert!(
                game.enter_command_level(sunrise_gameplay_common::game::Dim::Nether)
                    .is_some()
            );
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Full),
                Some(false),
            );
            assert!(
                game.enter_command_level(sunrise_gameplay_common::game::Dim::Overworld)
                    .is_some()
            );
            game.world.shulkers.remove(&[0, 64, 0]);
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Full),
                Some(true),
            );
            let moving = game
                .pack
                .assets
                .registry
                .default_state("minecraft:moving_piston")
                .unwrap();
            game.world.set(0, 64, 0, moving).unwrap();
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Full),
                Some(false),
            );
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.moving_pistons.push((
                [0, 64, 0],
                sunrise_world::piston_shape::MovingPistonState {
                    moved_state: stone,
                    direction: sunrise_core::geometry::Direction::East,
                    extending: true,
                    source: false,
                    progress: 1.0,
                    previous_progress: 1.0,
                },
            ));
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Full),
                Some(true),
            );
            game.world.moving_pistons[0].1.progress = 0.5;
            game.world.moving_pistons[0].1.previous_progress = 0.5;
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Full),
                Some(false),
            );
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert_eq!(
                stale.block_face_sturdy(game, [0, 64, 0], Face::Down, Support::Full),
                None
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_property_write_changes_only_the_addressed_level() {
        use mod_api::{BlockUpdateFlags, LevelWriteError};
        use std::sync::Arc;
        use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
        use sunrise_level::chunk::Chunk;
        use sunrise_world::chunk::BlockState;

        if run_fresh_cases(
            "cut_layout::game::server_side::native_entity_refs::tests::block_property_write_changes_only_the_addressed_level",
            "SUNRISE_BLOCK_PROPERTY_ROUTE_CHILD",
            &["parked"],
        ) {
            return;
        }
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let registry = game.pack.assets.registry;
            let lamp = |lit: &str| {
                registry
                    .resolve(&BlockState {
                        name: "minecraft:redstone_lamp".into(),
                        properties: [("lit".into(), lit.into())].into(),
                    })
                    .unwrap()
            };
            let unlit = lamp("false");
            let lit = lamp("true");
            let [x, y, z] = [2, 65, 3];
            let pos = [x, y, z];
            let flags = BlockUpdateFlags::CLIENTS | BlockUpdateFlags::KNOWN_SHAPE;
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.world.set(x, y, z, unlit).unwrap();
            let level = game.level("minecraft:overworld").unwrap();
            assert_eq!(
                level.set_block_property(game, pos, "lit", "false", flags),
                Ok(false)
            );

            let nether = sunrise_gameplay_common::game::Dim::Nether;
            game.enter_command_level(nether).unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.world.set(x, y, z, unlit).unwrap();

            game.storage_scope(|game, _| {
                assert_eq!(
                    level.set_block_property(game, pos, "lit", "true", flags),
                    Err(LevelWriteError::InTransaction)
                );
                ((), false)
            });
            assert_eq!(
                game.native_level_world(level).unwrap().block(x, y, z),
                unlit
            );

            assert_eq!(
                level.set_block_property(game, pos, "lit", "true", flags),
                Ok(true)
            );
            assert_eq!(game.dim, nether);
            assert_eq!(game.world.block(x, y, z), unlit);
            assert_eq!(game.native_level_world(level).unwrap().block(x, y, z), lit);
            assert_eq!(
                level.set_block_property(game, pos, "lit", "true", flags),
                Ok(false)
            );
            assert_eq!(game.dim, nether);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_mobs_equipment_drop_chance_reads_its_recorded_chance_per_slot() {
        use mod_api::{EntityRef, EquipmentSlot, LivingRef};
        use sunrise_level::chunk::Chunk;
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
            let id = game
                .spawn_mob(
                    "minecraft:zombie",
                    [2.5, 64.0, 2.5],
                    true,
                    sunrise_gameplay_common::game::creatures::SpawnSite::Finalizing(
                        sunrise_sim::mobs::spawn_rules::Reason::Command,
                    ),
                )
                .unwrap();
            let mob = game.living(game.entity(id).unwrap()).unwrap();
            let zombie = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
            zombie.extra.drop_chances = None;
            assert_eq!(
                mob.equipment_drop_chance(game, EquipmentSlot::Head),
                Some(0.085)
            );
            assert_eq!(
                mob.equipment_drop_chance(game, EquipmentSlot::Saddle),
                Some(2.0)
            );
            // `EQUIPMENT_SLOTS` order: mainhand, offhand, feet, legs, chest, head, body, saddle.
            let zombie = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
            zombie.extra.drop_chances = Some(Box::new([0.1, 0.2, 0.3, 0.4, 0.5, 2.0, 0.7, 0.0]));
            for (slot, chance) in [
                (EquipmentSlot::MainHand, 0.1),
                (EquipmentSlot::OffHand, 0.2),
                (EquipmentSlot::Feet, 0.3),
                (EquipmentSlot::Legs, 0.4),
                (EquipmentSlot::Chest, 0.5),
                (EquipmentSlot::Head, 2.0),
                (EquipmentSlot::Body, 0.7),
                (EquipmentSlot::Saddle, 0.0),
            ] {
                assert_eq!(
                    mob.equipment_drop_chance(game, slot),
                    Some(chance),
                    "{slot:?}"
                );
            }
            let gone = LivingRef::new(EntityRef::new(mob.entity().epoch(), 999));
            assert_eq!(gone.equipment_drop_chance(game, EquipmentSlot::Head), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn destroy_block_passes_breaker_to_loot_and_game_event() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::native_entity_refs::tests::destroy_block_passes_breaker_to_loot_and_game_event",
            || {
                use mod_api::{BlockUpdateFlags, EntityRef, Flow, Hooks, ModEntry, ServerHost};
                use std::sync::{Arc, Mutex};
                use sunrise_core::geometry::Vec3;
                use sunrise_game_data::who::EventSource;
                use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
                use sunrise_level::chunk::Chunk;
                use sunrise_sim::mobs::Mob;

                struct Observer {
                    drop_breakers: Arc<Mutex<Vec<Option<EntityRef>>>>,
                    destroy_causes: Arc<Mutex<Vec<Option<u32>>>>,
                }
                #[mod_api::__subscriptions]
                impl Hooks for Observer {
                    fn block_drops(
                        &self,
                        _: &dyn mod_api::LevelHost,
                        event: &mut mod_api::BlockDropsEvent<'_>,
                    ) -> Flow {
                        self.drop_breakers
                            .lock()
                            .unwrap()
                            .push(event.breaker_entity);
                        Flow::Continue
                    }
                    fn vanilla_game_event(
                        &self,
                        _: &mut dyn ServerHost,
                        event: &mut mod_api::VanillaGameEventEvent<'_>,
                    ) -> Flow {
                        if event.event == "minecraft:block_destroy" {
                            self.destroy_causes.lock().unwrap().push(event.cause);
                        }
                        Flow::Continue
                    }
                }

                let drop_breakers = Arc::new(Mutex::new(Vec::new()));
                let destroy_causes = Arc::new(Mutex::new(Vec::new()));
                mod_platform::testing::with_mods(
                    vec![ModEntry::new(
                        "destroy_breaker_observer",
                        Box::new(Observer {
                            drop_breakers: drop_breakers.clone(),
                            destroy_causes: destroy_causes.clone(),
                        }),
                    )]
                    .into(),
                    || {
                        with_game(|game| {
                            game.options.dedicated = Some(Vec::new());
                            game.sculk_prepare();
                            game.server
                                .streaming
                                .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
                            let level = game.level(game.dim.id()).unwrap();
                            let reg = game.pack.assets.registry;
                            let [chorus_flower, sensor, air] = [
                                "minecraft:chorus_flower",
                                "minecraft:sculk_sensor",
                                "minecraft:air",
                            ]
                            .map(|name| reg.default_state(name).unwrap());
                            let flags = BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS;
                            let pig_kind =
                                game.res.mob_types().get("minecraft:pig").unwrap().clone();
                            game.mobs
                                .push(Mob::new(42, pig_kind, Vec3::new(0.5, 64.0, 2.5), 0.0));
                            let pig = game.entity(42).unwrap();
                            assert_eq!(
                                game.level_set_block(level, [2, 64, 0], sensor, flags),
                                Ok(true)
                            );
                            game.sculk_sync_index();

                            let sensor_vibration =
                                |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                                    let Some(sunrise_level::containers::BlockEntity::Sculk(sculk)) =
                                        game.world.block_entities.get_mut(&(2, 64, 0))
                                    else {
                                        panic!("sculk sensor missing");
                                    };
                                    sculk
                                        .vibration
                                        .selector
                                        .current
                                        .take()
                                        .map(|(info, _)| info)
                                };

                            // Without a breaker, chorus flower loot has no `this` entity and sculk has no source.
                            assert_eq!(
                                game.level_set_block(level, [0, 64, 0], chorus_flower, flags),
                                Ok(true)
                            );
                            let _ = sensor_vibration(game);
                            assert_eq!(
                                game.level_destroy_block(
                                    level,
                                    [0, 64, 0],
                                    true,
                                    None,
                                    None,
                                    false
                                ),
                                Some(Vec::new())
                            );
                            assert!(game.dropped.is_empty());
                            let info = sensor_vibration(game).expect("sculk sensor heard break");
                            assert_eq!(info.source, None);

                            // Stale breaker refuses before breaking the block.
                            assert_eq!(
                                game.level_set_block(level, [0, 64, 0], chorus_flower, flags),
                                Ok(true)
                            );
                            let stale = EntityRef::new(
                                mod_api::WorldEpoch::new(pig.epoch().get() + 1),
                                pig.id(),
                            );
                            assert!(
                                game.level_destroy_block(
                                    level,
                                    [0, 64, 0],
                                    true,
                                    None,
                                    Some(stale),
                                    false
                                )
                                .is_none()
                            );
                            assert_eq!(game.world.block(0, 64, 0), chorus_flower);

                            // With a breaker outside a transaction: chorus_flower drops, hooks & sculk see pig.
                            let _ = sensor_vibration(game);
                            assert_eq!(
                                game.level_destroy_block(
                                    level,
                                    [0, 64, 0],
                                    true,
                                    None,
                                    Some(pig),
                                    false
                                ),
                                Some(Vec::new())
                            );
                            assert_eq!(game.world.block(0, 64, 0), air);
                            assert!(
                                game.dropped
                                    .iter()
                                    .any(|item| item.stack.item.as_ref()
                                        == "minecraft:chorus_flower")
                            );
                            let info = sensor_vibration(game).expect("sculk sensor heard break");
                            assert_eq!(info.source, Some(EventSource::Mob(42)));

                            // Same with a breaker inside a committed transaction.
                            game.dropped.clear();
                            assert_eq!(
                                game.level_set_block(level, [0, 64, 0], chorus_flower, flags),
                                Ok(true)
                            );
                            let _ = sensor_vibration(game);
                            let ok: Result<(), ()> = game.storage_transaction(|game, _| {
                                assert_eq!(
                                    game.level_destroy_block(
                                        level,
                                        [0, 64, 0],
                                        true,
                                        None,
                                        Some(pig),
                                        false
                                    ),
                                    Some(Vec::new())
                                );
                                assert!(game.dropped.is_empty());
                                Ok(())
                            });
                            assert!(ok.is_ok());
                            assert_eq!(game.world.block(0, 64, 0), air);
                            assert!(
                                game.dropped
                                    .iter()
                                    .any(|item| item.stack.item.as_ref()
                                        == "minecraft:chorus_flower")
                            );
                            let info = sensor_vibration(game).expect("sculk sensor heard break");
                            assert_eq!(info.source, Some(EventSource::Mob(42)));

                            assert_eq!(
                                *drop_breakers.lock().unwrap(),
                                vec![None, Some(pig), Some(pig)]
                            );
                            assert_eq!(
                                *destroy_causes.lock().unwrap(),
                                vec![None, Some(42), Some(42)]
                            );
                        });
                    },
                );
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn level_clip_fluid_modes_shapes_and_solid_on_tie() {
        use mod_api::{
            BlockFace as Face, ClipBlock as Block, ClipFluid as Fluid, CollisionContext as Context,
        };
        use std::sync::Arc;
        use sunrise_gameplay_common::game::tables::GameItems as _;
        use sunrise_level::chunk::Chunk;
        with_game(|game| {
            // Fluid clips read the tag tables once the world has built them (`native_tag_items`).
            let root = tempfile::tempdir().unwrap();
            game.server.session_lock.0 =
                Some(sunrise_sim::save::SessionLock::acquire(root.path()).unwrap());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.world
                .chunks
                .insert((0, -1), Arc::new(Chunk::empty(0, -1)));
            let level = game.level(game.dim.id()).unwrap();
            let stone = game.block_state_id("minecraft:stone").unwrap();
            let water_src = game.block_state_id("minecraft:water[level=0]").unwrap();
            let water_flow3 = game.block_state_id("minecraft:water[level=5]").unwrap();
            let water_flow4 = game.block_state_id("minecraft:water[level=4]").unwrap();
            let lava_src = game.block_state_id("minecraft:lava[level=0]").unwrap();
            let lava_flow6 = game.block_state_id("minecraft:lava[level=2]").unwrap();
            let slab_bot = game
                .block_state_id("minecraft:oak_slab[type=bottom,waterlogged=true]")
                .unwrap();
            let slab_top = game
                .block_state_id("minecraft:oak_slab[type=top,waterlogged=true]")
                .unwrap();
            let h8 = f64::from(8.0_f32 / 9.0);
            let h6 = f64::from(6.0_f32 / 9.0);
            let h3 = f64::from(3.0_f32 / 9.0);

            // Matrix over water/lava (source vs flowing) and waterlogged slab with stone below.
            let cells = [
                (0, water_src, h8, [false, true, true, true]),
                (1, water_flow3, h3, [false, false, true, true]),
                (2, lava_src, h8, [false, true, true, false]),
                (3, lava_flow6, h6, [false, false, true, false]),
                (4, slab_bot, h8, [false, true, true, true]),
            ];
            for &(x, state, _, _) in &cells {
                game.world.set(x, 63, 0, stone).unwrap();
                game.world.set(x, 64, 0, state).unwrap();
            }
            // Flowing water under same fluid lifts its shape height to 1.0.
            game.world.set(5, 64, 0, water_flow4).unwrap();
            game.world.set(5, 65, 0, water_src).unwrap();
            // Solid-on-tie comparison: plain water vs waterlogged top slab.
            game.world.set(6, 64, 0, water_src).unwrap();
            game.world.set(7, 64, 0, slab_top).unwrap();

            let modes = [Fluid::None, Fluid::SourceOnly, Fluid::Any, Fluid::Water];
            for &(x, state, height, stops) in &cells {
                let cx = f64::from(x) + 0.5;
                for (mode, expect_fluid) in modes.into_iter().zip(stops) {
                    let hit = hit_of(level.clip(
                        game,
                        [cx, 65.5, 0.5],
                        [cx, 62.5, 0.5],
                        Block::Outline,
                        mode,
                        Context::Empty,
                    ));
                    assert!(!hit.miss, "x={x} mode={mode:?}");
                    if expect_fluid {
                        assert_eq!(hit.block, [x, 64, 0]);
                        assert_eq!(hit.face, Face::Up);
                        assert_eq!(hit.location[1], 64.0 + height);
                    } else if state == slab_bot {
                        assert_eq!(hit.block, [x, 64, 0]);
                        assert_eq!(hit.location[1], 64.5);
                    } else {
                        assert_eq!(hit.block, [x, 63, 0]);
                        assert_eq!(hit.location[1], 64.0);
                    }
                }
            }

            let deep = hit_of(level.clip(
                game,
                [5.5, 64.95, -0.5],
                [5.5, 64.95, 0.5],
                Block::Outline,
                Fluid::Any,
                Context::Empty,
            ));
            assert!(!deep.miss);
            assert_eq!((deep.block, deep.face), ([5, 64, 0], Face::North));

            let plain = hit_of(level.clip(
                game,
                [6.5, 64.0 + h8 + 0.5, -0.5],
                [6.5, 64.0 + h8 - 0.5, 0.5],
                Block::Outline,
                Fluid::Any,
                Context::Empty,
            ));
            assert_eq!((plain.block, plain.face), ([6, 64, 0], Face::Up));
            let tied = hit_of(level.clip(
                game,
                [7.5, 64.0 + h8 + 0.5, -0.5],
                [7.5, 64.0 + h8 - 0.5, 0.5],
                Block::Outline,
                Fluid::Any,
                Context::Empty,
            ));
            assert_eq!((tied.block, tied.face), ([7, 64, 0], Face::North));

            // Tagging a non-water-named fluid into minecraft:water makes ClipFluid::Water hit it.
            let mut tagged = game.items().clone();
            tagged
                .fluid_tags
                .entry("minecraft:water".into())
                .or_default()
                .insert("minecraft:lava".into());
            game.common.server_items = Some(Arc::new(tagged));
            let tagged_hit = hit_of(level.clip(
                game,
                [2.5, 65.5, 0.5],
                [2.5, 62.5, 0.5],
                Block::Outline,
                Fluid::Water,
                Context::Empty,
            ));
            assert_eq!((tagged_hit.block, tagged_hit.face), ([2, 64, 0], Face::Up));
            assert_eq!(tagged_hit.location[1], 64.0 + h8);
            game.common.server_items = None;
            assert_eq!(game.world.chunks.len(), 2, "clip loads no chunks");
            game.server.session_lock.0 = None;
        });
    }

    /// The hit of a ray that found one; any other outcome fails the test.
    fn hit_of(outcome: Result<mod_api::ClipOutcome, mod_api::ClipError>) -> mod_api::ClipHit {
        match outcome {
            Ok(mod_api::ClipOutcome::Hit(hit)) => hit,
            other => panic!("expected a hit, got {other:?}"),
        }
    }

    /// `CollisionContext.empty()` against `CollisionContext.of(entity)`: scaffolding is solid to
    /// a ray with no entity and to one an entity above it asks for, and open to one an entity
    /// below it asks for (`ScaffoldingBlock.getCollisionShape` with `isAbove`).
    #[test]
    #[ignore = "daily"]
    fn level_clip_answers_shapes_for_the_entity_in_its_context() {
        use mod_api::{
            ClipBlock as Block, ClipFluid as Fluid, ClipOutcome, CollisionContext as Context,
        };
        use std::sync::Arc;
        use sunrise_core::geometry::Vec3;
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::mobs::Mob;
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let scaffolding = game
                .pack
                .assets
                .registry
                .default_state("minecraft:scaffolding")
                .unwrap();
            game.world.set(1, 64, 0, scaffolding).unwrap();
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(12, zombie.clone(), Vec3::new(1.5, 60.0, 0.5), 0.0));
            game.mobs
                .push(Mob::new(13, zombie, Vec3::new(1.5, 70.0, 0.5), 0.0));
            let (below, above) = (game.entity(12).unwrap(), game.entity(13).unwrap());
            let (from, to) = ([1.5, 66.0, 0.5], [1.5, 64.5, 0.5]);
            let clip = |game: &sunrise_gameplay::game::ServerGame<'_>, context| {
                level.clip(game, from, to, Block::Collider, Fluid::None, context)
            };
            let empty = hit_of(clip(game, Context::Empty));
            assert_eq!(empty.block, [1, 64, 0]);
            assert_eq!(empty.location, [1.5, 65.0, 0.5]);
            let from_above = hit_of(clip(game, Context::Entity(above)));
            assert_eq!(from_above.location, empty.location);
            assert_eq!(clip(game, Context::Entity(below)), Ok(ClipOutcome::Miss));
            let gone = mod_api::EntityRef::new(
                mod_api::WorldEpoch::new(level.epoch().get() + 1),
                above.id(),
            );
            assert_eq!(
                clip(game, Context::Entity(gone)),
                Err(mod_api::ClipError::StaleLevel)
            );
        });
    }

    /// `ClipContext.Block.VISUAL` (`BlockState.getVisualShape`): a ray passes glass, iron
    /// bars and powder snow, which a collider ray stops on (powder snow has no empty-context
    /// collision either), and stops on soul sand, mud, a snow layer and a fence at their
    /// visual top, not their collision top.
    #[test]
    #[ignore = "daily"]
    fn level_clip_visual_passes_see_through_blocks_and_stops_on_visual_shapes() {
        use mod_api::{
            BlockFace as Face, ClipBlock as Block, ClipFluid as Fluid, ClipOutcome,
            CollisionContext as Context,
        };
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let place = |game: &mut sunrise_gameplay::game::ServerGame<'_>, block: &str| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let state = game.pack.assets.registry.default_state(block).unwrap();
                game.world.set(1, 64, 0, state).unwrap();
            };
            let (from, to) = ([0.5, 64.5, 0.5], [2.5, 64.5, 0.5]);
            for (block, collider) in [
                ("minecraft:glass", Some([1.0, 64.5, 0.5])),
                ("minecraft:iron_bars", Some([1.4375, 64.5, 0.5])),
                ("minecraft:powder_snow", None),
            ] {
                place(game, block);
                assert_eq!(
                    level.clip(game, from, to, Block::Visual, Fluid::None, Context::Empty),
                    Ok(ClipOutcome::Miss),
                    "{block}"
                );
                let hit = level.clip(game, from, to, Block::Collider, Fluid::None, Context::Empty);
                let location = match hit {
                    Ok(ClipOutcome::Hit(hit)) => Some(hit.location),
                    Ok(ClipOutcome::Miss) => None,
                    other => panic!("{block}: {other:?}"),
                };
                assert_eq!(location, collider, "{block}");
            }
            let (from, to) = ([1.5, 66.0, 0.5], [1.5, 63.5, 0.5]);
            for (block, visual, collider) in [
                ("minecraft:soul_sand", 65.0, Some(64.875)),
                ("minecraft:mud", 65.0, Some(64.875)),
                ("minecraft:snow", 64.125, None),
                ("minecraft:oak_fence", 65.0, Some(65.5)),
            ] {
                place(game, block);
                let hit =
                    hit_of(level.clip(game, from, to, Block::Visual, Fluid::None, Context::Empty));
                assert!(!hit.miss, "{block}");
                assert_eq!(hit.block, [1, 64, 0], "{block}");
                assert_eq!(hit.face, Face::Up, "{block}");
                assert_eq!(hit.location, [1.5, visual, 0.5], "{block}");
                assert!(!hit.inside && !hit.world_border, "{block}");
                let hit = level.clip(game, from, to, Block::Collider, Fluid::None, Context::Empty);
                let top = match hit {
                    Ok(ClipOutcome::Hit(hit)) => Some(hit.location[1]),
                    Ok(ClipOutcome::Miss) => None,
                    other => panic!("{block}: {other:?}"),
                };
                assert_eq!(top, collider, "{block}");
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn level_clip_reads_parked_world_and_stops_at_unloaded_columns() {
        use mod_api::{
            BlockFace as Face, ClipBlock as Block, ClipError, ClipFluid as Fluid, ClipOutcome,
            CollisionContext as Context,
        };
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(1, 64, 0, stone).unwrap();
            let from = [0.5, 64.5, 0.5];
            let to = [2.5, 64.5, 0.5];
            for mode in [Block::Collider, Block::Outline] {
                let hit = hit_of(level.clip(game, from, to, mode, Fluid::None, Context::Empty));
                assert!(!hit.miss);
                assert_eq!(hit.block, [1, 64, 0]);
                assert_eq!(hit.face, Face::West);
                assert_eq!(hit.location, [1.0, 64.5, 0.5]);
                assert!(!hit.inside && !hit.world_border);
            }
            let chunks_before_gap = game.world.chunks.len();
            // Crossing into a missing column stops at its first cell: neither a miss nor air,
            // and nothing is loaded.
            assert_eq!(
                level.clip(
                    game,
                    [15.5, 64.5, 0.5],
                    [17.5, 64.5, 0.5],
                    Block::Collider,
                    Fluid::None,
                    Context::Empty,
                ),
                Ok(ClipOutcome::Unloaded { at: [16, 64, 0] })
            );
            // A ray that starts inside a missing column stops at its first cell too.
            assert_eq!(
                level.clip(
                    game,
                    [17.5, 64.5, 0.5],
                    [15.5, 64.5, 0.5],
                    Block::Outline,
                    Fluid::None,
                    Context::Empty,
                ),
                Ok(ClipOutcome::Unloaded { at: [17, 64, 0] })
            );
            assert_eq!(
                level.line_of_sight(game, [15.5, 64.5, 0.5], [17.5, 64.5, 0.5]),
                None,
                "an unloaded column answers neither way"
            );
            // Cells out of build height are air whatever the column below holds, so a ray there
            // is a miss, even across a missing column.
            for (from, to, mode) in [
                ([0.5, 325.5, 0.5], [2.5, 325.5, 0.5], Block::Collider),
                ([15.5, 325.5, 0.5], [17.5, 325.5, 0.5], Block::Collider),
                ([0.5, -70.5, 0.5], [0.5, -66.5, 0.5], Block::Outline),
                ([15.5, -70.5, 0.5], [17.5, -70.5, 0.5], Block::Outline),
            ] {
                assert_eq!(
                    level.clip(game, from, to, mode, Fluid::None, Context::Empty),
                    Ok(ClipOutcome::Miss),
                    "{from:?} to {to:?}"
                );
            }
            assert!(!game.world.chunks.contains_key(&(1, 0)));
            assert_eq!(game.world.chunks.len(), chunks_before_gap);
            // A straight ray through 300 loaded columns has no length cap: it hits a block more
            // than 4096 cells away, and misses the row above it.
            for cx in 1..=300 {
                game.world
                    .chunks
                    .insert((cx, 0), Arc::new(Chunk::empty(cx, 0)));
            }
            game.world.set(4805, 64, 0, stone).unwrap();
            let chunks_with_far = game.world.chunks.len();
            for mode in [Block::Collider, Block::Outline] {
                let far_hit = hit_of(level.clip(
                    game,
                    [15.5, 64.5, 0.5],
                    [4810.5, 64.5, 0.5],
                    mode,
                    Fluid::None,
                    Context::Empty,
                ));
                assert!(!far_hit.miss);
                assert_eq!(far_hit.block, [4805, 64, 0]);
                assert_eq!(far_hit.face, Face::West);
                assert_eq!(far_hit.location, [4805.0, 64.5, 0.5]);
                assert!(!far_hit.inside && !far_hit.world_border);
            }
            // Independent opposite hit face and a miss beyond 4096.
            let east_hit = hit_of(level.clip(
                game,
                [4810.5, 64.5, 0.5],
                [4800.5, 64.5, 0.5],
                Block::Collider,
                Fluid::None,
                Context::Empty,
            ));
            assert_eq!(east_hit.block, [4805, 64, 0]);
            assert_eq!(east_hit.face, Face::East);
            assert_eq!(east_hit.location, [4806.0, 64.5, 0.5]);
            assert_eq!(
                level.clip(
                    game,
                    [15.5, 65.5, 0.5],
                    [4810.5, 65.5, 0.5],
                    Block::Collider,
                    Fluid::None,
                    Context::Empty,
                ),
                Ok(ClipOutcome::Miss)
            );
            // `line_of_sight` delegates to the same segment clip with no 128-block entity cap.
            assert_eq!(
                level.line_of_sight(game, [15.5, 64.5, 0.5], [4810.5, 64.5, 0.5]),
                Some(false)
            );
            assert_eq!(
                level.line_of_sight(game, [15.5, 65.5, 0.5], [4810.5, 65.5, 0.5]),
                Some(true)
            );
            assert_eq!(game.world.chunks.len(), chunks_with_far);
            assert_eq!(
                level.clip(
                    game,
                    [0.5, 67.5, 0.5],
                    [2.5, 67.5, 0.5],
                    Block::Collider,
                    Fluid::None,
                    Context::Empty,
                ),
                Ok(ClipOutcome::Miss)
            );
            let inside = hit_of(level.clip(
                game,
                [1.5, 64.5, 0.5],
                [2.5, 64.5, 0.5],
                Block::Collider,
                Fluid::None,
                Context::Empty,
            ));
            assert!(inside.inside && !inside.miss);
            for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                for (from, to) in [
                    ([bad, 64.0, 0.0], to),
                    (from, [0.0, bad, 0.0]),
                    ([0.0, 64.0, bad], [bad, bad, bad]),
                ] {
                    assert_eq!(
                        level.clip(game, from, to, Block::Collider, Fluid::None, Context::Empty),
                        Err(ClipError::NonFiniteRay)
                    );
                }
            }
            let stale = mod_api::LevelRef::new(
                mod_api::WorldEpoch::new(level.epoch().get() + 1),
                level.dimension(),
            );
            assert_eq!(
                stale.clip(game, from, to, Block::Collider, Fluid::None, Context::Empty),
                Err(ClipError::StaleLevel)
            );
            use sunrise_world::world_ray::RayWorld;
            let mut ordinary = sunrise_gameplay_common::game::collider_rays::ColliderRays {
                scene: sunrise_level::collisions::SceneCollisions {
                    world: &game.world,
                    binding: game.collision,
                    subject: None,
                },
                context: sunrise_world::collision_context::CollisionContext::empty(false),
                clip_data: None,
                report_unloaded: false,
            };
            assert!(
                ordinary
                    .interaction_shape(
                        sunrise_world::registry::BlockStateId(u32::MAX),
                        sunrise_core::position::BlockPos::new(0, 0, 0)
                    )
                    .unwrap()
                    .is_empty()
            );
            // Dynamic collision shapes (bamboo) work on the collider branch; outline reports
            // an actual unsupported error instead of fabricating a miss.
            let bamboo = game
                .pack
                .assets
                .registry
                .default_state("minecraft:bamboo")
                .unwrap();
            game.world.set(1, 64, 2, bamboo).unwrap();
            let bx = 1.3833333402872086;
            let bz = 2.3166666701436043;
            let bamboo_hit = hit_of(level.clip(
                game,
                [bx, 66.0, bz],
                [bx, 62.0, bz],
                Block::Collider,
                Fluid::None,
                Context::Empty,
            ));
            assert!(!bamboo_hit.miss);
            assert_eq!(bamboo_hit.block, [1, 64, 2]);
            assert_eq!(bamboo_hit.face, Face::Up);
            assert_eq!(bamboo_hit.location, [bx, 65.0, bz]);
            assert_eq!(
                level.clip(
                    game,
                    [bx, 66.0, bz],
                    [bx, 62.0, bz],
                    Block::Outline,
                    Fluid::None,
                    Context::Empty,
                ),
                Err(ClipError::StaleLevel)
            );
            let plant = game
                .pack
                .assets
                .registry
                .default_state("minecraft:dandelion")
                .unwrap();
            game.world.set(4, 64, 4, plant).unwrap();
            let facts = game.res.facts().get(plant);
            assert!(facts.outline_offset);
            let offset = sunrise_gameplay_common::game::native_state_shapes::outline_offset(
                game.res.facts(),
                plant,
                4,
                4,
                facts.outline_offset,
            );
            assert_ne!(offset[0], 0.0);
            let b = facts.outline[0];
            let y = 64.0 + offset[1] + (b[1] + b[4]) / 2.0;
            let z = 4.0 + offset[2] + (b[2] + b[5]) / 2.0;
            let plant_hit = hit_of(level.clip(
                game,
                [3.0, y, z],
                [5.0, y, z],
                Block::Outline,
                Fluid::None,
                Context::Empty,
            ));
            assert_eq!(plant_hit.block, [4, 64, 4]);
            assert!((plant_hit.location[0] - (4.0 + offset[0] + b[0])).abs() < 1e-9);
            let cauldron = game
                .pack
                .assets
                .registry
                .default_state("minecraft:cauldron")
                .unwrap();
            game.world.set(0, 64, 2, cauldron).unwrap();
            let bowl = hit_of(level.clip(
                game,
                [0.5, 65.27, 0.5],
                [0.5, 64.85, 3.5],
                Block::Outline,
                Fluid::None,
                Context::Empty,
            ));
            assert!(!bowl.miss);
            assert_eq!(bowl.block, [0, 64, 2]);
            assert_eq!(bowl.face, Face::Up);
            assert!(
                game.enter_command_level(sunrise_gameplay_common::game::Dim::Nether)
                    .is_some()
            );
            assert_eq!(
                hit_of(level.clip(game, from, to, Block::Collider, Fluid::None, Context::Empty))
                    .block,
                [1, 64, 0]
            );
            assert_eq!(
                hit_of(level.clip(
                    game,
                    [15.5, 64.5, 0.5],
                    [4810.5, 64.5, 0.5],
                    Block::Collider,
                    Fluid::None,
                    Context::Empty,
                ))
                .block,
                [4805, 64, 0]
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn living_attributes_answer_for_mobs_stands_and_the_dragon() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::{entities::ArmorStand, mobs::Mob};
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            game.armor_stands.push(ArmorStand::new(13, Vec3::ZERO, 0.0));
            let mut dragon =
                sunrise_gameplay_common::game::dragon_common::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 14;
            game.dragon.replace(dragon);
            for (id, max) in [(12, 20.0), (13, 20.0), (14, 200.0)] {
                let living = game.living(game.entity(id).unwrap()).unwrap();
                assert_eq!(
                    living.attribute_base(game, "minecraft:max_health"),
                    Some(max)
                );
                assert_eq!(
                    living.attribute_value(game, "minecraft:max_health"),
                    Some(max)
                );
                assert_eq!(living.attribute_value(game, "missing:attribute"), None);
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn level_shapes_use_empty_context() {
        use crate::game::load_resources;
        use crate::settings::Settings;
        use mod_api::{Authority, BlockShapeKind};
        use std::{sync::Arc, time::Instant};
        use sunrise_gameplay_common::game::{Dim, Options, Progress};
        use sunrise_level::chunk::Chunk;

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        let saves = tempfile::tempdir().unwrap();
        options.game_dir = Some(saves.path().to_path_buf());
        options.resources = root.clone();
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
        game.server.world_root = saves.path().to_path_buf();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        {
            let host: &dyn mod_api::ServerHost = &game;
            assert_eq!(
                host.levels()
                    .visit_levels(&mut |_| panic!("level before world open")),
                Ok(ControlFlow::Continue(()))
            );
        }
        game.native_lifecycle
            .begin_open(Some(Authority::IntegratedServer));
        game.native_level_opened(Dim::Overworld);

        let level = game.level("minecraft:overworld").unwrap();
        assert!(game.enter_command_level(Dim::Nether).is_some());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(0, 64, 0, stone).unwrap();
        let nether = game.level("minecraft:the_nether").unwrap();
        let host: &dyn mod_api::ServerHost = &game;
        let levels = host.levels();
        let mut visited = Vec::new();
        assert_eq!(
            levels.visit_levels(&mut |level| {
                visited.push(level);
                ControlFlow::Continue(())
            }),
            Ok(ControlFlow::Continue(()))
        );
        // The parked Overworld precedes the active Nether.
        assert_eq!(visited, [level, nether]);
        visited.clear();
        assert_eq!(
            levels.visit_levels(&mut |level| {
                visited.push(level);
                ControlFlow::Break(())
            }),
            Ok(ControlFlow::Break(()))
        );
        assert_eq!(visited, [level]);
        assert!(game.enter_command_level(Dim::Overworld).is_some());
        let cube = vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]];
        assert_eq!(
            game.collect_level_block_shape(nether, stone, [0, 64, 0], BlockShapeKind::Outline),
            Some(cube.clone())
        );
        assert_eq!(
            game.collect_level_block_shape(nether, stone, [0, 64, 0], BlockShapeKind::Collision),
            Some(cube)
        );
        let shape = |game: &mut sunrise_gameplay::game::ServerGame<'_>, name: &str, kind| {
            let state = game.pack.assets.registry.default_state(name).unwrap();
            game.world.set(0, 64, 0, state).unwrap();
            game.collect_level_block_shape(level, state, [0, 64, 0], kind)
                .unwrap()
        };
        game.common.inventory[game.common.selected] = Some(sunrise_items::items::ItemStack {
            item: "minecraft:light".into(),
            count: 1,
            damage: 0,
            extra: None,
        });
        assert!(shape(&mut game, "minecraft:light", BlockShapeKind::Outline).is_empty());
        assert_eq!(
            game.outline_boxes([0, 64, 0]),
            vec![[0.0, 64.0, 0.0, 1.0, 65.0, 1.0]]
        );
        game.common.inventory[game.common.selected]
            .as_mut()
            .unwrap()
            .item = "minecraft:scaffolding".into();
        let scaffolding = shape(&mut game, "minecraft:scaffolding", BlockShapeKind::Outline);
        assert!(!scaffolding.is_empty());
        assert_ne!(scaffolding, vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]);
        assert_eq!(
            game.outline_boxes([0, 64, 0]),
            vec![[0.0, 64.0, 0.0, 1.0, 65.0, 1.0]]
        );
        assert_eq!(
            shape(&mut game, "minecraft:stone_slab", BlockShapeKind::Outline),
            vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]
        );
        assert_eq!(
            shape(&mut game, "minecraft:stone_slab", BlockShapeKind::Collision),
            vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]
        );
        let contains = |boxes: &[[f64; 6]], p: [f64; 3]| {
            boxes
                .iter()
                .any(|b| (0..3).all(|i| b[i] < p[i] && p[i] < b[i + 3]))
        };
        let stair = shape(&mut game, "minecraft:oak_stairs", BlockShapeKind::Outline);
        assert!(contains(&stair, [0.25, 0.25, 0.25]));
        let upper = [
            [0.25, 0.75, 0.25],
            [0.75, 0.75, 0.25],
            [0.25, 0.75, 0.75],
            [0.75, 0.75, 0.75],
        ];
        assert_eq!(
            upper.into_iter().filter(|p| contains(&stair, *p)).count(),
            2
        );
        let fence = shape(&mut game, "minecraft:oak_fence", BlockShapeKind::Outline);
        assert!(contains(&fence, [0.5, 0.5, 0.5]));
        assert!(!contains(&fence, [0.1, 0.5, 0.1]));
        let lectern = shape(&mut game, "minecraft:lectern", BlockShapeKind::Collision);
        assert!(contains(&lectern, [0.5, 0.5, 0.5]));
        assert!(!contains(&lectern, [0.1, 0.5, 0.1]));
    }

    #[test]
    fn uuid_words_are_most_significant_first() {
        let words = [0x0102_0304, -1, 0, 0x7fff_ffff];
        assert_eq!(
            sunrise_gameplay::game::native_entity_refs::uuid_from_words(words),
            0x0102_0304_ffff_ffff_0000_0000_7fff_ffff
        );
    }

    /// `LevelRef::drop_resources` (#5526) drops what `destroy_block` drops, without breaking
    /// the block: a stone gives its cobblestone and stays, a chest gives itself and keeps its
    /// contents, nothing drops under `doTileDrops` false, and air drops nothing. In a transaction the spawns wait for commit.
    #[test]
    #[ignore = "daily"]
    fn drop_resources_drops_without_breaking() {
        use crate::game::load_resources;
        use crate::settings::Settings;
        use mod_api::{Authority, BlockUpdateFlags, HostTransaction};
        use std::{sync::Arc, time::Instant};
        use sunrise_gameplay_common::game::{Options, Progress};
        use sunrise_items::items::ItemStack;
        use sunrise_level::{chunk::Chunk, containers::BlockEntity};

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        options.dedicated = Some(Vec::new());
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.native_lifecycle
            .begin_open(Some(Authority::IntegratedServer));

        let level = game.level("minecraft:overworld").unwrap();
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let chest = game
            .pack
            .assets
            .registry
            .default_state("minecraft:chest")
            .unwrap();
        let air = game
            .pack
            .assets
            .registry
            .default_state("minecraft:air")
            .unwrap();
        let flags = BlockUpdateFlags::NEIGHBORS;
        let count = |game: &sunrise_gameplay::game::ServerGame<'_>, item: &str| {
            game.dropped
                .iter()
                .filter(|dropped| dropped.stack.item.as_ref() == item)
                .map(|dropped| dropped.stack.count)
                .sum::<u32>()
        };
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], stone, flags),
            Ok(true)
        );
        let collected = game
            .level_drop_resources(level, [0, 64, 0], None, None, true)
            .unwrap();
        assert!(
            collected
                .iter()
                .any(|stack| stack.item.as_ref() == "minecraft:cobblestone")
        );
        assert!(game.dropped.is_empty(), "collected, not spawned");
        assert_eq!(game.world.block(0, 64, 0), stone, "the block stays");
        assert_eq!(
            game.level_drop_resources(level, [0, 64, 0], None, None, false),
            Some(Vec::new())
        );
        assert_eq!(count(&game, "minecraft:cobblestone"), 1);
        assert_eq!(game.world.block(0, 64, 0), stone);
        game.dropped.clear();

        assert_eq!(
            game.level_set_block(level, [0, 64, 0], chest, flags),
            Ok(true)
        );
        let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(0, 64, 0)) else {
            panic!("chest block entity missing")
        };
        items[0] = Some(ItemStack {
            item: "minecraft:diamond".into(),
            count: 3,
            damage: 0,
            extra: None,
        });
        assert!(
            game.level_drop_resources(level, [0, 64, 0], None, None, false)
                .is_some()
        );
        // The chest's own loot only: its contents spill when the block is removed
        // (`Containers.dropContents`), which this doesn't do.
        assert_eq!(count(&game, "minecraft:chest"), 1);
        assert_eq!(count(&game, "minecraft:diamond"), 0);
        assert_eq!(game.world.block(0, 64, 0), chest);
        let Some(BlockEntity::Chest(items)) = game.world.block_entities.get(&(0, 64, 0)) else {
            panic!("chest block entity missing")
        };
        assert_eq!(items[0].as_ref().map(|stack| stack.count), Some(3));
        game.dropped.clear();

        // Inside a transaction: rolled now, spawned at commit.
        let host: &mut dyn mod_api::ServerHost = &mut game;
        host.transaction(|host, _| {
            assert!(
                level
                    .drop_resources(host, [0, 64, 0], None, None, false)
                    .unwrap()
                    .is_some()
            );
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(count(&game, "minecraft:chest"), 1);
        assert_eq!(game.world.block(0, 64, 0), chest);
        game.dropped.clear();

        assert!(game.ui.rules.set(
            "block_drops",
            sunrise_game_data::gamerules::Value::Bool(false)
        ));
        assert_eq!(
            game.level_drop_resources(level, [0, 64, 0], None, None, true),
            Some(Vec::new())
        );
        assert!(game.dropped.is_empty());

        assert_eq!(
            game.level_set_block(level, [0, 64, 0], air, flags),
            Ok(true)
        );
        assert_eq!(
            game.level_drop_resources(level, [0, 64, 0], None, None, true),
            None
        );
    }
    #[test]
    #[ignore = "daily"]
    fn level_block_writes_validate_and_change_world() {
        use crate::game::load_resources;
        use crate::settings::Settings;
        use mod_api::{Authority, BlockUpdateFlags, HostTransaction};
        use std::{sync::Arc, time::Instant};
        use sunrise_gameplay_common::game::{Options, Progress};
        use sunrise_items::items::ItemStack;
        use sunrise_level::chunk::Chunk;

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        options.dedicated = Some(Vec::new());
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.native_lifecycle
            .begin_open(Some(Authority::IntegratedServer));

        let level = game.level("minecraft:overworld").unwrap();
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let chest = game
            .pack
            .assets
            .registry
            .default_state("minecraft:chest")
            .unwrap();
        let air = game
            .pack
            .assets
            .registry
            .default_state("minecraft:air")
            .unwrap();
        // Exercise the generated typed boundary, not only the native engine-input helpers.
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        let api_stone: mod_api::BlockState = stone.try_into_api().unwrap();
        let invalid: mod_api::BlockState =
            mod_api::__private::block_state_from_ordinal(api_stone.block(), u16::MAX);
        assert_eq!(
            level.set_block(&mut game, [0, 64, 0], invalid, BlockUpdateFlags::NONE),
            Err(mod_api::LevelWriteError::InvalidState(
                mod_api::BlockStateError::UnknownState
            ))
        );
        assert_eq!(game.world.block(0, 64, 0), air);
        game.world.block_log = Some(Vec::new());
        assert_eq!(
            level.update_neighbors(
                &mut game,
                [0, 64, 0],
                invalid,
                mod_api::NeighbourUpdate::Neighbours
            ),
            Err(mod_api::LevelWriteError::InvalidState(
                mod_api::BlockStateError::UnknownState
            ))
        );
        assert_eq!(
            level.neighbor_changed(&mut game, [0, 64, 0], invalid),
            Err(mod_api::LevelWriteError::InvalidState(
                mod_api::BlockStateError::UnknownState
            ))
        );
        assert_eq!(game.world.block(0, 64, 0), air);
        assert!(game.world.block_log.as_ref().unwrap().is_empty());
        // Flag-aware writes log only client updates; ordinary World::set still logs by default.
        assert_eq!(
            game.level_set_block(level, [2, 64, 0], stone, BlockUpdateFlags::NONE),
            Ok(true)
        );
        assert!(game.world.block_log.as_ref().unwrap().is_empty());
        assert_eq!(
            game.level_set_block(level, [3, 64, 0], stone, BlockUpdateFlags::CLIENTS),
            Ok(true)
        );
        assert_eq!(game.world.block_log.as_ref().unwrap(), &vec![(3, 64, 0)]);
        assert_eq!(
            game.level_set_block(
                level,
                [4, 64, 0],
                stone,
                BlockUpdateFlags::CLIENTS | BlockUpdateFlags::INVISIBLE
            ),
            Ok(true)
        );
        assert_eq!(
            game.world.block_log.as_ref().unwrap(),
            &vec![(3, 64, 0), (4, 64, 0)]
        );
        // `sendBlockUpdated` queues the block again without writing; out of height it queues nothing.
        game.world.modified.clear();
        let dirty_before = game.world.modified.clone();
        let block_before = game.world.block(5, 64, 0);
        assert!(game.level_send_block_updated(level, [5, 64, 0]));
        assert!(game.level_send_block_updated(level, [5, 64, 0]));
        assert_eq!(game.world.modified.clone(), dirty_before);
        assert_eq!(game.world.block(5, 64, 0), block_before);
        let host: &mut dyn mod_api::ServerHost = &mut game;
        let aborted: Result<(), mod_api::TransactionError<()>> = host.transaction(|host, _| {
            assert!(level.send_block_updated(host, [6, 64, 0]));
            Err(())
        });
        assert!(aborted.is_err());
        assert!(game.world.block_log.as_ref().unwrap().contains(&(6, 64, 0)));
        assert!(!game.level_send_block_updated(level, [5, 10_000, 0]));
        assert_eq!(
            game.world.block_log.as_ref().unwrap(),
            &vec![(3, 64, 0), (4, 64, 0), (5, 64, 0), (6, 64, 0)]
        );
        // A bare hand mines at the base speed; an unloaded position has none.
        assert_eq!(
            game.level_block_break_speed(level, [3, 64, 0], None),
            Some(1.0)
        );
        assert_eq!(
            game.level_block_break_speed(level, [3, 10_000, 0], None),
            None
        );
        let mut pickaxe = ItemStack::new("minecraft:iron_pickaxe", 1);
        assert_eq!(
            game.level_block_break_speed(level, [3, 64, 0], Some(&pickaxe)),
            Some(6.0)
        );
        pickaxe.enchant("minecraft:efficiency", 5);
        assert_eq!(
            game.level_block_break_speed(level, [3, 64, 0], Some(&pickaxe)),
            Some(6.0)
        );
        let mut custom_tool = ItemStack::new("minecraft:stick", 1);
        let components = sunrise_items::item_data::command_components(
            r#"[tool={rules:[{blocks:"stone",speed:7.0f}],default_mining_speed:2.0f}]"#,
        )
        .unwrap();
        (custom_tool.damage, custom_tool.extra) =
            sunrise_items::item_data::decode_components(Some(&components));
        assert_eq!(
            game.level_block_break_speed(level, [3, 64, 0], Some(&custom_tool)),
            Some(7.0)
        );
        assert_eq!(
            game.level_block_break_speed(level, [5, 64, 0], Some(&custom_tool)),
            Some(2.0)
        );
        // Server-side game: the client-only crack is refused at compile time.
        game.world.block_log = None;
        let waterlogged = game
            .pack
            .assets
            .registry
            .resolve(&sunrise_world::chunk::BlockState {
                name: "minecraft:oak_slab".into(),
                properties: [
                    ("type".into(), "bottom".into()),
                    ("waterlogged".into(), "true".into()),
                ]
                .into_iter()
                .collect(),
            })
            .unwrap();
        let water = game
            .pack
            .assets
            .registry
            .default_state("minecraft:water")
            .unwrap();
        for moved in [false, true] {
            game.world.set(5, 64, 0, waterlogged).unwrap();
            assert!(game.level_remove_block(level, [5, 64, 0], moved));
            assert_eq!(game.world.block(5, 64, 0), water);
            game.world.set(5, 64, 0, waterlogged).unwrap();
            let removed = game.with_level(|_, level, ticks| {
                let mut host =
                    sunrise_gameplay_common::game::mod_api_host::LevelTicks::new(level, ticks);
                mod_api::LevelWriteHost::remove_block(&mut host, [5, 64, 0], moved)
            });
            assert!(removed);
            assert_eq!(game.world.block(5, 64, 0), water);
        }
        let flags = BlockUpdateFlags::NEIGHBORS;
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], stone, flags),
            Ok(true)
        );
        assert_eq!(game.world.block(0, 64, 0), stone);
        assert_eq!(
            game.level_set_block(level, [16, 64, 0], stone, flags),
            Ok(false)
        );
        assert_eq!(
            game.level_set_block(level, [0, 10000, 0], stone, flags),
            Ok(false)
        );
        let stale = mod_api::LevelRef::new(
            mod_api::WorldEpoch::new(level.epoch().get() + 1),
            level.dimension(),
        );
        assert_eq!(
            game.level_set_block(stale, [0, 64, 0], stone, flags),
            Err(mod_api::LevelWriteError::Stale)
        );
        assert!(
            game.level_destroy_block(stale, [0, 64, 0], true, None, None, false)
                .is_none()
        );
        let fill_chest = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
            let Some(sunrise_level::containers::BlockEntity::Chest(items)) =
                game.world.block_entities.get_mut(&(0, 64, 0))
            else {
                panic!("chest block entity missing")
            };
            items[0] = Some(sunrise_items::items::ItemStack {
                item: "minecraft:diamond".into(),
                count: 3,
                damage: 0,
                extra: None,
            });
        };
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], chest, flags),
            Ok(true)
        );
        fill_chest(&mut game);
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], air, flags),
            Ok(true)
        );
        assert_eq!(
            game.dropped
                .iter()
                .filter(|item| item.stack.item.as_ref() == "minecraft:diamond")
                .map(|item| item.stack.count)
                .sum::<u32>(),
            3
        );
        game.dropped.clear();
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], chest, flags),
            Ok(true)
        );
        fill_chest(&mut game);
        assert_eq!(
            game.level_set_block(
                level,
                [0, 64, 0],
                air,
                BlockUpdateFlags::SKIP_BLOCK_ENTITY_SIDE_EFFECTS
            ),
            Ok(true)
        );
        assert!(!game.world.block_entities.contains_key(&(0, 64, 0)));
        assert!(game.dropped.is_empty());
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], stone, flags),
            Ok(true)
        );
        assert!(
            game.level_destroy_block(level, [0, 64, 0], false, None, None, false)
                .is_some()
        );
        assert!(game.dropped.is_empty());
        assert_eq!(game.world.block(0, 64, 0), air);
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], stone, flags),
            Ok(true)
        );
        assert!(
            game.level_destroy_block(level, [0, 64, 0], true, None, None, false)
                .is_some()
        );
        assert!(
            game.dropped
                .iter()
                .any(|item| item.stack.item.as_ref() == "minecraft:cobblestone")
        );
        assert!(
            game.level_destroy_block(level, [0, 64, 0], true, None, None, false)
                .is_none()
        );
        game.dropped.clear();
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], chest, flags),
            Ok(true)
        );
        fill_chest(&mut game);
        let host: &mut dyn mod_api::ServerHost = &mut game;
        let rolled_back: Result<(), mod_api::TransactionError<()>> = host.transaction(|host, _| {
            assert!(
                level
                    .destroy_block(host, [0, 64, 0], true, None, None, false)
                    .unwrap()
                    .is_some()
            );
            assert_eq!(
                level.block_state(host, [0, 64, 0]),
                Some(air.try_into_api().unwrap())
            );
            Err(())
        });
        assert_eq!(rolled_back, Err(mod_api::TransactionError::Aborted(())));
        assert_eq!(game.world.block(0, 64, 0), chest);
        assert!(game.dropped.is_empty());
        let Some(sunrise_level::containers::BlockEntity::Chest(restored)) =
            game.world.block_entities.get(&(0, 64, 0))
        else {
            panic!("transaction did not restore the chest");
        };
        assert_eq!(restored[0].as_ref().unwrap().count, 3);
        let host: &mut dyn mod_api::ServerHost = &mut game;
        host.transaction(|host, _| {
            assert!(
                level
                    .destroy_block(host, [0, 64, 0], true, None, None, false)
                    .unwrap()
                    .is_some()
            );
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(game.world.block(0, 64, 0), air);
        assert_eq!(
            game.dropped
                .iter()
                .filter(|item| item.stack.item.as_ref() == "minecraft:chest")
                .map(|item| item.stack.count)
                .sum::<u32>(),
            1
        );
        assert_eq!(
            game.dropped
                .iter()
                .filter(|item| item.stack.item.as_ref() == "minecraft:diamond")
                .map(|item| item.stack.count)
                .sum::<u32>(),
            3
        );
        game.dropped.clear();
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], stone, flags),
            Ok(true)
        );
        let host: &mut dyn mod_api::ServerHost = &mut game;
        let collected = host
            .transaction(|host, _| {
                Ok::<_, ()>(
                    level
                        .destroy_block(host, [0, 64, 0], true, None, None, true)
                        .unwrap()
                        .unwrap(),
                )
            })
            .unwrap();
        assert_eq!(collected.len(), 1);
        assert!(game.dropped.is_empty());
        assert_eq!(game.world.block(0, 64, 0), air);
        let copper = game
            .pack
            .assets
            .registry
            .default_state("minecraft:copper_chest")
            .unwrap();
        let exposed = game
            .pack
            .assets
            .registry
            .default_state("minecraft:exposed_copper_chest")
            .unwrap();
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], copper, flags),
            Ok(true)
        );
        fill_chest(&mut game);
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], exposed, flags),
            Ok(true)
        );
        let Some(sunrise_level::containers::BlockEntity::Chest(items)) =
            game.world.block_entities.get(&(0, 64, 0))
        else {
            panic!("copper chest entity missing")
        };
        assert_eq!(items[0].as_ref().unwrap().count, 3);
        assert!(game.dropped.is_empty());
        let sensor = game
            .pack
            .assets
            .registry
            .default_state("minecraft:sculk_sensor")
            .unwrap();
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], sensor, flags),
            Ok(true)
        );
        assert!(matches!(
            game.world.block_entities.get(&(0, 64, 0)),
            Some(sunrise_level::containers::BlockEntity::Sculk(_))
        ));
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], chest, flags),
            Ok(true)
        );
        assert!(matches!(
            game.world.block_entities.get(&(0, 64, 0)),
            Some(sunrise_level::containers::BlockEntity::Chest(_))
        ));
        assert!(
            game.enter_command_level(sunrise_gameplay_common::game::Dim::Nether)
                .is_some()
        );
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], stone, flags),
            Err(mod_api::LevelWriteError::Unavailable)
        );
        assert!(
            game.level_destroy_block(level, [0, 64, 0], true, None, None, false)
                .is_none()
        );
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_client_finds_the_entities_it_tracks_and_its_own_player_in_a_box() {
        if run_fresh_cases(
            "cut_layout::game::server_side::native_entity_refs::tests::a_client_finds_the_entities_it_tracks_and_its_own_player_in_a_box",
            "SUNRISE_CLIENT_ENTITY_BOX_QUERY_CHILD",
            &["client"],
        ) {
            return;
        }
        use mod_api::{Host, LevelRef, WorldEpoch};
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::connection::Pipe;
        with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "EntitiesInTest").unwrap();
            let remote = game.client.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            let mut local =
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            local.entity_id = 7;
            game.player = Some(local);
            game.spawn_entity(15, "minecraft:pig", [2.0, 0.0, 0.0], [0.0; 3]);
            game.pick_section_sync_id(15);
            game.spawn_entity(16, "minecraft:pig", [40.0, 0.0, 0.0], [0.0; 3]);
            game.pick_section_sync_id(16);
            game.bind_local_player_residency();
            let level = game.level("minecraft:overworld").unwrap();
            let ids = |host: &dyn Host, level: LevelRef, min, max| {
                collect_entities_in(host, level, min, max)
                    .map(|found| found.into_iter().map(|e| e.id()).collect::<Vec<_>>())
            };
            // The tracked pig, then the window's own player; the other pig is far away.
            assert_eq!(
                ids(game, level, [-1.0, -1.0, -1.0], [3.0, 2.0, 1.0]),
                Ok(vec![15, 7])
            );
            assert_eq!(
                ids(game, level, [39.0, -1.0, -1.0], [41.0, 2.0, 1.0]),
                Ok(vec![16])
            );
            // The pig's box ends at x 2.45.
            assert_eq!(
                ids(game, level, [2.5, 0.0, -1.0], [4.0, 1.0, 1.0]),
                Ok(vec![])
            );
            game.set_local_mode(sunrise_command::args::GameMode::Spectator);
            assert_eq!(
                ids(game, level, [-1.0, -1.0, -1.0], [3.0, 2.0, 1.0]),
                Ok(vec![15])
            );
            let nether = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            assert_eq!(
                ids(game, nether, [-1.0; 3], [3.0; 3]),
                Err(LevelReadError::Unavailable)
            );
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert_eq!(
                ids(game, stale, [-1.0; 3], [3.0; 3]),
                Err(LevelReadError::Stale)
            );
            game.client.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(
                ids(game, level, [-1.0; 3], [3.0; 3]),
                Err(LevelReadError::Unavailable)
            );
        });
    }

    /// A client's `visit_entities_in` visits another player standing in the box (added through the
    /// real `AddEntity` packet), after the entities the server tracks and before the window's
    /// own player; a player outside the box, and one the server removed, are not listed.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_client_lists_another_player_in_the_box_after_the_tracked_entities() {
        if run_fresh_cases(
            "cut_layout::game::server_side::native_entity_refs::tests::a_client_lists_another_player_in_the_box_after_the_tracked_entities",
            "SUNRISE_CLIENT_ENTITY_BOX_QUERY_CHILD",
            &["client"],
        ) {
            return;
        }
        use mod_api::{Host, LevelRef};
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::{
            connection::Pipe,
            packets::play::{
                AddEntity, Clientbound as C, PlayerInfo, PlayerInfoUpdate, RemoveEntities,
            },
        };
        with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "EntitiesInTest").unwrap();
            let remote = game.client.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            let player = remote
                .builtin
                .get("minecraft:entity_type")
                .unwrap()
                .id("minecraft:player")
                .unwrap();
            let mut local =
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            local.entity_id = 7;
            game.player = Some(local);
            game.spawn_entity(15, "minecraft:pig", [2.0, 0.0, 0.0], [0.0; 3]);
            game.pick_section_sync_id(15);
            let add_player =
                |game: &mut sunrise_client_gameplay::game::ClientGame<'_>, id, uuid, x| {
                    game.handle_packet(C::PlayerInfoUpdate(PlayerInfoUpdate {
                        actions: 1,
                        entries: vec![PlayerInfo {
                            uuid,
                            name: Some(format!("Player{id}")),
                            ..Default::default()
                        }],
                    }))
                    .unwrap();
                    game.handle_packet(C::AddEntity(AddEntity {
                        id,
                        uuid,
                        kind: player,
                        position: [x, 0.0, 0.0],
                        velocity: [0.0; 3],
                        pitch: 0,
                        yaw: 0,
                        head_yaw: 0,
                        data: 0,
                    }))
                    .unwrap();
                };
            // 18 stands in the box, 19 far outside it, 20 in it until the server removes it.
            add_player(game, 18, 9, 1.0);
            add_player(game, 19, 10, 40.0);
            add_player(game, 20, 11, 0.5);
            game.bind_local_player_residency();
            assert!(
                [18, 19, 20]
                    .iter()
                    .all(|id| game.mobs.iter().any(|m| m.id == *id))
            );
            let level = game.level("minecraft:overworld").unwrap();
            let ids = |host: &dyn Host, level: LevelRef, min, max| {
                collect_entities_in(host, level, min, max)
                    .map(|found| found.into_iter().map(|e| e.id()).collect::<Vec<_>>())
            };
            // Section entry order: the pig, then the other players, then the window's own.
            assert_eq!(
                ids(game, level, [-1.0, -1.0, -1.0], [3.0, 2.0, 1.0]),
                Ok(vec![15, 18, 20, 7])
            );
            // A box around only the far player finds it, and the near box did not.
            assert_eq!(
                ids(game, level, [39.0, -1.0, -1.0], [41.0, 2.0, 1.0]),
                Ok(vec![19])
            );
            game.handle_packet(C::RemoveEntities(RemoveEntities { ids: vec![20] }))
                .unwrap();
            assert_eq!(
                ids(game, level, [-1.0, -1.0, -1.0], [3.0, 2.0, 1.0]),
                Ok(vec![15, 18, 7])
            );
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_client_refuses_a_box_with_a_bound_that_is_not_finite_and_swaps_an_inverted_one() {
        if run_fresh_cases(
            "cut_layout::game::server_side::native_entity_refs::tests::a_client_refuses_a_box_with_a_bound_that_is_not_finite_and_swaps_an_inverted_one",
            "SUNRISE_CLIENT_ENTITY_BOX_QUERY_CHILD",
            &["client"],
        ) {
            return;
        }
        use mod_api::{Host, LevelRef};
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::connection::Pipe;
        with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "EntitiesInTest").unwrap();
            let remote = game.client.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            let mut local =
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            local.entity_id = 7;
            game.player = Some(local);
            game.spawn_entity(15, "minecraft:pig", [2.0, 0.0, 0.0], [0.0; 3]);
            game.pick_section_sync_id(15);
            game.bind_local_player_residency();
            let level = game.level("minecraft:overworld").unwrap();
            let ids = |host: &dyn Host, level: LevelRef, min, max| {
                collect_entities_in(host, level, min, max)
                    .map(|found| found.into_iter().map(|e| e.id()).collect::<Vec<_>>())
            };
            // Mod code chooses the bounds: one that is not finite is refused, never panics,
            // and nothing is walked by the box's size.
            for (min, max) in [
                ([f64::NAN, -1.0, -1.0], [3.0, 2.0, 1.0]),
                ([-1.0, -1.0, -1.0], [3.0, f64::NAN, 1.0]),
                ([f64::NEG_INFINITY, -1.0, -1.0], [3.0, 2.0, 1.0]),
                ([-1.0, -1.0, -1.0], [3.0, 2.0, f64::INFINITY]),
                ([f64::NEG_INFINITY; 3], [f64::INFINITY; 3]),
            ] {
                assert_eq!(
                    ids(game, level, min, max),
                    Err(LevelReadError::Unavailable),
                    "{min:?} {max:?}"
                );
            }
            // A minimum above its maximum on one axis is the same box swapped (vanilla's
            // AABB): the tracked pig, then the window's own player.
            assert_eq!(
                ids(game, level, [3.0, -1.0, -1.0], [-1.0, 2.0, 1.0]),
                Ok(vec![15, 7])
            );
            assert_eq!(
                ids(game, level, [3.0, 2.0, 1.0], [-1.0, -1.0, -1.0]),
                Ok(vec![15, 7])
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn server_box_queries_use_live_display_positions_and_section_order() {
        use mod_api::Host;
        use std::sync::Arc;
        use sunrise_core::nbt::Compound;
        use sunrise_level::chunk::Chunk;

        const LO: [f64; 3] = [13.0, 65.0, 2.0];
        const HI: [f64; 3] = [19.0, 66.0, 4.0];
        const ADMISSIONS: [(i32, &str, [f64; 3]); 4] = [
            (31, "minecraft:item_display", [17.0, 65.5, 3.0]),
            (92, "minecraft:block_display", [15.5, 65.5, 3.0]),
            (55, "minecraft:pig", [14.5, 65.5, 3.0]),
            (11, "minecraft:text_display", [13.5, 65.5, 3.0]),
        ];
        let ids = |host: &dyn Host, level, min, max| {
            collect_entities_in(host, level, min, max)
                .unwrap()
                .into_iter()
                .map(|e| e.id())
                .collect::<Vec<_>>()
        };
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            for x in 0..=2 {
                game.world
                    .chunks
                    .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
            }
            let level = game.level("minecraft:overworld").unwrap();
            let [item, block, pig, text] = ADMISSIONS.map(|(_, kind, at)| {
                game.level_summon(level, kind, at, Compound::new())
                    .unwrap()
                    .unwrap()
                    .id()
            });
            assert_eq!(ids(game, level, LO, HI), [block, pig, text, item]);
            for (id, (_, _, at)) in [item, block, pig, text].into_iter().zip(ADMISSIONS) {
                if id != pig {
                    let display = game.entity(id).unwrap();
                    assert_eq!(display.position(game), Some(at));
                    assert_eq!(display.bounding_box(game), Some([at; 2]));
                }
            }
            let at = [16.5, 65.5, 3.0];
            assert_eq!(
                game.place_teleport_filed(
                    sunrise_gameplay_common::game::commands_common::Target::Other(block),
                    at,
                    None,
                    [false; 3],
                    false
                ),
                Some(ADMISSIONS[1].2)
            );
            assert_eq!(ids(game, level, LO, HI), [pig, text, item, block]);
            assert_eq!(
                game.entity(block).unwrap().bounding_box(game),
                Some([at; 2])
            );
            let far_lo = [39.5, 65.0, 2.0];
            let far_hi = [41.5, 66.0, 4.0];
            assert!(ids(game, level, far_lo, far_hi).is_empty());
            let target = sunrise_gameplay_common::game::commands_common::Target::Other(block);
            let mut tag = game.command_target_nbt(target).unwrap();
            let pos = sunrise_core::nbt::Tag::List(
                [40.5, 65.5, 3.0]
                    .map(|v: f64| sunrise_core::nbt::Tag::Double(v.to_bits()))
                    .to_vec(),
            );
            if let sunrise_core::nbt::Tag::Compound(c) = &mut tag {
                c.insert("Pos".into(), pos);
            }
            game.apply_entity_nbt(target, tag).unwrap();
            assert_eq!(ids(game, level, LO, HI), [pig, text, item]);
            assert_eq!(ids(game, level, far_lo, far_hi), [block]);
        });
    }
}

#[cfg(test)]
#[path = "native_entity_refs/restitution_replay_tests.rs"]
pub(crate) mod restitution_replay_tests;

#[path = "native_entity_refs/dragon_query.rs"]
pub(crate) mod dragon_query;
