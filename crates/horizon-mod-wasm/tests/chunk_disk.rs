//! The acceptance job uses the existing real shared-module fixture.
#[path = "fixture/src/chunk_disk.rs"]
mod fixture;
#[path = "support/no_hooks.rs"]
mod no_hooks;
use horizon_mod_package::manifest::Package;
use mod_api::*;

#[derive(Default)]
struct Native;
#[mod_api::horizon_mod]
impl Hooks for Native {
    fn chunk_data_save(&self, host: &dyn ChunkDiskHost, event: &mut ChunkDataSaveEvent<'_>) {
        fixture::save(host, event.data);
    }
    fn chunk_data_load(&self, host: &dyn ChunkDiskHost, event: &mut ChunkDataLoadEvent<'_>) {
        fixture::load(host, event.data);
    }
}
fn context(phase: ChunkDiskPhase, pos: [i32; 2]) -> ChunkDiskContextWire {
    ChunkDiskContextWire::new(
        WorldEpoch::new(17),
        Identifier::parse("minecraft:overworld").unwrap(),
        pos,
        ChunkDiskKind::Full,
        phase,
    )
}
fn saved(value: i32) -> Attachments {
    let mut store = Attachments::default();
    fixture::VALUE
        .set(&mut store, Holder::Level, value)
        .unwrap();
    store
}
fn value(entries: Compound) -> i32 {
    fixture::VALUE
        .get(&mut Attachments::from_nbt(entries), Holder::Level)
        .unwrap()
}
fn wasm() -> Mods {
    let fixture = std::env::var("HORIZON_WASM_FIXTURE")
        .expect("build tests/fixture and set HORIZON_WASM_FIXTURE");
    let std = std::env::var("HORIZON_WASM_STD").expect("build wasm-std and set HORIZON_WASM_STD");
    let path = std::path::Path::new(&fixture);
    let manifest = Package::read(path).unwrap().manifest;
    let info = PackageInfo::new(
        manifest.id.clone(),
        manifest.version.clone(),
        PackageKind::Code,
        Side::Both,
        vec![manifest.id.clone()],
        Vec::new(),
    );
    let mut package = horizon_mod_wasm::WasmPackage {
        manifest,
        module: Package::read_wasm(path).unwrap(),
    };
    if !mod_points::STAGED {
        package.manifest.inject.clear();
    }
    let loaded = horizon_mod_wasm::load(&std::fs::read(std).unwrap(), vec![package]);
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    loaded.initialize(&[info]).unwrap();
    loaded.mods.into()
}

struct Stderr;
impl log::Log for Stderr {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        eprintln!("{}", record.args());
    }
    fn flush(&self) {}
}

// One test: a wasm group installs its replace points process-wide, so the
// fixture can be loaded once per test binary.
#[test]
#[ignore = "needs HORIZON_WASM_FIXTURE and HORIZON_WASM_STD, built by wasm-mods"]
fn native_and_wasm_execute_identical_job_and_fail_closed_on_trap() {
    crate::test_stack::on_guest_stack(|| {
        let _ = log::set_logger(&Stderr);
        log::set_max_level(log::LevelFilter::Warn);
        for (mods, trap_reason) in [
            (
                vec![ModEntry::new("wasm_fixture", Box::new(Native))].into(),
                "native listener unwound",
            ),
            (
                wasm(),
                "wasm listener trapped or its Wire response was refused",
            ),
        ] {
            testing::with_mods(mods, || {
                let mut live = saved(7);
                let job =
                    ChunkDiskJob::capture(&context(ChunkDiskPhase::Save, [2, -3]), &live).unwrap();
                fixture::VALUE.set(&mut live, Holder::Level, 9).unwrap();
                let resources = OwningWorldgen::new(&|_| None, &|_, _| None, &|_, _, _| None);
                assert_eq!(value(job.run(&resources).clone().unwrap()), 8);
                assert_eq!(fixture::VALUE.get(&mut live, Holder::Level).unwrap(), 9);
                let load =
                    ChunkDiskJob::capture(&context(ChunkDiskPhase::Load, [2, -3]), &saved(8))
                        .unwrap();
                assert_eq!(value(load.run(&resources).clone().unwrap()), 9);

                // The guest sets 8 and then traps: the job refuses and keeps 7.
                let job =
                    ChunkDiskJob::capture(&context(ChunkDiskPhase::Save, [99, 99]), &saved(7))
                        .unwrap();
                let refusal = job.run(&resources).as_ref().unwrap_err();
                assert_eq!(refusal.mod_id, "wasm_fixture");
                assert_eq!(refusal.reason, trap_reason);
                assert_eq!(value(job.captured().clone()), 7);
            });
        }
    });
}

#[path = "../src/test_stack.rs"]
mod test_stack;
