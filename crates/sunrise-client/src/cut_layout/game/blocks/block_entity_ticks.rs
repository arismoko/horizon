#[cfg(test)]
use sunrise_level::block_entities::Ticking;
#[cfg(test)]
use sunrise_level::containers::BlockEntity;
#[cfg(test)]
pub(crate) mod jukebox_stop_tests {

    use sunrise_items::items::ItemStack;

    /// A windowless game from the prepared resources with a resident chunk and
    /// a jukebox at (0, 64, 0), recording its game events; `None` without the
    /// resources. `f` gets the game and the disc.
    fn with_jukebox(
        f: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &ItemStack),
    ) -> Option<Vec<String>> {
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        };
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES")?);
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
        .ok()?;
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
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
        );
        let jukebox = game
            .pack
            .assets
            .registry
            .default_state("minecraft:jukebox")?;
        game.world.set(0, 64, 0, jukebox).unwrap();
        let disc = ItemStack::new("minecraft:music_disc_cat", 1);
        sunrise_gameplay::game::dispense_fixture::start_recording();
        f(&mut game, &disc);
        Some(sunrise_gameplay::game::dispense_fixture::stop_recording())
    }

    fn stops(lines: &[String]) -> Vec<&String> {
        lines
            .iter()
            .filter(|l| l.ends_with("minecraft:jukebox_stop_play"))
            .collect()
    }

    /// `JukeboxSongPlayer.stop` posts `JUKEBOX_STOP_PLAY` at the jukebox, and
    /// `popOutTheItem` (the disc taken out by hand) and the end of a song both
    /// reach it. Sunrise stopped the sound and told the parrots but posted the
    /// vibration only when the block itself was broken, so pulling the disc out
    /// was silent to a sculk sensor beside it.
    #[test]
    #[ignore = "daily"]
    fn a_jukebox_that_stops_tells_the_listeners() {
        let Some(lines) = with_jukebox(|game, disc| {
            assert!(
                game.use_jukebox(
                    [0, 64, 0],
                    game.world.block(0, 64, 0),
                    Some(disc),
                    sunrise_world::level::UseBranch::ItemOn,
                ) == Some(mod_api::InteractionResult::Success),
                "the disc goes in"
            );
            assert!(
                game.use_jukebox(
                    [0, 64, 0],
                    game.world.block(0, 64, 0),
                    None,
                    sunrise_world::level::UseBranch::WithoutItem,
                ) == Some(mod_api::InteractionResult::Success),
                "the empty hand pops it out"
            );
        }) else {
            return;
        };
        assert_eq!(
            stops(&lines),
            vec![&"G 0,64,0 minecraft:jukebox_stop_play".to_owned()],
            "{lines:?}"
        );

        // The end of a song is the same stop: tick until the song is over.
        let Some(lines) = with_jukebox(|game, disc| {
            assert!(
                game.use_jukebox(
                    [0, 64, 0],
                    game.world.block(0, 64, 0),
                    Some(disc),
                    sunrise_world::level::UseBranch::ItemOn,
                ) == Some(mod_api::InteractionResult::Success),
                "the disc goes in"
            );
            for _ in 0..40_000 {
                let playing = matches!(
                    game.world.block_entities.get(&(0, 64, 0)),
                    Some(sunrise_level::containers::BlockEntity::Jukebox(j)) if j.playing
                );
                if !playing {
                    break;
                }
                game.tick_jukebox((0, 64, 0));
            }
            // JukeboxSongPlayer.stop posts only while a song plays: taking
            // the finished disc out is silent.
            assert!(
                game.use_jukebox(
                    [0, 64, 0],
                    game.world.block(0, 64, 0),
                    None,
                    sunrise_world::level::UseBranch::WithoutItem,
                ) == Some(mod_api::InteractionResult::Success),
                "the empty hand pops it out"
            );
        }) else {
            return;
        };
        assert_eq!(
            stops(&lines).len(),
            1,
            "the song's own end posts it once, its removal after it nothing: {lines:?}"
        );

        // A broken jukebox posts it from the world's spill instead, and a
        // jukebox that never held a disc posts nothing at all.
        let Some(lines) = with_jukebox(|game, disc| {
            assert!(
                game.use_jukebox(
                    [0, 64, 0],
                    game.world.block(0, 64, 0),
                    Some(disc),
                    sunrise_world::level::UseBranch::ItemOn,
                ) == Some(mod_api::InteractionResult::Success),
                "the disc goes in"
            );
            game.jukebox_removed([0, 64, 0]);
        }) else {
            return;
        };
        assert!(stops(&lines).is_empty(), "not twice: {lines:?}");
        let Some(lines) = with_jukebox(|game, _| {
            for _ in 0..41 {
                game.tick_jukebox((0, 64, 0));
            }
        }) else {
            return;
        };
        assert!(lines.is_empty(), "{lines:?}");
    }
}

#[cfg(test)]
pub(crate) mod add_blocks_tests {
    use super::*;
    use mod_api::*;
    use std::sync::{Arc, OnceLock};

