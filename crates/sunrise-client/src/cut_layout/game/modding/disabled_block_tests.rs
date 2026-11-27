//! Switching an owner off holds scheduled work and prevents template writes.
#![cfg(test)]

#[cfg(test)]
pub(crate) mod tests {
    use super::super::native_entity_refs::tests as fixture;
    use mod_api::{
        BlockPart, BlockToolModificationEvent, CanBeHydratedEvent, DispenseMethodEvent, Dispensed,
        Flow, Hooks, Host, LevelWriteHost, Registrar, RegistrationContext, ServerHost,
        TickBlockEvent, ToolAbility, ToolModifiedStateEvent,
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_core::{position::BlockPos, ticks::TickPriority};
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_gameplay_common::game::api_values::{TryIntoApi, TryIntoEngine};
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::{Dim, Hit};
    use sunrise_items::items::ItemStack;
    use sunrise_level::chunk::Chunk;
    use sunrise_world::{
        level::{BlockMapValue, BlockMaps, Landing, LevelAccess},
        registry::BlockStateId,
    };

    pub(crate) const POS: BlockPos = BlockPos::new(3, 64, 5);
    pub(crate) const CARPET: &str = "inert_test_extra:carpet";

    #[derive(Default)]
    pub(in crate::game) struct ToolProbe {
        pub(in crate::game) candidates: Vec<(ToolAbility, Option<BlockStateId>)>,
        pub(in crate::game) replacement: Option<mod_api::BlockState>,
    }

    #[derive(Default)]
    pub(crate) struct Probe {
        pub(in crate::game) owner_ticks: AtomicUsize,
        pub(in crate::game) earlier_ticks: AtomicUsize,
        pub(in crate::game) earlier_dispenses: AtomicUsize,
        pub(in crate::game) dispenser_changes: AtomicUsize,
        pub(in crate::game) tool: Mutex<ToolProbe>,
    }

    pub(crate) struct Earlier(pub(crate) Arc<Probe>);
    #[mod_api::sunrise_part]
    impl BlockPart for Earlier {
        fn tick_block(&self, _: &mut dyn LevelWriteHost, _: &mut TickBlockEvent) -> Flow {
            self.0.earlier_ticks.fetch_add(1, Ordering::Relaxed);
            Flow::Cancel
        }
        fn dispense_method(
            &self,
            _: &mut dyn ServerHost,
            event: &mut DispenseMethodEvent<'_>,
        ) -> Option<Dispensed> {
            self.0.earlier_dispenses.fetch_add(1, Ordering::Relaxed);
            let mut remainder = event.stack.clone();
            if remainder.count() == 65 {
                remainder
                    .set_components(
                        [("minecraft:max_stack_size".into(), mod_api::Tag::Int(-1))].into(),
                    )
                    .unwrap();
            } else {
                remainder.set_count(2).unwrap();
            }
            Some((Some(remainder), 0))
        }
    }
    pub(crate) struct HydratesWithWater;
    #[mod_api::sunrise_part]
    impl BlockPart for HydratesWithWater {
        fn can_be_hydrated(
            &self,
            _: &dyn mod_api::LevelHost,
            event: &mut CanBeHydratedEvent,
        ) -> Option<bool> {
            (mod_api::__private::fluid_id(event.fluid.fluid) == Some("minecraft:water"))
                .then_some(true)
        }
    }

    pub(crate) struct HydratesWithLava;
    #[mod_api::sunrise_part]
    impl BlockPart for HydratesWithLava {
        fn can_be_hydrated(
            &self,
            _: &dyn mod_api::LevelHost,
            event: &mut CanBeHydratedEvent,
        ) -> Option<bool> {
            (mod_api::__private::fluid_id(event.fluid.fluid) == Some("minecraft:lava"))
                .then_some(true)
        }
    }

    pub(crate) struct HydratesWithAnyFluid;
    #[mod_api::sunrise_part]
    impl BlockPart for HydratesWithAnyFluid {
        fn can_be_hydrated(
            &self,
            _: &dyn mod_api::LevelHost,
            _: &mut CanBeHydratedEvent,
        ) -> Option<bool> {
            Some(true)
        }
    }

    pub(crate) struct EnabledPrefix(pub(crate) Arc<Probe>);
    #[mod_api::__subscriptions]
    impl Hooks for EnabledPrefix {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let block = r.block_key("inert_test:machine");
            r.block_part(block, Earlier(self.0.clone()));
            let farmland = r.block_key("inert_test:farmland");
            r.block_part(farmland, HydratesWithWater);
        }
    }

