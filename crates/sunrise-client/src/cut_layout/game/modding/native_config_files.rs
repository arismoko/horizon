#![cfg(test)]
#[cfg(feature = "client")]
use std::fs;
#[cfg(test)]
pub(crate) mod tests {

    #[cfg(feature = "client")]
    use super::*;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::native_configs::NativeConfigsGameExt1;

    /// A mod saves its registered config and a sibling file inside `ConfigContext::config_dir()`
    /// from `ConfigEvent::Loading`, and the reload poll hands the saved text back.
    /// Runs in a fresh process in a scratch folder since `config/` is beside the
    /// game's working folder.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_mod_saves_its_config_through_the_game_and_the_reload_hears_it() {
        use mod_api::{
            ConfigEvent, ConfigKind, ConfigListener, ConfigSnapshot, ConfigSpec, Hooks, HostSide,
            ModEntry, Mods, Registrar, RegistrationContext,
        };
        use std::sync::{Arc, Mutex};
        static HEARD: Mutex<Vec<String>> = Mutex::new(Vec::new());
        fn heard() -> std::sync::MutexGuard<'static, Vec<String>> {
            HEARD
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::native_config_files::tests::a_mod_saves_its_config_through_the_game_and_the_reload_hears_it",
            "SUNRISE_CONFIG_FILES_CHILD",
            &["game"],
        ) {
            return;
        }
        struct Proof;
        #[mod_api::__subscriptions]
        impl Hooks for Proof {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let listener: Arc<ConfigListener> =
                    Arc::new(|event: ConfigEvent<'_>, config: &ConfigSnapshot| {
                        match event {
                            ConfigEvent::Loading(context) => {
                                let config_dir = context.config_dir();
                                assert_eq!(config.path, config_dir.join("proof-common.toml"));
                                heard().push(format!("loading {}", config.text.trim()));
                                fs::write(&config.path, "count = 1\n").unwrap();
                                let state = config_dir.join("proof/state.bin");
                                fs::create_dir_all(state.parent().unwrap()).unwrap();
                                fs::write(&state, [1, 2, 3]).unwrap();
                                assert_eq!(fs::read(&state).unwrap(), [1, 2, 3]);
                            }
                            ConfigEvent::Reloading(HostSide::Client(host)) => {
                                assert_eq!(
                                    fs::read(host.config_dir().join("proof/state.bin")).unwrap(),
                                    [1, 2, 3]
                                );
                                heard().push(format!("reloading {}", config.text.trim()));
                            }
                            _ => {}
                        }
                        Ok(())
                    });
                r.register_config(
                    ConfigSpec::new(ConfigKind::Common, "count = 0\n"),
                    Some(listener),
                )
                .unwrap();
            }
        }
        let mods: Mods = vec![ModEntry::new("proof", Box::new(Proof))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                game.native_configs_client_start();
                game.native_configs_tick_client();
            });
        });
        assert_eq!(*heard(), ["loading count = 0", "reloading count = 1"]);
        let config = std::env::current_dir().unwrap().join("config");
        assert_eq!(fs::read(config.join("proof/state.bin")).unwrap(), [1, 2, 3]);
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn world_bulk_files_survive_save_reload_backup_and_world_deletion() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_game};
        use std::io::Read;
        use std::path::PathBuf;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use sunrise_gameplay::game::world_loading::WorldRequest;

        struct ScratchDir(PathBuf);
        impl ScratchDir {
            fn new(name: &str) -> Self {
                let root = std::env::temp_dir().join(format!(
                    "sunrise-config-storage-{name}-{}",
                    std::process::id()
                ));
                let _ = fs::remove_dir_all(&root);
                fs::create_dir_all(&root).unwrap();
                Self(root)
            }
        }
        impl Drop for ScratchDir {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        if run_fresh_cases(
            "cut_layout::game::server_side::native_config_files::tests::world_bulk_files_survive_save_reload_backup_and_world_deletion",
            "SUNRISE_BULK_FILES_CHILD",
            &["game"],
        ) {
            return;
        }
        static WARNINGS: AtomicUsize = AtomicUsize::new(0);
        struct SettingsLog;
        impl log::Log for SettingsLog {
            fn enabled(&self, _: &log::Metadata<'_>) -> bool {
                true
            }
            fn log(&self, record: &log::Record<'_>) {
                if record.level() == log::Level::Warn
                    && record.target() == "sunrise_world::native_load_context"
                {
                    WARNINGS.fetch_add(1, Ordering::Relaxed);
                }
            }
            fn flush(&self) {}
        }
        log::set_logger(&SettingsLog).unwrap();
        log::set_max_level(log::LevelFilter::Warn);
        let scratch = ScratchDir::new("world-lifecycle");
        let saves = scratch.0.join("saves");
        let world = saves.join("Bulk World");
        let request = |create: bool| {
            let rules = std::sync::Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla());
            let options = sunrise_sim::world_options::WorldOptions {
                world_type: sunrise_sim::save::WorldPreset::Flat,
                rules: sunrise_game_data::gamerules::GameRules::with_registry(rules.clone(), &[]),
                ..Default::default()
            };
            let mut request = WorldRequest::new(
                "Bulk World",
                Some("Bulk World"),
                Some(1),
                true,
                create.then_some(&options),
                rules,
            )
            .unwrap();
            request.save_root = world.clone();
            request
        };
        let mut instance_file = PathBuf::new();
        with_game(|game| {
            game.server.world_root.clear();
            let host: &dyn mod_api::ServerHost = game;
            assert_eq!(host.world_dir(), None);

            game.load_world(request(true)).unwrap();
            let host: &dyn mod_api::ServerHost = game;
            instance_file = host.instance_dir().join("sunrise-mod-data/fd/server-cache");
            fs::create_dir_all(instance_file.parent().unwrap()).unwrap();
            fs::write(&instance_file, b"game-cache").unwrap();
            let world_file = host
                .world_dir()
                .unwrap()
                .join("sunrise-mod-data/fd/tiles/nether/0.bin");
            fs::create_dir_all(world_file.parent().unwrap()).unwrap();
            fs::write(&world_file, b"world:42").unwrap();
            assert_eq!(fs::read(&world_file).unwrap(), b"world:42");
            let settings = sunrise_gameplay_common::game::mod_data_root();
            fs::create_dir_all(settings.join("fd")).unwrap();
            fs::write(settings.join("fd/enabled"), b"true").unwrap();
            WARNINGS.store(0, Ordering::Relaxed);
            let captured = sunrise_world::native_load_context::capture_settings(&settings, ["fd"]);
            assert_eq!(captured["fd"].len(), 1);
            assert_eq!(captured["fd"]["enabled"], "true");
            assert_eq!(WARNINGS.load(Ordering::Relaxed), 0);
            game.save_game().unwrap();
        });
        with_game(|game| {
            game.load_world(request(false)).unwrap();
            let host: &dyn mod_api::ServerHost = game;
            let tiles = host
                .world_dir()
                .unwrap()
                .join("sunrise-mod-data/fd/tiles/nether");
            assert_eq!(fs::read(tiles.join("0.bin")).unwrap(), b"world:42");
            let names: Vec<_> = fs::read_dir(&tiles)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            assert_eq!(names, [std::ffi::OsStr::new("0.bin")]);
        });
        let (backup, _) =
            sunrise_client::title::world_management::backup(&saves, "Bulk World").unwrap();
        let mut zip = zip::ZipArchive::new(fs::File::open(backup).unwrap()).unwrap();
        let mut bytes = Vec::new();
        zip.by_name("Bulk World/sunrise-mod-data/fd/tiles/nether/0.bin")
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, b"world:42");
        sunrise_client::title::world_management::delete(&saves, "Bulk World").unwrap();
        assert!(!world.exists());
        assert_eq!(fs::read(&instance_file).unwrap(), b"game-cache");
    }
}