    #[test]
    #[ignore = "daily"]
    fn vanilla_template_membership_creates_ticks_and_saves() {
        const FLAG: &str = "SUNRISE_ADD_BLOCKS_CASE";
        const BLOCK: &str = "add_blocks_test:vault";
        const SIGN: &str = "add_blocks_test:sign";
        const DECLARED: &str = "add_blocks_test:declared";
        if std::env::var_os(FLAG).is_none() {
            for case in ["valid", "quiet", "bad_class", "bad_block", "bad_type"] {
                let scratch = std::env::temp_dir()
                    .join(format!("sunrise-add-blocks-{}-{case}", std::process::id()));
                std::fs::create_dir_all(&scratch).unwrap();
                let mut child = std::process::Command::new(std::env::current_exe().unwrap());
                child.args(["--ignored", "--exact", "cut_layout::game::server_side::block_entity_ticks::add_blocks_tests::vanilla_template_membership_creates_ticks_and_saves", "--nocapture"])
                    .current_dir(&scratch).env(FLAG, case).env("SUNRISE_NO_AUDIO", "1").env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1");
                for key in [
                    "SUNRISE_HOME",
                    "SUNRISE_LAUNCHER_HOME",
                    "XDG_DATA_HOME",
                    "XDG_CONFIG_HOME",
                ] {
                    child.env(key, scratch.join(key));
                }
                let status = child.status().unwrap();
                std::fs::remove_dir_all(&scratch).unwrap();
                assert!(status.success(), "case {case}");
            }
            return;
        }
        let case = std::env::var(FLAG).unwrap();
        #[derive(Default)]
        struct Keys(
            OnceLock<(BlockEntityTypeKey, BlockEntityTypeKey, BlockKey, BlockKey)>,
            std::sync::atomic::AtomicUsize,
        );
        struct Listener(Arc<Keys>, String);
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let kind = r.block_entity_type_key(if self.1 == "bad_type" {
                    "minecraft:missing"
                } else {
                    "minecraft:vault"
                });
                let extra = r.block_entity_type("extra").blocks(&[]).build();
                r.block_entity_type("own").blocks(&[DECLARED]).build();
                let declared = r.block_key(DECLARED);
                let block = r.block_key(if self.1 == "bad_block" {
                    "add_blocks_test:missing"
                } else {
                    BLOCK
                });
                assert_eq!(
                    *self.0.0.get_or_init(|| (kind, extra, block, declared)),
                    (kind, extra, block, declared)
                );
            }
            fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
                for (id, template) in [
                    (
                        BLOCK,
                        if self.1 == "bad_class" {
                            "minecraft:stone"
                        } else {
                            "minecraft:vault"
                        },
                    ),
                    (SIGN, "minecraft:oak_sign"),
                    (DECLARED, "minecraft:vault"),
                ] {
                    event.register(mod_api::Block::new(id, template)).unwrap();
                }
            }
            fn block_entity_type_add_blocks(
                &self,
                _: &RegistrationContext,
                event: &mut BlockEntityTypeAddBlocksEvent<'_>,
            ) {
                self.0.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let &(kind, extra, block, declared) = self.0.0.get().unwrap();
                event.modify(kind, &[block, declared]);
                event.modify_matching(|key| key == kind || key == extra, &[block]);
            }
        }
        struct Quiet(Listener);
        #[mod_api::__subscriptions]
        impl Hooks for Quiet {
            fn register(&self, r: &mut dyn Registrar, context: &RegistrationContext) {
                self.0.register(r, context);
            }
            fn blocks(&self, context: &RegistrationContext, event: &mut BlocksEvent<'_>) {
                self.0.blocks(context, event);
            }
        }
        let observed = Arc::new(Keys::default());
        let listener = Listener(observed.clone(), case.clone());
        let hooks: Box<dyn Hooks> = if case == "quiet" {
            Box::new(Quiet(listener))
        } else {
            Box::new(listener)
        };
        let mods: Mods = vec![ModEntry::new("add_blocks_test", hooks)].into();
        mod_platform::__private::install(mods.clone());
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root = sunrise_gameplay_common::game::mods::content_root(&base, &Default::default());
        if case.starts_with("bad_") {
            let error = root
                .expect_err("invalid addition must fail startup")
                .to_string();
            assert!(
                error.contains(match case.as_str() {
                    "bad_class" => "must inherit VaultBlock",
                    "bad_block" => "unknown block",
                    _ => "unknown block entity type",
                }),
                "{error}"
            );
            return;
        }
        let root = root.unwrap();
        // Reload reuses frozen validity without redispatching registration.
        sunrise_gameplay_common::game::mods::content_root(&base, &Default::default()).unwrap();
        assert_eq!(
            observed.1.load(std::sync::atomic::Ordering::SeqCst),
            usize::from(case != "quiet")
        );
        sunrise_gameplay::game::native_entity_refs::tests::with_game_at(
            root.prepared().to_path_buf(),
            |game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                let registry = game.pack.assets.registry;
                let state = registry.default_state(BLOCK).unwrap();
                let level = game.level(game.dim.id()).unwrap();
                assert!(
                    game.level_set_block(level, [1, 64, 1], state, BlockUpdateFlags::NONE)
                        .unwrap()
                );
                assert!(matches!(
                    BlockEntity::for_block(SIGN),
                    Some(BlockEntity::Sign(_))
                ));
                assert!(matches!(
                    BlockEntity::for_block("minecraft:vault"),
                    Some(BlockEntity::Vault(_))
                ));
                if case == "quiet" {
                    assert!(game.world.block_entities.get(&(1, 64, 1)).is_none());
                    return;
                }
                let states = game.ticking_states();
                assert_eq!(states[state.0 as usize], Ticking::Yes);
                game.bind_server_ticking_states();
                assert!(
                    game.world
                        .block_entity_tickers()
                        .iter()
                        .any(|slot| slot.0 == (1, 64, 1))
                );
                game.world.game_time = 100;
                game.tick_vault((1, 64, 1)).unwrap();
                let entity = game.world.block_entities.get(&(1, 64, 1)).unwrap();
                let BlockEntity::Vault(vault) = entity else {
                    panic!("must create vanilla vault")
                };
                assert_eq!(vault.state_updating_resumes_at, 120);
                let tag = sunrise_sim::save::block_entity_encode::encode(
                    (1, 64, 1),
                    entity,
                    BLOCK,
                    &Default::default(),
                );
                let (_, restored) =
                    sunrise_sim::save::decode_block_entity_for_block(&tag, BLOCK).unwrap();
                assert_eq!(
                    sunrise_sim::save::block_entity_encode::encode(
                        (1, 64, 1),
                        &restored,
                        BLOCK,
                        &Default::default()
                    ),
                    tag
                );
                let BlockEntity::Vault(restored) = restored else {
                    panic!("saved type must stay vanilla")
                };
                assert_eq!(restored.state_updating_resumes_at, 120);
                assert!(matches!(
                    sunrise_sim::save::decode_block_entity_for_block(&tag, DECLARED),
                    Some((_, BlockEntity::Vault(_)))
                ));
                let own_state = registry.default_state(DECLARED).unwrap();
                assert!(
                    game.level_set_block(level, [3, 64, 1], own_state, BlockUpdateFlags::NONE)
                        .unwrap()
                );
                assert!(
                    matches!(game.world.block_entities.get(&(3, 64, 1)), Some(BlockEntity::Mod(entity)) if entity.id() == "add_blocks_test:own")
                );
                assert_eq!(
                    sunrise_game_data::block_entity_types::valid(BLOCK, "minecraft:vault"),
                    Some(true)
                );
                assert_eq!(
                    sunrise_game_data::block_entity_types::valid(BLOCK, "add_blocks_test:extra"),
                    Some(true)
                );
                assert_eq!(sunrise_game_data::block_entity_types::type_for(BLOCK), None);
                assert_eq!(
                    sunrise_game_data::block_entity_types::factory_block(BLOCK),
                    "minecraft:vault"
                );
            },
        );
    }
}

