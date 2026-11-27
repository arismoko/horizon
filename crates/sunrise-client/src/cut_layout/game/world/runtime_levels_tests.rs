//! Levels a mod opens at run time: the table's rules, and a mod's calls against a game.

use super::native_entity_refs::tests::with_game;
use mod_api::{BiomePoint, RuntimeBiomeSource};
use mod_api::{
    ChunkRef, ChunkUnloadEvent, Hooks, HostSide, LevelLoadEvent, LevelRef, LevelTickPostEvent,
    LevelUnloadEvent, ModEntry, RuntimeGenerator, RuntimeLevel, RuntimeLevelError, ServerHost,
};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use sunrise_gameplay::game::server_side::runtime_levels::{
    Generate, OpenLevel, RETAINED_TABLE_LIMIT, RuntimeTable, classify_biome_source_error,
    retained_metadata_counts, runtime_biome_source_json,
};
use sunrise_gameplay_common::game::Dim;

fn flat() -> RuntimeLevel {
    RuntimeLevel::new("minecraft:overworld", RuntimeGenerator::Flat, 1)
}

fn admitted_flat(seed: i64) -> OpenLevel {
    OpenLevel {
        dimension_type: "minecraft:overworld".into(),
        shape: sunrise_level::chunk::LevelShape::OVERWORLD,
        facts: sunrise_world::dimension_type::DimensionFacts::overworld().clone(),
        seed,
        generate: Generate::Layers {
            flat: true,
            biome: 0,
        },
    }
}

#[test]
fn runtime_biome_source_validates_local_fields() {
    assert_eq!(
        RuntimeBiomeSource::Fixed("minecraft:plains".into()).invalid_field(),
        None
    );
    assert!(
        RuntimeBiomeSource::Fixed(String::new())
            .invalid_field()
            .is_some()
    );
    assert!(
        RuntimeBiomeSource::Checkerboard {
            biomes: vec![],
            scale: 2,
        }
        .invalid_field()
        .is_some()
    );
    assert!(
        RuntimeBiomeSource::Checkerboard {
            biomes: vec!["minecraft:plains".into()],
            scale: 63,
        }
        .invalid_field()
        .is_some()
    );
    assert_eq!(
        RuntimeBiomeSource::Checkerboard {
            biomes: vec!["minecraft:plains".into(), "minecraft:desert".into()],
            scale: 62,
        }
        .invalid_field(),
        None
    );
    assert_eq!(
        RuntimeBiomeSource::Checkerboard {
            biomes: vec!["#minecraft:is_forest".into()],
            scale: 0,
        }
        .invalid_field(),
        None
    );
    assert!(
        RuntimeBiomeSource::Checkerboard {
            biomes: vec!["minecraft:plains".into(), "#minecraft:is_forest".into()],
            scale: 0,
        }
        .invalid_field()
        .is_some()
    );
    assert!(
        RuntimeBiomeSource::Checkerboard {
            biomes: vec!["#minecraft:is_forest".into(), "#minecraft:is_ocean".into()],
            scale: 0,
        }
        .invalid_field()
        .is_some()
    );
    assert!(
        RuntimeBiomeSource::MultiNoise(String::new())
            .invalid_field()
            .is_some()
    );
    assert_eq!(
        RuntimeBiomeSource::MultiNoise("minecraft:overworld".into()).invalid_field(),
        None
    );
    assert_eq!(RuntimeBiomeSource::End.invalid_field(), None);
    assert!(RuntimeBiomeSource::Points(vec![]).invalid_field().is_some());
    assert_eq!(
        RuntimeBiomeSource::Points(vec![BiomePoint {
            biome: "minecraft:plains".into(),
            temperature: (-1.0, 1.0),
            humidity: (0.0, 0.0),
            continentalness: (0.0, 0.0),
            erosion: (0.0, 0.0),
            depth: (0.0, 0.0),
            weirdness: (0.0, 0.0),
            offset: 0.0,
        }])
        .invalid_field(),
        None
    );
    assert!(
        RuntimeBiomeSource::Registered(String::new())
            .invalid_field()
            .is_some()
    );
    assert_eq!(
        RuntimeBiomeSource::Registered("mymod:rings".into()).invalid_field(),
        None
    );
}

#[test]
fn biome_source_admission_errors_separate_invalid_from_unknown() {
    use std::collections::{BTreeMap, BTreeSet};
    use sunrise_world::worldgen::{
        biome_source::{BiomeSourceCatalog, BiomeSourceError},
        mod_biome_sources::{LevelBiomeSource, ModBiomeSource, ModBiomeSources},
    };
    let biomes = BTreeSet::from(["minecraft:plains".into(), "minecraft:desert".into()]);
    let tags = BTreeMap::from([(
        "minecraft:is_dry".into(),
        vec!["minecraft:desert".into(), "minecraft:plains".into()],
    )]);
    let presets = BTreeSet::from(["minecraft:overworld".into()]);
    let catalog = BiomeSourceCatalog::new(&biomes, &tags, &presets).unwrap();
    let bind_err = |json: &str| {
        classify_biome_source_error(
            catalog
                .bind_custom_source(json, &BTreeMap::new(), 0)
                .unwrap_err(),
        )
    };
    for invalid in [
        r#"{"type":"minecraft:fixed","biome":"BAD!ID"}"#,
        r#"{"type":"minecraft:fixed","biome":"test:bad names the unknown biome test:missing"}"#,
        r#"{"type":"minecraft:fixed","biome":""}"#,
        r##"{"type":"minecraft:checkerboard","biomes":"#BAD!TAG","scale":0}"##,
        r#"{"type":"minecraft:checkerboard","biomes":[],"scale":0}"#,
        r#"{"type":"minecraft:checkerboard","biomes":["minecraft:plains"],"scale":63}"#,
        r#"{"type":"minecraft:checkerboard","biomes":{"type":"neoforge:or"},"scale":0}"#,
        r#"{"type":"minecraft:checkerboard","biomes":{"type":"neoforge:or","values":["test:bad names the unknown biome test:missing"]}}"#,
        r#"{"type":"minecraft:multi_noise","preset":"BAD!ID"}"#,
    ] {
        assert_eq!(
            bind_err(invalid),
            RuntimeLevelError::InvalidBiomeSource,
            "{invalid}"
        );
    }
    for unknown in [
        r#"{"type":"minecraft:fixed","biome":"mymod:missing"}"#,
        r##"{"type":"minecraft:checkerboard","biomes":"#mymod:missing","scale":0}"##,
        r#"{"type":"minecraft:checkerboard","biomes":{"type":"neoforge:or","values":["test:missing"]}}"#,
        r#"{"type":"minecraft:multi_noise","preset":"mymod:missing"}"#,
        r#"{"type":"minecraft:the_end"}"#,
        r#"{"type":"mymod:missing_source"}"#,
    ] {
        assert_eq!(
            bind_err(unknown),
            RuntimeLevelError::UnknownBiomeSource,
            "{unknown}"
        );
    }
    struct Source(Vec<String>);
    impl ModBiomeSource for Source {
        fn possible_biomes(&self) -> &[String] {
            &self.0
        }
        fn noise_biome(&self, _: [i32; 3], _: i64) -> Result<u16, String> {
            unreachable!("a refused source is not sampled")
        }
    }
    for (entries, expected) in [
        (vec![], RuntimeLevelError::InvalidBiomeSource),
        (
            vec!["test:missing".into()],
            RuntimeLevelError::UnknownBiomeSource,
        ),
    ] {
        let sources = ModBiomeSources::from([(
            "test:source".into(),
            Arc::new(Source(entries)) as Arc<dyn ModBiomeSource>,
        )]);
        let error =
            LevelBiomeSource::<()>::resolve("test:source", &sources, 0, |_| None).unwrap_err();
        assert_eq!(classify_biome_source_error(error.into()), expected);
    }
    assert_eq!(
        classify_biome_source_error(BiomeSourceError::MissingPreset),
        RuntimeLevelError::UnknownBiomeSource
    );

    assert_eq!(
        runtime_biome_source_json(&RuntimeBiomeSource::Checkerboard {
            biomes: vec!["minecraft:plains".into(), "#minecraft:is_dry".into()],
            scale: 0,
        }),
        Err(RuntimeLevelError::InvalidBiomeSource)
    );
    assert_eq!(
        runtime_biome_source_json(&RuntimeBiomeSource::Checkerboard {
            biomes: vec!["#minecraft:is_dry".into(), "#minecraft:is_forest".into()],
            scale: 0,
        }),
        Err(RuntimeLevelError::InvalidBiomeSource)
    );
    assert_eq!(
        runtime_biome_source_json(&RuntimeBiomeSource::Checkerboard {
            biomes: vec!["#minecraft:is_dry".into()],
            scale: 0,
        })
        .unwrap()["biomes"],
        serde_json::json!("#minecraft:is_dry")
    );
    for builtin in [
        "minecraft:fixed",
        "fixed",
        "minecraft:checkerboard",
        "minecraft:multi_noise",
        "minecraft:the_end",
    ] {
        assert_eq!(
            runtime_biome_source_json(&RuntimeBiomeSource::Registered(builtin.into())),
            Err(RuntimeLevelError::UnknownBiomeSource),
            "{builtin}"
        );
    }
    for invalid in ["", "BAD!ID"] {
        assert_eq!(
            runtime_biome_source_json(&RuntimeBiomeSource::Registered(invalid.into())),
            Err(RuntimeLevelError::InvalidBiomeSource),
            "{invalid}"
        );
    }
}

#[test]
fn runtime_table_has_no_artificial_level_cap() {
    let mut table = RuntimeTable::default();
    for n in 0..80 {
        assert!(
            table
                .queue_open("test", &format!("cap:l{n}"), admitted_flat(1))
                .is_ok()
        );
    }
}

