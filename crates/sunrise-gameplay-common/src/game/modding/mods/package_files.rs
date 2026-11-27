//! The content index captured for each package at startup, shared by registration and loading.
use super::ModsState;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use sunrise_mod_package::{
    Content, Manifest,
    content::{AdmittedContentSource, ContentSource, ReadError},
    manifest::MissingJar,
    selection::SelectedPackage,
};

pub(in crate::game) struct PackageContent {
    pub(in crate::game) path: PathBuf,
    pub(in crate::game) opened: Result<AdmittedContentSource, OpenError>,
    pub(in crate::game) visibility: Visibility,
}

#[derive(PartialEq, Eq)]
pub(in crate::game) enum Visibility {
    Visible,
    Disabled,
    RefusedContent,
}

pub(in crate::game) enum OpenError {
    Read(String),
    Namespace(String),
}

impl OpenError {
    pub(in crate::game) fn detail(&self) -> &str {
        match self {
            Self::Read(detail) | Self::Namespace(detail) => detail,
        }
    }
}

impl PackageContent {
    pub(in crate::game) fn namespace_refusal(&self) -> Option<&str> {
        match &self.opened {
            Err(OpenError::Namespace(detail)) => Some(detail),
            _ => None,
        }
    }

    pub(in crate::game) fn read(&self, manifest: &Manifest) -> Result<Content, String> {
        Content::read_opened(
            self.opened.as_ref().map_err(|e| e.detail().to_owned())?,
            manifest,
        )
    }
}

impl mod_api::PackageFileSource for PackageContent {
    fn names(&self) -> Result<Arc<[String]>, String> {
        self.opened
            .as_ref()
            .map(|opened| opened.source().names())
            .map_err(|e| e.detail().to_owned())
    }

    fn read(&self, name: &str) -> Result<Vec<u8>, mod_api::PackageReadError> {
        self.opened
            .as_ref()
            .map_err(|error| mod_api::PackageReadError::Read(error.detail().to_owned()))?
            .source()
            .reader()
            .read(name)
            .map_err(|error| match error {
                ReadError::NotFound(_) => mod_api::PackageReadError::NotFound,
                ReadError::Other(reason) => mod_api::PackageReadError::Read(reason),
            })
    }
}

pub struct StartupAdmission {
    /// The startup selection whose JAR requirements were checked; a later resource reload may
    /// have selected different packages.
    pub(in crate::game) selected: Arc<[SelectedPackage]>,
    pub(in crate::game) native: Arc<sunrise_mod_package::runtime::Runtime>,
    pub(in crate::game) refused: Vec<MissingJar>,
    pub(in crate::game) packages: BTreeMap<String, Arc<PackageContent>>,
}

impl StartupAdmission {
    pub(in crate::game) fn matches_selection(&self, selected: &Arc<[SelectedPackage]>) -> bool {
        Arc::ptr_eq(&self.selected, selected)
    }
}

impl StartupAdmission {
    pub(in crate::game) fn open(
        mods: &ModsState,
        refused: Vec<MissingJar>,
    ) -> sunrise_game_data::Result<Self> {
        let native = mods
            .native
            .packages()
            .iter()
            .map(|p| (&p.manifest, &p.path, true));
        let content = mods
            .selected
            .iter()
            .map(|p| (&p.package.manifest, &p.path, false));
        let packages: Vec<_> = native
            .chain(content)
            .filter(|(m, _, _)| !refused.iter().any(|missing| missing.owners.contains(&m.id)))
            .collect();
        let opened = super::read_package_inputs(&packages, |(manifest, path, _)| {
            ContentSource::open(path)
                .map_err(OpenError::Read)?
                .admit(manifest)
                .map_err(OpenError::Namespace)
        });
        let mut indexed = BTreeMap::new();
        for ((manifest, path, built_native), opened) in packages.into_iter().zip(opened) {
            if let Err(OpenError::Namespace(detail)) = &opened
                && manifest.served_by.is_some()
            {
                return Err(Box::new(super::ServedContentLoadError {
                    mod_id: manifest.id.clone(),
                    stage: super::ServedContentStage::Read,
                    detail: detail.clone(),
                }));
            }
            indexed.insert(
                manifest.id.clone(),
                Arc::new(PackageContent {
                    path: path.clone(),
                    // Disabled code still needs the installed manifest for registration authority.
                    visibility: if built_native && mods.startup_code_disabled(&manifest.id) {
                        Visibility::Disabled
                    } else if !built_native && matches!(opened, Err(OpenError::Namespace(_))) {
                        Visibility::RefusedContent
                    } else {
                        Visibility::Visible
                    },
                    opened,
                }),
            );
        }
        Ok(Self {
            selected: mods.selected.clone(),
            native: mods.native.clone(),
            refused,
            packages: indexed,
        })
    }

