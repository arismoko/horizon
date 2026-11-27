use super::*;

/// `DirectoryLock`: `session.lock` holds "\u{2603}" (UTF-16BE) under an exclusive
/// file lock for as long as the world is open, so two instances cannot share it.
pub struct SessionLock(pub(super) fs::File);
impl Drop for SessionLock {
    /// `DirectoryLock.close`: release the lock, then close the file. Closing
    /// alone keeps the lock while any copy of the descriptor lives on, such as
    /// the one a process spawned on another thread holds until its `exec`.
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
impl SessionLock {
    pub fn acquire(world: &Path) -> Result<Self> {
        use std::io::Write;
        fs::create_dir_all(world)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(world.join("session.lock"))?;
        file.try_lock()
            .map_err(|_| format!("{} is already open in another instance", world.display()))?;
        file.set_len(0)?;
        file.write_all(&[0x26, 0x03])?;
        file.flush()?;
        Ok(Self(file))
    }
    /// `DirectoryLock.isLocked`: another copy holds the world's `session.lock`.
    /// A world without the file is not locked; one whose file may not be opened
    /// is.
    pub fn is_locked(world: &Path) -> bool {
        let file = match fs::OpenOptions::new()
            .write(true)
            .open(world.join("session.lock"))
        {
            Ok(file) => file,
            Err(e) => return e.kind() == std::io::ErrorKind::PermissionDenied,
        };
        match file.try_lock() {
            Err(fs::TryLockError::WouldBlock) => true,
            Err(_) => false,
            // Take the probe's own lock back at once, as `SessionLock` does on drop.
            Ok(()) => {
                let _ = file.unlock();
                false
            }
        }
    }
}
/// Bring a world last saved by an older game version to the current format, as
/// 26.3 does when it opens one: the file fixes and level.dat first
/// (`FileFixerUpper`), then the DataFixers upgrades of every chunk, entity chunk,
/// POI chunk, player, saved data, advancements and statistics file. Vanilla runs
/// the latter as it reads each file; Sunrise runs them all on opening, which reads
/// the same data, and region records still older in a world whose level.dat is
/// current are upgraded as they are read (`upgrade_record`).
pub fn upgrade_older_world(world: &Path) -> Result<()> {
    upgrade_older_world_with(
        world,
        &sunrise_world::datafix::progress::UpgradeProgress::new(),
    )
}
/// [`upgrade_older_world`] reporting to `progress`, which `FileFixerProgressScreen` reads.
pub fn upgrade_older_world_with(
    world: &Path,
    progress: &sunrise_world::datafix::progress::UpgradeProgress,
) -> Result<()> {
    let Some(root) = read_dat(&world.join("level.dat")) else {
        return Ok(());
    };
    let version = root.get("Data").map_or(i64::from(DATA_VERSION), |d| {
        number(d.get("DataVersion")).unwrap_or(-1) // saves of 1.8 and before have none
    });
    if version >= i64::from(DATA_VERSION) {
        return Ok(());
    }
    if version < i64::from(sunrise_world::datafix::OLDEST_SUPPORTED) {
        return Err(format!(
            "{} was saved with DataVersion {version}; worlds older than DataVersion {} \
             cannot be upgraded yet (tools/upgrade_world.py runs vanilla's own upgrader)",
            world.display(),
            sunrise_world::datafix::OLDEST_SUPPORTED
        )
        .into());
    }
    sunrise_world::datafix::files::upgrade_world_with(world, progress)
        .map_err(|e| format!("upgrading {}: {e}", world.display()))?;
    Ok(())
}
/// Whether opening the world runs file fixes, which vanilla asks for a backup
/// first (`BackupStatus.FILE_FIXING_REQUIRED`). Saves of 1.8 and before have no
/// DataVersion, read as -1.
pub fn needs_file_fixes(world: &Path) -> bool {
    let Some(root) = read_dat(&world.join("level.dat")) else {
        return false;
    };
    let Some(data) = root.get("Data") else {
        return false;
    };
    let version = number(data.get("DataVersion")).unwrap_or(-1);
    i32::try_from(version).is_ok_and(sunrise_world::datafix::files::requires_file_fixing)
}
/// `LevelVersion.parse`: the saved game version's name, this build's when the `Version`
/// compound has no `Name`, and empty without a `Version` (`getWorldVersionName` then
/// shows `selectWorld.versionUnknown`).
fn saved_version_name(version: Option<&Tag>) -> String {
    match version {
        Some(v) => text(v.get("Name")).unwrap_or_else(|| VERSION_NAME.to_string()),
        None => String::new(),
    }
}
/// `LevelSummary.isCompatible` (`DataVersion.isCompatible`: the saved series must be
/// the current `main` one). None when the world opens; otherwise the saved game
/// version's name, empty when it has none (`getWorldVersionName`).
pub fn incompatible_version(world: &Path) -> Option<String> {
    let root = read_dat(&world.join("level.dat"))?;
    let version = root.get("Data")?.get("Version");
    let series = text(version.and_then(|v| v.get("Series"))).unwrap_or_else(|| "main".into());
    if series == "main" {
        return None;
    }
    Some(saved_version_name(version))
}
/// `LevelSummary.backupStatus` DOWNGRADE: the saved game version's name (empty when it
/// has none) when the world's `Version.Id` is newer than this build's `DataVersion`
/// (`LevelVersion.parse`: no `Version` reads as 0, no `Id` as the current one).
pub fn downgraded_from(world: &Path) -> Option<String> {
    let root = read_dat(&world.join("level.dat"))?;
    let version = root.get("Data")?.get("Version");
    let id = version.map_or(0, |v| {
        number(v.get("Id")).unwrap_or_else(|| i64::from(DATA_VERSION))
    });
    if id <= i64::from(DATA_VERSION) {
        return None;
    }
    Some(saved_version_name(version))
}

/// Move a world saved by an older Sunrise build (pre-26.3 layout with a
/// DataVersion that keeps vanilla from upgrading it) into the 26.3 layout.
pub fn migrate_legacy(world: &Path) -> Result<()> {
    for (old, dimension) in [
        ("", "minecraft:overworld"),
        ("DIM-1", "minecraft:the_nether"),
        ("DIM1", "minecraft:the_end"),
    ] {
        let from = if old.is_empty() {
            world.to_owned()
        } else {
            world.join(old)
        };
        let to = dimension_dir(world, dimension);
        for folder in ["region", "entities", "poi"] {
            let (src, dst) = (from.join(folder), to.join(folder));
            if src.is_dir() && !dst.exists() {
                fs::create_dir_all(&to)?;
                fs::rename(&src, &dst)?;
            }
        }
        if !old.is_empty() && from.is_dir() && fs::read_dir(&from)?.next().is_none() {
            let _ = fs::remove_dir(&from);
        }
    }
    Ok(())
}
