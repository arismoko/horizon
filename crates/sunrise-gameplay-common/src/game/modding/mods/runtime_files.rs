//! Each package's runtime data files, checked against their `[runtime]` hashes once when the
//! package opens and handed to registration, which serves them by mod id.
use super::ModsState;
use mod_api::RuntimePathError;
use std::{collections::BTreeMap, path::Path, sync::Arc};
use sunrise_mod_package::manifest::RuntimeData;

/// One package's checked files, as the mod API's source of them.
struct PackageRuntime(RuntimeData);

impl mod_api::RuntimeFileSource for PackageRuntime {
    fn names(&self) -> &[String] {
        self.0.names()
    }
    fn file(&self, name: &str) -> Option<&[u8]> {
        self.0.file(name).map(|(bytes, _)| bytes)
    }
    fn path(&self, name: &str) -> Result<&Path, RuntimePathError> {
        let (_, path) = self.0.file(name).ok_or(RuntimePathError::NotListed)?;
        path.ok_or(RuntimePathError::NotAFolder)
    }
}

/// The runtime files of every package that declares `[runtime]`, by mod id. A compiled native
/// mod's were checked when the game validated its package at startup (a mismatch ends the
/// start there, as any compiled mod's package failing to read does: `mod-host/src/loading.rs`).
pub fn sources(mods: &ModsState) -> BTreeMap<String, Arc<dyn mod_api::RuntimeFileSource>> {
    let mut sources: BTreeMap<String, Arc<dyn mod_api::RuntimeFileSource>> = BTreeMap::new();
    for package in mods.native.packages() {
        if let Some(runtime) = &package.runtime {
            sources.insert(
                package.manifest.id.clone(),
                Arc::new(PackageRuntime(runtime.clone())),
            );
        }
    }
    sources
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(any())]
    use crate::game::api_values;
    #[cfg(any())]
    use crate::game::native_entity_refs::tests::run_fresh_cases;
    #[cfg(any())]
    use mod_api::{
        Hooks, ModEntry, PackageInfo, PackageKind, Registrar, RegistrationContext, Side,
    };
    #[cfg(any())]
    use std::sync::Mutex;
    use sunrise_mod_package::manifest::Package;

    #[cfg(any())]
    const NAME: &str =
        "game::mods::runtime_files::tests::a_mod_reads_runtime_files_by_mod_id_during_registration";
    const BYTES: &[u8] = b"baked lookup table";

    /// A native package folder with one listed runtime file, opened as the engine opens it.
    fn open_package(folder: &std::path::Path) -> RuntimeData {
        let hash = sunrise_mod_package::manifest::sha512_of(BYTES);
        std::fs::create_dir_all(folder.join("src")).unwrap();
        std::fs::create_dir_all(folder.join("runtime")).unwrap();
        std::fs::write(
            folder.join("sunrise-mod.toml"),
            format!(
                "id='reader_a'\nname='A'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n\
                 [native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'\n[runtime]\n\"nav.bin\"='{hash}'\n",
                "a".repeat(40),
                "b".repeat(40),
            ),
        )
        .unwrap();
        std::fs::write(
            folder.join("Cargo.toml"),
            "[package]\nname='a'\nversion='1.0.0'\n",
        )
        .unwrap();
        std::fs::write(folder.join("src/lib.rs"), "").unwrap();
        std::fs::write(folder.join("runtime/nav.bin"), BYTES).unwrap();
        let package = Package::read(folder).unwrap();
        package.read_runtime(folder).unwrap().unwrap()
    }

    /// A package folder whose name is not UTF-8 gives its files' paths back byte for byte.
    #[cfg(unix)]
    #[test]
    fn a_non_unicode_package_folder_keeps_its_exact_path() {
        use mod_api::RuntimeFileSource as _;
        use std::os::unix::ffi::OsStrExt;
        let parent = tempfile::tempdir().unwrap();
        let folder = parent
            .path()
            .join(std::ffi::OsStr::from_bytes(b"ruby-\xff\x80"));
        let source = PackageRuntime(open_package(&folder));
        let path = source.path("nav.bin").unwrap();
        assert_eq!(path.as_os_str(), folder.join("runtime/nav.bin").as_os_str());
        assert_eq!(std::fs::read(path).unwrap(), BYTES);
    }

    #[test]
    fn a_live_path_does_not_replace_the_verified_bytes() {
        use mod_api::RuntimeFileSource as _;
        let folder = tempfile::tempdir().unwrap();
        let runtime = open_package(folder.path());
        let source = PackageRuntime(runtime.clone());
        let path = source.path("nav.bin").unwrap();
        let (verified, stored_path) = runtime.file("nav.bin").unwrap();
        assert!(std::ptr::eq(path, stored_path.unwrap()));
        assert!(std::ptr::eq(source.names(), runtime.names()));
        assert!(std::ptr::eq(source.file("nav.bin").unwrap(), verified));

        std::fs::write(path, b"edited after opening").unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"edited after opening");
        assert_eq!(source.file("nav.bin"), Some(BYTES));
    }

    #[test]
    fn runtime_paths_distinguish_unlisted_and_archived_files() {
        use mod_api::RuntimeFileSource as _;
        let folder = tempfile::tempdir().unwrap();
        let package = folder.path().join("package");
        let source = PackageRuntime(open_package(&package));
        let archive = folder.path().join("package.smod");
        std::fs::write(&archive, Package::pack_directory(&package).unwrap()).unwrap();
        let packaged = Package::read(&archive).unwrap();
        let archived = PackageRuntime(packaged.read_runtime(&archive).unwrap().unwrap());

        assert_eq!(source.path("other.bin"), Err(RuntimePathError::NotListed));
        assert_eq!(archived.path("other.bin"), Err(RuntimePathError::NotListed));
        assert_eq!(archived.path("nav.bin"), Err(RuntimePathError::NotAFolder));
        assert_eq!(archived.file("nav.bin"), Some(BYTES));
    }

    /// What each mod's registration saw when looking up runtime files by mod id.
    #[cfg(any())]
    struct Reader(&'static str, &'static str, Arc<Mutex<Vec<String>>>);
    #[cfg(any())]
    #[mod_api::__subscriptions]
    impl Hooks for Reader {
        fn register(&self, _: &mut dyn Registrar, cx: &RegistrationContext) {
            let record = |target: &str| match cx.runtime_files(target) {
                Ok(files) => format!(
                    "{}->{target} names={:?} nav={:?} other={:?} path={}",
                    self.0,
                    files.names(),
                    files.file("nav.bin").map(|b| b == BYTES),
                    files.file("other.bin"),
                    files
                        .file_path("nav.bin")
                        .unwrap()
                        .ends_with("runtime/nav.bin"),
                ),
                Err(error) => format!("{}->{target} {error}", self.0),
            };
            let mut seen = self.2.lock().unwrap();
            seen.push(record(self.1));
            if self.1 != "reader_a" {
                seen.push(record("reader_a"));
            }
        }
    }

    /// Registration reads runtime files by explicit mod id: `reader_a` has a package with one
    /// file, `reader_b` can read `reader_a`'s files by id while reading its own returns
    /// `NoRuntimeFiles`, and a name the manifest does not list is `None`.
    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn a_mod_reads_runtime_files_by_mod_id_during_registration() {
        if run_fresh_cases(NAME, "SUNRISE_RUNTIME_FILES_CHILD", &["registration"]) {
            return;
        }
        let cwd = std::env::current_dir().unwrap();
        let launch = sunrise_mod_package::selection::LaunchArguments::parse(
            std::iter::empty::<String>(),
            &cwd,
            &cwd,
            None,
        )
        .unwrap();
        sunrise_level::runtime_env::runtime_environment::capture(&launch);
        let folder = tempfile::tempdir().unwrap();
        let runtime = open_package(folder.path());
        let mut files: BTreeMap<String, Arc<dyn mod_api::RuntimeFileSource>> = BTreeMap::new();
        files.insert("reader_a".into(), Arc::new(PackageRuntime(runtime)));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let entries = vec![
            ModEntry::new("reader_a", Box::new(Reader("a", "reader_a", seen.clone()))),
            ModEntry::new("reader_b", Box::new(Reader("b", "reader_b", seen.clone()))),
        ];
        let inventory: Vec<_> = ["reader_a", "reader_b"]
            .map(|id| {
                PackageInfo::new(
                    id.into(),
                    "1.0.0".into(),
                    PackageKind::Code,
                    Side::Both,
                    vec![id.into()],
                    vec![],
                )
            })
            .into();
        api_values::install_test_platform();
        mod_platform::__private::install_packages(&inventory, &[]);
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        crate::game::mods::install_registration_data(&root, cwd, files, BTreeMap::new()).unwrap();
        mod_platform::testing::with_mods(entries.into(), || {
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
        });
        let mut seen = seen.lock().unwrap().clone();
        // Registration may run more than once; each run must see the same.
        seen.sort();
        seen.dedup();
        assert_eq!(
            seen,
            [
                "a->reader_a names=[\"nav.bin\"] nav=Some(true) other=None path=true",
                "b->reader_a names=[\"nav.bin\"] nav=Some(true) other=None path=true",
                "b->reader_b package declares no [runtime] files",
            ]
        );
    }
}