    pub(in crate::game) fn registration_visible(&self, id: &str) -> bool {
        self.packages
            .get(id)
            .is_some_and(|package| package.visibility == Visibility::Visible)
    }

    pub(in crate::game) fn namespace_refusal(&self, id: &str) -> Option<&str> {
        self.packages
            .get(id)
            .filter(|p| p.visibility == Visibility::RefusedContent)?
            .namespace_refusal()
    }

    /// Built native metadata for namespace and side authority, independent of the saved switch.
    pub fn native_packages(
        &self,
    ) -> impl Iterator<Item = &sunrise_mod_package::packages::AdjacentManifest> {
        self.native
            .packages()
            .iter()
            .filter(|p| self.packages.contains_key(&p.manifest.id))
    }

    pub fn content_packages(&self) -> impl Iterator<Item = &SelectedPackage> {
        self.selected
            .iter()
            .filter(|p| self.registration_visible(&p.package.manifest.id))
    }

    pub(in crate::game) fn package(&self, id: &str, path: &Path) -> Option<&PackageContent> {
        self.packages
            .get(id)
            .filter(|source| source.path == path)
            .map(Arc::as_ref)
    }
}

/// The sources for the same admitted packages as `RegistrationContext::mods`.
pub fn sources(
    admission: &StartupAdmission,
) -> BTreeMap<String, Arc<dyn mod_api::PackageFileSource>> {
    admission
        .packages
        .iter()
        .filter(|(_, source)| source.visibility == Visibility::Visible)
        .map(|(id, source)| {
            let source: Arc<dyn mod_api::PackageFileSource> = source.clone();
            (id.clone(), source)
        })
        .collect()
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;
    use crate::game::{api_values::TryIntoApi, mods::validate_startup_packages};
    use mod_api::{
        __private::{game::RegistrationContextNew, native::BuiltMod},
        PackageFilesError, RegistrationContext,
    };
    use std::fs;
    use sunrise_mod_package::{mod_list::NATIVE_DIR, runtime::Runtime};

    const SCRIPT: &str = "data/demo/lumen/main.luau";
    const NATIVE_MODEL: &str = "assets/native_demo/models/item/gem.json";
    const BUILT: &[BuiltMod] = &[BuiltMod {
        id: "native_demo",
        version: "1.0.0",
        side: mod_api::Side::Both,
    }];

    fn folder(root: &Path, id: &str, extra: &str) -> PathBuf {
        let path = root.join(id);
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("sunrise-mod.toml"),
            format!("id='{id}'\nname='{id}'\nversion='1.0.0'\napi='0.2'\nside='both'\n{extra}"),
        )
        .unwrap();
        path
    }

    fn write(path: &Path, name: &str, bytes: &[u8]) {
        let file = path.join(name);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, bytes).unwrap();
    }

    fn native_folder(root: &Path) -> PathBuf {
        let native = folder(
            &root.join(NATIVE_DIR),
            "native_demo",
            &format!(
                "capabilities=['registries']\nnamespaces=['native_extra']\n[native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'\n",
                "a".repeat(40),
                "b".repeat(40)
            ),
        );
        write(
            &native,
            "Cargo.toml",
            b"[package]\nname='native_demo'\nversion='1.0.0'\n",
        );
        write(&native, "src/lib.rs", b"");
        native
    }

    fn with_registration(admission: &StartupAdmission, f: impl FnOnce(RegistrationContext)) {
        let packages = admission
            .native_packages()
            .map(|p| p.try_into_api().unwrap())
            .chain(
                admission
                    .content_packages()
                    .map(|p| p.package.try_into_api().unwrap()),
            )
            .collect();
        mod_platform::__private::with_test_package_inventory(packages, || {
            let data = mod_api::__private::RegistrationData::new(
                Default::default(),
                Default::default(),
                mod_api::InstanceDirectory::from_path(Path::new("unused")).unwrap(),
                mod_api::RuntimeEnvironment::new(
                    mod_api::Dist::Client,
                    "26.3".into(),
                    "test".into(),
                    false,
                ),
            )
            .with_package_files(sources(admission));
            mod_platform::__private::with_test_registration_data(data, || {
                let _call = mod_api::__private::enter("reader", "register");
                f(RegistrationContext::new());
            });
        });
    }

    #[test]
    fn registration_files_and_metadata_follow_startup_admission() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let demo = folder(root, "demo", "");
        write(&demo, SCRIPT, b"return 1");
        let missing = folder(
            root,
            "missing",
            &format!(
                "[[jar]]\nid='original'\nversion='1'\nsha512='{}'\n",
                "a".repeat(128)
            ),
        );
        write(&missing, "data/missing/lumen/main.luau", b"return 2");
        let native = native_folder(root);
        write(&native, NATIVE_MODEL, b"{}");

        let mut mods = ModsState {
            native: Arc::new(Runtime::load(&root.join("sunrise-client"), BUILT).unwrap()),
            dev: vec![demo, missing],
            ..Default::default()
        };
        mods.capture_code_switches(&["native_demo".to_owned()].into());
        mods.select(root, &Default::default());
        validate_startup_packages(&mods).unwrap();
        let admission = mods.admit_startup_packages(&root.join("jars")).unwrap();
        assert_eq!(admission.refused.len(), 1);
        assert_eq!(admission.refused[0].owners, ["missing"]);
        with_registration(&admission, |cx| {
            assert_eq!(cx.mods().ids().collect::<Vec<_>>(), ["demo"]);
            let demo = cx.package_files("demo").unwrap();
            assert_eq!(demo.list("").collect::<Vec<_>>(), [SCRIPT]);
            assert_eq!(demo.read(SCRIPT).unwrap(), b"return 1");
            assert_eq!(
                cx.package_files("native_demo").err(),
                Some(PackageFilesError::NotEnabled)
            );
            assert!(mod_api::__private::owns_namespace(
                "native_demo",
                "native_extra"
            ));
            assert_eq!(
                cx.package_files("missing").err(),
                Some(PackageFilesError::NotEnabled)
            );
        });
    }

    #[test]
    fn unowned_namespace_refuses_content_but_keeps_built_native_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let demo = folder(root, "demo", "");
        let foreign = "data/other/lumen/main.luau";
        write(&demo, foreign, b"return 1");
        let native = native_folder(root);
        write(&native, foreign, b"return 2");
        let mut mods = ModsState {
            native: Arc::new(Runtime::load(&root.join("sunrise-client"), BUILT).unwrap()),
            dev: vec![demo.clone()],
            ..Default::default()
        };
        mods.select(root, &Default::default());
        validate_startup_packages(&mods).unwrap();
        let admission = mods.admit_startup_packages(&root.join("jars")).unwrap();
        with_registration(&admission, |cx| {
            assert!(cx.mods().get("demo").is_none());
            assert_eq!(
                cx.package_files("demo").err(),
                Some(PackageFilesError::NotEnabled)
            );
            assert!(cx.mods().get("native_demo").is_some());
            assert!(matches!(
                cx.package_files("native_demo"),
                Err(PackageFilesError::Unreadable(_))
            ));
        });
        // Repairing the folder does not undo the original admission refusal.
        fs::remove_file(demo.join(foreign)).unwrap();
        assert!(
            admission
                .package("demo", &demo)
                .unwrap()
                .read(&mods.selected[0].package.manifest)
                .is_err()
        );
        let mut rows = [sunrise_mod_package::mod_list::Installed {
            id: "demo".into(),
            title: "Demo".into(),
            description: String::new(),
            problem: None,
            dev: true,
            native: None,
        }];
        mods.report_refusals(&mut rows);
        assert!(rows[0].problem.is_some());
    }

    #[cfg(any())]
    fn mount(
        root: &Path,
        mods: &ModsState,
        path: &Path,
    ) -> (Content, sunrise_assets::resource_source::ResourceRoot) {
        use crate::game::mods::{ContentRequest, read_contents};
        use sunrise_client::mod_content_client::resolved_resources;
        use sunrise_sim::mod_content::ContentMod;
        let id = &mods.selected[0].package.manifest.id;
        let content = read_contents(&[ContentRequest {
            path,
            manifest: &mods.selected[0].package.manifest,
            admitted: mods.startup_admission().and_then(|a| a.package(id, path)),
        }])
        .pop()
        .unwrap()
        .unwrap();
        let base = root.join("base");
        fs::create_dir_all(&base).unwrap();
        let mounted = [ContentMod {
            id: id.clone(),
            content,
            jars: Vec::new(),
        }];
        let resources = resolved_resources(
            &base,
            base.clone(),
            &root.join("jars"),
            &mounted,
            root.join("cache"),
        )
        .unwrap();
        let [ContentMod { content, .. }] = mounted;
        (content, resources)
    }

    #[test]
    #[cfg(any())]
    fn registration_and_resource_loading_keep_the_admitted_archive_after_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("demo.smod");
        let replacement = dir.path().join("replacement.smod");
        let manifest = "id='demo'\nname='Demo'\nversion='1.0'\napi='0.2'\nside='both'\n";
        let archive = |bytes: &[u8]| {
            sunrise_mod_package::Package::write_files(manifest, &[(SCRIPT, bytes)]).unwrap()
        };
        fs::write(&path, archive(b"original")).unwrap();
        let mut mods = ModsState {
            dev: vec![path.clone()],
            ..Default::default()
        };
        mods.select(dir.path(), &Default::default());
        validate_startup_packages(&mods).unwrap();
        let admission = mods
            .admit_startup_packages(&dir.path().join("jars"))
            .unwrap();
        with_registration(&admission, |cx| {
            let files = cx.package_files("demo").unwrap();
            assert_eq!(files.list("").collect::<Vec<_>>(), [SCRIPT]);
            fs::write(&replacement, archive(b"replacement")).unwrap();
            fs::rename(&replacement, &path).unwrap();
            assert_eq!(files.read(SCRIPT).unwrap(), b"original");
        });
        let (loaded, resources) = mount(dir.path(), &mods, &path);
        assert_eq!(loaded.file(SCRIPT).unwrap(), b"original");
        assert_eq!(resources.mod_layers()[0].read(SCRIPT).unwrap(), b"original");
    }

    #[test]
    #[cfg(any())]
    fn registration_declarations_and_resources_share_the_index_and_read_loose_bytes_live() {
        use mod_api::PackageReadError;

        let dir = tempfile::tempdir().unwrap();
        let demo = folder(dir.path(), "demo", "");
        write(&demo, SCRIPT, b"return 1");
        let mut mods = ModsState {
            dev: vec![demo.clone()],
            ..Default::default()
        };
        mods.select(dir.path(), &Default::default());
        validate_startup_packages(&mods).unwrap();
        let admission = mods
            .admit_startup_packages(&dir.path().join("jars"))
            .unwrap();
        let later = "data/demo/lumen/later.luau";
        write(&demo, later, b"return 3");
        write(&demo, SCRIPT, b"return 2");

        let (loaded, resources) = mount(dir.path(), &mods, &demo);
        let layer = &resources.mod_layers()[0];
        assert!(!layer.contains(later));
        assert_eq!(layer.read(SCRIPT).unwrap(), b"return 2");
        with_registration(&admission, |cx| {
            let files = cx.package_files("demo").unwrap();
            assert_eq!(files.list("").collect::<Vec<_>>(), [SCRIPT]);
            assert_eq!(files.list("").collect::<Vec<_>>(), loaded.files.as_ref());
            assert_eq!(files.read(later), Err(PackageReadError::NotFound));
            assert_eq!(files.read(SCRIPT).unwrap(), b"return 2");
            assert_eq!(loaded.file(SCRIPT).unwrap(), b"return 2");
            fs::remove_file(demo.join(SCRIPT)).unwrap();
            assert_eq!(files.read(SCRIPT), Err(PackageReadError::NotFound));
        });
    }
}
