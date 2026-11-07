//! The CLI and Storage page share one fixed-path inventory.
use super::*;
use crate::{backend, instances, native_mods};
use serde::Serialize;

const BUILD_CACHES: [BuildCache; 13] = [
    BuildCache::Stage,
    BuildCache::PendingStage,
    BuildCache::NewStage,
    BuildCache::Patched,
    BuildCache::Source,
    BuildCache::Tools,
    BuildCache::PrebuiltOutput,
    BuildCache::InternalsSync,
    BuildCache::Prebuilt,
    BuildCache::ServerBuilds,
    BuildCache::Rust,
    BuildCache::Cargo,
    BuildCache::Scratch,
];

#[derive(Serialize)]
pub struct Row {
    pub category: Category,
    pub bytes: u64,
    pub freed_bytes: u64,
    pub remaining_bytes: u64,
}

#[derive(Serialize)]
pub struct Report {
    pub rows: Vec<Row>,
    pub total_bytes: u64,
}

impl Category {
    pub const ALL: [Self; 5] = [
        Self::BuildFiles,
        Self::PrebuiltGames,
        Self::SpareGameBuilds,
        Self::RustCompiler,
        Self::OtherCaches,
    ];
}

struct Entry {
    category: Category,
    directory: Option<Dir>,
    relative: PathBuf,
    path: PathBuf,
}

