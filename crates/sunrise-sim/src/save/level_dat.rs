use super::*;

pub fn read_dat(path: &Path) -> Option<Tag> {
    try_read_dat(path).ok()
}
/// `read_dat`, with why the file did not read.
pub(super) fn try_read_dat(path: &Path) -> std::result::Result<Tag, DatIssue> {
    let bytes = fs::read(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => DatIssue::Missing,
        _ => DatIssue::Unreadable(e.to_string()),
    })?;
    let raw = match Compression::Gzip.decode(&bytes, Limits::saved().max_bytes) {
        Ok(raw) => raw,
        Err(_) => bytes,
    };
    nbt::read_named(&raw, Limits::saved())
        .map(|(named, _)| named.tag)
        .map_err(|e| DatIssue::Unreadable(e.to_string()))
}
/// What `LevelStorageAccess.collectIssues` finds wrong with a level.dat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DatIssue {
    /// No such file (`FileNotFoundException`).
    Missing,
    /// It exists but does not read as a level: why.
    Unreadable(String),
}
/// `LevelStorageAccess.collectIssues`: why level.dat (level.dat_old with
/// `old`) can't be read as the world's level data, `None` when it can.
pub fn level_dat_issue(dir: &Path, old: bool) -> Option<DatIssue> {
    let file = if old { "level.dat_old" } else { "level.dat" };
    match try_read_dat(&dir.join(file)) {
        Ok(root) if matches!(root.get("Data"), Some(Tag::Compound(_))) => None,
        Ok(_) => Some(DatIssue::Unreadable("Invalid root tag".into())),
        Err(issue) => Some(issue),
    }
}
/// `WorldOpenFlows.openWorldLoadLevelData`: what keeps `world`'s level data from reading, which
/// opens RecoverWorldDataScreen instead of the world. A folder with neither level.dat nor
/// level.dat_old is a world not made yet, not a broken one.
pub fn level_data_needs_recovery(world: &Path) -> Option<DatIssue> {
    match level_dat_issue(world, false)? {
        DatIssue::Missing if !world.join("level.dat_old").exists() => None,
        issue => Some(issue),
    }
}
/// `LevelStorageAccess.restoreLevelDataFromOld` (`Util.safeReplaceOrMoveFile`):
/// level.dat moves aside to `level.dat_corrupted_<time>` and level.dat_old
/// takes its place. Whether it worked. Vanilla passes `noRollback` and reports
/// success even when level.dat_old could not be moved in; here the broken
/// level.dat goes back and the failure is reported.
pub fn restore_level_data_from_old(dir: &Path) -> bool {
    let (current, latest) = (dir.join("level.dat"), dir.join("level.dat_old"));
    let backup = dir.join(format!("level.dat_corrupted_{}", backup_stamp()));
    if current.exists() && fs::rename(&current, &backup).is_err() {
        return false;
    }
    if fs::rename(&latest, &current).is_err() {
        if backup.exists() {
            let _ = fs::rename(&backup, &current);
        }
        return false;
    }
    true
}
/// `FileNameDateFormatter`: the time in the names of backed-up unreadable files.
fn backup_stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string()
}
/// `LevelStorageAccess.getUnfixedDataTagWithFallback`, which the dedicated server's `Main` calls:
/// a level.dat that does not read falls back to level.dat_old, which is restored in its place
/// (`restoreLevelDataFromOld`). Whether it did; `false` leaves the world as it was, for the
/// caller to refuse (both files broken, or nothing to fall back to).
pub fn fall_back_to_level_dat_old(world: &Path) -> bool {
    level_dat_issue(world, false).is_some()
        && world.join("level.dat_old").exists()
        && level_dat_issue(world, true).is_none()
        && restore_level_data_from_old(world)
}
/// `LevelStorageSource.readLevelDataTagFixed`: level.dat, else level.dat_old.
pub(super) fn read_level_dat(dir: &Path) -> Option<Tag> {
    read_dat(&dir.join("level.dat")).or_else(|| read_dat(&dir.join("level.dat_old")))
}
/// `restoreLevelDataFromOld`: an unreadable level.dat is kept as
/// `level.dat_corrupted_<time>` and level.dat_old takes its place, so the next
/// write does not bury either file.
pub(super) fn restore_level_dat(dir: &Path) {
    let path = dir.join("level.dat");
    if read_dat(&path).is_some() {
        return;
    }
    if path.exists() {
        let _ = fs::rename(
            &path,
            dir.join(format!("level.dat_corrupted_{}", backup_stamp())),
        );
    }
    let old = dir.join("level.dat_old");
    if read_dat(&old).is_some() {
        let _ = fs::rename(old, path);
    }
}
/// `PlayerDataStorage.backup`: a player file that fails to load is copied to
/// `<uuid>_corrupted_<time>.dat` before it is written over.
pub(super) fn back_up_unreadable_player(path: &Path) {
    if path.is_file()
        && read_dat(path).is_none()
        && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
    {
        let _ = fs::copy(
            path,
            path.with_file_name(format!("{stem}_corrupted_{}.dat", backup_stamp())),
        );
    }
}
/// Write a gzip NBT file the way vanilla does (`<name>_new`, keep `_old`).
pub fn write_dat(path: &Path, tag: Tag) -> Result<()> {
    write_dat_as(path, tag, false)
}

