//! The saved worlds the title screen lists: the folders under the saves directory,
//! and the level.dat summary each row shows (`LevelStorageSource.findLevelCandidates`).
use super::Level;
use crate::lang;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct WorldEntry {
    pub folder: String,
    pub name: String,
    /// `GameType`: 0 survival, 1 creative, 2 adventure, 3 spectator.
    pub game_mode: i32,
    pub hardcore: bool,
    pub modified: std::time::SystemTime,
    pub commands: bool,
    pub version: String,
    /// `LevelSummary.isLocked`: another running copy has the world open.
    pub locked: bool,
    /// `LevelSummary.CorruptedLevelSummary`: level.dat did not read, so the row
    /// only knows its folder and offers recovery instead of play.
    pub corrupted: bool,
    /// `LevelSummary.isCompatible` false: the saved game version's name, for a world
    /// from another series.
    pub incompatible: Option<String>,
    /// `LevelSummary.requiresFileFixing`: opening the world upgrades its files first.
    pub file_fixing: bool,
    /// `BackupStatus.DOWNGRADE`: a newer version saved the world.
    pub downgrade: bool,
    /// `LevelSummary.isExperimental`: an experimental feature flag is enabled.
    pub experimental: bool,
}
impl WorldEntry {
    /// `LevelSummary.isCompatible`: the saved series is this build's.
    fn compatible(&self) -> bool {
        self.incompatible.is_none()
    }
    /// `LevelSummary.isDisabled`.
    pub fn disabled(&self) -> bool {
        self.locked || !self.compatible()
    }
    /// `LevelSummary.primaryActionActive`: a corrupted world offers recovery.
    pub fn can_play(&self) -> bool {
        self.corrupted || !self.disabled()
    }
    /// `LevelSummary.canEdit` and `canRecreate`.
    pub fn can_edit(&self) -> bool {
        !self.corrupted && !self.disabled() && !self.file_fixing
    }
    /// `BackupStatus.shouldBackup`: the world needs its files fixed or comes from
    /// a newer version.
    pub fn should_backup(&self) -> bool {
        self.file_fixing || self.downgrade
    }
    /// `CorruptedLevelSummary`: named after its folder, nothing else known.
    pub fn corrupted(folder: String, modified: std::time::SystemTime) -> Self {
        Self {
            name: folder.clone(),
            folder,
            game_mode: 0,
            hardcore: false,
            modified,
            commands: false,
            version: String::new(),
            locked: false,
            corrupted: true,
            incompatible: None,
            file_fixing: false,
            downgrade: false,
            experimental: false,
        }
    }
}

/// The latest `LastPlayed` kept: the end of year 9999, in milliseconds. A later time does not fit a
/// Windows `SystemTime`, and adding it panics.
const MAX_LAST_PLAYED_MILLIS: u64 = 253_402_300_799_000;

/// The time a world's `LastPlayed` (milliseconds since the epoch) names, clamped so that a damaged
/// or hostile `level.dat` cannot panic the world list.
fn last_played(millis: u64) -> std::time::SystemTime {
    std::time::UNIX_EPOCH + std::time::Duration::from_millis(millis.min(MAX_LAST_PLAYED_MILLIS))
}

