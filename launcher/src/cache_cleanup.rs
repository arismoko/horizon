//! Descriptor-bounded snapshot cleanup through held, no-follow directory capabilities.
//! Cargo has no launcher exclusion contract: target/profile and package-cache lock inodes and
//! their ancestry are retained. Locks are checked before enumeration, then reacquired through
//! each file's removal. A late Cargo either blocks deletion or starts after it has completed.
//! Only snapshotted files are selected. Same-user swaps inside caches are not a security boundary.
pub use crate::native_cache::Error;
use crate::native_cache::{locked, Locked};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const LOCKS: [&str; 3] = [".cargo-lock", ".package-cache", ".package-cache-mutate"];

/// One root handle and value-only output records; no handle is retained per file or directory.
pub struct Plan {
    directory: Dir,
    outputs: Vec<PathBuf>,
}

fn exists(directory: &Dir, name: &str) -> Result<bool, Error> {
    match directory.symlink_metadata(name) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn guard(directory: &Dir, relative: &Path) -> Result<Vec<Locked>, Error> {
    let root = relative.as_os_str().is_empty();
    // Cargo locks target itself, target/<profile> and target/<triple>/<profile>.
    let target = relative
        .ancestors()
        .take(3)
        .any(|path| path.file_name().is_some_and(|name| name == "target"));
    let cargo = root || target || exists(directory, LOCKS[0])?;
    let packages = root || exists(directory, LOCKS[1])? || exists(directory, LOCKS[2])?;
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .follow(FollowSymlinks::No);
    let mut locks = Vec::new();
    for (name, needed) in [
        (LOCKS[0], cargo),
        (LOCKS[1], packages),
        (LOCKS[2], packages),
    ] {
        if needed {
            locks.push(locked(directory.open_with(name, &options)?.into_std())?);
        }
    }
    Ok(locks)
}

impl Plan {
    /// Launcher Cargo output roots must be named target; existing locks anywhere are honored.
    /// Arbitrary late-created locks in other same-user directories are outside this contract.
    pub fn pin(directory: &Dir) -> Result<Self, Error> {
        Self::pin_observed(directory, &|| {})
    }

    fn pin_observed(directory: &Dir, observe: &impl Fn()) -> Result<Self, Error> {
        let mut plan = Self {
            directory: directory.try_clone()?,
            outputs: Vec::new(),
        };
        plan.visit(directory, Path::new(""), observe)?;
        Ok(plan)
    }

    pub fn pin_file(directory: &Dir, name: &Path) -> Result<Self, Error> {
        if name.components().count() != 1
            || LOCKS.iter().any(|lock| name == Path::new(lock))
            || name == Path::new(".build.lock")
        {
            return Err(Error::Unsafe(name.to_owned()));
        }
        let _locks = guard(directory, Path::new(""))?;
        let meta = directory.symlink_metadata(name)?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(Error::Unsafe(name.to_owned()));
        }
        Ok(Self {
            directory: directory.try_clone()?,
            outputs: vec![name.to_owned()],
        })
    }

    fn visit(
        &mut self,
        directory: &Dir,
        relative: &Path,
        observe: &impl Fn(),
    ) -> Result<(), Error> {
        let _locks = guard(directory, relative)?;
        observe();
        for entry in directory.entries()? {
            let name = PathBuf::from(entry?.file_name());
            observe();
            if LOCKS.iter().any(|lock| name == Path::new(lock)) || name == Path::new(".build.lock")
            {
                continue;
            }
            let meta = directory.symlink_metadata(&name)?;
            if meta.file_type().is_symlink() {
                return Err(Error::Unsafe(name));
            }
            let path = relative.join(&name);
            if meta.is_dir() {
                self.visit(&directory.open_dir_nofollow(&name)?, &path, observe)?;
            } else if meta.is_file() {
                self.outputs.push(path);
            } else {
                return Err(Error::Unsafe(name));
            }
        }
        Ok(())
    }

    /// Trash into the caller's held root on the same volume; preserve lock inode ancestry.
    /// Busy/errors can leave a partially cleared tree; no freed-byte count is returned on error.
    pub fn clear(self, trash: &Dir) -> Result<(), Error> {
        self.clear_observed(trash, &|_| {}, &|| {})
    }

    fn clear_observed(
        self,
        trash: &Dir,
        before_lock: &impl Fn(&Path),
        observe: &impl Fn(),
    ) -> Result<(), Error> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for output in &self.outputs {
            before_lock(output);
            let mut parent = self.directory.try_clone()?;
            let mut relative = PathBuf::new();
            let mut locks = guard(&parent, &relative)?;
            for component in output.parent().unwrap_or(Path::new("")).components() {
                parent = parent.open_dir_nofollow(component.as_os_str())?;
                relative.push(component);
                locks.extend(guard(&parent, &relative)?);
                observe();
            }
            let file = Path::new(output.file_name().unwrap());
            let meta = match parent.symlink_metadata(file) {
                Ok(meta) => meta,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if !meta.is_file() {
                continue;
            }
            let name = format!(
                ".trash-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            parent.rename(file, trash, &name)?;
            observe();
            #[cfg(windows)]
            if meta.permissions().readonly() {
                use cap_std::fs::OpenOptionsExt;
                use windows_sys::Win32::Storage::FileSystem::FILE_WRITE_ATTRIBUTES;
                let mut options = OpenOptions::new();
                options
                    .read(true)
                    .access_mode(FILE_WRITE_ATTRIBUTES)
                    .follow(FollowSymlinks::No);
                let mut permissions = meta.permissions();
                permissions.set_readonly(false);
                trash
                    .open_with(&name, &options)?
                    .set_permissions(permissions)?;
            }
            trash.remove_file(&name)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "cache_cleanup/tests.rs"]
mod tests;