#[cfg(test)]
pub(crate) mod furnace_changed_tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };
    use sunrise_items::items::ItemStack;
    use sunrise_level::containers::{BlockEntity, Furnace, FurnaceKind};
    use {
        sunrise_gameplay::game::game_tests::tests::server_game,
        sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    };

    type Burn = ([i32; 3], String);
    struct Burns(Arc<Mutex<Vec<Burn>>>);

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Burns {
        fn furnace_burn(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::FurnaceBurnEvent<'_>,
        ) {
            self.0.lock().unwrap().push((event.pos, event.fuel.into()));
        }
    }

    fn with_burns(test: impl FnOnce(&Mutex<Vec<Burn>>)) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "furnace_burn_probe",
            Box::new(Burns(seen.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || test(&seen));
    }

    /// `AbstractFurnaceBlockEntity.serverTick`'s changed flag: `burn` sets it,
    /// and the old furnace calls `setChanged` at the end of the tick. A smelt
    /// that completes while the furnace stays LIT changes the output and the
    /// input without changing the block state, so nothing else marks it.
    #[test]
    #[ignore = "daily"]
    fn a_finished_smelt_marks_the_chunk() {
        let mut game_resources = None;
        let mut game = server_game(&mut game_resources);
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:furnace")
            .unwrap();
        let pos = (8, 64, 8);
        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
        game.world.block_entities.insert(
            pos,
            BlockEntity::Furnace(Furnace {
                kind: FurnaceKind::Furnace,
                items: [
                    Some(ItemStack::new("minecraft:raw_iron", 1)),
                    Some(ItemStack::new("minecraft:coal", 1)),
                    None,
                ],
                lit_remaining: 5,
                lit_total: 200,
                cooking_timer: 199,
                cooking_total: 200,
                speed: 1.0,
                recipes_used: HashMap::new(),
            }),
        );
        game.world.modified.clear();

        game.tick_furnace(pos).expect("the furnace ticks");

        let Some(BlockEntity::Furnace(f)) = game.world.block_entities.get(&pos) else {
            panic!("no furnace");
        };
        assert_eq!(
            f.items[2].as_ref().map(|s| &*s.item),
            Some("minecraft:iron_ingot"),
            "the smelt completed"
        );
        assert!(
            game.world.modified.contains(&(0, 0)),
            "a finished smelt left the chunk unmarked"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_refuel_while_lit_marks_the_chunk() {
        if run_fresh_cases(
            "cut_layout::game::server_side::block_entity_ticks::furnace_changed_tests::a_refuel_while_lit_marks_the_chunk",
            "SUNRISE_FURNACE_CHANGED_CASE",
            &["refuel"],
        ) {
            return;
        }
        // litTimeRemaining goes 1 -> 0 and the furnace takes new fuel in the
        // same tick: it stays LIT, and consuming the fuel sets changed.
        with_burns(|seen| {
            let mut game_resources = None;
            let mut game = server_game(&mut game_resources);
            let registry = &game.pack.assets.registry;
            let default = registry.default_state("minecraft:furnace").unwrap();
            let mut lit = registry.state(default).unwrap().clone();
            lit.properties.insert("lit".into(), "true".into());
            let state = registry.resolve(&lit).unwrap();
            let pos = (8, 64, 8);
            game.world.set(pos.0, pos.1, pos.2, state).unwrap();
            game.world.block_entities.insert(
                pos,
                BlockEntity::Furnace(Furnace {
                    kind: FurnaceKind::Furnace,
                    items: [
                        Some(ItemStack::new("minecraft:raw_iron", 1)),
                        Some(ItemStack::new("minecraft:coal", 2)),
                        None,
                    ],
                    lit_remaining: 1,
                    lit_total: 1600,
                    cooking_timer: 10,
                    cooking_total: 200,
                    speed: 1.0,
                    recipes_used: HashMap::new(),
                }),
            );
            game.world.modified.clear();

            game.tick_furnace(pos).expect("the furnace ticks");
            assert_eq!(game.world.block(pos.0, pos.1, pos.2), state);

            let Some(BlockEntity::Furnace(f)) = game.world.block_entities.get(&pos) else {
                panic!("no furnace");
            };
            assert_eq!(f.items[1].as_ref().map(|s| s.count), Some(1), "fuel used");
            assert!(f.items[2].is_none(), "nothing smelted");
            assert!(
                game.world.modified.contains(&(0, 0)),
                "a refuel left the chunk unmarked"
            );
            let expected = [([8, 64, 8], "minecraft:coal".to_owned())];
            assert_eq!(
                &*seen.lock().unwrap(),
                &expected,
                "the refuel fires the hook"
            );

            game.tick_furnace(pos).expect("the furnace keeps burning");
            assert_eq!(
                &*seen.lock().unwrap(),
                &expected,
                "continued burning does not fire the hook again"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_nonpositive_fuel_does_not_mark_or_notify_before_a_real_ignition() {
        if run_fresh_cases(
            "cut_layout::game::server_side::block_entity_ticks::furnace_changed_tests::a_nonpositive_fuel_does_not_mark_or_notify_before_a_real_ignition",
            "SUNRISE_FURNACE_CHANGED_CASE",
            &["nonpositive"],
        ) {
            return;
        }
        with_burns(|seen| {
            let mut resources = None;
            let mut game = server_game(&mut resources);
            let pos = (8, 64, 8);
            let state = game
                .pack
                .assets
                .registry
                .default_state("minecraft:furnace")
                .unwrap();
            game.world.set(pos.0, pos.1, pos.2, state).unwrap();
            let fuel = |duration| {
                sunrise_items::item_data::parse_stack(&format!(
                r#"{{id:"minecraft:coal",count:2,components:{{"minecraft:cooking_fuel":{{burn_time:{duration},speed_multiplier:1.0f}}}}}}"#,
            )).unwrap()
            };
            game.world.block_entities.insert(
                pos,
                BlockEntity::Furnace(Furnace {
                    kind: FurnaceKind::Furnace,
                    items: [Some(ItemStack::new("minecraft:raw_iron", 1)), fuel(0), None],
                    lit_remaining: 0,
                    lit_total: 0,
                    cooking_timer: 0,
                    cooking_total: 200,
                    speed: 1.0,
                    recipes_used: HashMap::new(),
                }),
            );
            game.world.modified.clear();
            game.tick_furnace(pos).unwrap();
            assert!(!game.world.modified.contains(&(0, 0)));
            assert!(seen.lock().unwrap().is_empty());
            let Some(BlockEntity::Furnace(f)) = game.world.block_entities.get_mut(&pos) else {
                panic!("no furnace");
            };
            assert_eq!(f.items[1], fuel(0));
            f.items[1] = fuel(-5);
            for _ in 0..2 {
                game.tick_furnace(pos).unwrap();
                let Some(BlockEntity::Furnace(f)) = game.world.block_entities.get(&pos) else {
                    panic!("no furnace");
                };
                assert_eq!(f.lit_remaining, -5);
                assert_eq!(f.items[1], fuel(-5));
                assert!(!game.world.modified.contains(&(0, 0)));
                assert!(seen.lock().unwrap().is_empty());
            }
            let Some(BlockEntity::Furnace(f)) = game.world.block_entities.get_mut(&pos) else {
                panic!("no furnace");
            };
            f.items[1] = fuel(31);
            game.tick_furnace(pos).unwrap();
            let Some(BlockEntity::Furnace(f)) = game.world.block_entities.get(&pos) else {
                panic!("no furnace");
            };
            assert_eq!(f.lit_remaining, 31);
            assert_eq!(f.items[1].as_ref().map(|s| s.count), Some(1));
            assert!(game.world.modified.contains(&(0, 0)));
            assert_eq!(
                &*seen.lock().unwrap(),
                &[([8, 64, 8], "minecraft:coal".to_owned())]
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_world_pack_supplies_and_replaces_furnace_providers() {
        use sunrise_gameplay::game::world_loading::WorldRequest;
        use sunrise_items::item_data::parse_stack;

        if run_fresh_cases(
            "cut_layout::game::server_side::block_entity_ticks::furnace_changed_tests::a_world_pack_supplies_and_replaces_furnace_providers",
            "SUNRISE_WORLD_FURNACE_PROVIDERS",
            &["enabled"],
        ) {
            return;
        }
        let save = tempfile::tempdir().unwrap();
        let root = save.path();
        let pack = root.join("datapacks/furnace");
        let write = |path: &std::path::Path, body: &str| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        };
        write(
            &root.join("datapacks.json"),
            r#"{"enabled":["file/furnace"],"disabled":[]}"#,
        );
        write(
            &pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"furnace provider fixture"}}"#,
        );
        let definitions = [
            ("data/test/context_int_provider/pack_only.json", "37", "71"),
            (
                "data/test/context_float_provider/pack_only.json",
                "1.75",
                "2.25",
            ),
            (
                "data/minecraft/context_int_provider/cooking/time_coal.json",
                "53",
                "83",
            ),
            (
                "data/minecraft/context_float_provider/cooking/speed_default.json",
                "1.5",
                "2.5",
            ),
        ];
        for (path, initial, _) in definitions {
            write(&pack.join(path), initial);
        }
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let mut request = WorldRequest::new(
            "furnace",
            Some("furnace"),
            Some(1),
            true,
            None,
            Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
        )
        .unwrap();
        request.save_root = root.to_owned();
        game.load_world(request).unwrap();
        assert!(
            game.server
                .fns
                .0
                .enabled()
                .iter()
                .any(|name| name == "file/furnace")
        );
        game.world
            .chunks
            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:furnace")
            .unwrap();
        for reload in [false, true] {
            if reload {
                for (path, _, replacement) in definitions {
                    write(&pack.join(path), replacement);
                }
                game.cmd_reload().unwrap();
            }
            let cases = if reload {
                [(10, 71, 2.5_f32), (11, 83, 2.25_f32)]
            } else {
                [(8, 37, 1.5_f32), (9, 53, 1.75_f32)]
            };
            for ((x, duration, multiplier), (burn, speed)) in cases.into_iter().zip([
                ("test:pack_only", "minecraft:cooking/speed_default"),
                ("minecraft:cooking/time_coal", "test:pack_only"),
            ]) {
                let pos = (x, 64, 8);
                game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                let fuel = parse_stack(&format!(r#"{{id:"minecraft:diamond",count:1,components:{{"minecraft:cooking_fuel":{{burn_time:"{burn}",speed_multiplier:"{speed}"}}}}}}"#)).unwrap();
                game.world.block_entities.insert(
                    pos,
                    BlockEntity::Furnace(Furnace {
                        kind: FurnaceKind::Furnace,
                        items: [Some(ItemStack::new("minecraft:raw_iron", 1)), fuel, None],
                        lit_remaining: 0,
                        lit_total: 0,
                        cooking_timer: 0,
                        cooking_total: 200,
                        speed: 1.0,
                        recipes_used: HashMap::new(),
                    }),
                );
                game.tick_furnace(pos).unwrap();
                let Some(BlockEntity::Furnace(furnace)) = game.world.block_entities.get(&pos)
                else {
                    panic!("no furnace");
                };
                assert!(furnace.items[1].is_none());
                assert_eq!(furnace.lit_remaining, duration);
                assert_eq!(furnace.speed.to_bits(), multiplier.to_bits());
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod slot_guard_tests {
    use mod_api::content::crash::{ModCall, current_native_call};
    use sunrise_gameplay::game::game_tests::tests::server_game;
    use {
        sunrise_gameplay::game::server_side::block_entity_ticks::BlockEntityTick,
        sunrise_gameplay::game::server_side::block_entity_ticks::BlockEntityTickSlot,
        sunrise_gameplay::game::server_side::block_entity_ticks::TickedKind,
    };
    use {
        sunrise_gameplay_common::game::published_loops::Attributed,
        sunrise_gameplay_common::game::published_loops::LoopFlow,
        sunrise_gameplay_common::game::published_loops::LoopMod,
        sunrise_gameplay_common::game::published_loops::LoopSlots,
    };

    struct Row;
    impl LoopMod for Row {
        const CALL: ModCall = ModCall {
            id: "slot_guard",
            hook: "loop block_entities.tick",
            part: Some("slot_guard::Probe"),
        };
    }

    /// What ran, and the mod call each ran inside.
    #[derive(Default)]
    struct Probe {
        before: Vec<Option<ModCall>>,
        after: Vec<Option<ModCall>>,
    }
    #[derive(Default)]
    struct Both(Probe);
    impl BlockEntityTickSlot for Both {
        fn before(&mut self, _: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
            self.0.before.push(current_native_call());
            LoopFlow::Continue
        }
        fn after(&mut self, _: &mut BlockEntityTick<'_, '_>) {
            self.0.after.push(current_native_call());
        }
    }

    /// Each call of a slot, before and after an entry, runs inside its mod's guard.
    #[test]
    #[ignore = "daily"]
    fn a_slot_is_entered_as_its_mod_before_and_after_an_entry() {
        let mut game_resources = None;
        let mut game = server_game(&mut game_resources);
        let mut tick = BlockEntityTick {
            game: &mut game,
            pos: (8, 64, 8),
            kind: TickedKind::Hopper,
            mod_type: None,
        };
        let mut slots = <(Attributed<Row, Both>, ())>::make();
        assert_eq!(slots.before(&mut tick), LoopFlow::Continue);
        slots.after(&mut tick);
        let seen = slots.0.call(|slot| std::mem::take(&mut slot.0)).unwrap();
        assert_eq!(seen.before, [Some(Row::CALL)]);
        assert_eq!(seen.after, [Some(Row::CALL)]);
        assert_eq!(current_native_call(), None);
    }
}

/// What a slot that changes the block entities does to the pass (`BlockEntityTickSlot`'s docs).
#[cfg(test)]
pub(crate) mod slot_mutation_tests {
    use sunrise_gameplay_common::game::GameGameExt1;
    use {
        sunrise_gameplay::game::server_side::block_entity_ticks::BlockEntityTick,
        sunrise_gameplay::game::server_side::block_entity_ticks::BlockEntityTickSlot,
    };

    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay::game::game_tests::tests::server_game;
    use sunrise_gameplay_common::game::published_loops::LoopFlow;
    use sunrise_items::items::ItemStack;
    use sunrise_level::block_entities::Pos;
    use sunrise_level::containers::BlockEntity;

    const A: Pos = (1, 64, 1);
    const B: Pos = (3, 64, 1);
    const C: Pos = (5, 64, 1);

    /// Notes the entities it is asked about and those it sees after their tick, and once, on its
    /// first visit, does `act`.
    struct Mutate {
        act: Option<fn(&mut BlockEntityTick<'_, '_>)>,
        before: Vec<Pos>,
        after: Vec<Pos>,
    }
    impl Mutate {
        fn new(act: fn(&mut BlockEntityTick<'_, '_>)) -> Self {
            Self {
                act: Some(act),
                before: Vec::new(),
                after: Vec::new(),
            }
        }
    }
    impl BlockEntityTickSlot for Mutate {
        fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
            self.before.push(tick.pos);
            if let Some(act) = self.act.take() {
                act(tick);
            }
            LoopFlow::Continue
        }
        fn after(&mut self, tick: &mut BlockEntityTick<'_, '_>) {
            self.after.push(tick.pos);
        }
    }

    fn place(game: &mut sunrise_gameplay::game::ServerGame<'_>, pos: Pos, block: &str) {
        let state = game.pack.assets.registry.default_state(block).unwrap();
        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
        game.world
            .block_entities
            .insert(pos, BlockEntity::for_block(block).unwrap());
    }

    /// A server game with a player beside `A`, `B` and `C`, whose chunk ticks blocks, and
    /// `blocks` placed in order.
    fn with_blocks(
        blocks: &[(Pos, &str)],
        test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
    ) {
        let mut game_resources = None;
        let mut game = server_game(&mut game_resources);
        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 65.0, 8.5)).unwrap(),
        );
        game.update_chunk_tickets();
        assert!(game.block_ticking_area()(A.0, A.2));
        for (pos, block) in blocks {
            place(&mut game, *pos, block);
        }
        test(&mut game);
    }

    fn stone_above(game: &mut sunrise_gameplay::game::ServerGame<'_>, pos: Pos) {
        let at = [
            f64::from(pos.0) + 0.5,
            f64::from(pos.1) + 1.0,
            f64::from(pos.2) + 0.5,
        ];
        game.spawn_item(ItemStack::new("minecraft:stone", 1), at, [0.0; 3], 0);
    }
    fn hopper_count(game: &sunrise_gameplay::game::ServerGame<'_>, pos: Pos) -> u32 {
        match game.world.block_entities.get(&pos) {
            Some(BlockEntity::Chest(items)) => items.iter().flatten().map(|s| s.count).sum(),
            other => panic!("no hopper at {pos:?}: {other:?}"),
        }
    }

    /// A slot that removes the entity it is asked about and answers Continue: it does not tick,
    /// and no `after` is called for it; the next one ticks.
    #[test]
    #[ignore = "daily"]
    fn a_removed_current_entity_does_not_tick() {
        with_blocks(
            &[(A, "minecraft:hopper"), (B, "minecraft:hopper")],
            |game| {
                let mut slot = Mutate::new(|tick| {
                    tick.game.world.block_entities.remove(&tick.pos);
                });
                game.tick_block_entities_with(&mut slot).unwrap();
                assert_eq!(slot.before, [A, B]);
                assert_eq!(slot.after, [B]);
            },
        );
    }

    /// A slot that puts another kind of block entity in the visited one's place: the old kind's
    /// tick is not run on it, and no `after` is called for it.
    #[test]
    #[ignore = "daily"]
    fn a_replaced_current_entity_of_another_kind_does_not_tick() {
        with_blocks(
            &[(A, "minecraft:hopper"), (B, "minecraft:hopper")],
            |game| {
                stone_above(game, A);
                let mut slot = Mutate::new(|tick| {
                    let furnace = BlockEntity::for_block("minecraft:furnace").unwrap();
                    tick.game.world.block_entities.insert(tick.pos, furnace);
                });
                game.tick_block_entities_with(&mut slot).unwrap();
                assert_eq!(slot.before, [A, B]);
                assert_eq!(slot.after, [B]);
                assert!(matches!(
                    game.world.block_entities.get(&A),
                    Some(BlockEntity::Furnace(_))
                ));
                assert_eq!(game.dropped.len(), 1, "no hopper tick took the item");
            },
        );
    }

    /// One a slot adds is not visited in this pass (the pass's list was taken when it started)
    /// and is from the next. The two tickers already there do not sleep, unlike an empty hopper,
    /// so this stays about registration order.
    #[test]
    #[ignore = "daily"]
    fn an_added_entity_ticks_from_the_next_pass() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::block_entity_ticks::slot_mutation_tests::an_added_entity_ticks_from_the_next_pass",
            || {
                with_blocks(
                    &[(A, "minecraft:spawner"), (B, "minecraft:spawner")],
                    |game| {
                        let mut slot = Mutate::new(|tick| place(tick.game, C, "minecraft:hopper"));
                        game.tick_block_entities_with(&mut slot).unwrap();
                        assert_eq!(slot.before, [A, B]);
                        assert_eq!(slot.after, [A, B]);
                        let mut next = Mutate::new(|_| {});
                        game.tick_block_entities_with(&mut next).unwrap();
                        assert_eq!(next.before, [A, B, C]);
                        assert_eq!(next.after, [A, B, C]);
                    },
                );
            },
        );
    }

    /// A slot that runs the whole pass again from inside it: the nested pass ticks everything,
    /// then the enclosing pass goes on with its own scratch. Its later hopper takes an item, and
    /// its spawner list holds the spawner once, as its own pass ticked it.
    #[test]
    #[ignore = "daily"]
    fn a_nested_pass_leaves_the_enclosing_pass_its_scratch() {
        with_blocks(
            &[
                (A, "minecraft:hopper"),
                (B, "minecraft:spawner"),
                (C, "minecraft:hopper"),
            ],
            |game| {
                let mut slot = Mutate::new(|tick| {
                    tick.game.tick_block_entities().unwrap();
                    // The nested pass ticked the spawner; the outer one has not yet.
                    assert_eq!(tick.game.spawner_tick_positions, []);
                    assert!(
                        tick.game.server.item_columns.is_some(),
                        "the outer pass's columns"
                    );
                    stone_above(tick.game, C);
                });
                game.tick_block_entities_with(&mut slot).unwrap();
                assert_eq!(slot.before, [A, B, C]);
                assert_eq!(slot.after, [A, B, C]);
                assert_eq!(
                    hopper_count(game, C),
                    1,
                    "the outer pass's hopper took the item"
                );
                assert!(game.dropped.is_empty());
                assert_eq!(game.spawner_tick_positions, [B]);
                assert!(game.server.item_columns.is_none());
            },
        );
    }

    /// A slot that catches a nested pass's panic and goes on: the enclosing pass still has its own
    /// scratch, so the spawner it ticked is listed once and its later hopper takes an item.
    #[test]
    #[ignore = "daily"]
    fn a_caught_nested_panic_leaves_the_enclosing_pass_its_scratch() {
        struct Boom;
        impl BlockEntityTickSlot for Boom {
            fn before(&mut self, _: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
                panic!("the nested pass's slot");
            }
        }
        struct Catch;
        impl BlockEntityTickSlot for Catch {
            fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
                if tick.pos == C {
                    assert_eq!(tick.game.spawner_tick_positions, [A]);
                    let nested = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        tick.game.tick_block_entities_with(&mut Boom)
                    }));
                    assert!(nested.is_err());
                    assert_eq!(tick.game.spawner_tick_positions, [A]);
                    assert!(
                        tick.game.server.item_columns.is_some(),
                        "the outer pass's columns"
                    );
                    assert!(
                        tick.game.server.pass_mobs.is_some(),
                        "the outer pass's mobs"
                    );
                    stone_above(tick.game, C);
                }
                LoopFlow::Continue
            }
        }
        with_blocks(
            &[
                (A, "minecraft:spawner"),
                (B, "minecraft:hopper"),
                (C, "minecraft:hopper"),
            ],
            |game| {
                game.tick_block_entities_with(&mut Catch).unwrap();
                assert_eq!(
                    hopper_count(game, C),
                    1,
                    "the outer pass's hopper took the item"
                );
                assert_eq!(game.spawner_tick_positions, [A]);
                assert!(game.server.item_columns.is_none());
            },
        );
    }
}

#[cfg(any())]
pub(crate) mod own_block_entity_tests {

    use sunrise_gameplay_common::game::server_side::block_entity_ticks::own_block_entity_tests::A;

    use sunrise_gameplay_common::game::server_side::block_entity_ticks::own_block_entity_tests::B;

    use sunrise_gameplay_common::game::server_side::block_entity_ticks::own_block_entity_tests::DETECTOR;

    use sunrise_gameplay::game::server_side::block_entity_ticks::own_block_entity_tests::place_mod_entity;

    use sunrise_gameplay::game::server_side::block_entity_ticks::own_block_entity_tests::ticks_of;

    use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game_at};

    use std::sync::{Arc, OnceLock};

    use sunrise_core::geometry::Vec3;

    use sunrise_level::block_entities::Pos;

    use sunrise_level::containers::{BlockEntity, ModBlockEntity};

    /// `mods` installed, a game on the prepared resources with a player over chunk (0, 0), whose
    /// blocks tick, and `placed` set there as a player's placement sets them.
    fn with_machines(
        mods: Mods,
        placed: &[(Pos, &str)],
        test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
    ) {
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            mod_api::__private::mark_game_thread();
            let base =
                std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
            let root =
                sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
                    .unwrap();
            with_game_at(root.prepared().to_path_buf(), |game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                game.player = Some(
                    sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 65.0, 8.5))
                        .unwrap(),
                );
                game.update_chunk_tickets();
                let level = game.level(game.dim.id()).unwrap();
                for &((x, y, z), block) in placed {
                    let state = game.pack.assets.registry.default_state(block).unwrap();
                    assert!(
                        game.level_set_block(level, [x, y, z], state, BlockUpdateFlags::NONE)
                            .unwrap()
                    );
                }
                test(game);
            });
        });
    }
    /// Two mods' block entities and a hopper in one world, one of mod B's standing on mod A's block
    /// (B made its type valid there): each slot sees all awake tickers but counts only its own
    /// entities, whatever block they stand on. The hopper is offered only on the first pass, and
    /// an id with no type counts nothing.
    #[test]
    #[ignore = "daily"]
    fn each_slot_counts_only_its_own_mods_block_entities() {
        // The content root freezes the process's block entity types.
        if run_fresh_cases(
            "cut_layout::game::server_side::block_entity_ticks::own_block_entity_tests::each_slot_counts_only_its_own_mods_block_entities",
            "SUNRISE_OWN_BLOCK_ENTITIES_CHILD",
            &["run"],
        ) {
            return;
        }
        let mods: Mods = vec![
            ModEntry::new("own_a", Box::new(Machines::new(A, None))),
            ModEntry::new("own_b", Box::new(Machines::new(B, Some(A)))),
        ]
        .into();
        let (a, b, c) = ((1, 64, 1), (3, 64, 1), (5, 64, 1));
        let hopper = (7, 64, 1);
        let placed = [(a, A), (b, B), (c, A), (hopper, "minecraft:hopper")];
        with_machines(mods, &placed, |game| {
            // Both registered keys take the native save and NBT paths, independent of namespace.
            for (pos, kind) in [(a, A), (b, B)] {
                let entity = game.world.block_entities.get(&pos).unwrap();
                let saved = sunrise_client::save::block_entity_encode::encode(
                    pos,
                    entity,
                    kind,
                    game.items(),
                );
                assert!(sunrise_client::block_entity_nbt::canonical(&saved).is_some());
                let (at, decoded) =
                    sunrise_client::save::decode_block_entity_for_block(&saved, kind).unwrap();
                assert_eq!(at, pos);
                assert!(matches!(decoded, BlockEntity::Mod(ref native) if native.id() == kind));
                game.world.block_entities.insert(pos, decoded);
            }
            // Mod B's entity on mod A's block.
            let d = (9, 64, 1);
            assert!(sunrise_game_data::block_entity_types::additional_valid(
                A, B
            ));
            place_mod_entity(game, d, A, B);
            let mut slots = (
                Count::new(&[A]),
                (Count::new(&[B]), Count::new(&["own_a:missing"])),
            );
            game.tick_block_entities_with(&mut slots).unwrap();
            let (own_a, (own_b, missing)) = &slots;
            assert_eq!((own_a.visited, own_b.visited, missing.visited), (5, 5, 5));
            assert!(game.world.block_entities.is_hopper_asleep(&hopper));

            game.tick_block_entities_with(&mut slots).unwrap();
            let (own_a, (own_b, missing)) = &mut slots;
            assert_eq!((own_a.visited, own_b.visited, missing.visited), (9, 9, 9));
            own_a.mine.sort_unstable();
            own_b.mine.sort_unstable();
            assert_eq!(own_a.mine, [a, a, c, c]);
            assert_eq!(own_b.mine, [b, b, d, d]);
            assert_eq!(missing.mine, []);
            assert_eq!(ticks_of(game, a), [1, 1]);
            assert_eq!(ticks_of(game, b), [2, 2]);
        });
    }
    /// Mod B's block entity standing on a vanilla block, a daylight detector, that B made its type
    /// valid on, beside each mod's own block and a detector with its vanilla block entity: mod B's
    /// slot counts it, mod A's doesn't, and it ticks as B's. Neither counts the vanilla detector.
    #[test]
    #[ignore = "daily"]
    fn a_mods_block_entity_on_a_vanilla_block_is_only_its_own_mods() {
        // The content root freezes the process's block entity types.
        if run_fresh_cases(
            "cut_layout::game::server_side::block_entity_ticks::own_block_entity_tests::a_mods_block_entity_on_a_vanilla_block_is_only_its_own_mods",
            "SUNRISE_OWN_BLOCK_ENTITIES_CHILD",
            &["run"],
        ) {
            return;
        }
        let mods: Mods = vec![
            ModEntry::new("own_a", Box::new(Machines::new(A, None))),
            ModEntry::new("own_b", Box::new(Machines::new(B, Some(DETECTOR)))),
        ]
        .into();
        let (a, b, vanilla) = ((1, 64, 1), (3, 64, 1), (7, 64, 1));
        with_machines(mods, &[(a, A), (b, B), (vanilla, DETECTOR)], |game| {
            assert!(matches!(
                game.world.block_entities.get(&vanilla),
                Some(BlockEntity::DaylightDetector)
            ));
            let d = (5, 64, 1);
            assert!(sunrise_game_data::block_entity_types::additional_valid(
                DETECTOR, B
            ));
            assert!(!sunrise_game_data::block_entity_types::additional_valid(
                DETECTOR, A
            ));
            place_mod_entity(game, d, DETECTOR, B);
            let mut slots = (Count::new(&[A]), Count::new(&[B]));
            for _ in 0..2 {
                game.tick_block_entities_with(&mut slots).unwrap();
            }
            let (own_a, own_b) = &mut slots;
            assert_eq!((own_a.visited, own_b.visited), (8, 8));
            own_a.mine.sort_unstable();
            own_b.mine.sort_unstable();
            assert_eq!(own_a.mine, [a, a]);
            assert_eq!(own_b.mine, [b, b, d, d]);
            assert_eq!(ticks_of(game, d), [2, 2]);
        });
    }
    /// `LevelRef::block_entities` names each block entity's own type: vanilla ones (chest-backed
    /// ones by their block, a copper chest's being a chest's), mod B's on its own block and on a daylight detector (an additional
    /// valid type the detector's factory doesn't make), before and while each is taken out, even
    /// when the block there is replaced meanwhile, and a moving piston's. Entries a chunk saved but hasn't made live are
    /// not listed. It reads the level parked too, and answers None for a stale handle or on a
    /// client.
    #[test]
    #[ignore = "daily"]
    fn level_block_entities_name_each_type_while_taken_out_or_parked() {
        // The content root freezes the process's block entity types.
        if run_fresh_cases(
            "cut_layout::game::server_side::block_entity_ticks::own_block_entity_tests::level_block_entities_name_each_type_while_taken_out_or_parked",
            "SUNRISE_OWN_BLOCK_ENTITIES_CHILD",
            &["run"],
        ) {
            return;
        }
        let mods: Mods = vec![ModEntry::new(
            "own_b",
            Box::new(Machines::new(B, Some(DETECTOR))),
        )]
        .into();
        let on_detector = (5, 64, 1);
        let placed = [
            ((3, 64, 1), B),
            ((7, 64, 1), DETECTOR),
            ((9, 64, 1), "minecraft:chest"),
            ((11, 64, 1), "minecraft:barrel"),
            ((13, 64, 1), "minecraft:copper_chest"),
        ];
        with_machines(mods, &placed, |game| {
            place_mod_entity(game, on_detector, DETECTOR, B);
            // Saved in chunk (1, 0) but not made live: never listed.
            let mut pending = sunrise_level::chunk::Chunk::empty(1, 0);
            pending
                .block_entities
                .push(((20, 64, 1), BlockEntity::Chest(vec![None; 27])));
            game.world.chunks.insert((1, 0), Arc::new(pending));
            // Pistons mid-extension restored with their chunks, before the copy collision reads
            // is refreshed: one here, one in chunk (2, 0), which is not loaded.
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            for pos in [(15, 64, 1), (40, 64, 1)]
                .map(|(x, y, z)| sunrise_core::position::BlockPos::new(x, y, z))
            {
                let piston = sunrise_world::level::pistons::MovingPiston {
                    moved_state: stone,
                    direction: sunrise_core::geometry::Direction::East,
                    extending: true,
                    source: false,
                    progress: 0.5,
                    progress_o: 0.5,
                    last_ticked: 0,
                    death_ticks: 0,
                    id: 0,
                };
                game.block_ticks.load_moving_piston(pos, piston);
            }
            let level = game.level(game.dim.id()).unwrap();
            let (min, max) = ([0, 64, 0], [48, 65, 16]);
            let listed = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                let mut found: Vec<([i32; 3], String)> = level
                    .block_entities(game, min, max)
                    .unwrap()
                    .into_iter()
                    .map(|at| (at.pos, at.kind.id(game).unwrap().to_string()))
                    .collect();
                found.sort();
                found
            };
            let expected: Vec<([i32; 3], String)> = [
                ([3, 64, 1], B),
                ([5, 64, 1], B),
                ([7, 64, 1], DETECTOR),
                ([9, 64, 1], "minecraft:chest"),
                ([11, 64, 1], "minecraft:barrel"),
                ([13, 64, 1], "minecraft:chest"),
                ([15, 64, 1], "minecraft:piston"),
            ]
            .map(|(pos, kind)| (pos, kind.to_string()))
            .into();
            assert_eq!(listed(game), expected);
            let air = game
                .pack
                .assets
                .registry
                .default_state("minecraft:air")
                .unwrap();
            for (pos, _) in placed.iter().copied().chain([(on_detector, DETECTOR)]) {
                let (lease, taken) = game.world.block_entities.take(&pos).unwrap();
                assert_eq!(listed(game), expected, "{pos:?} taken out");
                if !matches!(taken, BlockEntity::Chest(_)) {
                    // Its block replaced while it is out, as a removal callback sees it.
                    let block = game.world.block(pos.0, pos.1, pos.2);
                    game.world.set(pos.0, pos.1, pos.2, air).unwrap();
                    assert_eq!(listed(game), expected, "{pos:?} out, its block replaced");
                    game.world.set(pos.0, pos.1, pos.2, block).unwrap();
                }
                assert!(game.world.block_entities.put(lease, taken));
            }
            game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
                .unwrap();
            assert_eq!(listed(game), expected, "parked");
            game.enter_level(sunrise_gameplay_common::game::Dim::Overworld)
                .unwrap();
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert_eq!(stale.block_entities(game, min, max), None);
            // A moving piston gone from the ticks is gone from the list at once.
            game.block_ticks
                .moving_pistons
                .retain(|(pos, _)| pos.x != 15);
            let mut without = expected.clone();
            without.retain(|(pos, _)| *pos != [15, 64, 1]);
            assert_eq!(listed(game), without);
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
            game.options.dedicated = None;
            assert_eq!(level.block_entities(game, min, max), None);
        });
    }
    /// A slot's `before` replaces mod A's block entity with one of mod B's type and continues: a
    /// later slot's `before` in the same entry still gets the cursor from before the edit (mod
    /// A's type, though B's entity stands there), the replaced entity does not tick, as A's or at
    /// all, and no slot's `after` sees it. The next pass visits it as B's.
    #[test]
    #[ignore = "daily"]
    fn an_entity_replaced_by_another_mods_type_does_not_tick_as_the_old_one() {
        // The content root freezes the process's block entity types.
        if run_fresh_cases(
            "cut_layout::game::server_side::block_entity_ticks::own_block_entity_tests::an_entity_replaced_by_another_mods_type_does_not_tick_as_the_old_one",
            "SUNRISE_OWN_BLOCK_ENTITIES_CHILD",
            &["run"],
        ) {
            return;
        }
        let mods: Mods = vec![
            ModEntry::new("own_a", Box::new(Machines::new(A, None))),
            ModEntry::new("own_b", Box::new(Machines::new(B, Some(A)))),
        ]
        .into();
        let (a, c) = ((1, 64, 1), (3, 64, 1));
        with_machines(mods, &[(a, A), (c, A)], |game| {
            let mut slots = (
                Swap {
                    at: Some(a),
                    after: Vec::new(),
                },
                Seen::new(),
            );
            game.tick_block_entities_with(&mut slots).unwrap();
            let (swap, seen) = &mut slots;
            seen.before.sort_unstable();
            assert_eq!(seen.before, [(a, true, false, B), (c, true, false, A)]);
            assert_eq!(seen.after, [(c, true, false, A)]);
            assert_eq!(swap.after, [c]);
            assert!(ticks_of(game, a).is_empty());
            assert_eq!(ticks_of(game, c), [1]);

            let (swap, seen) = &mut slots;
            (swap.after, seen.before, seen.after) = Default::default();
            game.tick_block_entities_with(&mut slots).unwrap();
            let (swap, seen) = &mut slots;
            seen.before.sort_unstable();
            seen.after.sort_unstable();
            swap.after.sort_unstable();
            assert_eq!(seen.before, [(a, false, true, B), (c, true, false, A)]);
            assert_eq!(seen.after, [(a, false, true, B), (c, true, false, A)]);
            assert_eq!(swap.after, [a, c]);
            assert_eq!(ticks_of(game, a), [2]);
            assert_eq!(ticks_of(game, c), [1, 1]);
        });
    }
}

#[cfg(test)]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
