//! What a wasm mod costs against the same mod built native: `tests/bench/hook/` runs one chat hook
//! doing a chosen amount of work, compiled into this test and loaded as `mod.wasm`. Each case
//! reports the median time per call over batches, and the test checks both builds answered
//! alike. CI builds the mod first, points `HORIZON_WASM_BENCH` at its `.hmod` and
//! `HORIZON_WASM_STD` at the std member, runs this in release and adds the table to the job
//! summary; the test is ignored elsewhere. Guest projects are siblings under `bench/` so
//! packaging one never traverses the host tests or the other project.
use std::time::{Duration, Instant};

use mod_api::__private::game::Fields;
use mod_api::__private::wasm_host::{Budgets, Fuel};
use mod_api::{Hooks, Host, LevelRef, NativeWorldEpoch, ServerChatEvent, ServerHost, Text};

#[path = "bench/internals/src/allocations.rs"]
mod allocations;
#[path = "bench/hook/src/lib.rs"]
mod bench_mod;
#[path = "bench/internals.rs"]
mod internals_bench;

/// One overworld, and nothing else.
struct World;

impl mod_api::prelude::EntitiesHost for World {
    fn level(&self, dimension: &str) -> Option<LevelRef> {
        (dimension == "minecraft:overworld")
            .then(|| LevelRef::new(NativeWorldEpoch::new(1), mod_api::DimensionKey::OVERWORLD))
    }
    fn entity(&self, _id: u32) -> Option<mod_api::EntityRef> {
        None
    }
}
impl Host for World {}
mod_api::impl_host_areas!(World, except(entities));

impl mod_api::prelude::ConfigServerHost for World {
    fn tick_stats(&self) -> (f32, f64) {
        (20.0, 50.0)
    }
}
impl ServerHost for World {}
mod_api::impl_server_host_areas!(World, except(config));

/// One call of the hook `work` names: the server tick for `tick`, else the chat hook with
/// `work` as the message; what the mod wrote back.
fn chat(hooks: &dyn Hooks, work: &str) -> String {
    if work == "tick" {
        let context = mod_api::__private::built::NativeLifecycleContext {
            authority: mod_api::NativeAuthority::DedicatedServer,
            epoch: NativeWorldEpoch::new(1),
            tick: mod_api::NativeTickId::new(1),
        }
        .into();
        let mut event = mod_api::ServerTickEvent {
            context: &context,
            phase: mod_api::ServerTickPhase::ServerTickStart {
                world_running: true,
            },
        };
        hooks.server_tick(&mut World, &mut event);
        return String::new();
    }
    let mut message = Text::literal("");
    // `untouched`: a large writable field the mod leaves as it was.
    let mut replies: Vec<Text> = match work {
        "untouched" => (0..100)
            .map(|i| Text::literal(format!("reply {i}")))
            .collect(),
        _ => Vec::new(),
    };
    let mut event = ServerChatEvent {
        player_id: 1,
        player: 1,
        username: "bench",
        raw_text: work,
        message: &mut message,
        replies: &mut replies,
    };
    hooks.server_chat(&mut World, &mut event);
    message.plain()
}

/// The median time per call of `calls` calls, over 9 batches after one warm-up batch.
fn per_call(hooks: &dyn Hooks, work: &str, calls: u32) -> Duration {
    timed(calls, || {
        std::hint::black_box(chat(hooks, std::hint::black_box(work)));
    })
}

/// The median time of one `call`, over 9 batches of `calls` after one warm-up batch.
fn timed(calls: u32, mut call: impl FnMut()) -> Duration {
    let mut batches: Vec<f64> = (0..10)
        .map(|_| {
            let started = Instant::now();
            for _ in 0..calls {
                call();
            }
            started.elapsed().as_secs_f64() / f64::from(calls)
        })
        .skip(1)
        .collect();
    batches.sort_by(f64::total_cmp);
    Duration::from_secs_f64(batches[batches.len() / 2])
}

fn shown(time: Duration) -> String {
    match time.as_nanos() {
        0 => "under 1 ns".to_owned(),
        n if n < 10_000 => format!("{n} ns"),
        n if n < 10_000_000 => format!("{:.1} µs", n as f64 / 1e3),
        n => format!("{:.1} ms", n as f64 / 1e6),
    }
}

/// The bench mod loaded with `budgets`.
fn load(budgets: Budgets) -> horizon_mod_wasm::Loaded {
    let path = |var: &str| std::path::PathBuf::from(std::env::var(var).expect(var));
    let package = path("HORIZON_WASM_BENCH");
    let std_member = std::fs::read(path("HORIZON_WASM_STD")).unwrap();
    let loaded = horizon_mod_wasm::load(
        &horizon_mod_wasm::RuntimeConfig {
            budgets,
            ..Default::default()
        },
        &std_member,
        vec![horizon_mod_wasm::WasmPackage {
            manifest: horizon_mod_package::Package::read(&package)
                .unwrap()
                .manifest,
            module: horizon_mod_package::Package::read_wasm(&package).unwrap(),
        }],
    );
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    loaded
}

