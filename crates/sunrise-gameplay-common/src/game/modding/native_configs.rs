//! Native mods' config files, loaded and reloaded (NeoForge's `ModConfig`
//! and `ModConfigEvent`, docs/modding/src/how-to/mod-settings.md). Mods
//! register their configs during `Hooks::register` (`Registrar::register_config`),
//! each a file of a kind: `Client` on the window, `Common` once per process
//! (the window, or a dedicated server), `Server` on the server for each world
//! it opens, which a world may override in its `serverconfig/` folder. The
//! game writes a missing base file's default text, reads it and delivers
//! `ConfigEvent::Loading` with a `ConfigSnapshot` only to the config's
//! registered owner. Once a second it checks tracked files' text and delivers
//! `ConfigEvent::Reloading` to the owning listener when text changed. A
//! server's `Server` configs unload with `ConfigEvent::Unloading` when it stops.

use mod_api::{
    __private::{RegisteredConfig, parts_installation::registered_configs},
    ConfigEvent, ConfigKind, ConfigSnapshot,
};
pub use std::fs;
pub use std::path::{Path, PathBuf};
pub use std::time::{Duration, Instant};

/// How often tracked config files are checked for edits.
pub const CHECK_EVERY: Duration = Duration::from_secs(1);

/// A loaded config, its frozen registration and the last successfully read snapshot.
pub struct Tracked {
    pub config: &'static RegisteredConfig,
    pub snapshot: ConfigSnapshot,
}

/// One game's configs: the window's, a server thread's or a dedicated
/// server's.
#[derive(Default)]
pub struct NativeConfigs {
    pub(in crate::game) started: bool,
    pub files: Vec<Tracked>,
    pub(in crate::game) next_check: Option<Instant>,
}

/// The file `config` is read from: the world's existing `serverconfig/` copy
/// of a `Server` config when present, else `config_dir`'s base path, creating
/// defaults only when that base file is missing.
pub(in crate::game) fn locate(
    config: &RegisteredConfig,
    config_dir: &Path,
    world: Option<&Path>,
) -> Result<PathBuf, (PathBuf, std::io::Error)> {
    if config.kind == ConfigKind::Server
        && let Some(world) = world
    {
        let own = world.join("serverconfig").join(&config.file);
        // FML ConfigTracker.resolveBasePath uses Files.exists, which follows
        // symlinks so a dangling override falls through to the base config.
        match fs::metadata(&own) {
            Ok(_) => return Ok(own),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err((own, error)),
        }
    }
    let path = config_dir.join(&config.file);
    match fs::symlink_metadata(&path) {
        Ok(_) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
                && let Err(error) = fs::create_dir_all(parent)
            {
                return Err((path, error));
            }
            if let Err(error) = fs::write(&path, &config.default) {
                return Err((path, error));
            }
            Ok(path)
        }
        Err(error) => Err((path, error)),
    }
}

impl NativeConfigs {
    #[cfg(any(test, feature = "test-support"))]
    pub fn reset_poll_deadline_for_test(&mut self) {
        self.next_check = None;
    }