impl Entry {
    fn fixed(root: &Path, relative: &Path, category: Category) -> Result<Self, Error> {
        let directory = match Dir::open_ambient_dir(root, cap_std::ambient_authority()) {
            Ok(directory) => Some(directory),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let mut path = root.to_owned();
        checked(root, true)?;
        if root.exists() && root.canonicalize()? != root {
            return Err(Error::Unsafe(path));
        }
        for component in relative.components() {
            let std::path::Component::Normal(component) = component else {
                return Err(Error::Unsafe(relative.to_owned()));
            };
            path.push(component);
            match fs::symlink_metadata(&path) {
                Ok(meta) if linked(&meta) => return Err(Error::Unsafe(path)),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(Self {
            category,
            directory,
            relative: relative.to_owned(),
            path,
        })
    }

    fn clear(&self) -> Result<(), Error> {
        if !self.path.try_exists()? {
            return Ok(());
        }
        let Some(root) = &self.directory else {
            return Ok(());
        };
        let mut parent = root.try_clone()?;
        for part in self.relative.parent().unwrap_or(Path::new("")).components() {
            parent = parent.open_dir_nofollow(part.as_os_str())?;
        }
        Storage::clear_entry(root, &parent, Path::new(self.relative.file_name().unwrap()))
    }
}

fn add(
    entries: &mut Vec<Entry>,
    root: &Path,
    relative: &str,
    category: Category,
) -> Result<(), Error> {
    entries.push(Entry::fixed(root, Path::new(relative), category)?);
    Ok(())
}

impl Storage {
    fn entries(&self) -> Result<Vec<Entry>, Error> {
        let mut entries = Vec::new();
        for cache in BUILD_CACHES {
            add(&mut entries, &self.root, cache.relative(), cache.category())?;
        }
        for name in [
            "built.json",
            "points.json",
            "internals.json",
            "build.log",
            "catalog.partial",
        ] {
            add(
                &mut entries,
                &self.root,
                &format!("native/{name}"),
                Category::BuildFiles,
            )?;
        }
        // The target lock files must remain in place. Their contents are inventoried here but
        // deletion goes through Cache, which also acquires Cargo's locks.
        add(
            &mut entries,
            &self.root,
            "native/target",
            Category::BuildFiles,
        )?;
        let data = backend::data_dir();
        let played: HashSet<_> = instances::list()
            .into_iter()
            .filter_map(|instance| instance.resolve_version(None).ok())
            .collect();
        for version in backend::installed() {
            if native_mods::is_native(&version)
                && !played.contains(&version)
                && !backend::in_use(&version)
            {
                add(
                    &mut entries,
                    &data,
                    &format!("versions/{version}"),
                    Category::SpareGameBuilds,
                )?;
            }
        }
        for relative in ["cache/github", "mod-browser", "profile-heads"] {
            add(&mut entries, &data, relative, Category::OtherCaches)?;
        }
        for instance in instances::list() {
            // A missing version cannot prove that the instance is idle. Keep its caches.
            let Ok(version) = instance.resolve_version(None) else {
                continue;
            };
            if backend::in_use(&version) {
                continue;
            }
            old_logs(&mut entries, &instance.home)?;
            unused_jars(&mut entries, &instance.home)?;
            for relative in [".horizon-cache", "wasm-cache", "cache"] {
                add(
                    &mut entries,
                    &instance.home,
                    relative,
                    Category::OtherCaches,
                )?;
            }
        }
        // Detached trees can remain beside any managed cache after an interrupted removal.
        let parents: HashSet<_> = entries
            .iter()
            .filter_map(|entry| entry.path.parent().map(Path::to_owned))
            .chain([self.root.clone(), data])
            .collect();
        for parent in parents {
            if checked(&parent, true)?.is_none() {
                continue;
            }
            for entry in fs::read_dir(&parent)? {
                let entry = entry?;
                if entry.file_name().to_string_lossy().starts_with(".trash-") {
                    entries.push(Entry::fixed(
                        &parent,
                        Path::new(&entry.file_name()),
                        Category::OtherCaches,
                    )?);
                }
            }
        }
        Ok(entries)
    }

    pub fn report(&self, clear: Option<Category>, all: bool) -> Result<Report, Error> {
        // Also refuse an external Cargo holding a target lock before starting a cleanup.
        self.lease.bytes()?;
        self.apply(self.entries()?, clear, all)
    }

    fn apply(
        &self,
        entries: Vec<Entry>,
        clear: Option<Category>,
        all: bool,
    ) -> Result<Report, Error> {
        // Validate every descendant before any deletion and deduplicate allocated blocks per row.
        let mut rows = Vec::new();
        for category in Category::ALL {
            let mut seen = HashSet::new();
            let mut bytes = 0u64;
            for entry in entries.iter().filter(|entry| entry.category == category) {
                bytes = bytes.saturating_add(walk(&entry.path, &mut seen)?);
            }
            rows.push(Row {
                category,
                bytes,
                freed_bytes: 0,
                remaining_bytes: bytes,
            });
        }
        for entry in &entries {
            if all || clear == Some(entry.category) {
                if entry.path == self.root.join("native/target") {
                    self.lease.clear()?;
                } else {
                    entry.clear()?;
                }
            }
        }
        for row in &mut rows {
            if all || clear == Some(row.category) {
                let mut seen = HashSet::new();
                let mut remaining = 0u64;
                for entry in entries
                    .iter()
                    .filter(|entry| entry.category == row.category)
                {
                    remaining = remaining.saturating_add(walk(&entry.path, &mut seen)?);
                }
                row.freed_bytes = row.bytes.saturating_sub(remaining);
                row.remaining_bytes = remaining;
            }
        }
        Ok(Report {
            total_bytes: rows
                .iter()
                .fold(0u64, |total, row| total.saturating_add(row.bytes)),
            rows,
        })
    }
}

fn old_logs(entries: &mut Vec<Entry>, home: &Path) -> Result<(), Error> {
    for directory in ["logs", "crash-reports"] {
        let folder = Entry::fixed(home, Path::new(directory), Category::OtherCaches)?.path;
        if checked(&folder, true)?.is_none() {
            continue;
        }
        let mut files = Vec::new();
        for entry in fs::read_dir(&folder)? {
            let entry = entry?;
            let Some(meta) = checked(&entry.path(), false)? else {
                continue;
            };
            files.push((meta.modified()?, entry.file_name()));
        }
        files.sort();
        files.pop(); // Always retain the latest report, even when it is older than fourteen days.
        for (modified, name) in files {
            if modified
                .elapsed()
                .is_ok_and(|age| age.as_secs() > 14 * 86400)
            {
                entries.push(Entry::fixed(
                    home,
                    &Path::new(directory).join(name),
                    Category::OtherCaches,
                )?);
            }
        }
    }
    Ok(())
}

fn unused_jars(entries: &mut Vec<Entry>, home: &Path) -> Result<(), Error> {
    use horizon_mod_package::{
        mod_list::{DIR, NATIVE_DIR},
        Package,
    };
    let folder = Entry::fixed(home, Path::new("jars"), Category::OtherCaches)?.path;
    if checked(&folder, true)?.is_none() {
        return Ok(());
    }
    let mut referenced = HashSet::new();
    for directory in [DIR, NATIVE_DIR] {
        let path = Entry::fixed(home, Path::new(directory), Category::OtherCaches)?.path;
        for package in horizon_mod_package::find(&path) {
            // A malformed package cannot prove a jar is unused. Retain all jars in that case.
            let Ok(package) = Package::read(&package) else {
                return Ok(());
            };
            referenced.extend(
                package
                    .manifest
                    .jar
                    .iter()
                    .map(|jar| jar.cache_path(&folder)),
            );
        }
    }
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        if !referenced.contains(&entry.path()) {
            entries.push(Entry::fixed(
                home,
                &Path::new("jars").join(entry.file_name()),
                Category::OtherCaches,
            )?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seeded_home_reports_categories_and_clears_only_selected_row() {
        let home = crate::storage::tests::Home::new();
        home.seed("native/source/rev/lib.rs");
        home.seed("native/prebuilt/rev/game");
        home.seed("settings.json");
        let storage = Storage::at(&home.0).unwrap();
        let entries = BUILD_CACHES
            .into_iter()
            .map(|cache| {
                Entry::fixed(&home.0, Path::new(cache.relative()), cache.category()).unwrap()
            })
            .collect();
        let report = storage
            .apply(entries, Some(Category::BuildFiles), false)
            .unwrap();
        assert_eq!(report.rows.len(), 5);
        assert!(
            report
                .rows
                .iter()
                .find(|row| row.category == Category::BuildFiles)
                .unwrap()
                .freed_bytes
                > 0
        );
        assert!(!home.0.join("native/source/rev/lib.rs").exists());
        assert!(home.0.join("native/prebuilt/rev/game").exists());
        assert!(home.0.join("settings.json").exists());
        assert!(Entry::fixed(&home.0, Path::new("../worlds"), Category::OtherCaches).is_err());
    }
}
