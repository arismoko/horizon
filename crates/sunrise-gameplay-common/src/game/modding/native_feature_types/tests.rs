use super::super::native_structure_types::book_example;
use super::*;
use mod_api::FeatureError;
use sunrise_core::random::LegacyRandom;
use sunrise_world::registry::BlockStateId;
use sunrise_world::worldgen::mod_features::ModFeatureRandom;

/// A world of air with dirt at y 63 (and nothing readable below 0), recording every write. Its
/// biome is `biomes[0]` west of x 0 and `biomes[1]` from x 0 east.
struct Ground {
    registry: Option<&'static BlockRegistry>,
    writes: Vec<([i32; 3], BlockStateId, i32)>,
    entities: Vec<([i32; 3], Compound)>,
    biomes: [Option<BiomeKey>; 2],
}
impl ModFeatureTarget for Ground {
    fn registry(&self) -> Option<&'static BlockRegistry> {
        self.registry
    }
    fn block(&mut self, pos: [i32; 3]) -> Option<BlockStateId> {
        if pos[1] < 0 {
            return None;
        }
        let written = self.writes.iter().rev().find(|w| w.0 == pos).map(|w| w.1);
        Some(written.unwrap_or(BlockStateId(if pos[1] == 63 { 1 } else { 0 })))
    }
    fn ensure_can_write(&mut self, pos: [i32; 3]) -> bool {
        (0..320).contains(&pos[1])
    }
    fn set_block(&mut self, pos: [i32; 3], state: BlockStateId, flags: i32) -> bool {
        if !self.ensure_can_write(pos) {
            return false;
        }
        self.writes.push((pos, state, flags));
        true
    }
    fn block_in_tag(&mut self, pos: [i32; 3], tag: &str) -> Option<bool> {
        Some(tag == "minecraft:dirt" && self.block(pos) == Some(BlockStateId(1)))
    }
    fn biome(&mut self, pos: [i32; 3]) -> Option<BiomeKey> {
        if pos[1] < 0 {
            return None;
        }
        self.biomes[usize::from(pos[0] >= 0)].clone()
    }
    fn min_y(&mut self) -> Option<i32> {
        Some(-64)
    }
    fn max_y(&mut self) -> Option<i32> {
        Some(319)
    }
    fn height(
        &mut self,
        _: sunrise_world::heightmap::HeightmapType,
        _: i32,
        _: i32,
    ) -> Option<i32> {
        Some(64)
    }
    fn set_block_entity(&mut self, pos: [i32; 3], nbt: Compound) -> bool {
        self.entities.push((pos, nbt));
        true
    }
}

struct Random(LegacyRandom);
impl ModFeatureRandom for Random {
    fn int(&mut self, bound: i32) -> i32 {
        use sunrise_core::random::RandomSource;
        self.0.next_int_bound(bound).unwrap_or(0)
    }
    fn int_unbounded(&mut self) -> i32 {
        use sunrise_core::random::RandomSource;
        self.0.next_int()
    }
    fn long(&mut self) -> i64 {
        use sunrise_core::random::RandomSource;
        self.0.next_long()
    }
    fn float(&mut self) -> f32 {
        use sunrise_core::random::RandomSource;
        self.0.next_float()
    }
    fn double(&mut self) -> f64 {
        use sunrise_core::random::RandomSource;
        self.0.next_double()
    }
    fn boolean(&mut self) -> bool {
        use sunrise_core::random::RandomSource;
        self.0.next_boolean()
    }
    fn gaussian(&mut self) -> f64 {
        use sunrise_core::random::RandomSource;
        self.0.next_gaussian()
    }
}