    /// Resolves, reads and tracks one registration, answering its initial snapshot. An I/O
    /// failure is logged with owner and path, without substituting another path, retrying or
    /// latching, and the registration is skipped.
    pub fn open(
        &mut self,
        config: &'static RegisteredConfig,
        config_dir: &Path,
        world: Option<&Path>,
    ) -> Option<ConfigSnapshot> {
        let path = match locate(config, config_dir, world) {
            Ok(path) => path,
            Err((path, error)) => {
                log::warn!(
                    "Sunrise: mod {} config {}: {error}",
                    config.mod_id(),
                    path.display()
                );
                return None;
            }
        };
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                log::warn!(
                    "Sunrise: mod {} config {}: {error}",
                    config.mod_id(),
                    path.display()
                );
                return None;
            }
        };
        let snapshot = config.snapshot(path, text);
        self.files.push(Tracked {
            config,
            snapshot: snapshot.clone(),
        });
        Some(snapshot)
    }

    /// The tracked files whose text changed since they were read, read again;
    /// a file that is gone or unreadable surfaces its I/O failure with owner
    /// and path and keeps its last successfully read snapshot. The text is
    /// compared, not the file's modification time: a save inside the same
    /// timestamp tick as the load (a mod writing its config from
    /// `ConfigEvent::Loading`) keeps the time and the length and still changed.
    pub fn changed(&mut self) -> Vec<(&'static RegisteredConfig, ConfigSnapshot)> {
        let mut changed = Vec::new();
        for tracked in &mut self.files {
            let text = match fs::read_to_string(&tracked.snapshot.path) {
                Ok(text) => text,
                Err(error) => {
                    log::warn!(
                        "Sunrise: mod {} config {}: {error}",
                        tracked.config.mod_id(),
                        tracked.snapshot.path.display()
                    );
                    continue;
                }
            };
            if text != tracked.snapshot.text {
                tracked.snapshot.text = text;
                changed.push((tracked.config, tracked.snapshot.clone()));
            }
        }
        changed
    }

    /// Whether a second passed since the last look at the files.
    pub fn due(&mut self, now: Instant) -> bool {
        if self.files.is_empty() || self.next_check.is_some_and(|t| now < t) {
            return false;
        }
        self.next_check = Some(now + CHECK_EVERY);
        true
    }

    /// Drops the server's configs and answers each registration with its last
    /// successfully read snapshot.
    pub fn drop_server(&mut self) -> Vec<(&'static RegisteredConfig, ConfigSnapshot)> {
        let (server, rest) = std::mem::take(&mut self.files)
            .into_iter()
            .partition(|t| t.config.kind == ConfigKind::Server);
        self.files = rest;
        server.into_iter().map(|t| (t.config, t.snapshot)).collect()
    }
}

pub trait NativeConfigsGameExt1<'__sg>:
    crate::game::SideGame<'__sg> + mod_api::ConfigContext
{
    /// Loads the installed configs of `kinds` and notifies each registration's owner.
    fn native_configs_load(&mut self, kinds: &[ConfigKind], world: Option<&Path>) {
        let configs = registered_configs();
        self.native_configs.started = true;
        // FML `ConfigTracker.loadConfigs` opens one registration, delivers its event, then
        // opens the next, so a callback that edits a later file is seen by that file's own
        // initial snapshot.
        for config in configs.iter().filter(|c| kinds.contains(&c.kind)) {
            let common = self.common_mut();
            if let Some(snapshot) = common
                .native_configs
                .open(config, &common.config_root, world)
            {
                config.notify_snapshot(ConfigEvent::Loading(self), &snapshot);
            }
        }
    }

    /// The window's `Client` and `Common` configs, once: after client setup
    /// and before its title, connection or ordinary runtime callbacks.
    fn native_configs_client_start(&mut self) {
        if !self.native_configs.started {
            use ConfigKind::{Client, Common};
            self.native_configs_load(&[Client, Common], None);
        }
    }

    /// A server about to open `world`: a dedicated server loads its
    /// `Common` configs too, a single player's server thread only `Server`.
    fn native_configs_server_start(&mut self, world: &Path, dedicated: bool) {
        use ConfigKind::{Common, Server};
        let kinds: &[ConfigKind] = if dedicated && !self.native_configs.started {
            &[Common, Server]
        } else {
            &[Server]
        };
        self.native_configs_load(kinds, Some(world));
    }

    /// Hands changed files over again, at most once a second.
    fn native_configs_poll(&mut self) {
        if !self.native_configs.due(Instant::now()) {
            return;
        }
        for (config, snapshot) in self.native_configs.changed() {
            config.notify_snapshot(
                ConfigEvent::Reloading(super::execution_role::host_side(self)),
                &snapshot,
            );
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg> + mod_api::ConfigContext> NativeConfigsGameExt1<'__sg>
    for G
{
}

#[cfg(test)]
#[path = "native_configs/tests.rs"]
pub(super) mod tests;

#[cfg(test)]
#[path = "native_configs/storage_tests.rs"]
mod storage_tests;