    pub(crate) struct Machine(pub(crate) Arc<Probe>);
    #[mod_api::sunrise_part]
    impl BlockPart for Machine {
        fn tick_block(&self, _: &mut dyn LevelWriteHost, _: &mut TickBlockEvent) -> Flow {
            self.0.owner_ticks.fetch_add(1, Ordering::Relaxed);
            Flow::Cancel
        }
        fn dispense_method(
            &self,
            _: &mut dyn ServerHost,
            event: &mut DispenseMethodEvent<'_>,
        ) -> Option<Dispensed> {
            Some((Some(event.stack.clone()), 0))
        }
        fn tool_modified_state(&self, _: &dyn Host, _: &mut ToolModifiedStateEvent<'_>) -> Flow {
            Flow::Continue
        }
    }
    pub(crate) struct Survival;
    #[mod_api::sunrise_part]
    impl BlockPart for Survival {
        fn can_survive(
            &self,
            _: mod_api::LevelSide<'_>,
            _: &mut mod_api::CanSurviveEvent,
        ) -> Option<bool> {
            Some(true)
        }
    }

    pub(crate) struct Plant;
    #[mod_api::sunrise_part]
    impl BlockPart for Plant {
        fn may_place_on(
            &self,
            _: mod_api::LevelSide<'_>,
            _: &mut mod_api::MayPlaceOnEvent,
        ) -> Option<bool> {
            Some(true)
        }
    }

    pub(crate) struct Soil;
    #[mod_api::sunrise_part]
    impl BlockPart for Soil {
        fn can_sustain_plant(
            &self,
            _: mod_api::LevelSide<'_>,
            _: &mut mod_api::CanSustainPlantEvent,
        ) -> Option<bool> {
            Some(true)
        }
    }