/// The book's pillar, registered by a native mod and run through the game's adapter on a worker
/// thread (no game or Host there): its config is decoded against its schema at load (a bad one is
/// refused then, naming the field), its states and flags reach the target in engine terms, and
/// its answers come back as given.
#[test]
fn the_books_pillar_places_through_the_adapter_on_a_worker() {
    let registry = registry(&[
        mod_api::ModEntry::new("mymod", Box::new(book_example::features::FeatureMod)),
        mod_api::ModEntry::new("logmod", Box::new(LoggedMod)),
    ]);
    let types = installed_feature_types();
    assert_eq!(
        types.keys().collect::<Vec<_>>(),
        ["logmod:logged", "mymod:pillar"]
    );
    let adapter = types["mymod:pillar"].ty.clone();
    std::thread::spawn(move || {
        let config = |json: serde_json::Value| adapter.prepare_config(&json);
        let place = |origin: [i32; 3], config: &ModFeatureConfig| {
            let mut ground = Ground {
                registry: Some(registry),
                writes: Vec::new(),
                entities: Vec::new(),
                biomes: [None, None],
            };
            let result = adapter.place(
                &mut ground,
                &mut Random(LegacyRandom::new(0)),
                origin,
                &**config,
            );
            (result.unwrap(), ground.writes)
        };
        let three = config(serde_json::json!({"type": "mymod:pillar", "height": 3})).unwrap();
        // On dirt: three logs up, axis y (state 2), with Feature.setBlock's flags (3).
        let (result, writes) = place([0, 64, 0], &three);
        assert!(result);
        assert_eq!(
            writes,
            [64, 65, 66].map(|y| ([0, y, 0], BlockStateId(2), 3))
        );
        // Not on dirt: false, nothing written.
        assert_eq!(place([0, 70, 0], &three), (false, vec![]));
        // The schema's default.
        let default = config(serde_json::json!({"type": "mymod:pillar"})).unwrap();
        assert_eq!(place([0, 64, 0], &default).1.len(), 4);
        // Refused at load, naming the field: out of range, wrong type, undeclared.
        for (json, field) in [
            (
                serde_json::json!({"type": "mymod:pillar", "height": 99}),
                "height",
            ),
            (
                serde_json::json!({"type": "mymod:pillar", "height": "tall"}),
                "height",
            ),
            (
                serde_json::json!({"type": "mymod:pillar", "width": 2}),
                "width",
            ),
        ] {
            let error = config(json).err().unwrap();
            assert!(error.contains(field), "{error}");
        }
    })
    .join()
    .unwrap();
}

/// Block-entity data must fit the current block, including a block this call did not place.
/// A world without the game's registry is a named failure, not a silent false.
#[test]
fn block_entity_data_must_fit_its_block_and_a_registry_is_required() {
    let registry = registry(&[mod_api::ModEntry::new("logmod", Box::new(LoggedMod))]);
    let adapter = installed_feature_types()["logmod:logged"].ty.clone();
    std::thread::spawn(move || {
        let config = adapter
            .prepare_config(&serde_json::json!({"type": "logmod:logged"}))
            .unwrap();
        let log = [0, 64, 0];
        let chest = [1, 64, 0];
        let mut ground = Ground {
            registry: Some(registry),
            writes: vec![(log, BlockStateId(2), 3), (chest, BlockStateId(5), 3)],
            entities: Vec::new(),
            biomes: [None, None],
        };
        let mut random = Random(LegacyRandom::new(0));
        assert!(
            !adapter
                .place(&mut ground, &mut random, log, &*config)
                .unwrap()
        );
        assert!(ground.entities.is_empty());
        assert!(
            adapter
                .place(&mut ground, &mut random, chest, &*config)
                .unwrap()
        );
        assert_eq!(ground.writes.len(), 2);
        assert_eq!(
            ground.entities,
            [(
                chest,
                Compound::from([
                    ("id".into(), Tag::String("minecraft:chest".into())),
                    ("x".into(), Tag::Int(1)),
                    ("y".into(), Tag::Int(64)),
                    ("z".into(), Tag::Int(0)),
                ])
            )]
        );
        let mut bare = Ground {
            registry: None,
            writes: Vec::new(),
            entities: Vec::new(),
            biomes: [None, None],
        };
        let placed = adapter.place(
            &mut bare,
            &mut Random(LegacyRandom::new(0)),
            [0, 64, 0],
            &*config,
        );
        assert_eq!(
            placed.unwrap_err().to_string(),
            "this world has no block registry for mod features"
        );
    })
    .join()
    .unwrap();
}

