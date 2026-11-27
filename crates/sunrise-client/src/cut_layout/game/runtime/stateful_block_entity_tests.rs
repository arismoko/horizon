use sunrise_gameplay_common::game::{
    GameGameExt1, native_hooks::SharedGameExt1 as _, transactions::TransactionsGameExt as _,
};
#[test]
#[ignore = "daily"]
fn stateful_creation_uses_actual_state_and_preserves_existing_entity() {
    use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
    if !run_fresh_cases(
        "cut_layout::game::stateful_block_entity_tests::stateful_creation_uses_actual_state_and_preserves_existing_entity",
        "SUNRISE_STATEFUL_CREATION_GAME_CHILD",
        &["valid"],
    ) {
        stateful_creation_fixture(false);
    }
}

#[test]
#[ignore = "daily"]
fn strict_edit_retires_the_staged_placement_entity_after_its_notification() {
    use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
    if !run_fresh_cases(
        "cut_layout::game::stateful_block_entity_tests::strict_edit_retires_the_staged_placement_entity_after_its_notification",
        "SUNRISE_STATEFUL_STRICT_REPLAY_CHILD",
        &["strict"],
    ) {
        stateful_creation_fixture(true);
    }
}

fn stateful_creation_fixture(strict: bool) {
    use mod_api::{self, *};
    use std::sync::{Arc, Mutex, OnceLock};
    use sunrise_gameplay::game::native_entity_refs::tests::record_key;
    use sunrise_gameplay::game::native_guest_turn_tests::with_guests_at;
    use sunrise_level::{chunk::Chunk, containers::BlockEntity};

    const MIXED: [i32; 3] = [2, 64, 0];
    const BLOCK: &str = "state_factory_test:banner";
    const KIND: &str = "state_factory_test:machine";
    #[derive(Default)]
    struct Observations {
        populated: OnceLock<BlockState>,
        calls: Mutex<Vec<BlockState>>,
        kind: OnceLock<BlockEntityTypeKey>,
        staged: OnceLock<BlockEntityRef>,
        published: OnceLock<BlockEntityRef>,
    }
    #[derive(Clone)]
    struct Factory(Arc<Observations>);
    #[mod_api::__subscriptions]
    impl Hooks for Factory {
        fn entity_place(&self, host: &mut dyn ServerHost, event: &mut EntityPlaceEvent) -> Flow {
            let level = host.level("minecraft:overworld").unwrap();
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            let entity = host
                .block_entity(level, pos)
                .expect("entity at the staged property");
            assert_eq!(entity.kind(host).as_ref(), self.0.kind.get());
            assert_eq!(
                entity.data(host).unwrap().get(&"progress".into()),
                Some(&Tag::Int(37))
            );
            self.0.staged.set(entity).unwrap();
            Flow::Continue
        }

        fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let kind = registrar
                .block_entity_type("machine")
                .blocks(&[BLOCK])
                .build();
            registrar.block_entity_part(&kind, self.clone()).unwrap();
            record_key(&self.0.kind, kind);
        }
        fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut BlocksEvent<'_>) {
            event
                .register(mod_api::Block::new(BLOCK, "minecraft:white_banner"))
                .unwrap();
        }
    }
    #[mod_api::sunrise_part]
    impl BlockPart for Factory {
        fn on_block_state_change(&self, level: LevelSide<'_>, event: &mut OnBlockStateChangeEvent) {
            let LevelSide::Server(level) = level else {
                return;
            };
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            if pos == MIXED
                && Some(event.new) == self.0.populated.get().copied()
                && let Some(entity) = level.block_entity(pos)
            {
                self.0.published.set(entity).unwrap();
            }
        }
    }
    #[mod_api::sunrise_part]
    impl BlockEntityPart for Factory {
        fn stateful_block_entity(&self, _: &dyn Host, event: &mut StatefulBlockEntityEvent<'_>) {
            if event.block == BLOCK {
                event.stateful = true;
            }
        }
        fn block_entity(
            &self,
            _: LevelSide<'_>,
            event: &mut BlockEntityEvent<'_>,
        ) -> Option<Compound> {
            if event.block != BLOCK {
                return None;
            }
            self.0.calls.lock().unwrap().push(event.state);
            (event.state == *self.0.populated.get().unwrap())
                .then(|| Compound::from([("progress".into(), Tag::Int(37))]))
        }
    }

    let observed = Arc::new(Observations::default());
    let mods: Mods = vec![ModEntry::new(
        "state_factory_test",
        Box::new(Factory(observed.clone())),
    )]
    .into();
    mod_platform::__private::install(mods.clone());
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let root =
        sunrise_gameplay_common::game::mods::content_root(&base, &Default::default()).unwrap();
    with_guests_at(root.prepared().to_path_buf(), |game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let registry = game.pack.assets.registry;
        let default = registry.default_state(BLOCK).unwrap();
        let populated = sunrise_gameplay_common::game::native_hooks::with_property(
            registry, default, "rotation", "1",
        )
        .unwrap();
        let empty = sunrise_gameplay_common::game::native_hooks::with_property(
            registry, default, "rotation", "2",
        )
        .unwrap();
        assert_ne!(
            populated, default,
            "the factory must see a non-default state"
        );
        assert_ne!(empty, populated);
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        let populated_api = populated.try_into_api().unwrap();
        let empty_api = empty.try_into_api().unwrap();
        observed.populated.set(populated_api).unwrap();
        let level = game.level(game.dim.id()).unwrap();
        let stone = registry.default_state("minecraft:stone").unwrap();
        for x in [1, 2, 3] {
            assert_eq!(
                game.level_set_block(level, [x, 63, 0], stone, BlockUpdateFlags::NONE),
                Ok(true)
            );
        }
        assert!(observed.calls.lock().unwrap().is_empty());

        if strict {
            let player = game.player(13).unwrap();
            let chest = registry.default_state("minecraft:chest").unwrap();
            let [x, y, z] = MIXED;
            let edit = BlockEdit {
                palette: vec![chest.try_into_api().unwrap()],
                filters: Vec::new(),
                runs: vec![BlockRun {
                    x,
                    y,
                    z,
                    height: 1,
                    surface: None,
                    block: 0,
                    only: None,
                }],
            };
            game.world.block_log = Some(Vec::new());
            let strict_row = game
                .storage_transaction(|game, _| {
                    player
                        .place_block(game, level, MIXED, populated_api, BlockFace::Up, false)
                        .unwrap();
                    assert_eq!(
                        game.block_entity(level, MIXED).as_ref(),
                        observed.staged.get()
                    );
                    let host: &mut dyn ServerHost = game;
                    assert_eq!(level.edit_blocks(host, &edit), Ok(1));
                    let strict_row = host.block_entity(level, MIXED).unwrap();
                    assert_ne!(Some(&strict_row), observed.staged.get());
                    assert!(observed.published.get().is_none());
                    Ok::<_, ()>(strict_row)
                })
                .unwrap();
            assert!(observed.staged.get().is_some());
            assert_eq!(
                observed.published.get(),
                observed.staged.get(),
                "ordinary publication sees the original staged generation before the strict edit"
            );
            assert_eq!(*observed.calls.lock().unwrap(), [populated_api]);
            assert_eq!(game.block_entity(level, MIXED), Some(strict_row));
            assert_eq!(game.world.block(x, y, z), chest);
            assert!(game.world.bulk_log.contains(&(x >> 4, z >> 4)));
            assert!(game.dropped.is_empty());
            return;
        }

        // The whole-block Host write must create inline, not call the factory
        // again after LevelChunk.setBlockState has already installed its entity.
        assert_eq!(
            game.level_set_block(level, [1, 64, 0], populated, BlockUpdateFlags::NONE),
            Ok(true)
        );
        assert_eq!(*observed.calls.lock().unwrap(), [populated_api]);
        let handle = game
            .block_entity(level, [1, 64, 0])
            .expect("factory entity");
        assert_eq!(handle.kind(game).as_ref(), observed.kind.get());
        let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get_mut(&(1, 64, 0)) else {
            panic!("registered type must override its vanilla banner template");
        };
        assert_eq!(entity.id(), KIND);
        assert_eq!(entity.nbt.get(&"progress".into()), Some(&Tag::Int(37)));
        entity.nbt.insert("progress".into(), Tag::Int(91));

        // This state would answer None on a fresh placement, but a property
        // change must preserve both the live payload and the handle generation.
        assert_eq!(
            game.level_set_block(level, [1, 64, 0], empty, BlockUpdateFlags::NONE),
            Ok(true)
        );
        assert_eq!(*observed.calls.lock().unwrap(), [populated_api]);
        assert_eq!(handle.kind(game).as_ref(), observed.kind.get());
        let saved = game
            .block_entity_data(handle)
            .expect("original handle remains live");
        assert_eq!(saved.get(&"progress".into()), Some(&Tag::Int(91)));

        // A fresh unanswered stateful placement is absent, not an empty declared
        // mod entity or the template's built-in banner entity.
        assert_eq!(
            game.level_set_block(level, [2, 64, 0], empty, BlockUpdateFlags::NONE),
            Ok(true)
        );
        assert_eq!(*observed.calls.lock().unwrap(), [populated_api, empty_api]);
        assert!(game.world.block_entities.get(&(2, 64, 0)).is_none());
        assert!(game.block_entity(level, [2, 64, 0]).is_none());

        // A chosen state of the same block can create its first entity. The
        // veto sees it, and publication preserves that instance without asking again.
        let player = game.player(13).unwrap();
        assert_eq!(
            player.place_block(game, level, [2, 64, 0], populated_api, BlockFace::Up, false),
            Ok(())
        );
        assert_eq!(
            game.block_entity(level, [2, 64, 0]).as_ref(),
            observed.staged.get()
        );
        assert_eq!(
            *observed.calls.lock().unwrap(),
            [populated_api, empty_api, populated_api]
        );

        // A hook's LevelWriteHost write takes the GameLevel path without the
        // outer Game write wrapper and still creates exactly one instance.
        let written = game.with_level(|_, level, ticks| {
            LevelWriteHost::set_block(
                &mut sunrise_gameplay_common::game::mod_api_host::LevelTicks::new(level, ticks),
                [3, 64, 0],
                populated_api,
                BlockUpdateFlags::NONE,
            )
        });
        assert!(written);
        assert_eq!(
            *observed.calls.lock().unwrap(),
            [populated_api, empty_api, populated_api, populated_api]
        );
        let second = game
            .block_entity(level, [3, 64, 0])
            .expect("level write entity");
        assert_eq!(second.kind(game).as_ref(), observed.kind.get());
        assert_eq!(
            game.block_entity_data(second)
                .unwrap()
                .get(&"progress".into()),
            Some(&Tag::Int(37))
        );
        // The placement and survival view reads the same live entity.
        game.with_level(|_, level, ticks| {
            let view = level.view(ticks);
            let data = mod_api::LevelHost::block_entity_data(&view, second)
                .expect("the view forwards a block entity's data");
            assert_eq!(data.get(&"progress".into()), Some(&Tag::Int(37)));
        });
    });
}

