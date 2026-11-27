use super::*;
use crate::game::native_hooks::base::GameHooks as _;
use crate::game::side_ops::modapi_hooks::ModapiHooksSideGameExt as _;
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use mod_api::{SettingAccessError, SettingKey, SettingKind, SettingSpec, SettingValue};
use std::collections::BTreeMap;
use std::io::{ErrorKind, Write};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy)]
struct LiveSetting {
    value: SettingValue,
    revision: u64,
}

/// The game side's authoritative store for native mod settings registered through
/// `SettingsRegistrar::register_setting`. One store serves the whole instance settings root: a
/// window and its integrated server share it ([`SharedSettings`]), so a write on either side
/// compares, persists and reads through the same live values.
#[derive(Default)]
pub struct NativeSettings {
    /// Cached live values and monotonic write revisions by `(owner, key)` for
    /// every registered setting.
    values: BTreeMap<(&'static str, &'static str), LiveSetting>,
    /// Monotonic revision bumped whenever a setting value changes (write,
    /// reload, or single-owner page refresh), so a pending reload/refresh
    /// notification superseded by a synchronous write inside an earlier
    /// callback is not delivered a second time.
    revision: u64,
}

/// The instance's settings authority, shared by the window and its integrated server. Never
/// hold its lock across a listener call: take what the call needs, release, then notify.
pub type SharedSettings = std::sync::Arc<std::sync::Mutex<NativeSettings>>;

impl NativeSettings {
    fn next_revision(&mut self) -> u64 {
        self.revision += 1;
        self.revision
    }

    /// Populate live values from a captured `LoopContext` snapshot.
    pub fn sync_from_loop_context(
        &mut self,
        context: &sunrise_world::native_load_context::LoopContext,
        covers: impl Fn(&str) -> bool,
    ) -> (u64, Vec<(&'static str, &'static str)>) {
        let mut changed = Vec::new();
        let reload_revision = self.next_revision();
        for (owner, spec) in mod_api::__private::settings() {
            if !covers(owner) {
                continue;
            }
            let key = spec.key.as_str();
            let raw = context.setting(owner, key);
            let value = spec.read(raw);
            let slot = (*owner, key);
            let previous = self.values.get(&slot).copied();
            let previous_value = previous
                .map(|entry| entry.value)
                .unwrap_or_else(|| spec.default_value());
            let entry_revision = if previous.is_some() && !previous_value.exact_eq(value) {
                changed.push(slot);
                reload_revision
            } else {
                previous.map_or(reload_revision, |entry| entry.revision)
            };
            self.values.insert(
                slot,
                LiveSetting {
                    value,
                    revision: entry_revision,
                },
            );
        }
        (reload_revision, changed)
    }

    /// Refresh one mod's live values from disk by its known registered keys.
    pub fn refresh_owner_from_disk(
        &mut self,
        root: &Path,
        owner: &str,
    ) -> (u64, Vec<(&'static str, &'static str)>) {
        let keys = mod_api::__private::settings()
            .iter()
            .filter(|(registered_owner, _)| *registered_owner == owner)
            .map(|(registered_owner, spec)| (*registered_owner, spec.key.as_str()));
        let captured = sunrise_world::native_load_context::capture_registered_settings(root, keys);
        let owner_map = captured.get(owner);
        let refresh_revision = self.next_revision();
        let mut changed = Vec::new();
        for (registered_owner, spec) in mod_api::__private::settings() {
            if *registered_owner != owner {
                continue;
            }
            let key = spec.key.as_str();
            let raw = owner_map.and_then(|map| map.get(key)).map(String::as_str);
            let value = spec.read(raw);
            let slot = (*registered_owner, key);
            let previous = self.values.get(&slot).copied();
            let previous_value = previous
                .map(|entry| entry.value)
                .unwrap_or_else(|| spec.default_value());
            let entry_revision = if previous.is_some() && !previous_value.exact_eq(value) {
                changed.push(slot);
                refresh_revision
            } else {
                previous.map_or(refresh_revision, |entry| entry.revision)
            };
            self.values.insert(
                slot,
                LiveSetting {
                    value,
                    revision: entry_revision,
                },
            );
        }
        (refresh_revision, changed)
    }

