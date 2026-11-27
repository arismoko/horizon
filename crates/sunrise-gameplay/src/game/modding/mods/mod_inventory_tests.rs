use super::*;
use mod_api::Hooks;
#[cfg(test)]
use mod_api::ModEntry;
#[cfg(test)]
use mod_platform::testing::with_mods;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
use sunrise_mod_package::selection::SelectedPackage;

pub struct EmptyCode;
#[mod_api::__subscriptions]
impl Hooks for EmptyCode {}

#[cfg(test)]
pub(in crate::game) fn selected(fixture: &Fixture, id: &str, side: &str) -> SelectedPackage {
    let manifest = format!(
        "id='{id}'\nname='{id}'\nversion='1'\napi='0.2'\nside='{side}'\nnamespaces=['shared_space']\n"
    );
    let path = fixture.0.join(format!("{id}.smod"));
    std::fs::write(
        &path,
        sunrise_mod_package::Package::write_files(&manifest, &[]).unwrap(),
    )
    .unwrap();
    SelectedPackage {
        package: sunrise_mod_package::Package::read(&path).unwrap(),
        path,
    }
}

#[cfg(test)]
#[test]
fn admission_uses_active_native_entries_and_accepted_content_not_startup_metadata() {
    let fixture = Fixture::new("inventory-admission");
    smod(&fixture.0, "machine_sample");
    let sources = ModsState {
        native: Arc::new(fixture.runtime()),
        selected: vec![
            selected(&fixture, "plain", "both"),
            selected(&fixture, "refused_content", "both"),
        ]
        .into(),
        ..Default::default()
    };
    with_mods(
        vec![ModEntry::new("machine_sample", Box::new(EmptyCode))].into(),
        || {
            mod_platform::__private::with_test_packages(&["refused_code", "shared_space"], || {
                assert!(
                    mod_api::__private::loaded_mods()
                        .get("refused_code")
                        .is_some()
                );
                let mut startup = sources.clone();
                let disabled = std::collections::BTreeSet::from(["machine_sample".into()]);
                startup.capture_code_switches(&disabled);
                assert!(mod_api::__private::mod_index_on(
                    mod_api::__private::mod_index("machine_sample").unwrap()
                ));
                assert_eq!(
                    startup
                        .enabled_inventory(|id| id == "plain")
                        .server_ids()
                        .collect::<Vec<_>>(),
                    ["plain"]
                );
                startup.apply_code_switches(&disabled);
                assert!(mod_api::__private::set_mod_enabled("machine_sample", true));
                assert_eq!(
                    startup
                        .enabled_inventory(|id| id == "plain")
                        .server_ids()
                        .collect::<Vec<_>>(),
                    ["machine_sample", "plain"]
                );
                let initial = sources.enabled_inventory(|id| id == "plain");
                assert_eq!(
                    initial.client_ids().collect::<Vec<_>>(),
                    ["machine_sample", "plain"]
                );
                assert_eq!(
                    initial.server_ids().collect::<Vec<_>>(),
                    ["machine_sample", "plain"]
                );
                assert!(mod_api::__private::set_mod_enabled("machine_sample", false));
                let paused = sources.enabled_inventory(|id| id == "plain");
                assert_eq!(paused.server_ids().collect::<Vec<_>>(), ["plain"]);
                assert_eq!(paused.client_ids().collect::<Vec<_>>(), ["plain"]);
                assert_eq!(
                    initial.server_ids().collect::<Vec<_>>(),
                    ["machine_sample", "plain"]
                );
            });
        },
    );
}

pub struct Guard(pub Arc<AtomicBool>);
impl mod_api::ServerReloadListener for Guard {
    fn prepare(
        &self,
        _: &mod_api::ServerReloadContext,
    ) -> std::result::Result<Box<dyn FnOnce() + Send + 'static>, mod_api::ReloadResourceError> {
        if self.0.load(Ordering::Relaxed) {
            return Err(mod_api::ReloadResourceError::new(
                mod_api::Identifier::parse("reload_guard:guard").unwrap(),
                std::io::Error::other("inventory fixture rejected the new generation"),
            ));
        }
        Ok(Box::new(|| {}))
    }
}
#[mod_api::__subscriptions]
impl Hooks for Guard {
    fn add_server_reload_listeners(
        &self,
        _: &dyn mod_api::ReloadContext,
        event: &mut mod_api::AddServerReloadListenersEvent<'_>,
    ) {
        event
            .listeners
            .add_listener(
                mod_api::Identifier::parse("reload_guard:guard").unwrap(),
                mod_api::FileToIdConverter::json("inventory_test").unwrap(),
                Arc::new(Guard(self.0.clone())),
            )
            .unwrap();
    }
}

/// Capture metadata uses the same typed runtime provenance as loading.
#[cfg(test)]
#[test]
fn capture_metadata_distinguishes_content_native_and_unknown_code() {
    use sunrise_perf_format::v1::ModKind;
    let fixture = Fixture::new("perf-inventory");
    smod(&fixture.0, "machine_sample");
    let sources = ModsState {
        native: Arc::new(fixture.runtime()),
        ..Default::default()
    };
    let inventory: Vec<_> = [
        ("plain", mod_api::PackageKind::Content),
        ("machine_sample", mod_api::PackageKind::Code),
        ("builtin_code", mod_api::PackageKind::Code),
    ]
    .into_iter()
    .map(|(id, kind)| {
        mod_api::PackageInfo::new(
            id.to_owned(),
            "2.3".to_owned(),
            kind,
            mod_api::Side::Both,
            vec![id.to_owned()],
            vec![],
        )
    })
    .collect();
    let metadata = sources.perf_metadata(inventory.iter());
    assert_eq!(
        metadata
            .iter()
            .map(|m| (m.id.as_str(), m.version.as_str(), m.kind))
            .collect::<Vec<_>>(),
        vec![
            ("plain", "2.3", ModKind::Content as i32),
            ("machine_sample", "2.3", ModKind::Native as i32),
            ("builtin_code", "2.3", ModKind::Unspecified as i32),
        ]
    );
    assert!(metadata.iter().all(|m| m.crate_name.is_empty()));
}
