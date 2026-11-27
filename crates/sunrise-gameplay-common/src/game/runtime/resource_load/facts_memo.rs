//! Immutable file-derived tables shared only by test resource loads.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{Mutex, MutexGuard, OnceLock},
};
use sunrise_items::blocks::GameplayProjection;
use sunrise_world::registry::{BlockRegistry, BlockStateId};

type Tables = (BlockFacts, Vec<GameplayProjection>);
#[derive(PartialEq, Eq)]
struct Key {
    root: PathBuf,
    inputs: [u8; 32],
    parallel: bool,
}
/// The most recently used inputs, oldest first.
#[derive(Default)]
struct Memo(Mutex<Vec<(Key, Arc<Tables>)>>);
const KEPT: usize = 8;
static MEMO: OnceLock<Memo> = OnceLock::new();

pub(in crate::game) fn load(
    root: &std::path::Path,
    registry: &BlockRegistry,
    behaviours: &serde_json::Value,
    parallel: bool,
) -> std::result::Result<Tables, String> {
    // Fluid classification is the fact loader's one registration-dependent branch.
    if !sunrise_world::fluid::mod_fluids().is_empty() {
        return BlockFacts::load_with_projection(root, registry, behaviours, parallel)
            .map_err(|e| e.to_string());
    }
    MEMO.get_or_init(Memo::default)
        .load(root, registry, behaviours, parallel)
        .map(|tables| tables.as_ref().clone())
}