#[test]
#[ignore = "daily"]
fn a_runtime_checkerboard_resolves_tags_before_admission_and_uses_their_biomes() {
    use sunrise_gameplay_common::game::test_fixtures::run_fresh_cases;
    use sunrise_world::worldgen::mod_generators::{
        ChunkAccess, ModChunkGenerator, ModChunkGenerators,
    };

    if run_fresh_cases(
        "cut_layout::game::server_side::runtime_levels_tests::a_runtime_checkerboard_resolves_tags_before_admission_and_uses_their_biomes",
        "SUNRISE_RUNTIME_CHECKERBOARD_CHILD",
        &["admission"],
    ) {
        return;
    }
    struct Terrain;
    impl ModChunkGenerator for Terrain {
        fn build_terrain(&self, _: &mut dyn ChunkAccess) {}
    }
    let generators = ModChunkGenerators::from([(
        mod_api::Identifier::parse("checker:test").unwrap(),
        Arc::new(Terrain) as Arc<dyn ModChunkGenerator>,
    )]);
    with_opener("checker", "checker:level", flat(), |game, _| {
        let level = |biomes| {
            RuntimeLevel::new(
                "minecraft:overworld",
                RuntimeGenerator::Mod("checker:test".into()),
                0,
            )
            .with_biome_source(RuntimeBiomeSource::Checkerboard { biomes, scale: 0 })
        };
        assert_eq!(
            game.test_runtime_level_queue_open(
                "checker:level",
                level(vec!["#checker:absent".into()]),
                &generators,
            ),
            Err(RuntimeLevelError::UnknownBiomeSource)
        );
        assert_eq!(
            game.test_runtime_level_queue_open(
                "checker:level",
                level(vec!["plains".into(), "#checker:absent".into()]),
                &generators,
            ),
            Err(RuntimeLevelError::InvalidBiomeSource)
        );
        assert_eq!(
            game.test_runtime_level_queue_open(
                "checker:level",
                level(vec!["BAD!ID".into()]),
                &generators,
            ),
            Err(RuntimeLevelError::InvalidBiomeSource)
        );
        assert_eq!(
            game.test_runtime_level_queue_open(
                "checker:level",
                RuntimeLevel::new(
                    "minecraft:overworld",
                    RuntimeGenerator::Mod("checker:test".into()),
                    0,
                )
                .with_biome_source(RuntimeBiomeSource::Fixed(
                    "test:bad names the unknown biome test:missing".into(),
                )),
                &generators,
            ),
            Err(RuntimeLevelError::InvalidBiomeSource)
        );
        assert_eq!(
            game.test_runtime_level_queue_open(
                "checker:level",
                RuntimeLevel::new(
                    "minecraft:overworld",
                    RuntimeGenerator::Mod("checker:test".into()),
                    0,
                )
                .with_biome_source(RuntimeBiomeSource::Registered("minecraft:fixed".into())),
                &generators,
            ),
            Err(RuntimeLevelError::UnknownBiomeSource)
        );
        assert_eq!(
            game.runtime_level_open(
                "checker:level",
                flat().with_biome_source(RuntimeBiomeSource::End),
            ),
            Err(RuntimeLevelError::IncompatibleBiomeSource)
        );
        // A no-stage checkerboard needs only biome and tag data.
        let minimal_resources = tempfile::tempdir().unwrap();
        let minimal_worldgen = minimal_resources.path().join("worldgen");
        std::fs::create_dir_all(&minimal_worldgen).unwrap();
        std::fs::write(
            minimal_worldgen.join("biomes.json"),
            r#"{"minecraft:forest":"{}","minecraft:flower_forest":"{}"}"#,
        )
        .unwrap();
        std::fs::write(
            minimal_worldgen.join("biome_tags.json"),
            r#"{"minecraft:is_forest":"{\"values\":[\"minecraft:forest\",\"minecraft:flower_forest\"]}"}"#,
        )
        .unwrap();
        let original_resources = std::mem::replace(
            &mut game.common.options.resources,
            minimal_resources.path().to_owned(),
        );
        let queued = game.test_runtime_level_queue_open(
            "checker:level",
            level(vec!["#minecraft:is_forest".into()]),
            &generators,
        );
        game.common.options.resources = original_resources;
        queued.unwrap();
        assert!(!game.runtime_level_is_open("checker:level"));
        game.apply_runtime_levels();
        assert!(game.runtime_level_is_open("checker:level"));
        game.enter_level(Dim::extra("checker:level").unwrap())
            .unwrap();
        for (cx, name) in [(0, "minecraft:forest"), (1, "minecraft:flower_forest")] {
            assert!(
                game.server
                    .streaming
                    .set_forced_loaded(&mut game.common.world, cx, 0, true)
            );
            let chunk = &game.world.chunks[&(cx, 0)];
            assert_eq!(
                sunrise_gameplay_common::world_biomes!(game.common).name(chunk.biome(0, 0, 0)),
                name
            );
        }
        game.enter_level(Dim::Overworld).unwrap();
        game.runtime_level_unload("checker:level").unwrap();
        game.apply_runtime_levels();
    });
}

#[test]
fn an_id_is_open_once_and_each_open_has_its_own_generation() {
    let mut table = RuntimeTable::default();
    let first = table.queue_open("test", "gen:a", admitted_flat(1)).unwrap();
    assert_eq!(
        table.queue_open("test", "gen:a", admitted_flat(1)),
        Err(RuntimeLevelError::AlreadyOpen)
    );
    assert_eq!(table.generation_of("gen:a"), first);
    table.remove("gen:a");
    let second = table.queue_open("test", "gen:a", admitted_flat(1)).unwrap();
    assert_ne!(first, second);
    assert_eq!(table.generation_of("gen:a"), second);
    assert_eq!(table.generation_of("minecraft:the_nether"), 0);
}

#[test]
fn a_level_is_open_after_its_request_is_applied_and_until_its_close_is() {
    let mut table = RuntimeTable::default();
    table
        .queue_open("test", "state:a", admitted_flat(1))
        .unwrap();
    assert!(!table.is_open("state:a"));
    assert_eq!(
        table.queue_unload("test", "state:a"),
        Err(RuntimeLevelError::NotOpen)
    );
    table.mark_open("state:a");
    assert!(table.is_open("state:a"));
    assert_eq!(table.queue_unload("test", "state:a"), Ok(()));
    assert!(
        table.is_open("state:a"),
        "still open until the close is applied"
    );
    assert_eq!(
        table.queue_unload("test", "state:a"),
        Err(RuntimeLevelError::NotOpen)
    );
    assert_eq!(
        table.queue_unload("test", "state:none"),
        Err(RuntimeLevelError::NotOpen)
    );
}

type Seen<T> = Arc<Mutex<Vec<T>>>;

/// What the mod does in each level tick: open `id`, delete it, or nothing.
const OPEN: u8 = 0;
const DELETE: u8 = 1;
const IDLE: u8 = 2;
/// Nothing in a level tick; a Nether chunk's unload forces chunk (0, 0) of `id`.
const RESCUE: u8 = 3;

/// A mod that opens `id` in `level_tick_post` and says what each call answered.
struct Opener {
    id: &'static str,
    level: RuntimeLevel,
    mode: Arc<AtomicU8>,
    deletes: Seen<Result<(), RuntimeLevelError>>,
    answers: Seen<Result<LevelRef, RuntimeLevelError>>,
    open_right_after: Seen<bool>,
    loads: Seen<String>,
    load_contexts: Seen<(String, String, bool)>,
    unloads: Seen<String>,
    rescued: Seen<bool>,
}

#[mod_api::__subscriptions]
impl Hooks for Opener {
    fn level_tick_post(&self, host: HostSide, _: &mut LevelTickPostEvent) {
        let HostSide::Server(host) = host else {
            return;
        };
        match self.mode.load(Ordering::SeqCst) {
            OPEN => {}
            DELETE => {
                let answer = host.runtime_levels().delete(self.id);
                self.deletes.lock().unwrap().push(answer);
                let open = host.runtime_levels().is_open(self.id);
                self.open_right_after.lock().unwrap().push(open);
                return;
            }
            _ => return,
        }
        let answer = host.runtime_levels().open(self.id, self.level.clone());
        self.answers.lock().unwrap().push(answer);
        let open = host.runtime_levels().is_open(self.id);
        self.open_right_after.lock().unwrap().push(open);
    }
    fn level_load(&self, host: &mut dyn mod_api::ServerHost, event: &mut LevelLoadEvent<'_>) {
        self.loads.lock().unwrap().push(event.dimension.to_owned());
        self.load_contexts.lock().unwrap().push((
            event.dimension.to_owned(),
            host.dimension().to_owned(),
            host.level(event.dimension).unwrap().in_place(host),
        ));
    }
    fn level_unload(&self, _: &mut dyn mod_api::ServerHost, event: &mut LevelUnloadEvent<'_>) {
        self.unloads
            .lock()
            .unwrap()
            .push(event.dimension.to_owned());
    }
    fn chunk_unload(&self, host: &mut dyn ServerHost, event: &mut ChunkUnloadEvent<'_>) {
        if self.mode.load(Ordering::SeqCst) != RESCUE
            || event.dimension != "minecraft:the_nether"
            || event.chunk != [0, 0]
        {
            return;
        }
        let level = host.level(self.id).unwrap();
        let forced = ChunkRef::new(level, [0, 0]).set_forced(host, true);
        self.rescued.lock().unwrap().push(forced);
    }
}

struct Fixture {
    mode: Arc<AtomicU8>,
    deletes: Seen<Result<(), RuntimeLevelError>>,
    answers: Seen<Result<LevelRef, RuntimeLevelError>>,
    open_right_after: Seen<bool>,
    loads: Seen<String>,
    load_contexts: Seen<(String, String, bool)>,
    unloads: Seen<String>,
    rescued: Seen<bool>,
}