    /// Read a registered setting's current live value without touching the filesystem.
    pub fn value(&self, owner: &'static str, spec: &'static SettingSpec) -> SettingValue {
        self.values
            .get(&(owner, spec.key.as_str()))
            .map(|entry| entry.value)
            .unwrap_or_else(|| spec.default_value())
    }

    fn revision_of(&self, owner: &'static str, key: &'static str) -> Option<u64> {
        self.values.get(&(owner, key)).map(|entry| entry.revision)
    }

    fn commit(
        &mut self,
        root: &Path,
        owner: &'static str,
        spec: &'static SettingSpec,
        value: SettingValue,
    ) -> Result<bool, SettingAccessError> {
        let encoded = encode_setting(spec, value)?;
        let current = self.value(owner, spec);
        if current.exact_eq(value) {
            return Ok(false);
        }
        persist_setting(root, owner, &spec.key, &encoded)
            .map_err(|_| SettingAccessError::IoFailed)?;
        let revision = self.next_revision();
        self.values
            .insert((owner, spec.key.as_str()), LiveSetting { value, revision });
        Ok(true)
    }
}

fn encode_setting(spec: &SettingSpec, value: SettingValue) -> Result<String, SettingAccessError> {
    let kind_matches = matches!(
        (&spec.kind, value),
        (SettingKind::Toggle { .. }, SettingValue::Toggle(_))
            | (SettingKind::Choice { .. }, SettingValue::Choice(_))
            | (SettingKind::Range { .. }, SettingValue::Range(_))
            | (SettingKind::Color { .. }, SettingValue::Color(_))
    );
    if !kind_matches {
        return Err(SettingAccessError::WrongKind);
    }
    spec.store(value).ok_or(SettingAccessError::InvalidValue)
}

fn is_plain_component(component: &str) -> bool {
    let mut components = Path::new(component).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
}

fn persist_setting(root: &Path, owner: &str, key: &str, value: &str) -> std::io::Result<()> {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    if !is_plain_component(owner) || !is_plain_component(key) {
        return Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            "unsafe setting path component",
        ));
    }
    let root_dir = open_or_create_dir_nofollow(root)?;
    let owner_dir = open_or_create_child_dir_nofollow(&root_dir, owner)?;
    match owner_dir.symlink_metadata(key) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => {
            return Err(std::io::Error::new(
                ErrorKind::InvalidInput,
                "setting target is not a regular file",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let temp = format!(
        ".{key}.{}-{}.tmp",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    );
    write_via_temp(&owner_dir, &temp, value, key)
}

fn write_via_temp(owner_dir: &Dir, temp: &str, value: &str, key: &str) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let mut file = owner_dir.open_with(temp, &options)?;
    let result = file
        .write_all(value.as_bytes())
        .and_then(|()| file.sync_all())
        .and_then(|()| owner_dir.rename(temp, owner_dir, key));
    if result.is_err() {
        let _ = owner_dir.remove_file(temp);
    }
    result
}

fn open_or_create_dir_nofollow(root: &Path) -> std::io::Result<Dir> {
    let parent = root.parent().ok_or_else(|| {
        std::io::Error::new(ErrorKind::InvalidInput, "settings root has no parent")
    })?;
    let name = root
        .file_name()
        .ok_or_else(|| std::io::Error::new(ErrorKind::InvalidInput, "settings root has no name"))?;
    if !parent.as_os_str().is_empty() {
        fs::create_dir_all(parent)?;
    }
    let parent_dir = Dir::open_ambient_dir(
        if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        },
        cap_std::ambient_authority(),
    )?;
    open_or_create_child_dir_nofollow(&parent_dir, name)
}