#[test]
#[ignore = "daily"]
fn admitted_minecraft_and_custom_blocks_share_stateful_tables_capabilities_and_menu() {
    use mod_api::{self, *};
    use std::sync::Arc;
    use sunrise_gameplay::game::native_entity_refs::tests::{run_fresh_cases, with_game_at};
    use sunrise_level::{chunk::Chunk, containers::BlockEntity};

    if run_fresh_cases(
        "cut_layout::game::stateful_block_entity_tests::admitted_minecraft_and_custom_blocks_share_stateful_tables_capabilities_and_menu",
        "SUNRISE_MINECRAFT_BLOCK_TABLES_CHILD",
        &["valid"],
    ) {
        return;
    }

    const MC_BLOCK: &str = "minecraft:p3_04_machine";
    const ALT_BLOCK: &str = "p3_04_tables:machine";

    #[derive(Clone)]
    struct MachineMod;
    #[mod_api::__subscriptions]
    impl Hooks for MachineMod {
        fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let kind = registrar
                .block_entity_type("machine")
                .blocks(&[MC_BLOCK, ALT_BLOCK])
                .build();
            registrar.block_entity_part(&kind, self.clone()).unwrap();
            for id in [MC_BLOCK, ALT_BLOCK] {
                let key = registrar.block_key(id);
                registrar.block_container(
                    &key,
                    ModContainer {
                        slots: 9,
                        faces: std::array::from_fn(|_| (0..9).collect()),
                        inputs: (0..9).collect(),
                        outputs: (0..9).collect(),
                        menu: Some("minecraft:generic_9x1".into()),
                        ..Default::default()
                    },
                );
                let mut tanks = ModFluidTanks::default();
                tanks.tanks = vec![4000];
                tanks.faces = std::array::from_fn(|_| vec![0]);
                tanks.inputs = vec![0];
                tanks.outputs = vec![0];
                registrar.block_fluids(&key, tanks);
                registrar.block_energy(
                    &key,
                    ModEnergyBuffer {
                        capacity: 1000,
                        max_insert: 200,
                        max_extract: 200,
                        insert_faces: [true; 6],
                        extract_faces: [true; 6],
                    },
                );
            }
        }
        fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut BlocksEvent<'_>) {
            for id in [MC_BLOCK, ALT_BLOCK] {
                event
                    .register(mod_api::Block::new(id, "minecraft:stone"))
                    .unwrap();
            }
        }
    }
    #[mod_api::sunrise_part]
    impl BlockPart for MachineMod {}
    #[mod_api::sunrise_part]
    impl BlockEntityPart for MachineMod {
        fn stateful_block_entity(&self, _: &dyn Host, event: &mut StatefulBlockEntityEvent<'_>) {
            if event.block == MC_BLOCK || event.block == ALT_BLOCK {
                event.stateful = true;
            }
        }
        fn block_entity(
            &self,
            _: LevelSide<'_>,
            event: &mut BlockEntityEvent<'_>,
        ) -> Option<Compound> {
            (event.block == MC_BLOCK || event.block == ALT_BLOCK)
                .then(|| Compound::from([("tier".into(), Tag::Int(2))]))
        }
    }

    let mods: Mods = vec![ModEntry::new("p3_04_tables", Box::new(MachineMod))].into();
    mod_platform::__private::install(mods.clone());
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let root =
        sunrise_gameplay_common::game::mods::content_root(&base, &Default::default()).unwrap();
    with_game_at(root.prepared().to_path_buf(), |game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.ensure_stateful_entity_table();
        let containers = game.native_containers();
        game.world.block_entities.set_mod_containers(containers);
        let fluids = game.native_fluid_tables();
        game.world.block_entities.set_mod_fluids(fluids);
        let energy = game.native_energy_tables();
        game.world.block_entities.set_mod_energy(energy);
        let level = game.level("minecraft:overworld").unwrap();
        let registry = game.pack.assets.registry;
        for (x, id) in [(1, MC_BLOCK), (2, ALT_BLOCK)] {
            let pos = [x, 64, 0];
            let state = registry.default_state(id).unwrap();
            assert_eq!(
                game.level_set_block(level, pos, state, BlockUpdateFlags::NONE),
                Ok(true),
                "{id}"
            );
            let entity = game.block_entity(level, pos).expect(id);
            assert_eq!(
                game.block_entity_data(entity).unwrap().get(&"tier".into()),
                Some(&Tag::Int(2)),
                "{id}"
            );
            assert!(
                level.capability(game, &ITEMS, pos, None).is_some(),
                "ITEMS missing for {id}"
            );
            assert!(
                level.capability(game, &FLUIDS, pos, None).is_some(),
                "FLUIDS missing for {id}"
            );
            assert!(
                level.capability(game, &ENERGY, pos, None).is_some(),
                "ENERGY missing for {id}"
            );
            assert_eq!(game.mod_menu_at((x, 64, 0)), Some("minecraft:generic_9x1"));
            assert!(
                game.open_mod_menu((x, 64, 0)),
                "open_mod_menu failed for {id}"
            );
            let Some(BlockEntity::Mod(opened)) = game.world.block_entities.get(&(x, 64, 0)) else {
                panic!("expected mod block entity for {id}");
            };
            assert_eq!(opened.items.len(), 9, "{id}");
        }
        // Built-in vanilla blocks preserve their own paths and never enter mod tables.
        let chest_pos = [3, 64, 0];
        let chest = registry.default_state("minecraft:chest").unwrap();
        assert_eq!(
            game.level_set_block(level, chest_pos, chest, BlockUpdateFlags::NONE),
            Ok(true)
        );
        assert!(game.world.block_entities.mod_container(chest.0).is_none());
        assert!(game.world.block_entities.mod_fluids(chest.0).is_none());
        assert!(game.world.block_entities.mod_energy(chest.0).is_none());
        assert_eq!(game.mod_menu_at((3, 64, 0)), None);
        assert!(!game.open_mod_menu((3, 64, 0)));
    });
}

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;

use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