    pub(crate) struct Owner(pub(crate) Arc<Probe>);
    #[mod_api::__subscriptions]
    impl Hooks for Owner {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let block = r.block_key("inert_test:machine");
            r.block_part(block, Machine(self.0.clone()));
            let carpet = r.block_key(CARPET);
            r.block_part(carpet, Survival);
            let sand = r.block_key("inert_test:sand");
            r.block_part(sand, Survival);
            let plant = r.block_key("inert_test:plant");
            r.block_part(plant, Plant);
            let soil = r.block_key("inert_test:soil");
            r.block_part(soil, Soil);
            let farmland = r.block_key("inert_test:farmland");
            r.block_part(farmland, HydratesWithLava);
        }
        fn blocks(&self, _: &RegistrationContext, e: &mut mod_api::BlocksEvent<'_>) {
            e.register(
                mod_api::Block::new("inert_test:machine", "minecraft:observer").with_item(false),
            )
            .unwrap();
            let mut carpet = mod_api::Block::new(CARPET, "minecraft:white_carpet");
            carpet.properties.push_reaction = Some(mod_api::PushReaction::PushPull);
            e.register(carpet).unwrap();
            e.register(mod_api::Block::new("inert_test:sand", "minecraft:sand"))
                .unwrap();
            let mut plant = mod_api::Block::new("inert_test:plant", "minecraft:dandelion");
            plant.properties.push_reaction = Some(mod_api::PushReaction::PushPull);
            e.register(plant).unwrap();
            e.register(mod_api::Block::new("inert_test:soil", "minecraft:stone"))
                .unwrap();
            for id in ["inert_test:farmland", "inert_test:unsubscribed_farmland"] {
                e.register(mod_api::Block::new(id, "minecraft:farmland"))
                    .unwrap();
            }
        }
    }
    pub(crate) struct DispenseNotices(pub(crate) Arc<Probe>);
    #[mod_api::sunrise_part]
    impl BlockPart for DispenseNotices {
        fn contents_changed(&self, _: &mut dyn ServerHost, _: &mut mod_api::ContentsChangedEvent) {
            self.0.dispenser_changes.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) struct Observer(pub(crate) Arc<Probe>);
    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let farmland = r.block_key("inert_test:farmland");
            r.block_part(farmland, HydratesWithAnyFluid);
            let dispenser = r.block_key("minecraft:dispenser");
            r.block_part(dispenser, DispenseNotices(self.0.clone()));
        }
        fn block_tool_modification(
            &self,
            _: &dyn Host,
            event: &mut BlockToolModificationEvent<'_>,
        ) -> Flow {
            let mut tool = self.0.tool.lock().unwrap();
            tool.candidates.push((
                event.tool.ability,
                event
                    .final_state
                    .map(|state| state.try_into_engine(&()).unwrap()),
            ));
            if event.tool.ability == ToolAbility::AxeWaxOff
                && let Some(replacement) = tool.replacement
            {
                *event.final_state = Some(replacement);
            }
            Flow::Continue
        }
    }

    pub(crate) struct ConflictingCode;
    #[mod_api::__subscriptions]
    impl Hooks for ConflictingCode {}

    pub(crate) fn with_inert_block(
        name: &str,
        test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, BlockStateId, &Probe),
    ) {
        if fixture::run_fresh_cases(name, "SUNRISE_INERT_BLOCK_CHILD", &["valid"]) {
            return;
        }
        use mod_api::__private::native::BuiltMod;
        use sunrise_mod_package::{mod_list::NATIVE_DIR, runtime::Runtime};
        const BUILT: &[BuiltMod] = &[
            BuiltMod {
                id: "inert_test",
                version: "1.0.0",
                side: mod_api::Side::Both,
            },
            BuiltMod {
                id: "namespace_claimant",
                version: "1.0.0",
                side: mod_api::Side::Both,
            },
        ];
        let dir = tempfile::tempdir().unwrap();
        // Both packages declare the same alias. The dependency orders admission; the
        // later package's compiled code stays installed when its content is refused.
        for (package, dependencies) in BUILT.iter().zip(["", "[dependencies]\ninert_test='*'"]) {
            let id = package.id;
            let path = dir.path().join(NATIVE_DIR).join(id);
            std::fs::create_dir_all(path.join("src")).unwrap();
            std::fs::write(path.join("sunrise-mod.toml"), format!(
                "id='{id}'\nname='{id}'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\nnamespaces=['inert_test_extra']\n{dependencies}\n[native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'\n",
                "a".repeat(40), "b".repeat(40),
            )).unwrap();
            std::fs::write(
                path.join("Cargo.toml"),
                format!("[package]\nname='{id}'\nversion='1.0.0'\n"),
            )
            .unwrap();
            std::fs::write(path.join("src/lib.rs"), "").unwrap();
        }
        let sources = sunrise_gameplay_common::game::mods::ModsState {
            native: Arc::new(Runtime::load(&dir.path().join("sunrise-client"), BUILT).unwrap()),
            ..Default::default()
        };
        let probe = Arc::new(Probe::default());
        let mods: mod_api::Mods = vec![
            mod_api::ModEntry::new("enabled_prefix", Box::new(EnabledPrefix(probe.clone()))),
            mod_api::ModEntry::new("inert_test", Box::new(Owner(probe.clone()))),
            mod_api::ModEntry::new("namespace_claimant", Box::new(ConflictingCode)),
            mod_api::ModEntry::new("enabled_observer", Box::new(Observer(probe.clone()))),
        ]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            crate::game::api_values::install_test_platform();
            sunrise_gameplay_common::game::api_values::install_packages(
                sources.native.packages(),
                &[],
            )
            .unwrap();
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            mod_api::__private::mark_game_thread();
            let base =
                std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
            let root = sunrise_gameplay_common::game::mods::content_root(&base, &sources).unwrap();
            fixture::with_game_at(root.prepared().to_owned(), |game| {
                game.options.dedicated = Some(Vec::new());
                game.native_level_opened(Dim::Overworld);
                for x in -1..=1 {
                    for z in -1..=1 {
                        let mut chunk = Chunk::empty(x, z);
                        chunk.lit = true;
                        game.world.chunks.insert((x, z), Arc::new(chunk));
                    }
                }
                let state = game
                    .pack
                    .assets
                    .registry
                    .default_state("inert_test:machine")
                    .unwrap();
                game.world.set(POS.x, POS.y, POS.z, state).unwrap();
                test(game, state, &probe);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn reenabled_owner_resumes_held_scheduled_tick() {
        with_inert_block(
            "cut_layout::game::server_side::disabled_block_tests::tests::reenabled_owner_resumes_held_scheduled_tick",
            |game, state, probe| {
                game.server
                    .tickets
                    .0
                    .update(Some(sunrise_sim::ticket_levels::Sources {
                        forced: vec![(0, 0)],
                        ..Default::default()
                    }));
                game.with_level(|_, _, ticks| {
                    ticks
                        .add_chunk(POS.chunk(), Default::default(), Default::default())
                        .unwrap();
                });
                let calls = || {
                    [
                        probe.earlier_ticks.load(Ordering::Relaxed),
                        probe.owner_ticks.load(Ordering::Relaxed),
                    ]
                };
                for stepped in [false, true] {
                    let tick = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                        if stepped {
                            game.tick_scheduled().unwrap();
                        } else {
                            game.with_level(|b, level, ticks| b.tick_scheduled(level, ticks));
                        }
                        assert_eq!(game.world.block(POS.x, POS.y, POS.z), state);
                    };
                    for (part, prefix_enabled) in [true, false].into_iter().enumerate() {
                        assert!(mod_api::__private::set_mod_enabled(
                            "enabled_prefix",
                            prefix_enabled
                        ));
                        let before = calls();
                        game.with_level(|b, level, ticks| {
                            ticks.schedule_block(
                                level.game_time(),
                                POS,
                                b.block(state),
                                0,
                                TickPriority::High,
                            );
                        });
                        assert!(mod_api::__private::set_mod_enabled("inert_test", false));
                        for _ in 0..3 {
                            tick(game);
                            assert_eq!(calls(), before, "no Part runs while the owner is off");
                        }
                        assert!(mod_api::__private::set_mod_enabled("inert_test", true));
                        let mut expected = before;
                        expected[part] += 1;
                        tick(game);
                        assert_eq!(calls(), expected);
                        tick(game);
                        assert_eq!(calls(), expected, "the held tick runs exactly once");
                    }
                }
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn disabled_hydration_holds_farmland_until_the_owner_returns() {
        with_inert_block(
            "cut_layout::game::server_side::disabled_block_tests::tests::disabled_hydration_holds_farmland_until_the_owner_returns",
            |game, _, _| {
                let registry = game.pack.assets.registry;
                let state = |id| registry.default_state(id).unwrap();
                let (air, dirt, water, lava) = (
                    state("minecraft:air"),
                    state("minecraft:dirt"),
                    state("minecraft:water"),
                    state("minecraft:lava"),
                );
                let dry = state("inert_test:farmland");
                let control = state("inert_test:unsubscribed_farmland");
                let [two, three, wet] = game.with_level(|b, _, _| {
                    [2, 3, 7].map(|moisture| b.table.set_int(dry, b.props.moisture, moisture))
                });
                let fluid_pos = POS.offset(2, 0, 0);
                let tick = |game: &mut sunrise_gameplay::game::ServerGame<'_>, initial, fluid| {
                    game.with_level(|b, level, ticks| {
                        level.set_raw(POS, initial);
                        level.set_raw(fluid_pos, fluid);
                        b.random_tick(level, ticks, POS);
                        level.block_state(POS)
                    })
                };
                let level = game.level(game.dim.id()).unwrap();
                let at = [POS.x, POS.y, POS.z];
                assert!(level.set_weather_levels(game, 0.0, 0.0));

                assert!(mod_api::__private::set_mod_enabled("inert_test", false));
                // The later Part accepts lava, but the disabled owner precedes it.
                assert_eq!(tick(game, three, lava), three);
                assert_eq!(tick(game, dry, lava), dry);
                assert_eq!(tick(game, dry, air), dry);
                assert_eq!(tick(game, control, lava), dirt);
                assert_eq!(
                    tick(game, dry, water),
                    wet,
                    "the earlier enabled Part can answer"
                );
                let ticket = level
                    .add_water_ticket(game, at, [POS.x + 1, POS.y + 1, POS.z + 1])
                    .unwrap();
                assert_eq!(
                    tick(game, three, lava),
                    wet,
                    "the ticket wets held farmland even when the earlier Part does not accept the fluid"
                );
                assert!(mod_api::__private::set_mod_enabled("enabled_prefix", false));
                assert_eq!(
                    tick(game, three, air),
                    wet,
                    "a water ticket waters it even without fluid"
                );
                assert!(ticket.remove(game));
                assert_eq!(tick(game, dry, water), dry);

                game.install_live_features();
                let plains = game.res.biomes().id("minecraft:plains");
                let mut chunk = Chunk::empty(0, 0);
                for qy in 0..game.world.shape.height / 4 {
                    for qx in 0..4 {
                        for qz in 0..4 {
                            chunk.set_biome_cell(qx, qy, qz, plains);
                        }
                    }
                }
                game.world.chunks.insert((0, 0), Arc::new(chunk));
                assert!(level.set_weather_levels(game, 1.0, 0.0));
                assert_eq!(
                    level.is_raining_at(game, [POS.x, POS.y + 1, POS.z]),
                    Some(true)
                );
                assert_eq!(
                    tick(game, three, air),
                    three,
                    "rain does not wet held farmland"
                );

                assert!(mod_api::__private::set_mod_enabled(
                    "enabled_observer",
                    false
                ));
                assert!(mod_api::__private::set_mod_enabled("inert_test", true));
                assert_eq!(tick(game, three, air), wet, "rain wets unheld farmland");
                assert!(level.set_weather_levels(game, 0.0, 0.0));
                assert_eq!(tick(game, dry, lava), wet);
                assert_eq!(tick(game, three, air), two);
                assert_eq!(tick(game, dry, air), dirt);
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn disabled_blocks_survive_piston_pushes_without_authorizing_placement() {
        with_inert_block(
            "cut_layout::game::server_side::disabled_block_tests::tests::disabled_blocks_survive_piston_pushes_without_authorizing_placement",
            |game, _, _| {
                let base = BlockPos::new(3, 70, 5);
                let registry = &game.pack.assets.registry;
                let piston = registry
                    .resolve(&sunrise_world::chunk::BlockState {
                        name: "minecraft:piston".into(),
                        properties: [
                            ("facing".into(), "east".into()),
                            ("extended".into(), "false".into()),
                        ]
                        .into(),
                    })
                    .unwrap();
                let state = |name| registry.default_state(name).unwrap();
                let moving = state("minecraft:moving_piston");
                let redstone = state("minecraft:redstone_block");
                let cases = [
                    (CARPET, "minecraft:stone", "minecraft:air"),
                    ("inert_test:plant", "minecraft:dirt", "minecraft:stone"),
                    ("inert_test:plant", "minecraft:dirt", "inert_test:soil"),
                ]
                .map(|(block, source, target)| {
                    (target, state(block), state(source), state(target))
                });
                game.server
                    .tickets
                    .0
                    .update(Some(sunrise_sim::ticket_levels::Sources {
                        forced: vec![(0, 0)],
                        ..Default::default()
                    }));
                game.with_level(|b, level, ticks| {
                    ticks.add_empty_chunk(base.chunk()).unwrap();
                    assert!(mod_api::__private::set_mod_enabled("inert_test", false));
                    for (i, (ground, carried, source_floor, dest_floor)) in
                        cases.into_iter().enumerate()
                    {
                        let piston_pos = base.offset(0, 0, (i as i32) * 3);
                        let start = piston_pos.offset(1, 0, 0);
                        let end = piston_pos.offset(2, 0, 0);
                        level.set_raw(piston_pos, piston).unwrap();
                        level.set_raw(start.offset(0, -1, 0), source_floor).unwrap();
                        if !b.is_air(dest_floor) {
                            level.set_raw(end.offset(0, -1, 0), dest_floor).unwrap();
                        }
                        level.set_raw(start, carried).unwrap();
                        assert!(b.can_survive(level, ticks, start, carried));
                        assert!(
                            !b.can_survive(level, ticks, end, carried),
                            "new placement over {ground}"
                        );
                        b.set_block(level, ticks, piston_pos.offset(-1, 0, 0), redstone, 3);
                        b.run_block_events(level, ticks);
                        assert_eq!(b.block(level.block_state(end)), b.block(moving));
                        for _ in 0..4 {
                            b.tick_block_entities(level, ticks);
                        }
                        assert_eq!(
                            level.block_state(end),
                            carried,
                            "the moved block must survive over {ground}"
                        );
                        assert!(!b.can_survive(level, ticks, start, carried));
                    }
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn disabled_sand_survives_falling_entity_landing() {
        with_inert_block(
            "cut_layout::game::server_side::disabled_block_tests::tests::disabled_sand_survives_falling_entity_landing",
            |game, _, _| {
                let target = POS.offset(1, 0, 0);
                let sand = game
                    .pack
                    .assets
                    .registry
                    .default_state("inert_test:sand")
                    .unwrap();
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.with_level(|b, level, ticks| {
                    level.set_raw(target.offset(0, -1, 0), stone).unwrap();
                    assert!(mod_api::__private::set_mod_enabled("inert_test", false));
                    assert!(!b.can_survive(level, ticks, target, sand));
                    assert_eq!(
                        b.falling_block_landed(level, ticks, target, sand, false),
                        Landing::Placed(sand)
                    );
                    assert_eq!(level.block_state(target), sand);
                });
            },
        );
    }

    #[cfg(feature = "client")]
    pub(crate) const DISPENSER: BlockPos = BlockPos::new(POS.x - 1, POS.y, POS.z);
    #[cfg(feature = "client")]
    pub(crate) const DISPENSER_SLOT: (i32, i32, i32) = (DISPENSER.x, DISPENSER.y, DISPENSER.z);

    #[cfg(feature = "client")]
    pub(crate) fn place_dispenser(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
    ) -> BlockStateId {
        let base = game
            .pack
            .assets
            .registry
            .default_state("minecraft:dispenser")
            .unwrap();
        let dispenser = game.with_props(base, &[("facing", "east")]).unwrap();
        let level = game.level(Dim::Overworld.id()).unwrap();
        let pos = [DISPENSER.x, DISPENSER.y, DISPENSER.z];
        assert_eq!(
            game.level_set_block(level, pos, dispenser, mod_api::BlockUpdateFlags::NONE),
            Ok(true)
        );
        dispenser
    }

    #[test]
    #[ignore = "daily"]
    fn inert_tool_keeps_block_but_global_listener_sees_vanilla_candidate() {
        with_inert_block(
            "cut_layout::game::server_side::disabled_block_tests::tests::inert_tool_keeps_block_but_global_listener_sees_vanilla_candidate",
            |game, state, probe| {
                let registry = game.pack.assets.registry;
                let candidate = registry.default_state("minecraft:observer").unwrap();
                let replacement = registry
                    .default_state("minecraft:stripped_oak_log")
                    .unwrap();
                game.res
                    .behaviours()
                    .set_block_maps(Some(Arc::new(BlockMaps::from_values(
                        [(
                            "minecraft:observer",
                            BlockMapValue::Waxable("inert_test:machine".into()),
                        )],
                        |_| 0,
                    ))));
                let hit = Hit {
                    pos: [POS.x, POS.y, POS.z],
                    face: [0, 1, 0],
                    point: [3.5, 65.0, 5.5],
                    inside: false,
                };
                let axe = ItemStack::new("minecraft:iron_axe", 1);
                let candidates = [
                    (ToolAbility::AxeStrip, None),
                    (ToolAbility::AxeScrape, None),
                    (ToolAbility::AxeWaxOff, Some(candidate)),
                ];
                assert!(mod_api::__private::set_mod_enabled("inert_test", false));
                assert!(!game.use_block_transformer(&axe, &hit).unwrap());
                assert_eq!(game.world.block(POS.x, POS.y, POS.z), state);
                {
                    let mut tool = probe.tool.lock().unwrap();
                    assert_eq!(std::mem::take(&mut tool.candidates), candidates);
                    tool.replacement = Some(replacement.try_into_api().unwrap());
                }
                assert!(game.use_block_transformer(&axe, &hit).unwrap());
                assert_eq!(game.world.block(POS.x, POS.y, POS.z), replacement);
                assert_eq!(probe.tool.lock().unwrap().candidates, candidates);
            },
        );
    }
}