fn open_or_create_child_dir_nofollow(parent: &Dir, name: impl AsRef<Path>) -> std::io::Result<Dir> {
    let name = name.as_ref();
    match parent.symlink_metadata(name) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
            return Err(std::io::Error::new(
                ErrorKind::InvalidInput,
                "settings directory is a symlink or not a directory",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => match parent.create_dir(name) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        },
        Err(error) => return Err(error),
    }
    parent.open_dir_nofollow(name)
}

/// Collect every registered setting's `(owner, key)` pair for per-key
/// capture at owning load and reload boundaries.
pub fn registered_setting_keys() -> impl Iterator<Item = (&'static str, &'static str)> {
    mod_api::__private::settings()
        .iter()
        .map(|(owner, spec)| (*owner, spec.key.as_str()))
}

/// Typed settings reads on `CommonGame` using owner-bound `SettingKey` handles.
impl crate::game::CommonGame<'_> {
    /// Read a setting through its owner-bound `SettingKey`.
    pub fn setting(&self, key: SettingKey) -> Result<SettingValue, SettingAccessError> {
        let (owner, spec) =
            mod_api::__private::setting_entry(key).ok_or(SettingAccessError::UnknownKey)?;
        Ok(self.live_setting(owner, spec))
    }

    /// A registered setting's live value in the instance's shared store.
    pub fn live_setting(&self, owner: &'static str, spec: &'static SettingSpec) -> SettingValue {
        self.settings_store().value(owner, spec)
    }

    /// The instance's settings store, locked: the guard must be gone before any listener runs.
    fn settings_store(&self) -> std::sync::MutexGuard<'_, NativeSettings> {
        self.native_settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The storage root bound to the active resource generation.
    pub fn native_settings_root(&self) -> PathBuf {
        let root = self.loop_context.settings_root();
        if root.as_os_str().is_empty() {
            crate::game::mod_data_root()
        } else {
            root.to_path_buf()
        }
    }

    /// Read a toggle setting through its owner-bound `SettingKey`.
    pub fn setting_bool(&self, key: SettingKey) -> Result<bool, SettingAccessError> {
        match self.setting(key)? {
            SettingValue::Toggle(on) => Ok(on),
            _ => Err(SettingAccessError::WrongKind),
        }
    }

    /// Read a choice setting's index through its owner-bound `SettingKey`.
    pub fn setting_choice(&self, key: SettingKey) -> Result<u32, SettingAccessError> {
        match self.setting(key)? {
            SettingValue::Choice(index) => {
                u32::try_from(index).map_err(|_| SettingAccessError::InvalidValue)
            }
            _ => Err(SettingAccessError::WrongKind),
        }
    }

    /// Read a range setting's number through its owner-bound `SettingKey`.
    pub fn setting_f64(&self, key: SettingKey) -> Result<f64, SettingAccessError> {
        match self.setting(key)? {
            SettingValue::Range(value) => Ok(value),
            _ => Err(SettingAccessError::WrongKind),
        }
    }

    /// Read a colour setting (`0xRRGGBB`) through its owner-bound `SettingKey`.
    pub fn setting_color(&self, key: SettingKey) -> Result<u32, SettingAccessError> {
        match self.setting(key)? {
            SettingValue::Color(rgb) => Ok(rgb),
            _ => Err(SettingAccessError::WrongKind),
        }
    }
}

pub trait ModSettingsGameExt<'__sg>: crate::game::SideGame<'__sg> {
    /// Write a setting through its owner-bound `SettingKey`, persisting
    /// to disk, updating live state and any open Options > Mods row, and
    /// notifying the owning mod synchronously on change.
    fn set_setting(
        &mut self,
        key: SettingKey,
        value: SettingValue,
    ) -> Result<bool, SettingAccessError> {
        let (owner, spec) =
            mod_api::__private::setting_entry(key).ok_or(SettingAccessError::UnknownKey)?;
        let root = self.native_settings_root();
        self.apply_native_setting_write(&root, owner, spec, value)
    }