pub fn list_worlds(saves: &Path) -> Vec<WorldEntry> {
    let since_epoch = |time: std::time::SystemTime| {
        time.duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
    };
    // Sorted by the time as saved: the clamped time shown would make far-off ones tie.
    let mut worlds: Vec<(std::time::Duration, WorldEntry)> = std::fs::read_dir(saves)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            // findLevelCandidates: a folder with a level.dat or a level.dat_old.
            let file = |name: &str| std::fs::metadata(path.join(name)).ok().filter(|m| m.is_file());
            // getFileModificationTime: level.dat's, else level.dat_old's.
            let file = file("level.dat").or_else(|| file("level.dat_old"))?;
            let modified = file.modified().unwrap_or(std::time::UNIX_EPOCH);
            let folder = e.file_name().to_string_lossy().into_owned();
            // readLevelSummary reads level.dat alone; when it fails the world is
            // still listed, as a CorruptedLevelSummary named after its folder.
            let level = Level::read_summary(&path);
            let root = crate::save::read_dat(&path.join("level.dat"));
            let (Some(level), Some(root)) = (level, root) else {
                return Some((since_epoch(modified), WorldEntry::corrupted(folder, modified)));
            };
            let data = root.get("Data")?;
            let (played, modified) = match data.get("LastPlayed") {
                Some(sunrise_core::nbt::Tag::Long(millis)) if *millis >= 0 => {
                    (std::time::Duration::from_millis(*millis as u64), last_played(*millis as u64))
                }
                _ => (since_epoch(modified), modified),
            };
            Some((played, WorldEntry {
                folder,
                name: level.name,
                game_mode: level.game_mode,
                hardcore: level.hardcore,
                modified,
                commands: matches!(data.get("allowCommands"), Some(sunrise_core::nbt::Tag::Byte(v)) if *v != 0),
                version: data.get("Version").and_then(|v| v.get("Name"))
                    .and_then(|v| match v { sunrise_core::nbt::Tag::String(s) => Some(s.to_string_lossy()), _ => None })
                    .unwrap_or_else(|| lang::get_or("selectWorld.versionUnknown", "Unknown")),
                locked: crate::save::SessionLock::is_locked(&path),
                corrupted: false,
                incompatible: crate::save::incompatible_version(&path),
                file_fixing: crate::save::needs_file_fixes(&path),
                downgrade: crate::save::downgraded_from(&path).is_some(),
                experimental: experimental_features(data),
            }))
        })
        .collect();
    worlds.sort_by_key(|(played, _)| std::cmp::Reverse(*played));
    worlds.into_iter().map(|(_, world)| world).collect()
}
/// `FeatureFlags.isExperimental` of `LevelStorageSource.parseFeatureFlagsFromSummary`:
/// level.dat's `enabled_features` name a known flag other than `minecraft:vanilla`
/// (unknown names are dropped).
fn experimental_features(data: &sunrise_core::nbt::Tag) -> bool {
    const EXPERIMENTAL: [&str; 3] = [
        "minecraft:trade_rebalance",
        "minecraft:redstone_experiments",
        "minecraft:minecart_improvements",
    ];
    let Some(sunrise_core::nbt::Tag::List(flags)) = data.get("enabled_features") else {
        return false;
    };
    flags.iter().any(|flag| {
        let sunrise_core::nbt::Tag::String(name) = flag else {
            return false;
        };
        let name = name.to_string_lossy();
        let id = if name.contains(':') {
            name
        } else {
            format!("minecraft:{name}")
        };
        EXPERIMENTAL.contains(&id.as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};
    use sunrise_core::nbt::Tag;

    /// A `LastPlayed` too far ahead for a Windows `SystemTime` is clamped to the end of year 9999
    /// instead of panicking the Singleplayer list; an ordinary one is kept as it is.
    #[test]
    fn a_last_played_too_far_ahead_is_clamped_not_a_panic() {
        let end = UNIX_EPOCH + Duration::from_millis(253_402_300_799_000);
        assert_eq!(last_played(i64::MAX as u64), end);
        assert_eq!(last_played(1_000_000_000_000_000), end);
        assert_eq!(last_played(253_402_300_799_000), end);
        assert_eq!(
            last_played(1_700_000_000_000),
            UNIX_EPOCH + Duration::from_millis(1_700_000_000_000)
        );
    }

    #[test]
    fn worlds_played_past_the_clamp_keep_the_newest_first() {
        let saves =
            std::env::temp_dir().join(format!("sunrise-last-played-order-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&saves);
        // Both later than the clamp, the older one first in the folder.
        for (folder, millis) in [("A", i64::MAX - 1), ("B", i64::MAX)] {
            crate::save::write_dat(
                &saves.join(folder).join("level.dat"),
                Tag::compound([(
                    "Data",
                    Tag::compound([
                        ("LevelName", Tag::String("Twin".into())),
                        ("LastPlayed", Tag::Long(millis)),
                    ]),
                )]),
            )
            .unwrap();
        }
        let folders: Vec<String> = list_worlds(&saves).into_iter().map(|w| w.folder).collect();
        assert_eq!(folders, ["B", "A"]);
        std::fs::remove_dir_all(&saves).unwrap();
    }
}