impl Memo {
    fn load(
        &self,
        root: &std::path::Path,
        registry: &BlockRegistry,
        behaviours: &serde_json::Value,
        parallel: bool,
    ) -> std::result::Result<Arc<Tables>, String> {
        let mut hash = Sha256::new();
        // The files the projection loader reads; the registry and behaviours document are hashed below.
        for name in ["gameplay-facts.json", "block-behaviours.json"] {
            let bytes = sunrise_world::json_file::read_bytes(&root.join(name))
                .map_err(|e| e.to_string())?;
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        }
        // Include the actual document, not just the file from which the caller read it.
        let bytes = serde_json::to_vec(behaviours).map_err(|e| e.to_string())?;
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
        // Content mods may supply a different registry over the same prepared root.
        // Names alone are insufficient: waterlogging/level properties affect fluids.
        hash.update((registry.len() as u64).to_le_bytes());
        for id in 0..registry.len() {
            let state = registry
                .state(BlockStateId(id as u32))
                .ok_or("missing registry state")?;
            hash.update((state.properties.len() as u64).to_le_bytes());
            for text in std::iter::once(state.name.as_str()).chain(
                state
                    .properties
                    .iter()
                    .flat_map(|(k, v)| [k.as_str(), v.as_str()]),
            ) {
                hash.update((text.len() as u64).to_le_bytes());
                hash.update(text.as_bytes());
            }
        }
        hash.update(
            registry
                .default_state("minecraft:air")
                .map(|id| id.0)
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        let key = Key {
            root: root.canonicalize().map_err(|e| e.to_string())?,
            inputs: hash.finalize().into(),
            parallel,
        };
        if let Some(tables) = hit(&mut self.entries(), &key) {
            return Ok(tables);
        }
        // Built outside the lock, so one test's miss never holds up another's hit; two tests
        // missing the same key at once both build, and the first to finish is kept.
        let tables = Arc::new(
            BlockFacts::load_with_projection(root, registry, behaviours, parallel)
                .map_err(|e| e.to_string())?,
        );
        let mut entries = self.entries();
        if let Some(kept) = hit(&mut entries, &key) {
            return Ok(kept);
        }
        // Least recently used goes first: a run of temporary content roots can push the common
        // root out, and its next load puts it back.
        if entries.len() == KEPT {
            entries.remove(0);
        }
        entries.push((key, tables.clone()));
        Ok(tables)
    }

    fn entries(&self) -> MutexGuard<'_, Vec<(Key, Arc<Tables>)>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The entry for `key`, moved to most recently used.
fn hit(entries: &mut Vec<(Key, Arc<Tables>)>, key: &Key) -> Option<Arc<Tables>> {
    let index = entries.iter().position(|(k, _)| k == key)?;
    let entry = entries.remove(index);
    let tables = entry.1.clone();
    entries.push(entry);
    Some(tables)
}

#[test]
fn identical_inputs_hit_but_changed_behaviours_and_registries_reload() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let registry_json = br#"{"states":[["minecraft:air",true,false,"","minecraft:air"],["minecraft:stone",true,false,"","minecraft:stone"]],"defaults":{"minecraft:air":0,"minecraft:stone":1}}"#;
    std::fs::write(
        root.join("gameplay-facts.json"),
        br#"{"states":[{"id":0,"sound":{"identity":"other"}},{"id":1,"hardness":2,"sound":{"identity":"other"}}]}"#,
    )
    .unwrap();
    let mut behaviours = serde_json::json!({"states":[[0,0],[0,0]],"blocks":[{"name":"minecraft:air","class":["Block"]},{"name":"minecraft:stone","class":["Block"]},{"name":"minecraft:dirt","class":["Block"]}]});
    std::fs::write(root.join("block-behaviours.json"), behaviours.to_string()).unwrap();
    let registry = BlockRegistry::from_facts_json(&registry_json[..]).unwrap();
    let memo = Memo::default();
    let first = memo.load(root, &registry, &behaviours, true).unwrap();
    let hit = memo.load(root, &registry, &behaviours, true).unwrap();
    assert!(Arc::ptr_eq(&first, &hit));
    let fresh = BlockFacts::load_with_projection(root, &registry, &behaviours, true).unwrap();
    assert_eq!(
        format!("{:?}", hit.0.states),
        format!("{:?}", fresh.0.states)
    );
    assert_eq!(hit.0.air, fresh.0.air);
    assert_eq!(hit.0.woods, fresh.0.woods);
    assert_eq!(hit.0.sign_texts, fresh.0.sign_texts);
    assert_eq!(hit.1, fresh.1);
    behaviours["states"][1] = serde_json::json!([63, 1]);
    std::fs::write(root.join("block-behaviours.json"), behaviours.to_string()).unwrap();
    let changed = memo.load(root, &registry, &behaviours, true).unwrap();
    assert!(!Arc::ptr_eq(&hit, &changed));
    assert_eq!(changed.0.states[1].sturdy_full, 63);
    assert!(changed.0.states[1].legacy_solid);
    let other_json = String::from_utf8(registry_json.to_vec())
        .unwrap()
        .replace("minecraft:stone", "minecraft:dirt");
    let other_registry = BlockRegistry::from_facts_json(other_json.as_bytes()).unwrap();
    let other = memo.load(root, &other_registry, &behaviours, true).unwrap();
    assert!(!Arc::ptr_eq(&changed, &other));
    assert_eq!(&*other.0.states[1].name, "minecraft:dirt");
    // Registries that differ only in a state property (the same names) are different keys.
    let property = |value: &str| {
        BlockRegistry::from_facts_json(
            format!(
                r#"{{"states":[["minecraft:air",true,false,"","minecraft:air"],["minecraft:stone",true,false,"",{{"id":"minecraft:stone","properties":{{"waterlogged":"{value}"}}}}]],"defaults":{{"minecraft:air":0,"minecraft:stone":1}}}}"#
            )
            .as_bytes(),
        )
        .unwrap()
    };
    let dry = memo
        .load(root, &property("false"), &behaviours, true)
        .unwrap();
    let wet = memo
        .load(root, &property("true"), &behaviours, true)
        .unwrap();
    assert!(!Arc::ptr_eq(&dry, &wet));
    // The least recently used input goes first: a root used again outlives a run of other roots,
    // and a full memo still keeps a new input.
    let others: Vec<_> = (0..KEPT)
        .map(|_| {
            let dir = tempfile::tempdir().unwrap();
            for name in ["gameplay-facts.json", "block-behaviours.json"] {
                std::fs::copy(root.join(name), dir.path().join(name)).unwrap();
            }
            dir
        })
        .collect();
    let common = memo.load(root, &registry, &behaviours, true).unwrap();
    for dir in &others[..KEPT - 1] {
        memo.load(dir.path(), &registry, &behaviours, true).unwrap();
    }
    let again = memo.load(root, &registry, &behaviours, true).unwrap();
    assert!(Arc::ptr_eq(&common, &again));
    memo.load(others[KEPT - 1].path(), &registry, &behaviours, true)
        .unwrap();
    let again = memo.load(root, &registry, &behaviours, true).unwrap();
    assert!(Arc::ptr_eq(&common, &again));
    let late = memo
        .load(root, &property("late"), &behaviours, true)
        .unwrap();
    let late_again = memo
        .load(root, &property("late"), &behaviours, true)
        .unwrap();
    assert!(Arc::ptr_eq(&late, &late_again));
}
