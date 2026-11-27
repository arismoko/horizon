//! `CommandStorage`: named compounds for `/data ... storage <id>`, kept per
//! namespace in `data/<namespace>/command_storage.dat`.
use crate::save;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use sunrise_core::nbt::{Compound, Tag};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommandStorage {
    /// namespace -> path -> contents.
    namespaces: Arc<sunrise_world::loot::copy::StorageData>,
    dirty: bool,
}
fn split_id(id: &str) -> (&str, &str) {
    id.split_once(':').unwrap_or(("minecraft", id))
}
fn file(root: &Path, namespace: &str) -> PathBuf {
    root.join("data")
        .join(namespace)
        .join("command_storage.dat")
}
impl CommandStorage {
    /// Read every `data/<namespace>/command_storage.dat` under a world.
    pub fn load(root: &Path) -> Self {
        let mut s = Self::default();
        let Ok(dirs) = std::fs::read_dir(root.join("data")) else {
            return s;
        };
        for dir in dirs.flatten() {
            let namespace = dir.file_name().to_string_lossy().into_owned();
            let Some(data) =
                save::read_dat(&file(root, &namespace)).and_then(|t| t.get("data").cloned())
            else {
                continue;
            };
            if let Some(Tag::Compound(contents)) = data.get("contents") {
                let map = contents
                    .iter()
                    .map(|(k, v)| (k.to_string_lossy(), v.clone()))
                    .collect();
                Arc::make_mut(&mut s.namespaces).insert(namespace, map);
            }
        }
        s
    }
    pub fn save(&mut self, root: &Path) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        for (namespace, map) in self.namespaces.iter() {
            let contents: Compound = map
                .iter()
                .map(|(k, v)| (k.as_str().into(), v.clone()))
                .collect();
            if let Err(e) = save::write_dat(
                &file(root, namespace),
                Tag::compound([
                    (
                        "data",
                        Tag::compound([("contents", Tag::Compound(contents))]),
                    ),
                    ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
                ]),
            ) {
                log::warn!("command storage {namespace} was not written: {e}");
            }
        }
    }
    /// An immutable read view; subsequent writes detach only while a reader holds it.
    pub fn snapshot(&self) -> Arc<sunrise_world::loot::copy::StorageData> {
        self.namespaces.clone()
    }
    /// `get`: an empty compound when unset.
    pub fn get(&self, id: &str) -> Tag {
        let (ns, path) = split_id(id);
        self.namespaces
            .get(ns)
            .and_then(|m| m.get(path))
            .cloned()
            .unwrap_or_else(|| Tag::Compound(Compound::new()))
    }
    /// `set`: empty contents remove the entry.
    pub fn set(&mut self, id: &str, contents: Tag) {
        let (ns, path) = split_id(id);
        let empty = matches!(&contents, Tag::Compound(c) if c.is_empty());
        let map = Arc::make_mut(&mut self.namespaces)
            .entry(ns.to_owned())
            .or_default();
        if empty {
            map.remove(path);
        } else {
            map.insert(path.to_owned(), contents);
        }
        self.dirty = true;
    }
    /// What an in-place edit of `get`'s compound leaves behind when no
    /// `set` follows: a command that changed the stored compound and then
    /// failed (`DataCommands.manipulateData` throws before `setData`). Only
    /// an id that is stored has a compound to edit; the storage is not
    /// marked for saving.
    pub fn set_unsaved(&mut self, id: &str, contents: Tag) {
        let (ns, path) = split_id(id);
        if let Some(stored) = Arc::make_mut(&mut self.namespaces)
            .get_mut(ns)
            .and_then(|m| m.get_mut(path))
        {
            *stored = contents;
        }
    }
    /// Every stored id (`keys`).
    pub fn keys(&self) -> Vec<String> {
        self.namespaces
            .iter()
            .flat_map(|(ns, m)| m.keys().map(move |k| format!("{ns}:{k}")))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::nbt::parse_snbt;

    #[test]
    fn held_loot_snapshot_is_unchanged_by_storage_writes() {
        let mut storage = CommandStorage::default();
        storage.set("test:cache", Tag::compound([("value", Tag::Long(1))]));
        let held = storage.snapshot();
        storage.set("test:cache", Tag::compound([("value", Tag::Long(2))]));
        storage.set_unsaved("test:cache", Tag::compound([("value", Tag::Long(3))]));
        storage.set("test:other", Tag::compound([("value", Tag::Byte(4))]));
        assert_eq!(
            held["test"]["cache"],
            Tag::compound([("value", Tag::Long(1))])
        );
        assert!(!held["test"].contains_key("other"));
        assert_eq!(
            storage.get("test:cache"),
            Tag::compound([("value", Tag::Long(3))])
        );
    }

    #[test]
    fn stores_per_namespace_and_persists() {
        let dir = std::env::temp_dir().join(format!("hz-storage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = CommandStorage::default();
        assert_eq!(s.get("minecraft:x"), Tag::Compound(Compound::new()));
        s.set("minecraft:x", parse_snbt("{a:1}").unwrap().0);
        s.set("my:pack/y", parse_snbt("{b:[1,2]}").unwrap().0);
        s.save(&dir);
        let back = CommandStorage::load(&dir);
        assert_eq!(back.get("x"), parse_snbt("{a:1}").unwrap().0);
        assert_eq!(back.get("my:pack/y"), parse_snbt("{b:[1,2]}").unwrap().0);
        assert_eq!(back.keys(), vec!["minecraft:x", "my:pack/y"]);
        // Empty contents delete.
        let mut s = back;
        s.set("x", Tag::Compound(Compound::new()));
        assert_eq!(s.keys(), vec!["my:pack/y"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
