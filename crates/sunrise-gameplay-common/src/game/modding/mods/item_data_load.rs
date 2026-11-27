use super::*;
use sunrise_world::native_load_context::SettingsCapture;

type SettingsSnapshot = Arc<[(String, Arc<[(String, String)]>)]>;

/// Whether this owning load belongs to registration's startup publication.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OwningLoad {
    Startup,
    Later,
}

/// Execution side and settings ownership selected at the caller's load boundary.
#[derive(Clone, Copy)]
pub struct LoadRole {
    pub execution: ExecutionRole,
    pub owning: OwningLoad,
}

impl LoadRole {
    pub fn startup(execution: ExecutionRole) -> Self {
        Self {
            execution,
            owning: OwningLoad::Startup,
        }
    }
}

impl From<ExecutionRole> for LoadRole {
    fn from(execution: ExecutionRole) -> Self {
        Self {
            execution,
            owning: OwningLoad::Later,
        }
    }
}

impl OwningLoad {
    /// The settings one load captures: the startup load keeps registration's publication and
    /// reads only the registered keys it lacks, a later load reads the files again.
    fn capture(
        self,
        settings_root: &Path,
        ids: &[&str],
        registered: &[(&str, &str)],
        registration: Option<SettingsSnapshot>,
    ) -> SettingsCapture {
        if let Self::Startup = self
            && let Some(settings) = registration
        {
            return SettingsCapture::from_registration(
                settings_root,
                ids.iter().copied(),
                settings,
                registered.iter().copied(),
            );
        }
        SettingsCapture::new(
            settings_root,
            ids.iter().copied(),
            registered.iter().copied(),
        )
    }
}

/// Capture a load's settings at the resource-generation boundary, once, before anything that
/// reads them is built: the reload listeners registered for the generation and its item data
/// are given this one capture (`load_item_data`), so none of them reads a file the others
/// did not.
pub fn capture_load_settings(
    root: &Path,
    role: ExecutionRole,
    owning_load: OwningLoad,
) -> sunrise_game_data::Result<SettingsCapture> {
    let inventory = sunrise_mod_package::enabled::EnabledMods::read(root)?;
    // EnabledMods yields ids in sorted order. Keep this generation scoped to its side.
    let ids: Vec<_> = if role.is_logical_server() {
        inventory.server_ids().collect()
    } else {
        inventory.client_ids().collect()
    };
    let registered: Vec<_> = crate::game::native_hooks::mod_settings::registered_setting_keys()
        .filter(|(owner, _)| ids.binary_search(owner).is_ok())
        .collect();
    Ok(owning_load.capture(
        &crate::game::mod_data_root(),
        &ids,
        &registered,
        mod_api::__private::RegistrationData::initial_settings_snapshot(),
    ))
}

/// Load the item data over a settings capture taken at the generation boundary.
pub fn load_item_data(
    root: &Path,
    role: ExecutionRole,
    capture: SettingsCapture,
) -> sunrise_game_data::Result<ItemData> {
    let inputs = LoadInputs::new(builtin_registry_names(root)?, crate::game::mod_data_root());
    let context = LoadContext::with_capture(role.data_load_origin(), Arc::new(inputs), capture);
    Ok(ItemData::load(root, context)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_keeps_registration_settings_and_later_load_captures_changes() {
        let directory = tempfile::tempdir().unwrap();
        let reader = directory.path().join("reader");
        std::fs::create_dir(&reader).unwrap();
        std::fs::write(reader.join("count"), "3").unwrap();
        let settings: SettingsSnapshot =
            sunrise_world::native_load_context::capture_settings(directory.path(), ["reader"])
                .into_iter()
                .map(|(id, values)| (id, values.into_iter().collect()))
                .collect();
        let make = |role: OwningLoad, ids: &[&str], registration: Option<SettingsSnapshot>| {
            role.capture(directory.path(), ids, &[], registration)
        };
        assert_eq!(
            make(OwningLoad::Startup, &["reader"], Some(settings.clone()))
                .setting("reader", "count"),
            Some("3")
        );
        std::fs::write(reader.join("count"), "7").unwrap();
        assert_eq!(
            make(OwningLoad::Later, &["reader"], Some(settings.clone())).setting("reader", "count"),
            Some("7")
        );
        // Another initial consumer still has the registration publication.
        assert_eq!(
            make(OwningLoad::Startup, &["reader"], Some(settings.clone()))
                .setting("reader", "count"),
            Some("3")
        );
        assert_eq!(
            make(OwningLoad::Later, &["reader"], Some(settings.clone()))
                .setting("reader", "absent"),
            None
        );
        for role in [OwningLoad::Startup, OwningLoad::Later] {
            let capture = make(role, &["silent"], Some(settings.clone()));
            assert_eq!(capture.setting("silent", "count"), None);
            assert_eq!(capture.setting("reader", "count"), None);
        }
        let fallback = make(OwningLoad::Startup, &["reader"], None);
        assert_eq!(fallback.setting("reader", "count"), Some("7"));
    }
}
