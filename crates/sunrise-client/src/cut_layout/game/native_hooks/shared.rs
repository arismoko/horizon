#[cfg(test)]
pub(in crate::game) mod tests {
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_hooks::SharedGameExt1 as _;
    #[test]
    #[ignore = "daily"]
    fn declared_types_create_tick_and_reload_through_game() {
        use mod_api::{self, *};
        use std::sync::Arc;
        use sunrise_level::{chunk::Chunk, containers::BlockEntity};
        use sunrise_sim::save;
        use {
            sunrise_gameplay::game::native_entity_refs::tests::record_key,
            sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
            sunrise_gameplay::game::native_entity_refs::tests::with_game_at,
        };

        const CHILD: &str = "SUNRISE_DECLARED_BE_GAME_CHILD";
        if run_fresh_cases(
            "cut_layout::game::native_hooks::tests::declared_types_create_tick_and_reload_through_game",
            CHILD,
            &["valid", "unknown_block", "unknown_type", "block_not_type"],
        ) {
            return;
        }

        const FIRST: &str = "be_test:press";
        const SECOND: &str = "be_test:ornate_banner";
        const EMPTY: &str = "be_test:blank";
        const PLAIN: &str = "be_test:plain_banner";
        #[derive(Default)]
        struct Keys {
            own: std::sync::OnceLock<BlockEntityTypeKey>,
            shared: std::sync::OnceLock<BlockEntityTypeKey>,
            vanilla: std::sync::OnceLock<BlockEntityTypeKey>,
        }
        trait Probe {
            fn value(&self) -> u32;
        }
        struct Marker(u32);
        impl Probe for Marker {
            fn value(&self) -> u32 {
                self.0
            }
        }
        const PROBE: CapabilityKey<dyn Probe> = CapabilityKey::new("be_test:probe");
        struct Press;
        #[mod_api::sunrise_part]
        impl BlockPart for Press {}
        #[mod_api::sunrise_part]
        impl BlockEntityPart for Press {
            fn block_entity(
                &self,
                _: LevelSide<'_>,
                _: &mut BlockEntityEvent<'_>,
            ) -> Option<Compound> {
                Some(Compound::from([("energy".into(), Tag::Long(6300))]))
            }
            fn block_entity_tick(&self, event: &mut BlockEntityTickEvent<'_>) {
                event.subject.with_view(|mut entity| {
                    let key = "energy".into();
                    let Some(Tag::Long(energy)) = entity.nbt().get(&key) else {
                        panic!("factory payload missing");
                    };
                    assert!(entity.set_nbt(key, Some(Tag::Long(energy + 1))));
                });
            }
        }
        struct User {
            keys: Arc<Keys>,
            unknown: Option<&'static str>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for User {
            fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let kind = registrar.block_entity_type_key("be_test:shared");
                registrar
                    .block_entity_part(&kind, Press)
                    .expect("native type");
                registrar.block_entity_capability(&PROBE, &kind, |_, _, _, _, offer| {
                    offer.offer(|| Box::new(Marker(6300)))
                });
                record_key(&self.keys.shared, kind);
                let vanilla = registrar.block_entity_type_key("minecraft:chest");
                registrar.block_entity_capability(&PROBE, &vanilla, |_, _, _, _, offer| {
                    offer.offer(|| Box::new(Marker(26)))
                });
                record_key(&self.keys.vanilla, vanilla);
                if let Some(id) = self.unknown {
                    registrar.block_entity_type_key(id);
                }
            }
        }
        struct TestMod {
            invalid: bool,
            keys: Arc<Keys>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for TestMod {
            fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let kind = registrar
                    .block_entity_type("shared")
                    .blocks(if self.invalid {
                        &["be_test:missing"]
                    } else {
                        &[FIRST, SECOND]
                    })
                    .build();
                record_key(&self.keys.own, kind);
                registrar
                    .block_entity_type("empty")
                    .blocks(&[EMPTY])
                    .build();
            }
            fn blocks(&self, _context: &mod_api::RegistrationContext, event: &mut BlocksEvent<'_>) {
                for block in [FIRST, SECOND, EMPTY, PLAIN]
                    .map(|id| mod_api::Block::new(id, "minecraft:white_banner"))
                {
                    event.register(block).unwrap();
                }
            }
        }
        let case = std::env::var(CHILD).unwrap();
        let invalid = case == "unknown_block";
        let unknown = match case.as_str() {
            "unknown_type" => Some("other:missing"),
            "block_not_type" => Some("minecraft:red_shulker_box"),
            _ => None,
        };
        let keys = Arc::new(Keys::default());
        let mods: Mods = Arc::from([
            ModEntry::new(
                "be_user",
                Box::new(User {
                    keys: keys.clone(),
                    unknown,
                }),
            ),
            ModEntry::new(
                "be_test",
                Box::new(TestMod {
                    invalid,
                    keys: keys.clone(),
                }),
            ),
        ]);
        mod_platform::__private::install(mods.clone());
        // `be_user` attaches a Part to the type `be_test` declares, which names `be_test`
        // among its dependencies.
        let package = |id: &str, depends_on: &[&str]| {
            PackageInfo::new(
                id.into(),
                "1".into(),
                PackageKind::Code,
                Side::Both,
                vec![id.into()],
                depends_on
                    .iter()
                    .map(|owner| {
                        PackageDependency::new(
                            (*owner).into(),
                            "*".into(),
                            DependencyKind::Required,
                            DependencyOrder::After,
                            Side::Both,
                            None,
                            None,
                        )
                    })
                    .collect(),
            )
        };
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        mod_platform::__private::with_test_package_inventory(
            vec![package("be_test", &[]), package("be_user", &["be_test"])],
            || {
                sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
                let root =
                    sunrise_gameplay_common::game::mods::content_root(&base, &Default::default());
                if invalid {
                    let error = root.unwrap_err().to_string();
                    assert!(error.contains("be_test:shared"), "{error}");
                    assert!(error.contains("be_test:missing"), "{error}");
                    // A refused declaration builds no content root. The roots folder's own `.lock`
                    // is made by every start and never deleted, so it is not a root.
                    let built: Vec<_> = std::fs::read_dir(".sunrise-cache/mod-content")
                        .into_iter()
                        .flatten()
                        .map(|entry| entry.unwrap().file_name())
                        .filter(|name| name != ".lock")
                        .collect();
                    assert!(built.is_empty(), "{built:?}");
                    return;
                }
                if let Some(id) = unknown {
                    let error = root.unwrap_err().to_string();
                    assert!(error.contains("be_user"), "{error}");
                    assert!(error.contains("unknown block entity type"), "{error}");
                    assert!(error.contains(id), "{error}");
                    assert!(sunrise_game_data::block_entity_types::current().is_none());
                    return;
                }
                assert_eq!(keys.shared.get(), keys.own.get());
                with_game_at(root.unwrap().prepared().to_path_buf(), |game| {
                    game.options.dedicated = Some(Vec::new());
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
                    for (x, block) in [
                        (1, FIRST),
                        (2, SECOND),
                        (3, EMPTY),
                        (4, PLAIN),
                        (5, "minecraft:chest"),
                    ] {
                        let state = game.pack.assets.registry.default_state(block).unwrap();
                        assert_eq!(
                            game.level_set_block(level, [x, 63, 0], stone, BlockUpdateFlags::NONE),
                            Ok(true)
                        );
                        assert_eq!(
                            game.level_set_block(level, [x, 64, 0], state, BlockUpdateFlags::NONE),
                            Ok(true)
                        );
                        let pos = (x, 64, 0);
                        if block == PLAIN {
                            // No declared type claims it, and no mod adds it to `minecraft:banner`'s
                            // valid blocks, so its template creates no entity (`validateBlockState`).
                            assert!(game.world.block_entities.get(&pos).is_none());
                            assert!(game.block_entity(level, [x, 64, 0]).is_none());
                            continue;
                        }
                        let handle = game.block_entity(level, [x, 64, 0]).unwrap();
                        if block == "minecraft:chest" {
                            assert_eq!(handle.kind(game).as_ref(), keys.vanilla.get());
                            let probe = level.capability(game, &PROBE, [x, 64, 0], None).unwrap();
                            assert_eq!((probe.value(), probe.owner()), (26, "be_user".into()));
                            continue;
                        }
                        let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&pos)
                        else {
                            panic!("declared type did not override its banner template");
                        };
                        if block == EMPTY {
                            assert_eq!(entity.id(), "be_test:empty");
                            assert!(entity.nbt.is_empty());
                            continue;
                        }
                        assert_eq!(entity.id(), "be_test:shared");
                        assert_eq!(entity.nbt.get(&"energy".into()), Some(&Tag::Long(6300)));
                        assert_eq!(handle.kind(game).as_ref(), keys.shared.get());
                        let probe = level.capability(game, &PROBE, [x, 64, 0], None).unwrap();
                        assert_eq!((probe.value(), probe.owner()), (6300, "be_user".into()));
                        game.tick_mod_block_entity(pos);
                        let saved = save::block_entity_encode::encode(
                            pos,
                            game.world.block_entities.get(&pos).unwrap(),
                            block,
                            &Default::default(),
                        );
                        assert_eq!(saved.get("energy"), Some(&Tag::Long(6301)));
                        let (saved_pos, restored) =
                            save::decode_block_entity_for_block(&saved, block).unwrap();
                        assert_eq!(saved_pos, pos);
                        assert_eq!(restored.id(), "be_test:shared");
                        assert!(save::decode_block_entity_for_block(&saved, EMPTY).is_none());
                        game.world.block_entities.insert(pos, restored);
                        game.tick_mod_block_entity(pos);
                        let saved = save::block_entity_encode::encode(
                            pos,
                            game.world.block_entities.get(&pos).unwrap(),
                            block,
                            &Default::default(),
                        );
                        assert_eq!(saved.get("energy"), Some(&Tag::Long(6302)));
                    }

                    // A level hook's `LevelWriteHost::set_block` fires the same hook in its write.
                    let first = game.pack.assets.registry.default_state(FIRST).unwrap();
                    use sunrise_gameplay_common::game::api_values::TryIntoApi;
                    let written = game.with_level(|_, level, ticks| {
                        LevelWriteHost::set_block(
                            &mut sunrise_gameplay_common::game::mod_api_host::LevelTicks::new(
                                level, ticks,
                            ),
                            [5, 64, 0],
                            first.try_into_api().unwrap(),
                            BlockUpdateFlags::NONE,
                        )
                    });
                    assert!(written);
                    let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&(5, 64, 0))
                    else {
                        panic!("a level write did not create the declared block entity");
                    };
                    assert_eq!(entity.id(), "be_test:shared");
                    assert_eq!(entity.nbt.get(&"energy".into()), Some(&Tag::Long(6300)));
                });
            },
        );
    }

    #[cfg(feature = "client")]
    #[test]
    fn observed_overlay_prefers_first_open_screen() {
        assert_eq!(
            sunrise_client_gameplay::game::client_side::native_hooks_client::observed_overlay_id(
                &[(false, "sign"), (false, "book")]
            ),
            None
        );
        assert_eq!(
            sunrise_client_gameplay::game::client_side::native_hooks_client::observed_overlay_id(
                &[(false, "sign"), (true, "book")]
            ),
            Some("book")
        );
        assert_eq!(
            sunrise_client_gameplay::game::client_side::native_hooks_client::observed_overlay_id(
                &[(true, "sign"), (true, "book")]
            ),
            Some("sign")
        );
    }

    /// Exercise the gameplay constructor, where name-based entities compete
    /// with a mod block's sign template.
    #[test]
    #[ignore = "daily"]
    fn mod_sign_constructor_prioritizes_templates_over_suffixes() {
        // This fixture adds blocks that are absent from the shared vanilla
        // game's frozen process-wide block definitions.
        if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::native_hooks::tests::mod_sign_constructor_prioritizes_templates_over_suffixes",
            "SUNRISE_SIGN_CONSTRUCTOR_CHILD",
            &["signs"],
        ) {
            return;
        }
        use sunrise_mod_package::content::{Block, Content};
        use sunrise_sim::mod_content::{ContentMod, build};
        use {
            crate::game::Settings, crate::game::load_resources,
            sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        };

        let base = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES")
                .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
        );
        let root = std::env::temp_dir().join(format!(
            "sunrise-sign-constructor-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let cases = [
            ("ruby:notice_banner", "minecraft:oak_sign", false),
            ("ruby:board_shelf", "minecraft:oak_wall_sign", false),
            ("ruby:lantern_head", "minecraft:oak_hanging_sign", true),
            (
                "ruby:bracket_skull",
                "minecraft:oak_wall_hanging_sign",
                true,
            ),
        ];
        let mut content = Content::declared("ruby");
        let blocks: Vec<_> = cases
            .iter()
            .map(|&(id, template, _)| (id, template))
            .chain([("ruby:road_sign", "minecraft:stone")])
            .map(|(id, template)| Block {
                id: id.into(),
                template: template.into(),
                item: false,
                sign_text_color: (id == "ruby:notice_banner").then(|| "light_blue".into()),
                sign_text_glow: (id == "ruby:notice_banner").then_some(true),
                ..Default::default()
            })
            .collect();
        content.declare(&blocks, &[]).unwrap();
        build(
            &base,
            std::path::Path::new(""),
            &[ContentMod {
                id: "ruby".into(),
                content,
                jars: Vec::new(),
            }],
            &root,
            &sunrise_mod_package::enabled::EnabledMods::new([("ruby".into(), mod_api::Side::Both)]),
        )
        .unwrap();
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
            &mut |_, _| unreachable!("no menu handoff"),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
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

        for (name, hanging) in cases
            .map(|(name, _, hanging)| (name, hanging))
            .into_iter()
            .chain([
                ("minecraft:oak_sign", false),
                ("minecraft:oak_hanging_sign", true),
            ])
        {
            match game.new_block_entity(name) {
                Some(sunrise_level::containers::BlockEntity::Sign(sign)) => {
                    assert_eq!(sign.hanging, hanging, "{name}")
                }
                entity => panic!("{name}: expected a sign, got {entity:?}"),
            }
        }
        // `createDefaultSignText`: a declared colour and glow on both sides of a new sign, black
        // and unlit on one that declares none.
        let mut text = |name| match game.new_block_entity(name) {
            Some(sunrise_level::containers::BlockEntity::Sign(sign)) => {
                assert_eq!(sign.front.color, sign.back.color);
                assert_eq!(sign.front.glowing, sign.back.glowing);
                (sign.front.color, sign.front.glowing)
            }
            entity => panic!("{name}: expected a sign, got {entity:?}"),
        };
        assert_eq!(text("ruby:notice_banner"), (3, true));
        assert_eq!(text("ruby:board_shelf"), (15, false));
        assert_eq!(text("minecraft:oak_sign"), (15, false));
        assert!(game.new_block_entity("ruby:road_sign").is_none());
        assert!(matches!(
            game.new_block_entity("minecraft:white_banner"),
            Some(sunrise_level::containers::BlockEntity::Banner(_))
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn generated_fire_matches_dispatch() {
        use mod_api::{ClientHost, Flow, Hooks, ModEntry};
        use std::sync::{Arc, Mutex};
        use {
            crate::game::Audio, crate::game::Settings, crate::game::load_resources,
            sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        };

        #[derive(Default)]
        struct Listener(usize, Arc<Mutex<Vec<usize>>>);
        #[mod_api::sunrise_mod]
        impl Hooks for Listener {
            fn block_highlight(
                &self,
                _: &dyn ClientHost,
                _: &mut mod_api::BlockHighlightEvent,
            ) -> Flow {
                self.1.lock().unwrap().push(self.0);
                if self.0 == 1 {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
            fn screen_exclusion_areas(
                &self,
                _: &dyn ClientHost,
                event: &mut mod_api::ScreenExclusionAreasEvent<'_>,
            ) {
                self.1.lock().unwrap().push(self.0);
                event.areas.push([self.0 as f32, 0.0, 0.0, 0.0]);
            }
            fn movement_input_update(
                &self,
                _: &dyn ClientHost,
                event: &mut mod_api::MovementInputUpdateEvent<'_>,
            ) {
                self.1.lock().unwrap().push(self.0);
                event.input.move_vector[1] += self.0 as f32;
            }
        }
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
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
        let game = sunrise_client_gameplay::game::ClientGame::new(
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
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mods = (0..3)
            .map(|i| ModEntry::new("listener", Box::new(Listener(i, trace.clone()))))
            .collect::<Vec<_>>();
        mod_platform::testing::with_mods(mods.into(), || {
            let run = |fire: bool| {
                trace.lock().unwrap().clear();
                let mut canceled = false;
                if fire {
                    game.test_fire_block_highlight(
                        [1, 2, 3],
                        sunrise_gameplay_common::game::native_hooks::BlockStateId(7),
                        0.5,
                        &mut canceled,
                    );
                } else {
                    mod_api::dispatch_block_highlight(
                        &game,
                        [1, 2, 3],
                        sunrise_gameplay_common::game::api_values::TryIntoApi::try_into_api(
                            &sunrise_gameplay_common::game::native_hooks::BlockStateId(7),
                        )
                        .unwrap(),
                        0.5,
                        &mut canceled,
                    );
                }
                let cancel_trace = trace.lock().unwrap().clone();
                trace.lock().unwrap().clear();
                let view =
                    sunrise_client_gameplay::game::client_side::screen_build::ObservedScreen {
                        id: "inventory",
                        overlay: false,
                        mod_screen: None,
                        menu: None,
                        menu_generation: None,
                        dialog_key: None,
                        title: None,
                        rect: [0.0; 4],
                        screen_size: [0.0; 2],
                        slots: 0,
                        mouse: [0.0; 2],
                        partial_tick: 0.0,
                        regions: Vec::new(),
                    };
                let mut areas = Vec::new();
                if fire {
                    game.test_fire_screen_exclusion_areas(&view.view(), &mut areas);
                } else {
                    mod_api::dispatch_screen_exclusion_areas(&game, &view.view(), &mut areas);
                }
                let collect_trace = trace.lock().unwrap().clone();
                trace.lock().unwrap().clear();
                let forward = game.movement_dispatch_fixture(fire);
                (
                    canceled,
                    cancel_trace,
                    areas,
                    collect_trace,
                    forward,
                    trace.lock().unwrap().clone(),
                )
            };
            let old = run(false);
            assert!(old.0);
            assert_eq!(old.1, [0, 1]);
            assert_eq!(old.3, [0, 1, 2]);
            assert_eq!(old.4, 4.0);
            assert_eq!(old, run(true));
        });
    }

    /// With no window to lay a screen out in there is no slot under the pointer.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn no_stack_is_hovered_without_a_window() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::native_hooks::tests::no_stack_is_hovered_without_a_window",
            || {
                use {
                    crate::game::Audio, crate::game::Settings, crate::game::load_resources,
                    sunrise_gameplay_common::game::Options,
                    sunrise_gameplay_common::game::Progress,
                };

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
                    sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
                    &Progress::new(),
                    &mut |_, _| unreachable!("no menu handoff"),
                )
                .unwrap();
                let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
                options.resources = root.clone();
                let game = sunrise_client_gameplay::game::ClientGame::new(
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
                assert_eq!(game.screen_hovered_stack(), None);
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn declared_stock_storage_honors_shapes_and_rates() {
        use mod_api::{self, *};
        use std::sync::Arc;
        use sunrise_gameplay::game::native_entity_refs::tests::{run_fresh_cases, with_game_at};
        use sunrise_level::{chunk::Chunk, containers::BlockEntity};
        use sunrise_sim::save;

        const CHILD: &str = "SUNRISE_STOCK_STORAGE_CHILD";
        if run_fresh_cases(
            "cut_layout::game::native_hooks::tests::declared_stock_storage_honors_shapes_and_rates",
            CHILD,
            &["run"],
        ) {
            return;
        }

        const VAULT: &str = "stock_test:vault";
        const VAULT_SLOT: usize = u8::MAX as usize;
        const TANK: &str = "stock_test:tank";
        const BATTERY: &str = "stock_test:battery";

        struct StockMod;
        #[mod_api::__subscriptions]
        impl Hooks for StockMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.block_entity_type("vault").blocks(&[VAULT]).build();
                r.block_entity_type("tank").blocks(&[TANK]).build();
                r.block_entity_type("battery").blocks(&[BATTERY]).build();

                let vault = r.block_key(VAULT);
                let mut container_faces: [Vec<usize>; 6] = Default::default();
                container_faces[Direction::Up as usize] = vec![VAULT_SLOT];
                r.block_container(
                    &vault,
                    ModContainer {
                        slots: VAULT_SLOT + 1,
                        faces: container_faces,
                        inputs: vec![VAULT_SLOT],
                        outputs: vec![VAULT_SLOT],
                        insert_rules: Vec::new(),
                        menu: None,
                    },
                );

                let tank = r.block_key(TANK);
                let mut fluids = ModFluidTanks::default();
                fluids.tanks = vec![1000; 17];
                fluids.tanks[0] = 0;
                fluids.tanks[16] = 1_000_001;
                fluids.faces[Direction::Up as usize] = vec![0, 16];
                fluids.inputs = vec![0, 16];
                fluids.outputs = vec![0, 16];
                r.block_fluids(&tank, fluids);

                let battery = r.block_key(BATTERY);
                let mut extract_faces = [false; 6];
                extract_faces[Direction::East as usize] = true;
                r.block_energy(
                    &battery,
                    ModEnergyBuffer {
                        capacity: 0,
                        max_insert: 500,
                        max_extract: 500,
                        insert_faces: [false; 6],
                        extract_faces,
                    },
                );
            }

            fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
                for id in [VAULT, TANK, BATTERY] {
                    event
                        .register(mod_api::Block::new(id, "minecraft:stone"))
                        .unwrap();
                }
            }
        }

        let mods: Mods = Arc::from([ModEntry::new("stock_test", Box::new(StockMod))]);
        mod_platform::__private::install(mods.clone());
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root = sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
            .unwrap()
            .prepared()
            .to_path_buf();
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            // Populate stock tables through the game's real declarations -> table path.
            game.tick_block_entities_with(&mut ()).unwrap();

            let level = game.level(game.dim.id()).unwrap();
            let vault_pos = [1, 64, 0];
            let tank_pos = [2, 64, 0];
            let battery_pos = [3, 64, 0];
            for (pos, id) in [(vault_pos, VAULT), (tank_pos, TANK), (battery_pos, BATTERY)] {
                let state = game.pack.assets.registry.default_state(id).unwrap();
                assert_eq!(
                    game.level_set_block(level, pos, state, BlockUpdateFlags::NONE),
                    Ok(true)
                );
            }

            // The largest saved slot index remains accessible through the sided handler.
            let items = level
                .capability(game, &ITEMS, vault_pos, Some(Direction::Up))
                .unwrap();
            assert_eq!(items.owner(), "the game");
            assert_eq!(items.size(game), 1);
            let dirt = ItemResource::of(
                &mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:dirt").unwrap(),
                    1,
                )
                .unwrap(),
            );
            assert_eq!(
                game.transaction(|host, tx| Ok::<_, ()>(items.insert_at(host, tx, 0, &dirt, 7))),
                Ok(Ok(7))
            );
            assert_eq!(items.amount(game, 0), 7);
            assert_eq!(items.resource(game, 0), Some(dirt.clone()));
            let vault_key = (vault_pos[0], vault_pos[1], vault_pos[2]);
            let saved = save::block_entity_encode::encode(
                vault_key,
                game.world.block_entities.get(&vault_key).unwrap(),
                VAULT,
                &Default::default(),
            );
            let (saved_pos, restored) = save::decode_block_entity_for_block(&saved, VAULT).unwrap();
            assert_eq!(saved_pos, vault_key);
            let BlockEntity::Mod(reloaded) = &restored else {
                panic!("expected mod block entity for vault");
            };
            assert_eq!(reloaded.items.len(), VAULT_SLOT + 1);
            assert_eq!(
                reloaded.items[VAULT_SLOT].as_ref(),
                Some(&sunrise_items::items::ItemStack::new("minecraft:dirt", 7))
            );
            game.world.block_entities.insert(vault_key, restored);
            assert_eq!(items.amount(game, 0), 7);
            assert_eq!(items.resource(game, 0), Some(dirt));

            // 2. 17-tank declaration exposes tank 16 (capacity 1,000,001), zero-capacity tank 0
            // refuses fill, and a malformed negative saved amount decodes as empty without overflow.
            let fluids = level
                .capability(game, &FLUIDS, tank_pos, Some(Direction::Up))
                .unwrap();
            assert_eq!(fluids.owner(), "the game");
            assert_eq!(fluids.size(game), 2);
            let water = FluidResource::of(&FluidStack::new(
                mod_api::__private::fluid_key("minecraft:water").unwrap(),
                1,
            ));
            assert_eq!(fluids.capacity(game, 0, Some(&water)), 0);
            assert_eq!(fluids.capacity(game, 1, Some(&water)), 1_000_001);
            let tank_key = (tank_pos[0], tank_pos[1], tank_pos[2]);
            if let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get_mut(&tank_key) {
                let mut list = vec![Tag::Compound(Compound::new()); 17];
                list[16] = Tag::compound([
                    ("Fluid", Tag::String("minecraft:water".into())),
                    ("Amount", Tag::Int(i32::MIN)),
                ]);
                entity.nbt.insert("fluids".into(), Tag::List(list));
            } else {
                panic!("expected mod block entity for tank");
            }
            assert_eq!(fluids.amount(game, 1), 0);
            assert_eq!(fluids.resource(game, 1), None);
            assert_eq!(
                game.transaction(|host, tx| Ok::<_, ()>((
                    fluids.insert_at(host, tx, 0, &water, 1000),
                    fluids.insert_at(host, tx, 1, &water, 1_000_001),
                ))),
                Ok((Ok(0), Ok(1_000_001)))
            );
            assert_eq!(fluids.amount(game, 0), 0);
            assert_eq!(fluids.resource(game, 0), None);
            assert_eq!(fluids.amount(game, 1), 1_000_001);
            assert_eq!(fluids.resource(game, 1), Some(water));

            // 3. Zero-capacity energy buffer with saved energy and max_extract > capacity exposes
            // the sided handler and extracts inside the normal transaction.
            let battery_key = (battery_pos[0], battery_pos[1], battery_pos[2]);
            if let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get_mut(&battery_key)
            {
                entity.nbt.insert("energy".into(), Tag::Int(800));
            } else {
                panic!("expected mod block entity for battery");
            }
            assert!(
                level
                    .capability(game, &ENERGY, battery_pos, Some(Direction::West))
                    .is_none()
            );
            let battery = level
                .capability(game, &ENERGY, battery_pos, Some(Direction::East))
                .unwrap();
            assert_eq!(battery.owner(), "the game");
            assert_eq!(battery.capacity(game), 0);
            assert_eq!(battery.amount(game), 800);
            assert_eq!(
                game.transaction(|host, tx| Ok::<_, ()>((
                    battery.insert(host, tx, 100),
                    battery.extract(host, tx, 600),
                ))),
                Ok((Ok(0), Ok(500)))
            );
            assert_eq!(battery.amount(game), 300);
            let abort: Result<(), mod_api::TransactionError<()>> = game.transaction(|host, tx| {
                assert_eq!(battery.extract(host, tx, 200), Ok(200));
                Err(())
            });
            assert!(abort.is_err());
            assert_eq!(battery.amount(game), 300);
        });
    }
}

#[cfg(test)]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