    /// Shared commit path for typed writes and Options > Mods.
    fn apply_native_setting_write(
        &mut self,
        root: &Path,
        owner: &'static str,
        spec: &'static SettingSpec,
        value: SettingValue,
    ) -> Result<bool, SettingAccessError> {
        let changed = self.settings_store().commit(root, owner, spec, value)?;
        if changed {
            self.side_sync_native_setting_ui_row(owner, spec, value);
            self.native_setting_changed(owner, spec.key.as_str(), value);
        }
        Ok(changed)
    }

    /// Synchronize the live settings store from the active `LoopContext` snapshot
    /// and notify owners of every value that changed on reload.
    fn sync_native_settings_from_loop_context(&mut self) {
        let server_owners: Option<std::collections::BTreeSet<String>> =
            self.options.execution_role.is_logical_server().then(|| {
                sunrise_mod_package::enabled::EnabledMods::read(&self.options.resources)
                    .map(|inventory| inventory.server_ids().map(str::to_owned).collect())
                    .unwrap_or_default()
            });
        let (reload_revision, changed) = {
            let loop_context = &self.loop_context;
            self.native_settings
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .sync_from_loop_context(loop_context, |owner| {
                    server_owners
                        .as_ref()
                        .is_none_or(|owners| owners.contains(owner))
                })
        };
        for (owner, spec) in mod_api::__private::settings() {
            let value = self.live_setting(owner, spec);
            self.side_sync_native_setting_ui_row(owner, spec, value);
        }
        self.publish_reloaded_settings(reload_revision, changed);
    }

    /// Refresh one mod's live settings from disk and notify on external changes.
    fn refresh_native_settings_owner(&mut self, root: &Path, owner: &str) {
        if !self.options.execution_role.is_client() {
            return;
        }
        let (refresh_revision, changed) =
            self.settings_store().refresh_owner_from_disk(root, owner);
        self.publish_reloaded_settings(refresh_revision, changed);
    }