/// [`write_dat`], synced to the disk: the new file before it is renamed into place, and the
/// folder after, so a power loss leaves the old file or the new one. A hand-over's commit
/// point (`player-confirm`) replies only once its files are.
pub fn write_dat_durable(path: &Path, tag: Tag) -> Result<()> {
    write_dat_as(path, tag, true)
}

/// `bytes` written to `path` through `<name>_new` and a rename, synced to the disk as
/// [`write_dat_durable`] syncs: never a half-written file in place.
pub fn write_durable(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let file = path.file_name().and_then(|n| n.to_str()).unwrap_or("data");
    let temporary = path.with_file_name(format!("{file}_new"));
    write_synced(&temporary, bytes)?;
    fs::rename(&temporary, path)?;
    sync_folder(path)?;
    Ok(())
}

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// The folder of `path` synced, so that a rename into it is on the disk (POSIX; Windows has
/// no folder handle to sync, and its rename is journaled by NTFS).
fn sync_folder(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        fs::File::open(dir)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn write_dat_as(path: &Path, tag: Tag, durable: bool) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let bytes = nbt::write_named(
        &NamedTag {
            name: "".into(),
            tag,
        },
        Limits::saved(),
    )
    .map_err(|e| e.to_string())?;
    let compressed = Compression::Gzip.encode(&bytes)?;
    let file = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("data.dat");
    let temporary = path.with_file_name(format!("{file}_new"));
    if durable {
        write_synced(&temporary, &compressed)?;
    } else {
        fs::write(&temporary, compressed)?;
    }
    if path.exists() {
        let old = match file.strip_suffix(".dat") {
            Some(stem) => path.with_file_name(format!("{stem}.dat_old")),
            None => path.with_file_name(format!("{file}_old")),
        };
        // Util.safeReplaceFile: no backup, no replacement.
        fs::rename(path, old).map_err(|e| format!("{}: {e}", path.display()))?;
        use sunrise_world::crash_fixture::{CrashFixture, await_save_kill, selected};
        // Between the two renames, a reader needs the backup.
        match selected() {
            Some(CrashFixture::SaveLevel) if file == "level.dat" => await_save_kill("level"),
            Some(CrashFixture::SavePlayer)
                if path
                    .parent()
                    .is_some_and(|parent| parent.ends_with("players/data")) =>
            {
                await_save_kill("player");
            }
            _ => {}
        }
    }
    fs::rename(temporary, path)?;
    if durable {
        sync_folder(path)?;
    }
    Ok(())
}

/// `IOWorker.createOldDataForRegion`'s test of one record: saved before DataVersion 4997, or
/// with `blending_data`.
pub(super) fn record_is_old(root: &NamedTag) -> bool {
    let before_blending = match root.tag.get("DataVersion") {
        Some(Tag::Int(version)) => *version < 4997,
        _ => true,
    };
    before_blending || root.tag.get("blending_data").is_some()
}

/// `MobEffectInstance.Details` read under its effect's `id`, the
/// `hidden_effect` chain included.
pub(super) fn read_effect_details(e: &Tag, id: &str) -> crate::effects::SavedEffect {
    let get = |k: &str| number(e.get(k));
    // `Details` stores `amplifier` as `ExtraCodecs.UNSIGNED_BYTE`: the NBT
    // byte is signed, and `UnsignedBytes.toInt` maps it back to 0..=255. A
    // plain sign-extend turns 255 (byte -1) into -1, which
    // `EffectInstance::new` then clamps to the lowest level.
    let amplifier = match e.get("amplifier") {
        Some(Tag::Byte(v)) => i32::from(*v as u8),
        other => number(other).unwrap_or(0) as i32,
    };
    let particles = get("show_particles").is_none_or(|v| v != 0);
    crate::effects::SavedEffect {
        id: id.to_owned(),
        duration: get("duration").unwrap_or(0) as i32,
        amplifier,
        ambient: get("ambient").unwrap_or(0) != 0,
        particles,
        // Details.CODEC: `show_icon` defaults to `show_particles`.
        icon: get("show_icon").map_or(particles, |v| v != 0),
        hidden: e
            .get("hidden_effect")
            .map(|h| Box::new(read_effect_details(h, id))),
    }
}
/// `MobEffectInstance.Details.MAP_CODEC`'s fields, `hidden_effect` nested.
pub(super) fn effect_details(e: &crate::effects::SavedEffect) -> Vec<(&'static str, Tag)> {
    let mut details = vec![("duration", Tag::Int(e.duration))];
    // Amplifier defaults to zero; when written it is the codec's unsigned
    // byte: `Mth.clamp` to 0..=255, then `byteValue`.
    if e.amplifier != 0 {
        details.push((
            "amplifier",
            Tag::Byte(e.amplifier.clamp(0, 255) as u8 as i8),
        ));
    }
    // optionalFieldOf("ambient", false) and ("show_particles", true):
    // defaults are not written; show_icon is always written.
    if e.ambient {
        details.push(("ambient", Tag::Byte(1)));
    }
    if !e.particles {
        details.push(("show_particles", Tag::Byte(0)));
    }
    details.push(("show_icon", Tag::Byte(i8::from(e.icon))));
    if let Some(hidden) = &e.hidden {
        details.push(("hidden_effect", Tag::compound(effect_details(hidden))));
    }
    details
}