/// A mod named `name` that opens `id` each time a level ticks, in a game, for `test`.
fn with_opener(
    name: &'static str,
    id: &'static str,
    level: RuntimeLevel,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &Fixture),
) {
    let fixture = Fixture {
        mode: Arc::new(AtomicU8::new(OPEN)),
        deletes: Seen::default(),
        answers: Seen::default(),
        open_right_after: Seen::default(),
        loads: Seen::default(),
        load_contexts: Seen::default(),
        unloads: Seen::default(),
        rescued: Seen::default(),
    };
    let opener = Opener {
        id,
        level,
        mode: fixture.mode.clone(),
        deletes: fixture.deletes.clone(),
        answers: fixture.answers.clone(),
        open_right_after: fixture.open_right_after.clone(),
        loads: fixture.loads.clone(),
        load_contexts: fixture.load_contexts.clone(),
        unloads: fixture.unloads.clone(),
        rescued: fixture.rescued.clone(),
    };
    let mods = vec![ModEntry::new(name, Box::new(opener))].into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            // Levels' folders go under a world root of the test's own, never the checkout.
            let root = TempRoot::new(name);
            game.server.world_root = root.0.clone();
            test(game, &fixture);
        });
    });
}

/// A world root of one test, removed when the test ends, even by a failed assertion.
struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "sunrise-runtime-levels-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `RuntimeLevels::unload` as the mod `name` calls it.
fn unload_as(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    name: &'static str,
    id: &str,
) -> Result<(), RuntimeLevelError> {
    let _call = mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
        id: name,
        hook: "level_tick_post",
        part: None,
    });
    game.runtime_level_unload(id)
}

/// `RuntimeLevels::delete` as the mod `name` calls it.
fn delete_as(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    name: &'static str,
    id: &str,
) -> Result<(), RuntimeLevelError> {
    let _call = mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
        id: name,
        hook: "level_tick_post",
        part: None,
    });
    game.runtime_level_delete(id)
}

/// Generate the chunk (0, 0) of the open level `id`, put `block` at (1, -50, 1) and leave the
/// level again.
fn place_block(game: &mut sunrise_gameplay::game::ServerGame<'_>, id: &str, block: &str) {
    let state = game.pack.assets.registry.default_state(block).unwrap();
    let dim = Dim::extra(id).unwrap();
    game.enter_level(dim).unwrap();
    game.server.streaming.forced.insert((0, 0));
    for _ in 0..2000 {
        game.side_update_world();
        if game.world.chunks.contains_key(&(0, 0)) {
            break;
        }
        game.server
            .streaming
            .wait(std::time::Duration::from_millis(5));
    }
    assert!(
        game.world.chunks.contains_key(&(0, 0)),
        "the chunk generated"
    );
    game.world.set(1, -50, 1, state).unwrap();
    game.enter_level(Dim::Overworld).unwrap();
}

/// The block at (1, -50, 1) of the open level `id`, loading it as `place_block` does.
fn block_at(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    id: &str,
) -> sunrise_world::registry::BlockStateId {
    let dim = Dim::extra(id).unwrap();
    game.enter_level(dim).unwrap();
    game.server.streaming.forced.insert((0, 0));
    for _ in 0..2000 {
        game.side_update_world();
        if game.world.chunks.contains_key(&(0, 0)) {
            break;
        }
        game.server
            .streaming
            .wait(std::time::Duration::from_millis(5));
    }
    assert!(game.world.chunks.contains_key(&(0, 0)), "the chunk loaded");
    let block = game.world.block(1, -50, 1);
    game.enter_level(Dim::Overworld).unwrap();
    block
}

/// A file with some bytes in it, its folders made.
fn write_file(path: &std::path::Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"x").unwrap();
}

fn tick_post(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    let home = game.level("minecraft:overworld").unwrap();
    game.native_level_tick_post(home);
}

#[test]
#[ignore = "daily"]
fn a_mod_can_open_other_namespaces_but_not_reserved_or_invalid_ids() {
    for (id, expected) in [
        ("other:x", Ok(())),
        ("mymod:../x", Err(RuntimeLevelError::InvalidId)),
        ("mymod:a//b", Err(RuntimeLevelError::InvalidId)),
        ("minecraft:overworld", Err(RuntimeLevelError::ReservedId)),
        // Names Windows maps to a device or to another folder: refused before any folder exists.
        ("mymod:con", Err(RuntimeLevelError::InvalidId)),
        ("mymod:a/nul.txt", Err(RuntimeLevelError::InvalidId)),
        ("mymod:com1", Err(RuntimeLevelError::InvalidId)),
        ("mymod:lpt9/x", Err(RuntimeLevelError::InvalidId)),
        ("mymod:a.", Err(RuntimeLevelError::InvalidId)),
        ("mymod:x/b.", Err(RuntimeLevelError::InvalidId)),
        ("mymod:console", Ok(())),
        ("mymod:com10", Ok(())),
        ("mymod:instance/ab", Ok(())),
    ] {
        with_opener("mymod", id, flat(), |game, seen| {
            tick_post(game);
            let answer = seen.answers.lock().unwrap()[0].map(|_| ());
            assert_eq!(answer, expected, "{id}");
        });
    }
}

