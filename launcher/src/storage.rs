//! Disposable launcher storage. Player-owned files are never accepted as cleanup targets.
use crate::cache_cleanup::Plan;
pub use crate::native_cache::Error;
use crate::native_cache::{checked, linked, Cache};
use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

mod inventory;
pub use inventory::{Report, Row};

/// The five rows on the Storage page.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    BuildFiles,
    PrebuiltGames,
    SpareGameBuilds,
    RustCompiler,
    OtherCaches,
}

/// Fixed build-home caches. Instance and version inventory adds its protected entries separately.
#[derive(Clone, Copy, Debug)]
pub enum BuildCache {
    Stage,
    PendingStage,
    NewStage,
    Patched,
    Source,
    Tools,
    PrebuiltOutput,
    InternalsSync,
    Prebuilt,
    ServerBuilds,
    Rust,
    Cargo,
    Scratch,
}

impl BuildCache {
    fn relative(self) -> &'static str {
        match self {
            Self::Stage => "native/stage",
            Self::PendingStage => "native/stage.pending",
            Self::NewStage => "native/stage.new",
            Self::Patched => "native/patched",
            Self::Source => "native/source",
            Self::Tools => "native/tools",
            Self::PrebuiltOutput => "native/prebuilt-build",
            Self::InternalsSync => "native/internals-sync",
            Self::Prebuilt => "native/prebuilt",
            Self::ServerBuilds => "native/server-builds",
            Self::Rust => "rust",
            Self::Cargo => "cargo",
            Self::Scratch => "tmp",
        }
    }

    pub fn category(self) -> Category {
        match self {
            Self::Prebuilt => Category::PrebuiltGames,
            Self::ServerBuilds => Category::SpareGameBuilds,
            Self::Rust | Self::Cargo | Self::Scratch => Category::RustCompiler,
            _ => Category::BuildFiles,
        }
    }
}

/// A build-home inventory and cleanup operation holds the native build lease throughout.
pub struct Storage {
    root: PathBuf,
    lease: Cache,
    directory: Dir,
}

impl Storage {
    pub fn open() -> Result<Self, Error> {
        Self::at(&crate::backend::build_dir())
    }

    fn at(root: &Path) -> Result<Self, Error> {
        checked(root, true)?;
        fs::create_dir_all(root)?;
        let directory = Dir::open_ambient_dir(root, cap_std::ambient_authority())?;
        let lease = Cache::acquire(root)?;
        let storage = Self {
            root: root.canonicalize()?,
            directory,
            lease,
        };
        storage.validate_root()?;
        Ok(storage)
    }

    fn path(&self, cache: BuildCache) -> Result<PathBuf, Error> {
        let mut path = self.root.clone();
        // Validate every ancestor, not only the final directory (Windows junctions included).
        if path.canonicalize()? != path {
            return Err(Error::Unsafe(path));
        }
        for part in cache.relative().split('/') {
            path.push(part);
            checked(&path, true)?;
        }
        Ok(path)
    }

    pub fn bytes(&self, cache: BuildCache) -> Result<u64, Error> {
        allocated_bytes(&self.path(cache)?)
    }

    /// Detach on the same volume before removal. Failed removals remain for the next sweep.
    pub fn clear(&self, cache: BuildCache) -> Result<u64, Error> {
        self.trash(&self.path(cache)?)
    }

    pub fn clear_targets(&self) -> Result<u64, Error> {
        // Retain Cargo lock inodes; never rename an entire locked target directory.
        self.lease.clear()
    }

    fn validate_root(&self) -> Result<(), Error> {
        use cap_fs_ext::MetadataExt;
        checked(&self.root, true)?;
        let current =
            Dir::open_ambient_dir(&self.root, cap_std::ambient_authority())?.dir_metadata()?;
        let held = self.directory.dir_metadata()?;
        if (current.dev(), current.ino()) != (held.dev(), held.ino()) {
            return Err(Error::Unsafe(self.root.clone()));
        }
        Ok(())
    }

    fn trash(&self, path: &Path) -> Result<u64, Error> {
        self.validate_root()?;
        let bytes = allocated_bytes(path)?;
        if !path.try_exists()? {
            return Ok(0);
        }
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| Error::Unsafe(path.to_owned()))?;
        let mut parent = self.directory.try_clone()?;
        let mut parts = relative.components().peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                Self::clear_entry(&self.directory, &parent, Path::new(part.as_os_str()))?;
            } else {
                parent = parent.open_dir_nofollow(part.as_os_str())?;
            }
        }
        Ok(bytes.saturating_sub(allocated_bytes(path)?))
    }

    fn clear_entry(trash: &Dir, parent: &Dir, name: &Path) -> Result<(), Error> {
        let meta = parent.symlink_metadata(name)?;
        let plan = if meta.is_dir() && !meta.file_type().is_symlink() {
            Plan::pin(&parent.open_dir_nofollow(name)?)?
        } else {
            Plan::pin_file(parent, name)?
        };
        plan.clear(trash)
    }

    pub fn sweep(&self) -> Result<(), Error> {
        self.validate_root()?;
        for entry in self.directory.entries()? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with(".trash-") {
                Self::clear_entry(
                    &self.directory,
                    &self.directory,
                    Path::new(&entry.file_name()),
                )?;
            }
        }
        Ok(())
    }
}

pub(crate) fn allocated_bytes(path: &Path) -> Result<u64, Error> {
    walk(path, &mut HashSet::new())
}

fn walk(path: &Path, seen: &mut HashSet<(u64, u64)>) -> Result<u64, Error> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    if linked(&meta) || (!meta.is_file() && !meta.is_dir()) {
        return Err(Error::Unsafe(path.to_owned()));
    }
    let (id, bytes) = allocation(path, &meta)?;
    if !seen.insert(id) {
        return Ok(0);
    }
    let mut total = bytes;
    if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            total = total.saturating_add(walk(&entry?.path(), seen)?);
        }
    }
    Ok(total)
}

#[cfg(unix)]
fn allocation(_path: &Path, meta: &fs::Metadata) -> Result<((u64, u64), u64), Error> {
    use std::os::unix::fs::MetadataExt;
    Ok(((meta.dev(), meta.ino()), meta.blocks().saturating_mul(512)))
}

#[cfg(windows)]
fn allocation(path: &Path, _meta: &fs::Metadata) -> Result<((u64, u64), u64), Error> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        FileStandardInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
        BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_STANDARD_INFO,
    };
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let mut standard: FILE_STANDARD_INFO = unsafe { std::mem::zeroed() };
    // Both structures are OS-owned output buffers; file keeps the handle live.
    unsafe {
        if GetFileInformationByHandle(file.as_raw_handle(), &mut info) == 0
            || GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileStandardInfo,
                (&mut standard as *mut FILE_STANDARD_INFO).cast(),
                std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
            ) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    if info.dwFileAttributes & 0x400 != 0 {
        return Err(Error::Unsafe(path.to_owned()));
    }
    Ok((
        (
            info.dwVolumeSerialNumber as u64,
            ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        ),
        standard.AllocationSize as u64,
    ))
}

#[cfg(test)]
#[path = "storage/tests.rs"]
mod tests;