#[test]
#[ignore = "daily"]
fn a_panicking_feature_preserves_its_work_and_disables_only_its_registration() {
    use mod_api::__private::{HookPanicReport, take_hook_panics};
    use mod_api::{
        Hooks, ModEntry, PackageInfo, PackageKind, Registrar, RegistrationContext, Side,
    };

    // Run the worldgen fixture in its own process.
    const CHILD: &str = "SUNRISE_FEATURE_PANIC_TEST_CHILD";
    if crate::game::native_entity_refs::tests::run_fresh_cases(
        "game::native_feature_types::tests::a_panicking_feature_preserves_its_work_and_disables_only_its_registration",
        CHILD,
        &["1"],
    ) {
        return;
    }

    const OWNER: &str = "test_placer";
    const PANIC: &str = "test feature panicked after writing";
    struct Write {
        log: BlockKey,
        panic_after_write: bool,
    }
    impl FeatureType for Write {
        fn place(
            &self,
            context: &mut dyn FeatureContext,
            origin: [i32; 3],
        ) -> Result<bool, FeatureError> {
            let state = context.state(self.log);
            context.random().next_int();
            let written = context.set_block(origin, state, BlockUpdateFlags::CLIENTS);
            assert!(!self.panic_after_write, "{PANIC}");
            Ok(written)
        }
    }
    struct PlaceMod;
    #[mod_api::__subscriptions]
    impl Hooks for PlaceMod {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let log = r.block_key("minecraft:oak_log");
            r.register_feature_type(
                "trees:broken",
                vec![],
                Write {
                    log,
                    panic_after_write: true,
                },
            );
            r.register_feature_type(
                "healthy",
                vec![],
                Write {
                    log,
                    panic_after_write: false,
                },
            );
        }
    }

    // The panicking type is registered in an additional namespace, not its mod's ID.
    let package = PackageInfo::new(
        OWNER.into(),
        "1.0.0".into(),
        PackageKind::Code,
        Side::Both,
        vec![OWNER.into(), "trees".into()],
        vec![],
    );
    let mods: mod_api::Mods = vec![ModEntry::new(OWNER, Box::new(PlaceMod))].into();
    mod_platform::__private::with_test_package_inventory(vec![package], || {
        mod_platform::testing::with_mods(mods.clone(), || {
            let registry = registry(&mods);
            let types = installed_feature_types();
            let broken = &types["trees:broken"].ty;
            let healthy = &types["test_placer:healthy"].ty;
            let mut ground = Ground {
                registry: Some(registry),
                writes: Vec::new(),
                entities: Vec::new(),
                biomes: [None, None],
            };
            let mut random = Random(LegacyRandom::new(0));
            let mut place = |adapter: &dyn ModFeatureType, id, pos| {
                let config = adapter
                    .prepare_config(&serde_json::json!({"type": id}))
                    .unwrap();
                adapter
                    .place(&mut ground, &mut random, pos, &*config)
                    .unwrap()
            };
            assert!(!place(&**broken, "trees:broken", [0, 64, 0]));
            assert!(!place(&**broken, "trees:broken", [1, 64, 0]));
            assert!(place(&**healthy, "test_placer:healthy", [2, 64, 0]));
            assert_eq!(
                ground.writes,
                [
                    ([0, 64, 0], BlockStateId(2), 2),
                    ([2, 64, 0], BlockStateId(2), 2)
                ]
            );
            // The third Java Random.nextInt() from seed 0: only the first and last calls drew.
            assert_eq!(random.int_unbounded(), 1033096058);
            assert_eq!(
                take_hook_panics(),
                [HookPanicReport {
                    mod_id: OWNER,
                    hook: "FeatureType::place",
                }]
            );
        });
    });
}

/// A feature that copies the ground's dirt to its origin in plains and puts an oak log there in a
/// desert, reading the biome through its context; anywhere else it places nothing.
struct ByBiome {
    plains: BiomeKey,
    desert: BiomeKey,
    dirt: BlockKey,
    log: BlockKey,
}
impl FeatureType for ByBiome {
    fn place(
        &self,
        context: &mut dyn FeatureContext,
        origin: [i32; 3],
    ) -> Result<bool, mod_api::FeatureError> {
        let block = match context.biome(origin) {
            Some(biome) if biome == self.plains => self.dirt,
            Some(biome) if biome == self.desert => self.log,
            _ => return Ok(false),
        };
        let state = context.state(block);
        Ok(context.set_block(origin, state, BlockUpdateFlags::CLIENTS))
    }
}