#[test]
#[ignore = "daily"]
fn nothing_opens_inside_a_tick_and_the_queue_is_applied_after_the_levels() {
    with_opener("phase", "phase:a", flat(), |game, seen| {
        tick_post(game);
        assert!(seen.answers.lock().unwrap()[0].is_ok());
        assert_eq!(*seen.open_right_after.lock().unwrap(), [false]);
        assert!(!game.runtime_level_is_open("phase:a"));
        assert!(seen.loads.lock().unwrap().is_empty());
        game.tick_server(false).unwrap();
        assert!(game.runtime_level_is_open("phase:a"));
        assert_eq!(*seen.loads.lock().unwrap(), ["phase:a"]);
        assert_eq!(
            *seen.load_contexts.lock().unwrap(),
            [("phase:a".into(), "minecraft:overworld".into(), false)]
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_handle_from_before_an_unload_never_names_the_reopened_level() {
    with_opener("stale", "stale:a", flat(), |game, seen| {
        tick_post(game);
        game.apply_runtime_levels();
        let first = seen.answers.lock().unwrap()[0].unwrap();
        assert!(game.level_handle_current(first));
        assert!(game.native_level_world(first).is_some());
        unload_as(game, "stale", "stale:a").unwrap();
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("stale:a"));
        assert!(game.native_level_world(first).is_none());
        // The same id opens again: a new open, a new handle.
        seen.answers.lock().unwrap().clear();
        tick_post(game);
        game.apply_runtime_levels();
        let second = seen.answers.lock().unwrap()[0].unwrap();
        assert_ne!(first, second);
        assert!(game.native_level_world(second).is_some());
        assert!(!game.level_handle_current(first));
        assert!(game.native_level_world(first).is_none());
        assert_eq!(*seen.loads.lock().unwrap(), ["stale:a", "stale:a"]);
        assert_eq!(*seen.unloads.lock().unwrap(), ["stale:a"]);
    });
}

/// Idle chunk eviction preserves the level and its handle without firing unload or load on re-entry.
#[test]
#[ignore = "daily"]
fn the_idle_close_keeps_a_runtime_level_open_until_its_unload() {
    with_opener("idle", "idle:a", flat(), |game, seen| {
        tick_post(game);
        game.apply_runtime_levels();
        let handle = seen.answers.lock().unwrap()[0].unwrap();
        let dim = Dim::extra("idle:a").unwrap();
        game.enter_level(dim).unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        game.world.set(1, 64, 1, stone).unwrap();
        game.summon_with_nbt("minecraft:pig", [2.0, 64.0, 2.0], "{}")
            .unwrap()
            .unwrap();
        let save = game.server.save.0.clone();
        // Encoding is refused before the batch can enter the writer.
        game.server.save.0 = Arc::new(sunrise_sim::save::SaveDir::new(
            game.server.world_root.join("no-item-catalog"),
            game.world.shape,
            None,
        ));
        game.enter_level(Dim::Overworld).unwrap();
        let error = game.evict_idle_level_chunks(dim).unwrap_err();
        assert!(error.to_string().contains("without a loaded item catalog"));
        assert_eq!(game.dim, Dim::Overworld);
        game.enter_level(dim).unwrap();
        assert_eq!(game.world.block(1, 64, 1), stone);
        assert!(game.world.modified.contains(&(0, 0)));
        assert_eq!(game.mobs.len(), 1);
        game.server.save.0 = save.clone();
        game.enter_level(Dim::Overworld).unwrap();

        // After a successful submission, only the chunks and entities leave.
        game.evict_idle_level_chunks(dim).unwrap();
        assert_eq!(
            save.load(
                0,
                0,
                registry,
                game.world_biomes
                    .as_ref()
                    .unwrap_or_else(|| game.res.biomes())
            )
            .unwrap()
            .block(1, 64, 1),
            stone
        );
        assert_eq!(save.load_entities(0, 0).unwrap().len(), 1);
        assert!(game.server.levels.0.parked[&dim].world.chunks.is_empty());
        assert!(seen.unloads.lock().unwrap().is_empty());
        assert!(game.runtime_level_is_open("idle:a"));
        assert!(game.level_handle_current(handle));
        assert!(game.server.levels.0.parked.contains_key(&dim));
        game.enter_level(dim).unwrap();
        assert_eq!(*seen.loads.lock().unwrap(), ["idle:a"]);
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(unload_as(game, "idle", "idle:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("idle:a"));
        assert!(Dim::extra("idle:a").is_none());
        assert_eq!(*seen.unloads.lock().unwrap(), ["idle:a"]);
    });
}

#[test]
#[ignore = "daily"]
fn an_idle_close_with_outstanding_light_saves_the_column_unlit() {
    with_opener("lit", "lit:a", flat(), |game, _| {
        tick_post(game);
        game.apply_runtime_levels();
        place_block(game, "lit:a", "minecraft:stone");
        let dim = Dim::extra("lit:a").unwrap();
        game.enter_level(dim).unwrap();
        Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap()).lit = true;
        game.world.dirty_light.insert((0, 0));
        game.world.modified.insert((0, 0));
        game.enter_level(Dim::Overworld).unwrap();
        game.close_parked_level(dim).unwrap();
        game.enter_level(dim).unwrap();
        game.server.streaming.forced.insert((0, 0));
        for _ in 0..2000 {
            game.server_update_world();
            if game.world.chunks.contains_key(&(0, 0)) {
                break;
            }
            game.server
                .streaming
                .wait(std::time::Duration::from_millis(5));
        }
        let reopened = game.world.chunks.get(&(0, 0)).expect("the chunk reopened");
        assert!(!reopened.lit, "the saved column was marked lit");
        game.enter_level(Dim::Overworld).unwrap();
    });
}

#[test]
#[ignore = "daily"]
fn an_idle_level_aborts_an_abandoned_chunk_unload_transaction_before_it_closes() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::runtime_levels_tests::an_idle_level_aborts_an_abandoned_chunk_unload_transaction_before_it_closes",
        || {
            const ID: &str = "abandon:a";
            const POS: [i32; 3] = [1, -50, 1];

            struct Abandon(Seen<bool>);

            #[mod_api::__subscriptions]
            impl Hooks for Abandon {
                fn level_tick_post(&self, host: HostSide, _: &mut LevelTickPostEvent) {
                    if let HostSide::Server(host) = host {
                        host.runtime_levels().open(ID, flat()).unwrap();
                    }
                }

                fn chunk_unload(
                    &self,
                    host: &mut dyn ServerHost,
                    event: &mut ChunkUnloadEvent<'_>,
                ) {
                    if event.dimension == ID && event.chunk == [0, 0] {
                        let level = host.level(ID).unwrap();
                        assert_eq!(level.set_connection_group(host, POS, Some(30)), Ok(true));
                        self.0.lock().unwrap().push(!host.transaction_matches(None));
                    }
                }

                fn level_unload(
                    &self,
                    host: &mut dyn ServerHost,
                    event: &mut LevelUnloadEvent<'_>,
                ) {
                    if event.dimension == ID {
                        self.0.lock().unwrap().push(!host.transaction_matches(None));
                    }
                }
            }

            let open_frames = Seen::default();
            let mods = vec![ModEntry::new(
                "abandon",
                Box::new(Abandon(Arc::clone(&open_frames))),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_game(|game| {
                    let root = TempRoot::new("abandon");
                    game.server.world_root = root.0.clone();
                    tick_post(game);
                    game.apply_runtime_levels();
                    place_block(game, ID, "minecraft:stone");
                    let dim = Dim::extra(ID).unwrap();
                    let custom = game.level(ID).unwrap();
                    let home = game.level("minecraft:overworld").unwrap();
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                    assert_eq!(home.set_connection_group(game, POS, Some(10)), Ok(true));
                    game.enter_level(dim).unwrap();
                    assert_eq!(custom.set_connection_group(game, POS, Some(20)), Ok(true));
                    // Simulate an engine-abandoned transaction left open in `dim` before close.
                    let _ = game.open_storage_transaction();
                    assert_eq!(custom.set_connection_group(game, POS, Some(30)), Ok(true));
                    game.enter_level(Dim::Overworld).unwrap();
                    assert!(!game.transaction_matches(None));

                    game.close_parked_level(dim).unwrap();
                    assert!(!game.server.levels.0.parked.contains_key(&dim));
                    // `chunk_unload` runs after the abandoned frame is aborted; `level_unload` runs in its own frame.
                    assert_eq!(*open_frames.lock().unwrap(), [false, true]);
                    assert!(game.transaction_matches(None));
                    assert_eq!(home.connection_group(game, POS), Ok(Some(10)));
                    game.abort_abandoned_transactions(0);
                    assert_eq!(home.connection_group(game, POS), Ok(Some(10)));
                });
            });
        },
    );
}

/// A chunk-unload listener can reactivate a level before its turn in the idle eviction sweep.
#[test]
#[ignore = "daily"]
fn a_ticket_a_chunk_unload_listener_adds_keeps_an_idle_custom_level_open() {
    use sunrise_gameplay_common::game::test_fixtures::run_fresh_cases;
    use sunrise_level::chunk::Chunk;

    if run_fresh_cases(
        "cut_layout::game::server_side::runtime_levels_tests::a_ticket_a_chunk_unload_listener_adds_keeps_an_idle_custom_level_open",
        "SUNRISE_RUNTIME_LEVEL_RESCUE_CHILD",
        &["rescue"],
    ) {
        return;
    }
    with_opener("rescue", "rescue:a", flat(), |game, seen| {
        game.native_level_opened(Dim::Overworld);
        game.enter_level(Dim::Nether).unwrap();
        game.common.world.chunks.insert(
            (0, 0),
            Arc::new(Chunk::with_layout(0, 0, game.common.world.layout())),
        );
        let nether = ChunkRef::new(game.level("minecraft:the_nether").unwrap(), [0, 0]);
        game.enter_level(Dim::Overworld).unwrap();

        tick_post(game);
        game.apply_runtime_levels();
        let dim = Dim::extra("rescue:a").unwrap();
        game.enter_level(dim).unwrap();
        // Record a forced ticket without unrelated generation. The distant column would
        // leave if a stale eviction plan swept the reactivated level anyway.
        game.server.streaming.paused = true;
        game.common.world.chunks.insert(
            (16, 0),
            Arc::new(Chunk::with_layout(16, 0, game.common.world.layout())),
        );
        let level = game.level("rescue:a").unwrap();
        let forced = ChunkRef::new(level, [0, 0]);
        let distant = ChunkRef::new(level, [16, 0]);
        game.enter_level(Dim::Overworld).unwrap();

        seen.mode.store(RESCUE, Ordering::SeqCst);
        game.evict_idle_levels().unwrap();
        assert!(
            !game.chunk_loaded(nether),
            "the first idle level evicted immediately"
        );
        assert_eq!(*seen.rescued.lock().unwrap(), [true]);
        assert_eq!(game.chunk_forced(forced), Some(true));
        assert!(
            game.chunk_loaded(distant),
            "the later level was reactivated before its sweep"
        );

        assert!(game.chunk_set_forced(forced, false));
        game.evict_idle_levels().unwrap();
        assert!(
            !game.chunk_loaded(distant),
            "the level can evict again once unticketed"
        );
        assert_eq!(game.level("minecraft:the_nether"), Some(nether.level()));
        assert_eq!(game.level("rescue:a"), Some(level));
        assert!(seen.unloads.lock().unwrap().is_empty());
    });
}

/// A chunk whose generator panics (a mod's `build_terrain`) is `Failed` for `load_chunk`, which
/// returns instead of waiting forever, and again at once when asked again: the engine contains
/// the panic where vanilla would crash. The level stays open and generates elsewhere.
#[test]
#[ignore = "daily"]
fn load_chunk_fails_where_a_mods_generator_panics() {
    use mod_api::ChunkLoadError;
    use sunrise_gameplay_common::game::test_fixtures::run_fresh_cases;
    use sunrise_world::worldgen::mod_generators::{
        ChunkAccess, ModChunkGenerator, ModChunkGenerators, install_mod_chunk_generators,
    };

    // The mods' chunk generators are a process-wide table: a child process of its own.
    if run_fresh_cases(
        "cut_layout::game::server_side::runtime_levels_tests::load_chunk_fails_where_a_mods_generator_panics",
        "SUNRISE_RUNTIME_LEVEL_FAILED_CHUNK_CHILD",
        &["failed_chunk"],
    ) {
        return;
    }
    struct Breaks;
    impl ModChunkGenerator for Breaks {
        fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
            let pos = chunk.pos();
            assert!((pos.x, pos.z) != (0, 0), "the generator breaks at [0, 0]");
        }
    }
    let breaks: Arc<dyn ModChunkGenerator> = Arc::new(Breaks);
    install_mod_chunk_generators(ModChunkGenerators::from([(
        mod_api::Identifier::parse("broken:terrain").unwrap(),
        breaks,
    )]))
    .map_err(|_| "installed twice")
    .unwrap();
    let level = RuntimeLevel::new(
        "minecraft:overworld",
        RuntimeGenerator::Mod("broken:terrain".into()),
        1,
    );
    with_opener("broken", "broken:a", level, |game, _| {
        tick_post(game);
        game.apply_runtime_levels();
        let level = game.level("broken:a").unwrap();
        let host: &mut dyn ServerHost = game;
        assert_eq!(level.load_chunk(host, [0, 0]), Err(ChunkLoadError::Failed));
        assert_eq!(
            level.load_chunk(host, [0, 0]),
            Err(ChunkLoadError::Failed),
            "at once when asked again"
        );
        assert_eq!(
            level.load_chunk(host, [1, 0]),
            Err(ChunkLoadError::Failed),
            "a neighbour whose wait needs the failed column generated"
        );
        assert_eq!(
            level.load_chunk(host, [20, 20]),
            Ok(()),
            "the level generates elsewhere"
        );
        assert!(
            game.native_level_world(level).is_some(),
            "the level stays open"
        );
    });
}

/// A live `UNKNOWN` ticket prevents chunk eviction until expiry and leaves the level idle.
#[test]
#[ignore = "daily"]
fn a_live_loading_ticket_keeps_an_idle_custom_level_open_until_it_expires() {
    use sunrise_gameplay_common::game::test_fixtures::run_fresh_cases;
    use sunrise_level::chunk::Chunk;

    if run_fresh_cases(
        "cut_layout::game::server_side::runtime_levels_tests::a_live_loading_ticket_keeps_an_idle_custom_level_open_until_it_expires",
        "SUNRISE_RUNTIME_LEVEL_LOADING_TICKET_CHILD",
        &["loading_ticket"],
    ) {
        return;
    }
    with_opener("loading", "loading:a", flat(), |game, seen| {
        tick_post(game);
        game.apply_runtime_levels();
        seen.mode.store(IDLE, Ordering::SeqCst);
        let dim = Dim::extra("loading:a").unwrap();
        let level = game.level("loading:a").unwrap();
        let chunk = ChunkRef::new(level, [5, 5]);
        assert!(!game.chunk_loaded(chunk));
        let host: &mut dyn ServerHost = game;
        assert_eq!(level.load_chunk(host, [5, 5]), Ok(()));

        game.enter_level(dim).unwrap();
        game.common.world.chunks.insert(
            (16, 0),
            Arc::new(Chunk::with_layout(16, 0, game.common.world.layout())),
        );
        game.enter_level(Dim::Overworld).unwrap();
        let distant = ChunkRef::new(level, [16, 0]);
        game.tick_server(false).unwrap();
        assert!(game.chunk_loaded(chunk));
        assert!(game.chunk_loaded(distant));
        assert!(
            !game.levels_to_tick().contains(&dim),
            "the ticket does not make the level tick"
        );

        game.tick_server(false).unwrap();
        assert!(
            !game.chunk_loaded(distant),
            "the expired ticket allows idle eviction"
        );
        assert_eq!(game.level("loading:a"), Some(level));
        assert!(seen.unloads.lock().unwrap().is_empty());
    });
}

/// An unload whose save fails leaves the level open and in place: the mod can unload it again,
/// and that unload goes through once the save can be written.
#[test]
#[ignore = "daily"]
fn a_failed_unload_leaves_the_level_open_for_a_retry() {
    with_opener("stuck", "stuck:a", flat(), |game, _| {
        tick_post(game);
        game.apply_runtime_levels();
        let dim = Dim::extra("stuck:a").unwrap();
        // A broken carrier transfer refuses every save of the level (`admit_level_save`).
        game.enter_level(dim).unwrap();
        game.world.carrier_fences.mark_broken();
        assert_eq!(unload_as(game, "stuck", "stuck:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(game.runtime_level_is_open("stuck:a"));
        assert!(Dim::extra("stuck:a").is_some());
        assert_eq!(game.dim, dim);
        // The save can be written again: the same unload, asked again, goes through.
        game.world.carrier_fences = Default::default();
        assert_eq!(unload_as(game, "stuck", "stuck:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("stuck:a"));
        assert!(Dim::extra("stuck:a").is_none());
    });
}

/// A save a mod asked for, then an unload in the same tick: the save leaves with the level, so
/// the next tick does not open it again.
#[test]
#[ignore = "daily"]
fn an_unload_drops_a_save_waiting_for_its_level() {
    with_opener("saved", "saved:a", flat(), |game, seen| {
        tick_post(game);
        game.apply_runtime_levels();
        let dim = Dim::extra("saved:a").unwrap();
        game.server.levels.0.request_save(dim);
        assert_eq!(unload_as(game, "saved", "saved:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!game.server.levels.0.save_pending(dim));
        game.run_requested_saves();
        assert!(!game.runtime_level_is_open("saved:a"));
        assert!(!game.server.levels.0.parked.contains_key(&dim));
        assert_eq!(*seen.loads.lock().unwrap(), ["saved:a"]);
    });
}

#[test]
#[ignore = "daily"]
fn a_level_with_a_player_in_it_is_not_unloaded() {
    with_opener("crowd", "crowd:a", flat(), |game, seen| {
        tick_post(game);
        game.apply_runtime_levels();
        let dim = Dim::extra("crowd:a").unwrap();
        game.enter_level(dim).unwrap();
        // The player stands in the level in place.
        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::new(0.5, 80.0, 0.5),
            )
            .unwrap(),
        );
        assert_eq!(
            unload_as(game, "crowd", "crowd:a"),
            Err(RuntimeLevelError::PlayersInside)
        );
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(unload_as(game, "crowd", "crowd:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("crowd:a"));
        assert_eq!(*seen.unloads.lock().unwrap(), ["crowd:a"]);
    });
}

#[test]
#[ignore = "daily"]
fn a_reopened_level_comes_back_from_its_folder_with_its_block() {
    with_opener("keep", "keep:a", flat(), |game, seen| {
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        tick_post(game);
        game.apply_runtime_levels();
        let dim = Dim::extra("keep:a").unwrap();
        game.enter_level(dim).unwrap();
        game.server.streaming.forced.insert((0, 0));
        for _ in 0..2000 {
            game.side_update_world();
            if game.world.chunks.contains_key(&(0, 0)) {
                break;
            }
            game.server
                .streaming
                .wait(std::time::Duration::from_millis(5));
        }
        assert!(
            game.world.chunks.contains_key(&(0, 0)),
            "the chunk generated"
        );
        game.world.set(1, -50, 1, stone).unwrap();
        game.enter_level(Dim::Overworld).unwrap();
        unload_as(game, "keep", "keep:a").unwrap();
        game.apply_runtime_levels();
        assert!(
            game.server.world_root.join("dimensions/keep/a").is_dir(),
            "the folder stays"
        );

        seen.answers.lock().unwrap().clear();
        tick_post(game);
        game.apply_runtime_levels();
        let dim = Dim::extra("keep:a").unwrap();
        game.enter_level(dim).unwrap();
        game.server.streaming.forced.insert((0, 0));
        for _ in 0..2000 {
            game.side_update_world();
            if game.world.chunks.contains_key(&(0, 0)) {
                break;
            }
            game.server
                .streaming
                .wait(std::time::Duration::from_millis(5));
        }
        assert_eq!(game.world.block(1, -50, 1), stone);
        game.enter_level(Dim::Overworld).unwrap();
        unload_as(game, "keep", "keep:a").unwrap();
        game.apply_runtime_levels();
    });
}

#[test]
#[ignore = "daily"]
fn failed_construction_rolls_back_every_published_table() {
    with_opener("rollback", "rollback:a", flat(), |game, seen| {
        tick_post(game);
        let first = seen.answers.lock().unwrap()[0].unwrap();
        game.server.levels.0.runtime.fail_open_after_publication = true;
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("rollback:a"));
        assert!(!game.server.levels.0.runtime.holds("rollback:a"));
        assert!(Dim::extra("rollback:a").is_none());
        assert!(
            !game
                .server
                .level_shapes
                .extra
                .iter()
                .any(|level| level.id == "rollback:a")
        );
        assert!(seen.loads.lock().unwrap().is_empty());
        assert!(!game.level_handle_current(first));
        game.server.levels.0.runtime.fail_open_after_publication = false;
        seen.answers.lock().unwrap().clear();
        tick_post(game);
        game.apply_runtime_levels();
        assert!(game.runtime_level_is_open("rollback:a"));
        assert_ne!(seen.answers.lock().unwrap()[0].unwrap(), first);
        unload_as(game, "rollback", "rollback:a").unwrap();
        game.apply_runtime_levels();
    });
}

/// Deleting an open, empty level closes it, then removes its folder and the structure references
/// kept beside the world, and the folders they leave empty.
#[test]
#[ignore = "daily"]
fn a_delete_closes_the_level_and_removes_its_folder_and_references_file() {
    with_opener("erase", "erase:instance/ab", flat(), |game, seen| {
        tick_post(game);
        game.apply_runtime_levels();
        place_block(game, "erase:instance/ab", "minecraft:stone");
        let folder = game.server.world_root.join("dimensions/erase/instance/ab");
        let references = game
            .server
            .world_root
            .join("data/sunrise_structure_references_erase_instance/ab.json");
        write_file(&references);
        // The save of the unload writes the folder; a stray file is in it too.
        write_file(&folder.join("data/minecraft/raids.dat"));
        assert_eq!(delete_as(game, "another_mod", "erase:instance/ab"), Ok(()));
        assert!(
            game.runtime_level_is_open("erase:instance/ab"),
            "open until the delete is applied"
        );
        assert!(folder.is_dir() && references.is_file());
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("erase:instance/ab"));
        assert!(!folder.exists(), "the folder is gone");
        assert!(!references.exists(), "the references file is gone");
        assert!(
            !game
                .server
                .world_root
                .join("data/sunrise_structure_references_erase_instance")
                .exists(),
            "so is the folder it left empty"
        );
        assert!(!game.server.world_root.join("dimensions/erase").exists());
        assert!(game.server.world_root.join("dimensions").is_dir());
        assert!(game.server.world_root.join("data").is_dir());
        assert_eq!(*seen.unloads.lock().unwrap(), ["erase:instance/ab"]);
    });
}

/// A level that is not open, in the mod's namespace, is only files: what a crash left.
#[test]
#[ignore = "daily"]
fn a_delete_of_a_level_that_is_not_open_removes_its_leftovers() {
    with_opener("left", "left:room", flat(), |game, seen| {
        let folder = game.server.world_root.join("dimensions/left/room");
        let sibling = game.server.world_root.join("dimensions/left/other");
        let references = game
            .server
            .world_root
            .join("data/sunrise_structure_references_left_room.json");
        write_file(&folder.join("region/r.0.0.mca"));
        write_file(&sibling.join("region/r.0.0.mca"));
        write_file(&references);
        assert!(!game.runtime_level_is_open("left:room"));
        assert_eq!(delete_as(game, "left", "left:room"), Ok(()));
        assert!(
            folder.is_dir(),
            "nothing is removed before the queue is applied"
        );
        game.apply_runtime_levels();
        assert!(!folder.exists() && !references.exists());
        assert!(
            sibling.join("region/r.0.0.mca").is_file(),
            "another level of the namespace stays"
        );
        // Nothing left to remove is nothing to refuse.
        assert_eq!(delete_as(game, "left", "left:room"), Ok(()));
        game.apply_runtime_levels();
        assert!(seen.unloads.lock().unwrap().is_empty(), "it was never open");
    });
}

/// The folder of another mod's level, of a vanilla level and of a path that leaves the world is
/// never touched.
#[test]
#[ignore = "daily"]
fn a_delete_of_reserved_or_invalid_ids_is_refused_and_removes_nothing() {
    with_opener("mine", "mine:a", flat(), |game, _| {
        let vanilla = game
            .server
            .world_root
            .join("dimensions/minecraft/overworld/region/r.0.0.mca");
        let escape = game.server.world_root.join("dimensions/mine/x");
        write_file(&vanilla);
        write_file(&escape.join("keep.txt"));
        for (id, expected) in [
            ("minecraft:overworld", RuntimeLevelError::ReservedId),
            ("mine:../other/x", RuntimeLevelError::InvalidId),
            ("mine:x/../../other/x", RuntimeLevelError::InvalidId),
            ("mine:con", RuntimeLevelError::InvalidId),
        ] {
            assert_eq!(delete_as(game, "mine", id), Err(expected), "{id}");
        }
        game.apply_runtime_levels();
        assert!(vanilla.is_file() && escape.join("keep.txt").is_file());
    });
}

/// A level deleted and opened again starts empty: the block placed before is gone.
#[test]
#[ignore = "daily"]
fn a_level_opened_again_after_a_delete_starts_empty() {
    with_opener("fresh", "fresh:a", flat(), |game, seen| {
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        tick_post(game);
        game.apply_runtime_levels();
        let first = seen.answers.lock().unwrap()[0].unwrap();
        place_block(game, "fresh:a", "minecraft:stone");
        assert_eq!(block_at(game, "fresh:a"), stone);
        assert_eq!(delete_as(game, "fresh", "fresh:a"), Ok(()));
        // The open waits for the delete to be applied.
        seen.answers.lock().unwrap().clear();
        tick_post(game);
        assert_eq!(
            seen.answers.lock().unwrap()[0],
            Err(RuntimeLevelError::AlreadyOpen)
        );
        game.apply_runtime_levels();
        assert!(!game.server.world_root.join("dimensions/fresh/a").exists());
        seen.answers.lock().unwrap().clear();
        tick_post(game);
        game.apply_runtime_levels();
        let second = seen.answers.lock().unwrap()[0].unwrap();
        assert_ne!(first, second);
        assert!(!game.level_handle_current(first));
        let air = game
            .pack
            .assets
            .registry
            .default_state("minecraft:air")
            .unwrap();
        assert_eq!(block_at(game, "fresh:a"), air, "the stone is not there");
        delete_as(game, "fresh", "fresh:a").unwrap();
        game.apply_runtime_levels();
    });
}

/// An open made after the delete of a level that is not open is applied after it: the leftovers
/// are gone before the level opens.
#[test]
#[ignore = "daily"]
fn an_open_queued_behind_a_delete_of_leftovers_finds_no_folder() {
    with_opener("behind", "behind:a", flat(), |game, seen| {
        let leftover = game
            .server
            .world_root
            .join("dimensions/behind/a/region/r.0.0.mca");
        write_file(&leftover);
        assert_eq!(delete_as(game, "behind", "behind:a"), Ok(()));
        tick_post(game);
        assert!(seen.answers.lock().unwrap()[0].is_ok());
        game.apply_runtime_levels();
        assert!(game.runtime_level_is_open("behind:a"));
        assert!(!leftover.exists());
        delete_as(game, "behind", "behind:a").unwrap();
        game.apply_runtime_levels();
    });
}

#[test]
#[ignore = "daily"]
fn a_level_with_a_player_in_it_is_not_deleted() {
    with_opener("guest", "guest:a", flat(), |game, _| {
        tick_post(game);
        game.apply_runtime_levels();
        let dim = Dim::extra("guest:a").unwrap();
        game.enter_level(dim).unwrap();
        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::new(0.5, 80.0, 0.5),
            )
            .unwrap(),
        );
        let marker = game.server.world_root.join("dimensions/guest/a/marker.txt");
        write_file(&marker);
        assert_eq!(
            delete_as(game, "guest", "guest:a"),
            Err(RuntimeLevelError::PlayersInside)
        );
        game.apply_runtime_levels();
        assert!(game.runtime_level_is_open("guest:a"));
        assert!(marker.is_file());
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(delete_as(game, "guest", "guest:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!marker.exists());
        assert!(!game.runtime_level_is_open("guest:a"));
    });
}

/// A delete asked inside a level's tick leaves the level and its files as they are until the
/// queue is applied after the levels.
#[test]
#[ignore = "daily"]
fn a_delete_is_applied_after_the_levels_and_not_inside_a_tick() {
    with_opener("later", "later:a", flat(), |game, seen| {
        tick_post(game);
        game.apply_runtime_levels();
        let marker = game.server.world_root.join("dimensions/later/a/marker.txt");
        write_file(&marker);
        seen.open_right_after.lock().unwrap().clear();
        seen.mode.store(DELETE, Ordering::SeqCst);
        tick_post(game);
        assert_eq!(*seen.deletes.lock().unwrap(), [Ok(())]);
        assert_eq!(*seen.open_right_after.lock().unwrap(), [true]);
        assert!(marker.is_file(), "nothing is removed inside the tick");
        assert!(seen.unloads.lock().unwrap().is_empty());
        seen.mode.store(IDLE, Ordering::SeqCst);
        game.tick_server(false).unwrap();
        assert!(!marker.exists());
        assert!(!game.runtime_level_is_open("later:a"));
        assert_eq!(*seen.unloads.lock().unwrap(), ["later:a"]);
    });
}

/// A delete of a level whose open is still waiting is refused: its open comes first.
#[test]
#[ignore = "daily"]
fn a_delete_of_a_level_still_waiting_to_open_is_refused() {
    with_opener("wait", "wait:a", flat(), |game, _| {
        tick_post(game);
        assert_eq!(
            delete_as(game, "wait", "wait:a"),
            Err(RuntimeLevelError::AlreadyOpen)
        );
        game.apply_runtime_levels();
        assert!(game.runtime_level_is_open("wait:a"));
        delete_as(game, "wait", "wait:a").unwrap();
        game.apply_runtime_levels();
    });
}

#[test]
fn a_delete_marks_an_open_level_closing_and_a_second_unload_finds_it_not_open() {
    let mut table = RuntimeTable::default();
    table.queue_open("test", "del:a", admitted_flat(1)).unwrap();
    assert_eq!(
        table.queue_delete("test", "del:a"),
        Err(RuntimeLevelError::AlreadyOpen)
    );
    table.mark_open("del:a");
    assert_eq!(table.queue_delete("test", "del:a"), Ok(()));
    assert!(table.is_open("del:a"));
    assert_eq!(
        table.queue_unload("test", "del:a"),
        Err(RuntimeLevelError::NotOpen)
    );
    assert_eq!(
        table.queue_open("test", "del:a", admitted_flat(1)),
        Err(RuntimeLevelError::AlreadyOpen),
        "no reopen until the delete is applied"
    );
    // A level the table does not hold has only files to delete.
    assert_eq!(table.queue_delete("test", "del:none"), Ok(()));
    assert!(!table.holds("del:none"));
}

/// The folders of levels nest: a delete of a level with a level below it, open or only a folder,
/// is refused and removes nothing of either.
#[test]
#[ignore = "daily"]
fn a_delete_of_a_level_with_a_level_below_it_is_refused() {
    with_opener("kid", "kid:instance/ab", flat(), |game, _| {
        let parent_file = game
            .server
            .world_root
            .join("dimensions/kid/instance/region/r.0.0.mca");
        write_file(&parent_file);
        tick_post(game);
        // Waiting to open, then open: the table knows the level below.
        for _ in 0..2 {
            assert_eq!(
                delete_as(game, "kid", "kid:instance"),
                Err(RuntimeLevelError::LevelsBelow)
            );
            game.apply_runtime_levels();
        }
        assert!(game.runtime_level_is_open("kid:instance/ab"));
        place_block(game, "kid:instance/ab", "minecraft:stone");
        assert!(parent_file.is_file());
        // The child goes: the parent's own files stay, and the parent can go next.
        assert_eq!(delete_as(game, "kid", "kid:instance/ab"), Ok(()));
        game.apply_runtime_levels();
        assert!(
            !game
                .server
                .world_root
                .join("dimensions/kid/instance/ab")
                .exists()
        );
        assert!(
            parent_file.is_file(),
            "a level's own files are not the parent's"
        );
        assert_eq!(delete_as(game, "kid", "kid:instance"), Ok(()));
        game.apply_runtime_levels();
        assert!(
            !game
                .server
                .world_root
                .join("dimensions/kid/instance")
                .exists()
        );
    });
}

/// A level below that only exists as a folder, as after a crash, stops the delete too, even when
/// its folder has the name of one a level writes.
#[test]
#[ignore = "daily"]
fn a_delete_stops_at_a_folder_that_holds_a_level() {
    with_opener("fold", "fold:unused", flat(), |game, _| {
        let child = game
            .server
            .world_root
            .join("dimensions/fold/p/ab/region/r.0.0.mca");
        // `fold:q/region` is a level too, and its folder is where `fold:q` keeps its regions.
        let named_region = game
            .server
            .world_root
            .join("dimensions/fold/q/region/data/minecraft/raids.dat");
        let own = game
            .server
            .world_root
            .join("dimensions/fold/p/region/r.0.0.mca");
        write_file(&child);
        write_file(&own);
        write_file(&named_region);
        // `fold:r/data/ab` is a level that only exists as a folder under `data`.
        let in_data = game
            .server
            .world_root
            .join("dimensions/fold/r/data/ab/region/r.0.0.mca");
        let own_data = game
            .server
            .world_root
            .join("dimensions/fold/r/data/minecraft/raids.dat");
        write_file(&in_data);
        write_file(&own_data);
        for id in ["fold:p", "fold:q", "fold:r"] {
            assert_eq!(
                delete_as(game, "fold", id),
                Err(RuntimeLevelError::LevelsBelow),
                "{id}"
            );
        }
        game.apply_runtime_levels();
        assert!(child.is_file() && own.is_file() && named_region.is_file() && in_data.is_file());
        // Only the level itself: `fold:p/ab` has no folder below it.
        assert_eq!(delete_as(game, "fold", "fold:p/ab"), Ok(()));
        game.apply_runtime_levels();
        assert!(!child.exists() && own.is_file());
        assert_eq!(delete_as(game, "fold", "fold:p"), Ok(()));
        game.apply_runtime_levels();
        assert!(!own.exists());
        // What `data` holds besides a level goes with its level.
        assert_eq!(delete_as(game, "fold", "fold:r/data/ab"), Ok(()));
        game.apply_runtime_levels();
        assert_eq!(delete_as(game, "fold", "fold:r"), Ok(()));
        game.apply_runtime_levels();
        assert!(!own_data.exists() && !in_data.exists());
    });
}

/// Two ids can name one structure references file (`a_b:c`, `a:b_c`): a delete of one keeps the
/// file while the other exists, and removes it once the other is gone.
#[test]
#[ignore = "daily"]
fn a_delete_keeps_a_references_file_another_level_shares() {
    with_opener("a_b", "a_b:c", flat(), |game, _| {
        let mine = game
            .server
            .world_root
            .join("dimensions/a_b/c/region/r.0.0.mca");
        let other = game
            .server
            .world_root
            .join("dimensions/a/b_c/region/r.0.0.mca");
        let shared = game
            .server
            .world_root
            .join("data/sunrise_structure_references_a_b_c.json");
        write_file(&mine);
        write_file(&other);
        write_file(&shared);
        assert_eq!(delete_as(game, "a_b", "a_b:c"), Ok(()));
        game.apply_runtime_levels();
        assert!(!mine.exists());
        assert!(
            other.is_file(),
            "the other namespace's level is not touched"
        );
        assert!(shared.is_file(), "its references file is kept");
        std::fs::remove_dir_all(game.server.world_root.join("dimensions/a")).unwrap();
        assert_eq!(delete_as(game, "a_b", "a_b:c"), Ok(()));
        game.apply_runtime_levels();
        assert!(!shared.exists(), "nothing else shares it now");
    });
}

/// A removal that fails leaves the level closed, its id kept from `open` until a `delete` has
/// removed the files.
#[test]
#[ignore = "daily"]
fn a_failed_removal_leaves_the_level_closed_and_its_id_closed_to_open() {
    with_opener("stuck2", "stuck2:a", flat(), |game, seen| {
        tick_post(game);
        game.apply_runtime_levels();
        let first = seen.answers.lock().unwrap()[0].unwrap();
        let marker = game
            .server
            .world_root
            .join("dimensions/stuck2/a/data/marker.dat");
        write_file(&marker);
        game.server.levels.0.runtime.fail_erase = true;
        assert_eq!(delete_as(game, "stuck2", "stuck2:a"), Ok(()));
        game.apply_runtime_levels();
        // Closed: not open, out of the world's levels, its handle stale, `level_unload` heard.
        assert!(!game.runtime_level_is_open("stuck2:a"));
        assert!(Dim::extra("stuck2:a").is_none());
        assert!(!game.level_handle_current(first));
        assert_eq!(*seen.unloads.lock().unwrap(), ["stuck2:a"]);
        assert!(marker.is_file(), "the files are left");
        // Its id cannot be opened while they are.
        seen.answers.lock().unwrap().clear();
        tick_post(game);
        assert_eq!(
            seen.answers.lock().unwrap()[0],
            Err(RuntimeLevelError::Removing)
        );
        assert_eq!(delete_as(game, "stuck2", "stuck2:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(marker.is_file(), "the removal failed again");
        // Once it can, a second delete finishes it and the id opens again, empty.
        game.server.levels.0.runtime.fail_erase = false;
        assert_eq!(delete_as(game, "stuck2", "stuck2:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!marker.exists());
        seen.answers.lock().unwrap().clear();
        tick_post(game);
        game.apply_runtime_levels();
        assert!(seen.answers.lock().unwrap()[0].is_ok());
        assert!(game.runtime_level_is_open("stuck2:a"));
        assert!(!marker.exists());
    });
}

/// An `open` made after the `delete` of leftovers is applied after it; if the removal failed it
/// is dropped, so the level never starts from the files that are left.
#[test]
#[ignore = "daily"]
fn an_open_queued_behind_a_failed_removal_is_dropped() {
    with_opener("behind2", "behind2:a", flat(), |game, seen| {
        let leftover = game
            .server
            .world_root
            .join("dimensions/behind2/a/region/r.0.0.mca");
        write_file(&leftover);
        game.server.levels.0.runtime.fail_erase = true;
        assert_eq!(delete_as(game, "behind2", "behind2:a"), Ok(()));
        tick_post(game);
        assert!(seen.answers.lock().unwrap()[0].is_ok());
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("behind2:a"));
        assert!(!game.server.levels.0.runtime.holds("behind2:a"));
        assert!(Dim::extra("behind2:a").is_none());
        assert!(leftover.is_file());
        assert!(seen.loads.lock().unwrap().is_empty());
    });
}

/// A symbolic link, or on Windows a junction, from `at` to the folder `to`. Panics if the machine
/// cannot make one: these tests are about links, and a skip would look like a pass.
fn make_link(to: &std::path::Path, at: &std::path::Path) {
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    // A symbolic link needs a privilege on Windows; a junction does not.
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(to, at).or_else(|_| {
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(at.to_string_lossy().replace('/', "\\"))
            .arg(to.to_string_lossy().replace('/', "\\"))
            .output()
            .map(|_| ())
            .and_then(|()| std::fs::symlink_metadata(at).map(|_| ()))
    });
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(to, at);
    made.expect("this machine cannot create a symbolic link or a junction");
}

/// Remove the link `link` made by `make_link`, not what it leads to: a folder link on Windows, a
/// file-like symbolic link on Unix.
fn remove_link(link: &std::path::Path) {
    #[cfg(windows)]
    std::fs::remove_dir(link).unwrap();
    #[cfg(not(windows))]
    std::fs::remove_file(link).unwrap();
}

/// A link under `dimensions` or `data`, or one of the two being a link, is never followed out of
/// the world, and nothing is removed first.
#[test]
#[ignore = "daily"]
fn a_removal_does_not_follow_links_out_of_the_world() {
    let root = TempRoot::new("links");
    let outside = root.0.join("outside");
    let victim = outside.join("region/r.0.0.mca");
    write_file(&victim);
    let erase = |world: &std::path::Path, id: &str| {
        sunrise_gameplay::game::server_side::runtime_levels::erase_level_files(world, id, true)
    };
    // The namespace folder is a link.
    let world = root.0.join("world");
    make_link(&outside, &world.join("dimensions/lnk"));
    assert!(erase(&world, "lnk:x").is_err());
    // The level's own folder is a link.
    remove_link(&world.join("dimensions/lnk"));
    make_link(&outside, &world.join("dimensions/ns/x"));
    assert!(erase(&world, "ns:x").is_err());
    // The folder of the references file is a link; nothing of the level's own is removed first.
    let own = world.join("dimensions/ns/y/region/r.0.0.mca");
    write_file(&own);
    write_file(&outside.join("ab.json"));
    make_link(
        &outside,
        &world.join("data/sunrise_structure_references_ns_y"),
    );
    assert!(erase(&world, "ns:y/ab").is_err());
    assert!(victim.is_file() && outside.join("ab.json").is_file() && own.is_file());
    // `dimensions` is a link.
    let by_dimensions = root.0.join("by_dimensions");
    write_file(&outside.join("ns/x/region/r.0.0.mca"));
    make_link(&outside, &by_dimensions.join("dimensions"));
    assert!(erase(&by_dimensions, "ns:x").is_err());
    assert!(outside.join("ns/x/region/r.0.0.mca").is_file());
    // `data` is a link, with the references file behind it.
    let by_data = root.0.join("by_data");
    let reference = outside.join("sunrise_structure_references_ns_z.json");
    write_file(&reference);
    make_link(&outside, &by_data.join("data"));
    assert!(erase(&by_data, "ns:z").is_err());
    assert!(reference.is_file());
}

/// Something that makes the removal of a file in a folder fail, until it is dropped: on Unix the
/// folder loses its write permission, on Windows the file is held open without sharing.
struct Lock {
    #[cfg(windows)]
    _file: std::fs::File,
    #[cfg(unix)]
    folder: std::path::PathBuf,
}

impl Lock {
    /// None when the machine does not enforce it (a process that is root ignores permissions).
    fn on(file: &std::path::Path) -> Option<Self> {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let held = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(file)
                .unwrap();
            Some(Self { _file: held })
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let folder = file.parent().unwrap().to_owned();
            std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
            let lock = Self { folder };
            let probe = lock.folder.join("probe");
            if std::fs::write(&probe, b"x").is_ok() {
                let _ = std::fs::remove_file(&probe);
                return None;
            }
            Some(lock)
        }
    }
}

#[cfg(unix)]
impl Drop for Lock {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.folder, std::fs::Permissions::from_mode(0o755));
    }
}

/// A removal that really fails part way, a file that cannot be removed: the level is closed and
/// its id is refused to `open`, the file is still there, and a `delete` asked again, once the
/// file can go, finishes the work.
#[test]
#[ignore = "daily"]
fn a_removal_that_fails_part_way_is_finished_by_a_second_delete() {
    with_opener("part", "part:a", flat(), |game, seen| {
        tick_post(game);
        game.apply_runtime_levels();
        let folder = game.server.world_root.join("dimensions/part/a");
        let locked = folder.join("data/locked/x.dat");
        write_file(&locked);
        write_file(&folder.join("poi/r.0.0.mca"));
        let Some(lock) = Lock::on(&locked) else {
            panic!(
                "partial-removal coverage requires enforced file permissions; run as non-root on Unix or run on Windows"
            );
        };
        assert_eq!(delete_as(game, "part", "part:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("part:a"));
        assert!(Dim::extra("part:a").is_none());
        assert_eq!(*seen.unloads.lock().unwrap(), ["part:a"]);
        assert!(
            locked.is_file(),
            "the file that could not go is still there"
        );
        seen.answers.lock().unwrap().clear();
        tick_post(game);
        assert_eq!(
            seen.answers.lock().unwrap()[0],
            Err(RuntimeLevelError::Removing)
        );
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("part:a"));
        // The file can go: the same delete, asked again, finishes.
        drop(lock);
        assert_eq!(delete_as(game, "part", "part:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!folder.exists());
        seen.answers.lock().unwrap().clear();
        tick_post(game);
        game.apply_runtime_levels();
        assert!(seen.answers.lock().unwrap()[0].is_ok());
        assert!(game.runtime_level_is_open("part:a"));
        assert!(!locked.exists(), "the level starts empty");
    });
}

/// Holds a scratch directory unreadable, restoring its original mode even on assertion failure.
#[cfg(unix)]
struct UnreadableFolder {
    folder: std::path::PathBuf,
    permissions: std::fs::Permissions,
}

#[cfg(unix)]
impl UnreadableFolder {
    fn on(folder: &std::path::Path) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let guard = Self {
            folder: folder.to_owned(),
            permissions: std::fs::metadata(folder).unwrap().permissions(),
        };
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o000)).unwrap();
        assert_eq!(
            std::fs::read_dir(folder).err().map(|error| error.kind()),
            Some(std::io::ErrorKind::PermissionDenied),
            "unreadable-folder coverage requires enforced permissions; run as non-root on Unix"
        );
        guard
    }
}

#[cfg(unix)]
impl Drop for UnreadableFolder {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.folder, self.permissions.clone());
    }
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn a_delete_refuses_unreadable_data_and_removes_nothing() {
    with_opener("blind", "blind:a", flat(), |game, _| {
        let folder = game.server.world_root.join("dimensions/blind/a");
        let marker = folder.join("data/private/marker.dat");
        let region = folder.join("region/r.0.0.mca");
        let references = game
            .server
            .world_root
            .join("data/sunrise_structure_references_blind_a.json");
        write_file(&marker);
        write_file(&region);
        write_file(&references);
        let guard = UnreadableFolder::on(marker.parent().unwrap());
        assert_eq!(
            delete_as(game, "blind", "blind:a"),
            Err(RuntimeLevelError::LevelsBelow)
        );
        game.apply_runtime_levels();
        drop(guard);
        assert!(marker.is_file() && region.is_file() && references.is_file());
        assert_eq!(delete_as(game, "blind", "blind:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!folder.exists() && !references.exists());
    });
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn a_delete_keeps_references_when_shared_level_metadata_is_unreadable() {
    with_opener("a_b", "a_b:c", flat(), |game, _| {
        let folder = game.server.world_root.join("dimensions/a_b/c");
        let references = game
            .server
            .world_root
            .join("data/sunrise_structure_references_a_b_c.json");
        let other_namespace = game.server.world_root.join("dimensions/a");
        write_file(&folder.join("region/r.0.0.mca"));
        write_file(&references);
        std::fs::create_dir_all(&other_namespace).unwrap();
        let guard = UnreadableFolder::on(&other_namespace);
        assert_eq!(
            std::fs::symlink_metadata(other_namespace.join("b_c"))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
        assert_eq!(delete_as(game, "a_b", "a_b:c"), Ok(()));
        game.apply_runtime_levels();
        assert!(!folder.exists(), "the requested level folder was removed");
        assert!(
            references.is_file(),
            "unknown sharing must preserve references"
        );
        drop(guard);
        assert!(!other_namespace.join("b_c").exists());
        assert_eq!(delete_as(game, "a_b", "a_b:c"), Ok(()));
        game.apply_runtime_levels();
        assert!(!folder.exists() && !references.exists());
    });
}

/// Single player: the integrated server and the window's client share the process's table of
/// levels. A runtime level the server opened, which the client entered and left, stays the
/// server's: it still unloads, and, opened again, entered and left again, still deletes.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_level_the_player_left_in_single_player_still_unloads_and_deletes() {
    use sunrise_protocol::packets::play::{self, Clientbound as C};
    let travel = |dimension: &str| {
        C::Respawn(play::Respawn {
            spawn: crate::game::client_side::viewed_worlds::tests::spawn(dimension),
            keep: 3,
        })
    };
    with_opener("solo", "solo:a", flat(), |game, _| {
        tick_post(game);
        game.apply_runtime_levels();
        assert!(game.runtime_level_is_open("solo:a"));
        let mut resources = None;
        let mut window = crate::game::client_side::viewed_worlds::tests::client(&mut resources);
        window.handle_packet(travel("solo:a")).unwrap();
        window.handle_packet(travel("minecraft:overworld")).unwrap();
        assert!(Dim::extra("solo:a").is_some(), "still the server's level");
        assert_eq!(unload_as(game, "solo", "solo:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("solo:a"));
        assert!(Dim::extra("solo:a").is_none(), "closed by the server");
        // Opened again by the mod, entered and left again, then deleted.
        tick_post(game);
        game.apply_runtime_levels();
        assert!(game.runtime_level_is_open("solo:a"));
        window.handle_packet(travel("solo:a")).unwrap();
        window.handle_packet(travel("minecraft:overworld")).unwrap();
        assert_eq!(delete_as(game, "solo", "solo:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("solo:a"));
        assert!(!game.server.world_root.join("dimensions/solo/a").exists());
    });
}

/// Opening and unloading one id again and again, with a new seed each time, leaks its names,
/// facts and level table once: every later open hands the game the same copies, and the level
/// reads the seed of the open it is in (#4019).
#[test]
#[ignore = "daily"]
fn reopening_one_id_with_any_seed_reuses_the_metadata_its_first_open_leaked() {
    with_opener("cycle", "cycle:a", flat(), |game, seen| {
        seen.mode.store(IDLE, Ordering::SeqCst);
        let key = sunrise_core::LevelKey::parse("cycle:a").unwrap();
        let mut first = None;
        for seed in [1, 2, 3, i64::MIN, i64::MAX] {
            let level = RuntimeLevel::new("minecraft:overworld", RuntimeGenerator::Flat, seed);
            game.runtime_level_open("cycle:a", level).unwrap();
            game.apply_runtime_levels();
            assert!(game.runtime_level_is_open("cycle:a"));
            assert_eq!(
                game.server.levels.0.runtime.seeds().get("cycle:a").copied(),
                Some(seed)
            );
            let table = game.server.level_shapes.extra.clone();
            let level = table.iter().find(|level| level.id == "cycle:a").unwrap();
            let facts = sunrise_world::dimension_type::DimensionFacts::of_level(key);
            let leaked = (
                level.id.as_ptr(),
                level.dimension_type.as_ptr(),
                std::ptr::from_ref(facts),
            );
            assert_eq!(*first.get_or_insert(leaked), leaked);
            unload_as(game, "cycle", "cycle:a").unwrap();
            game.apply_runtime_levels();
            assert!(!game.runtime_level_is_open("cycle:a"));
            assert_eq!(
                game.server.levels.0.runtime.seeds().get("cycle:a").copied(),
                None
            );
        }
    });
}

/// Churning level ids preserves publication order and bounds metadata retention (#1367).
#[test]
#[ignore = "daily"]
fn churning_runtime_level_metadata_stays_within_the_retention_bound() {
    use super::native_entity_refs::tests::run_fresh_cases;

    // Real opens publish their ids in process-wide tables.
    if run_fresh_cases(
        "cut_layout::game::server_side::runtime_levels_tests::churning_runtime_level_metadata_stays_within_the_retention_bound",
        "SUNRISE_RUNTIME_LEVEL_METADATA_CHILD",
        &["churn"],
    ) {
        return;
    }
    with_opener("cycle", "cycle:seed", flat(), |game, _| {
        for batch in 0..(RETAINED_TABLE_LIMIT * 2) {
            let left = format!("cycle:left-{batch}");
            let right = format!("cycle:right-{batch}");
            let left_type = match batch % 3 {
                0 => "minecraft:overworld",
                1 => "minecraft:the_nether",
                _ => "minecraft:the_end",
            };
            let right_type = match (batch + 1) % 3 {
                0 => "minecraft:overworld",
                1 => "minecraft:the_nether",
                _ => "minecraft:the_end",
            };
            let mut ordered = vec![(left.as_str(), left_type), (right.as_str(), right_type)];
            if batch % 2 == 1 {
                ordered.reverse();
            }
            for (index, (id, dimension_type)) in ordered.iter().enumerate() {
                let level = RuntimeLevel::new(
                    *dimension_type,
                    RuntimeGenerator::Flat,
                    (batch * 2 + index) as i64,
                );
                game.runtime_level_open(id, level).unwrap();
                game.apply_runtime_levels();
                assert!(game.runtime_level_is_open(id));
            }
            assert_eq!(
                game.server
                    .level_shapes
                    .extra
                    .iter()
                    .map(|level| level.id)
                    .collect::<Vec<_>>(),
                ordered.iter().map(|(id, _)| *id).collect::<Vec<_>>()
            );
            for (id, _) in ordered.into_iter().rev() {
                unload_as(game, "cycle", id).unwrap();
                game.apply_runtime_levels();
                assert!(!game.runtime_level_is_open(id));
            }
        }
    });

    let (types, tables) = retained_metadata_counts();
    assert!(
        types <= sunrise_world::worldgen::overrides::mod_dimensions().len() + 4,
        "runtime dimension facts grew beyond startup-known types: {types}"
    );
    assert!(
        tables <= RETAINED_TABLE_LIMIT,
        "runtime level tables grew beyond their retention bound: {tables}"
    );
}

use sunrise_gameplay_common::game::native_hooks::base::GameHooks as _;

use sunrise_gameplay_common::game::side_ops::events::EventsSideGameExt as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