#[test]
#[ignore = "needs tests/bench/hook built to mod.wasm (CI's wasm-bench job)"]
fn a_call_past_its_time_stops_its_group() {
    let mut budgets = Budgets::default();
    budgets.call = Duration::from_millis(20);
    let loaded = load(budgets);
    let wasm = loaded.mods[0].fields().hooks;
    // About 200 ms of work, stopped at its limit: no answer, and the group answers nothing after.
    let started = Instant::now();
    assert_eq!(chat(wasm, "compute 10000000"), "");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(chat(wasm, "text"), "");
}

#[test]
#[ignore = "needs tests/bench/hook built to mod.wasm (CI's wasm-bench job)"]
fn a_call_past_its_fuel_stops_its_group() {
    let mut budgets = Budgets::default();
    budgets.fuel = Some(Fuel::default());
    let loaded = load(budgets);
    let wasm = loaded.mods[0].fields().hooks;
    assert_eq!(chat(wasm, "text"), "a message the mod wrote");
    assert_eq!(chat(wasm, "compute 1000000"), "");
    assert_eq!(chat(wasm, "text"), "");
}

/// A flat world's start context, all the bench's structure type needs.
struct Flat;

impl horizon_world::worldgen::structure::jigsaw::JigsawContext for Flat {
    fn min_y(&self) -> i32 {
        -64
    }
    fn max_y(&self) -> i32 {
        320
    }
    fn base_height(&mut self, _: i32, _: i32, _: mod_api::HeightmapType) -> i32 {
        65
    }
    fn column_has_valid_biome(&mut self, _: i32, _: i32, _: i32, _: i32) -> bool {
        true
    }
}

impl horizon_world::worldgen::structure::start::StartContext for Flat {
    fn seed(&self) -> i64 {
        1
    }
    fn sea_level(&self) -> i32 {
        63
    }
    fn base_column(&mut self, _: i32, _: i32, _: mod_api::HeightmapType) -> Vec<bool> {
        Vec::new()
    }
    fn base_columns(&mut self, _: i32, _: i32, _: &[mod_api::HeightmapType]) -> Vec<Vec<bool>> {
        Vec::new()
    }
    fn noise_column(&mut self, _: i32, _: i32) -> mod_api::NoiseColumn {
        mod_api::NoiseColumn::default()
    }
    fn cold_enough_to_snow(&mut self, _: mod_api::BiomeId, _: [i32; 3]) -> bool {
        false
    }
    fn generation(&self) -> horizon_world::worldgen::anchor::WorldGenerationContext {
        horizon_world::worldgen::anchor::WorldGenerationContext::new(-64, 384, 63)
    }
    fn noise_biome(&mut self, _: [i32; 3]) -> mod_api::BiomeId {
        mod_api::BiomeId(0)
    }
    fn noise_biomes(&mut self, quarts: &[[i32; 3]]) -> Vec<mod_api::BiomeId> {
        vec![mod_api::BiomeId(0); quarts.len()]
    }
    fn set_valid_biomes(&mut self, _: Option<&std::collections::BTreeSet<mod_api::BiomeId>>) {}
}

#[test]
#[ignore = "needs tests/bench/hook built to mod.wasm (CI's wasm-bench job)"]
fn worldgen_workers_sharing_a_wasm_structure_each_get_their_structure() {
    use horizon_core::random::{LegacyRandom, WorldgenRandom};
    // A hook call would be refused after 1 ms of waiting for the group; worldgen calls queue.
    let mut budgets = Budgets::default();
    budgets.wait = Duration::from_millis(1);
    let loaded = load(budgets);
    mod_api::__private::install_parts(&loaded.mods, &mod_api::__private::part_tests::rule_schema())
        .unwrap();
    let types = mod_api::__private::installed_structure_types();
    let spin = types
        .get("bench_mod:spin")
        .cloned()
        .expect("the bench mod registers bench_mod:spin");
    let config = std::sync::Arc::new(serde_json::json!({"type": "bench_mod:spin"}));
    let workers: Vec<_> = (0..8)
        .map(|i| {
            let (spin, config) = (spin.clone(), config.clone());
            std::thread::spawn(move || {
                let mut random = WorldgenRandom::new(LegacyRandom::new(0));
                random.set_large_feature_seed(1, i, 0);
                let chunk = mod_api::ChunkPos::new(i, 0);
                spin.find_generation_point(&mut Flat, chunk, &config, &mut random)
                    .map(|(position, pieces)| (position, pieces.len()))
            })
        })
        .collect();
    for (i, worker) in workers.into_iter().enumerate() {
        let (position, pieces) = worker.join().unwrap().expect("every chunk gets its start");
        assert_eq!((position[0], position[2], pieces), (i as i32 * 16, 0, 1));
    }
}