/// `FeatureContext::biome` answers the key of the biome the level holds at a position, so a
/// feature in a two-biome world gives each biome its own block, and `None` where the level can't
/// be read, where it places nothing.
#[test]
fn a_feature_places_by_the_biome_it_reads() {
    let registry = registry(&[]);
    let keys = mod_api::__private::BiomeKeys::new(
        7,
        ["minecraft:desert", "minecraft:plains"].map(|id| BiomeKey::parse(id).unwrap()),
    );
    let [plains, desert] =
        ["minecraft:plains", "minecraft:desert"].map(|id| keys.view(id).unwrap().key().clone());
    let index = &WorkerResources::get(registry).unwrap().index;
    let [dirt, log] = [1, 2].map(|state| index.to_api(BlockStateId(state)).unwrap().block());
    let adapter = Adapter {
        owner: CallbackOwner::for_mod("test_biome"),
        schema: vec![],
        inner: Arc::new(ByBiome {
            plains: plains.clone(),
            desert: desert.clone(),
            dirt,
            log,
        }),
    };
    std::thread::spawn(move || {
        let config = adapter.prepare_config(&serde_json::json!({})).unwrap();
        let place = |origin: [i32; 3]| {
            let mut ground = Ground {
                registry: Some(registry),
                writes: Vec::new(),
                entities: Vec::new(),
                biomes: [Some(plains.clone()), Some(desert.clone())],
            };
            let result = adapter.place(
                &mut ground,
                &mut Random(LegacyRandom::new(0)),
                origin,
                &*config,
            );
            (result, ground.writes)
        };
        // Plains: dirt (state 1); desert: an oak log, axis y (state 2); flags CLIENTS (2).
        assert_eq!(
            place([-5, 64, 0]),
            (Ok(true), vec![([-5, 64, 0], BlockStateId(1), 2)])
        );
        assert_eq!(
            place([5, 64, 0]),
            (Ok(true), vec![([5, 64, 0], BlockStateId(2), 2)])
        );
        // The level can't be read there: no biome, nothing placed.
        assert_eq!(place([5, -3, 0]), (Ok(false), vec![]));
    })
    .join()
    .unwrap();
}

/// A feature that tries to give the current block a chest's data without placing a block.
struct ChestData;
impl FeatureType for ChestData {
    fn place(
        &self,
        context: &mut dyn FeatureContext,
        origin: [i32; 3],
    ) -> Result<bool, mod_api::FeatureError> {
        let chest = Compound::from([("id".into(), Tag::String("minecraft:chest".into()))]);
        Ok(context.block_entity(origin, chest))
    }
}
struct LoggedMod;
#[mod_api::__subscriptions]
impl mod_api::Hooks for LoggedMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.register_feature_type("logged", vec![], ChestData);
    }
}

/// The caller's thread-local Parts with a small block registry resolved for the workers.
fn registry(mods: &[mod_api::ModEntry]) -> &'static BlockRegistry {
    mod_platform::__private::part_tests::install_parts_fixture(mods).unwrap();
    let registry: &'static BlockRegistry = Box::leak(Box::new(
        BlockRegistry::from_facts_json(br#"{"defaults":{"minecraft:air":0,"minecraft:dirt":1,"minecraft:oak_log":2,"minecraft:chest":5},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"","minecraft:dirt"],["s",false,false,"",{"id":"minecraft:oak_log","properties":{"axis":"y"}}],["s",false,false,"",{"id":"minecraft:oak_log","properties":{"axis":"x"}}],["s",false,false,"",{"id":"minecraft:oak_log","properties":{"axis":"z"}}],["s",false,false,"","minecraft:chest"]]}"#.as_slice())
            .unwrap(),
    ));
    crate::game::api_values::resolve_block_definitions(registry).unwrap();
    registry
}