    /// Sync UI rows for every reloaded key before callbacks run, then deliver
    /// `setting_changed` only for keys not superseded by a synchronous write.
    fn publish_reloaded_settings(
        &mut self,
        batch_revision: u64,
        changed: Vec<(&'static str, &'static str)>,
    ) {
        for &(owner, key) in &changed {
            if let Some(spec) = mod_api::__private::setting(owner, key) {
                let value = self.live_setting(owner, spec);
                self.side_sync_native_setting_ui_row(owner, spec, value);
            }
        }
        for (owner, key) in changed {
            let current = self.settings_store().revision_of(owner, key);
            if current == Some(batch_revision)
                && let Some(spec) = mod_api::__private::setting(owner, key)
            {
                let value = self.live_setting(owner, spec);
                self.native_setting_changed(owner, key, value);
            }
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> ModSettingsGameExt<'__sg> for G {}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{Hooks, ModEntry, Registrar, RegistrationContext, SettingSpec, SettingValue};
    use std::sync::Arc;
    use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};

    #[test]
    fn persist_setting_confines_paths_and_refuses_symlinks_and_directories() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("settings");
        fs::create_dir_all(root.join("b")).unwrap();
        fs::write(root.join("b/secret"), "secret").unwrap();

        assert!(persist_setting(&root, "a", "../b/secret", "stolen").is_err());
        assert!(persist_setting(&root, "../b", "secret", "stolen").is_err());
        assert_eq!(fs::read_to_string(root.join("b/secret")).unwrap(), "secret");

        persist_setting(&root, "a", "on", "true").unwrap();
        assert_eq!(fs::read_to_string(root.join("a/on")).unwrap(), "true");

        fs::remove_file(root.join("a/on")).unwrap();
        fs::create_dir(root.join("a/on")).unwrap();
        assert!(persist_setting(&root, "a", "on", "false").is_err());

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("b/secret"), root.join("a/link")).unwrap();
            assert!(persist_setting(&root, "a", "link", "overwrite").is_err());
            assert_eq!(fs::read_to_string(root.join("b/secret")).unwrap(), "secret");
        }
    }

    #[test]
    fn a_taken_temp_name_is_left_alone_and_the_setting_unwritten() {
        let dir = tempfile::tempdir().unwrap();
        let owner = Dir::open_ambient_dir(dir.path(), cap_std::ambient_authority()).unwrap();
        fs::write(dir.path().join("on"), "true").unwrap();
        fs::write(dir.path().join(".on.1-0.tmp"), "someone else's").unwrap();
        assert!(write_via_temp(&owner, ".on.1-0.tmp", "false", "on").is_err());
        assert_eq!(
            fs::read_to_string(dir.path().join(".on.1-0.tmp")).unwrap(),
            "someone else's"
        );
        assert_eq!(fs::read_to_string(dir.path().join("on")).unwrap(), "true");
        // A free name writes the value and leaves no temp behind.
        write_via_temp(&owner, ".on.1-1.tmp", "false", "on").unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("on")).unwrap(), "false");
        assert!(!dir.path().join(".on.1-1.tmp").exists());
    }

    #[test]
    fn native_settings_keeps_over_1024_keys_and_skips_superseded_revisions() {
        struct ManySettingsMod;

        #[mod_api::__subscriptions]
        impl Hooks for ManySettingsMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                for i in 0..1026 {
                    let key = format!("toggle_{i:04}");
                    r.register_setting(SettingSpec::toggle(key, "many.toggle", false))
                        .unwrap();
                }
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("settings");
        fs::create_dir_all(root.join("many")).unwrap();
        for i in 0..1025 {
            fs::write(root.join(format!("many/toggle_{i:04}")), "true").unwrap();
        }
        let mods: mod_api::Mods = vec![ModEntry::new("many", Box::new(ManySettingsMod))].into();

        mod_platform::testing::with_mods(mods.clone(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
            let mut store = NativeSettings::default();
            let load = LoadContext::new_with_registered(
                DataLoadOrigin::ClientPreload,
                Arc::new(LoadInputs::new(BTreeMap::new(), root.clone())),
                ["many"],
                registered_setting_keys(),
            );
            let (_, initial) = store
                .sync_from_loop_context(&load.captured_loop_context(LoopSide::Client), |_| true);
            assert!(initial.is_empty());

            // A large registration table retains keys through the initial load and live writes.
            let spec_1024 = mod_api::__private::setting("many", "toggle_1024").unwrap();
            let spec_1025 = mod_api::__private::setting("many", "toggle_1025").unwrap();
            assert_eq!(store.value("many", spec_1024), SettingValue::Toggle(true));
            assert_eq!(store.value("many", spec_1025), SettingValue::Toggle(false));
            assert_eq!(
                store.commit(&root, "many", spec_1025, SettingValue::Toggle(true)),
                Ok(true)
            );

            // Full reload and single-owner page refresh must preserve the 1,025th and 1,026th keys
            // without falling back to default or signaling spurious changes.
            let reload = LoadContext::new_with_registered(
                DataLoadOrigin::ClientPreload,
                Arc::new(LoadInputs::new(BTreeMap::new(), root.clone())),
                ["many"],
                registered_setting_keys(),
            );
            let (_, reload_changed) = store
                .sync_from_loop_context(&reload.captured_loop_context(LoopSide::Client), |_| true);
            assert!(reload_changed.is_empty());
            assert_eq!(store.value("many", spec_1024), SettingValue::Toggle(true));
            assert_eq!(store.value("many", spec_1025), SettingValue::Toggle(true));

            let (_, refresh_changed) = store.refresh_owner_from_disk(&root, "many");
            assert!(refresh_changed.is_empty());
            assert_eq!(store.value("many", spec_1024), SettingValue::Toggle(true));
            assert_eq!(store.value("many", spec_1025), SettingValue::Toggle(true));

            // When a reload changes two keys and a synchronous commit supersedes the second,
            // only the unsuperseded key still matches the reload batch revision.
            fs::write(root.join("many/toggle_1024"), "false").unwrap();
            fs::write(root.join("many/toggle_1025"), "false").unwrap();
            let reload_two = LoadContext::new_with_registered(
                DataLoadOrigin::ClientPreload,
                Arc::new(LoadInputs::new(BTreeMap::new(), root.clone())),
                ["many"],
                registered_setting_keys(),
            );
            let (batch_rev, changed) = store.sync_from_loop_context(
                &reload_two.captured_loop_context(LoopSide::Client),
                |_| true,
            );
            assert_eq!(
                changed,
                vec![("many", "toggle_1024"), ("many", "toggle_1025")]
            );
            assert_eq!(store.revision_of("many", "toggle_1024"), Some(batch_rev));
            assert_eq!(store.revision_of("many", "toggle_1025"), Some(batch_rev));

            // Writing the same value does not supersede the pending reload notification.
            assert_eq!(
                store.commit(&root, "many", spec_1025, SettingValue::Toggle(false)),
                Ok(false)
            );
            assert_eq!(store.revision_of("many", "toggle_1025"), Some(batch_rev));

            // Writing a new value advances the key's revision past the batch revision.
            assert_eq!(
                store.commit(&root, "many", spec_1025, SettingValue::Toggle(true)),
                Ok(true)
            );
            assert_ne!(store.revision_of("many", "toggle_1025"), Some(batch_rev));
            assert_eq!(store.revision_of("many", "toggle_1024"), Some(batch_rev));
        });
    }

    /// A capture that does not cover an owner (a server's holds the server mods' settings) leaves
    /// that owner's live values alone, so the other side's values are not reset to defaults.
    #[test]
    fn a_sync_leaves_the_owners_it_does_not_cover_alone() {
        struct OneSetting;

        #[mod_api::__subscriptions]
        impl Hooks for OneSetting {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.register_setting(SettingSpec::toggle("toggle_0000", "many.toggle", false))
                    .unwrap();
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("settings");
        fs::create_dir_all(root.join("many")).unwrap();
        fs::write(root.join("many/toggle_0000"), "true").unwrap();
        let mods: mod_api::Mods = vec![ModEntry::new("many", Box::new(OneSetting))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
            let capture = |_: ()| {
                LoadContext::new_with_registered(
                    DataLoadOrigin::ClientPreload,
                    Arc::new(LoadInputs::new(BTreeMap::new(), root.clone())),
                    ["many"],
                    registered_setting_keys(),
                )
                .captured_loop_context(LoopSide::Client)
            };
            let mut store = NativeSettings::default();
            store.sync_from_loop_context(&capture(()), |_| true);
            let spec = mod_api::__private::setting("many", "toggle_0000").unwrap();
            assert_eq!(store.value("many", spec), SettingValue::Toggle(true));

            fs::write(root.join("many/toggle_0000"), "false").unwrap();
            let (_, changed) = store.sync_from_loop_context(&capture(()), |owner| owner != "many");
            assert!(changed.is_empty());
            assert_eq!(store.value("many", spec), SettingValue::Toggle(true));
            let (_, changed) = store.sync_from_loop_context(&capture(()), |_| true);
            assert_eq!(changed, [("many", "toggle_0000")]);
        });
    }

    /// The window and the integrated server it starts from its options share one store.
    #[test]
    fn options_clones_share_the_settings_authority() {
        let options = crate::game::mods::ModsState::default();
        let server = options.clone();
        assert!(Arc::ptr_eq(&options.settings, &server.settings));
    }
}
