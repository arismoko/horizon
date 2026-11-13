//! The loaded packages' manifests as the `PackageInfo` snapshots mods read.
use super::TryIntoApi;
use horizon_mod_package::manifest::{Manifest, Property};

fn package_snapshot(
    manifest: &Manifest,
    kind: mod_api::PackageKind,
    runtime: Option<mod_api::PackageRuntime>,
    has_icon: bool,
) -> mod_api::PackageInfo {
    let mut details = mod_api::PackageDetails::default();
    details.name.clone_from(&manifest.name);
    details.description.clone_from(&manifest.description);
    details.authors.clone_from(&manifest.authors);
    details.license.clone_from(&manifest.license);
    details.homepage.clone_from(&manifest.homepage);
    details.issues.clone_from(&manifest.issues);
    details.icon_path = has_icon.then(|| "icon.png".to_owned());
    details.runtime = runtime;
    details.properties = manifest
        .properties
        .iter()
        .map(|(key, value)| {
            let value = match value {
                Property::Boolean(value) => mod_api::PropertyValue::Boolean(*value),
                Property::Integer(value) => mod_api::PropertyValue::Integer(*value),
                Property::Float(value) => mod_api::PropertyValue::Float(*value),
                Property::String(value) => mod_api::PropertyValue::String(value.clone()),
            };
            (key.clone(), value)
        })
        .collect();
    mod_api::PackageInfo::new(
        manifest.id.clone(),
        manifest.version.clone(),
        kind,
        manifest.side,
        std::iter::once(&manifest.id)
            .chain(&manifest.namespaces)
            .cloned()
            .collect(),
        horizon_mod_package::dependencies::from_manifest(&manifest.dependencies)
            .into_iter()
            .map(|d| mod_api::PackageDependency::new(d.id, d.requirement.to_string(), d.optional))
            .collect(),
    )
    .with_details(details)
}
impl TryIntoApi for horizon_mod_package::manifest::Package {
    type Api = mod_api::PackageInfo;
    type Error = std::convert::Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let (kind, runtime) = if self.manifest.wasm.is_some() {
            (
                mod_api::PackageKind::Code,
                Some(mod_api::PackageRuntime::Wasm),
            )
        } else {
            (mod_api::PackageKind::Content, None)
        };
        Ok(package_snapshot(
            &self.manifest,
            kind,
            runtime,
            self.icon.is_some(),
        ))
    }
}
impl TryIntoApi for horizon_mod_package::packages::AdjacentManifest {
    type Api = mod_api::PackageInfo;
    type Error = std::convert::Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(package_snapshot(
            &self.manifest,
            mod_api::PackageKind::Code,
            Some(mod_api::PackageRuntime::Native),
            self.has_icon,
        ))
    }
}

pub(in crate::game) fn install_packages(
    native: &[horizon_mod_package::packages::AdjacentManifest],
    content: &[horizon_mod_package::selection::SelectedPackage],
) -> Result<(), horizon_engine_slots::PlatformRefusal> {
    let native: Vec<_> = native.iter().map(|p| p.try_into_api().unwrap()).collect();
    let content: Vec<_> = content
        .iter()
        .map(|p| p.package.try_into_api().unwrap())
        .collect();
    (horizon_engine_slots::platform()?.install_packages)(&native, &content);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(extra: &str) -> Manifest {
        Manifest::parse(&format!(
            "id='test'\nname='Test'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n{extra}"
        ))
        .unwrap()
    }

    #[test]
    fn package_metadata_keeps_declared_side_and_namespace_order() {
        for (side, expected) in [
            ("client", mod_api::Side::Client),
            ("both", mod_api::Side::Both),
        ] {
            let snapshot = package_snapshot(
                &manifest(&format!("side='{side}'\n")),
                mod_api::PackageKind::Content,
                None,
                false,
            );
            assert_eq!(snapshot.side(), expected);
            assert_eq!(snapshot.namespaces().collect::<Vec<_>>(), ["test"]);
            assert_eq!(snapshot.kind(), mod_api::PackageKind::Content);
        }
    }

    #[test]
    fn package_metadata_carries_manifest_details_dependencies_and_properties() {
        let manifest = manifest(
            "homepage='https://example.org'\n\
             [dependencies]\nother = { version = '^1', optional = true }\n\
             [properties]\ntier = 3\n",
        );
        let info = package_snapshot(
            &manifest,
            mod_api::PackageKind::Code,
            Some(mod_api::PackageRuntime::Wasm),
            true,
        );
        assert_eq!(info.name(), "Test");
        assert_eq!(info.homepage(), Some("https://example.org"));
        assert_eq!(info.issues(), None);
        assert_eq!(info.icon_path(), Some("icon.png"));
        assert_eq!(info.runtime(), Some(mod_api::PackageRuntime::Wasm));
        let [dependency] = info.dependencies() else {
            panic!("one dependency")
        };
        assert_eq!(
            (dependency.id(), dependency.version(), dependency.optional()),
            ("other", "^1", true)
        );
        assert_eq!(
            info.property("tier"),
            Some(&mod_api::PropertyValue::Integer(3))
        );
        assert_eq!(info.property("missing"), None);
    }

    #[test]
    fn a_native_package_and_a_content_package_convert_with_their_runtime() {
        let native = horizon_mod_package::packages::AdjacentManifest {
            path: std::path::PathBuf::from("horizon-native-mods/test"),
            manifest: manifest("[properties]\ntier = 3\n"),
            has_icon: true,
        };
        let info = native.try_into_api().unwrap();
        assert_eq!(info.kind(), mod_api::PackageKind::Code);
        assert_eq!(info.runtime(), Some(mod_api::PackageRuntime::Native));
        assert_eq!(info.icon_path(), Some("icon.png"));
        assert_eq!(
            info.property("tier"),
            Some(&mod_api::PropertyValue::Integer(3))
        );
        let content = horizon_mod_package::manifest::Package {
            manifest: manifest(""),
            icon: None,
            layouts: Default::default(),
        };
        let info = content.try_into_api().unwrap();
        assert_eq!(info.kind(), mod_api::PackageKind::Content);
        assert_eq!(info.runtime(), None);
        assert_eq!(info.icon_path(), None);
    }
}