#[test]
#[ignore = "needs tests/bench/hook built to mod.wasm and a release build (CI's wasm-bench job)"]
fn a_wasm_mod_against_the_same_mod_native() {
    let loaded = load(Budgets::default());
    let wasm = loaded.mods[0].fields().hooks;
    let native = &*bench_mod::hooks();

    let cases = [
        ("tick", "a call with a small event", 20_000),
        ("empty", "a call with the chat event", 20_000),
        ("text", "a message lowered back", 20_000),
        ("untouched", "100 replies left as they were", 2_000),
        ("imports 100", "100 calls into the game", 1_000),
        (
            "entities 100",
            "100 entity lookups (an id in, none out)",
            1_000,
        ),
        (
            "tickstats 100",
            "100 tick stats (nothing in, two numbers out)",
            1_000,
        ),
        ("compute 1000", "logic on 1,000 values", 1_000),
        ("compute 10000", "logic on 10,000 values", 100),
        ("compute 100000", "logic on 100,000 values", 10),
    ];
    let mut table = String::from(
        "### A wasm mod against the same mod native\n\n\
         | Work | | Native | Wasm | Ratio |\n| --- | --- | --- | --- | --- |\n",
    );
    for (work, what, calls) in cases {
        assert_eq!(
            chat(native, work),
            chat(wasm, work),
            "{work}: both builds answer alike"
        );
        let native_time = per_call(native, work, calls);
        let wasm_time = per_call(wasm, work, calls);
        table.push_str(&format!(
            "| `{work}` | {what} | {} | {} | {:.1}× |\n",
            shown(native_time),
            shown(wasm_time),
            wasm_time.as_secs_f64() / native_time.as_secs_f64().max(1e-12)
        ));
    }
    println!("{table}");
    if let Ok(summary) = std::env::var("GITHUB_STEP_SUMMARY") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(summary)
            .unwrap();
        writeln!(file, "{table}").unwrap();
    }
}

/// A server's tick for one listener at the tick point.
fn tick_at_point(listen: impl FnOnce(mod_api::HostSide<'_>, &mut mod_points::TickArgs)) -> bool {
    let mut args = mod_points::TickArgs::__new();
    listen(mod_api::HostSide::Server(&mut World), &mut args);
    args.cancel
}

#[test]
#[ignore = "needs tests/bench/hook built to mod.wasm and a release build (CI's wasm-bench job)"]
fn a_point_filled_by_a_wasm_mod() {
    use std::sync::atomic::Ordering;
    // The bench mod and its manifest-generated tick listener.
    let path = |var: &str| std::path::PathBuf::from(std::env::var(var).expect(var));
    let package = path("HORIZON_WASM_BENCH");
    let manifest = horizon_mod_package::Package::read(&package)
        .unwrap()
        .manifest;
    let mut loaded = horizon_mod_wasm::load(
        &horizon_mod_wasm::RuntimeConfig::default(),
        &std::fs::read(path("HORIZON_WASM_STD")).unwrap(),
        vec![horizon_mod_wasm::WasmPackage {
            manifest,
            module: horizon_mod_package::Package::read_wasm(&package).unwrap(),
        }],
    );
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    for listening in std::mem::take(&mut loaded.listening) {
        listening.register().unwrap();
    }
    mod_points::__private::seal();
    // The game's slot table: a place nobody listens at costs this check.
    let slots = &mod_points::__private::POINTS;
    let quiet = mod_points::PointId::TickWorld as usize;
    assert!(!slots[quiet].load(Ordering::Relaxed));
    let empty = timed(1_000_000, || {
        std::hint::black_box(slots[std::hint::black_box(quiet)].load(Ordering::Relaxed));
    });
    let native_time = timed(20_000, || {
        std::hint::black_box(tick_at_point(bench_mod::tick));
    });
    let wasm_time = timed(20_000, || {
        std::hint::black_box(tick_at_point(mod_points::dispatch::tick));
    });
    let table = format!(
        "### A point filled by a mod\n\n| Slot | Per tick |\n| --- | --- |\n\
         | Nobody listens | {} |\n| A native listener | {} |\n| A wasm listener | {} |\n",
        shown(empty),
        shown(native_time),
        shown(wasm_time)
    );
    println!("{table}");
    if let Ok(summary) = std::env::var("GITHUB_STEP_SUMMARY") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(summary)
            .unwrap();
        writeln!(file, "{table}").unwrap();
    }
}
