//! The prepared vanilla data unit tests read, loaded once per test process.
//!
//! Decoding the block registry and reading the
//! movement tables takes seconds in a debug build, and a test that did it
//! for itself spent most of its time there. Tests only read these, so one
//! copy serves them all; each test still builds its own `World` over it.
#[cfg(test)]
use crate::{assets::Assets, blocks::BlockFacts, motion::MotionResources};
#[cfg(test)]
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};
#[cfg(test)]
use sunrise_world::block_collision::BlockCollisionRegistry;

#[cfg(test)]
pub(crate) struct Vanilla {
    /// The root everything below was read from: additional test data uses the same root.
    pub root: PathBuf,
    pub assets: Assets,
    pub facts: BlockFacts,
    pub collision: BlockCollisionRegistry<'static>,
    pub motion: MotionResources,
}

#[cfg(test)]
impl Vanilla {
    /// A collision binding of this test's own, for a test that marks part states: the marks live
    /// in the binding, so tests that share `collision` would see each other's.
    pub fn own_collision(&self) -> BlockCollisionRegistry<'static> {
        crate::collisions::load(&self.root, self.assets.registry).unwrap()
    }
}

/// The prepared vanilla data at `SUNRISE_TEST_RESOURCES`, or the checkout's
/// `.local/sunrise-client/<version>`; `None` when neither has it (CI).
#[cfg(test)]
pub(crate) fn vanilla() -> Option<&'static Vanilla> {
    static VANILLA: OnceLock<Option<Vanilla>> = OnceLock::new();
    let resources = VANILLA
        .get_or_init(|| {
            let root = std::env::var_os("SUNRISE_TEST_RESOURCES")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../.local/sunrise-client")
                        .join(sunrise_core::VANILLA_VERSION)
                });
            if !root.join("movement-properties.nbt").exists() {
                return None;
            }
            let assets = Assets::load(&root).unwrap();
            let facts = BlockFacts::load(&root, assets.registry).unwrap();
            let collision = crate::collisions::load(&root, assets.registry).unwrap();
            let motion = MotionResources::load(&root, &assets).unwrap();
            Some(Vanilla {
                root,
                facts,
                assets,
                collision,
                motion,
            })
        })
        .as_ref();
    if resources.is_none() {
        report_missing();
    }
    resources
}

/// Attribute missing prepared data to every calling test, including cached misses.
pub(crate) fn report_missing() {
    use std::{
        collections::HashSet,
        io::Write,
        sync::{LazyLock, Mutex},
    };
    static SKIPPED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("<unnamed test>");
    let mut skipped = SKIPPED.lock().unwrap();
    if skipped.insert(name.to_owned()) {
        // One write, so concurrent tests never split the line that run_test_binaries.py reads.
        let line = format!(
            "skipped: no vanilla data; test={name}; skipped tests={}\n",
            skipped.len()
        );
        std::io::stderr()
            .lock()
            .write_all(line.as_bytes())
            .expect("writing missing-data skip report");
    }
}

/// A state table containing just air, for tests that don't need prepared resources.
#[cfg(test)]
pub(crate) fn air_registry() -> &'static sunrise_world::registry::BlockRegistry {
    use sunrise_world::registry::BlockRegistry;
    static REGISTRY: std::sync::OnceLock<BlockRegistry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| {
        BlockRegistry::from_facts_json(
            br#"{"defaults":{"minecraft:air":0},"states":[["air",true,false,"","minecraft:air"]]}"#
                .as_slice(),
        )
        .unwrap()
    })
}
