use crate::game::native_entity_refs::tests::with_client_game;
use mod_api::ConfigKind::Common;
use mod_api::{
    ConfigEvent, ConfigSnapshot, ConfigSpec, Hooks, ModEntry, Mods, Registrar, RegistrationContext,
};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use sunrise_gameplay_common::game::native_configs::{NativeConfigs, NativeConfigsGameExt1 as _};

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "sunrise-native-configs-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A registration's initial callback that edits the next registration's file: the next one is
/// read after that edit, as FML's `ConfigTracker.loadConfigs` opens each registration only
/// when its turn comes.
#[test]
#[ignore = "daily"]
fn an_initial_callback_that_edits_the_next_file_is_seen_by_it() {
    let root = Folder::new("sequential");
    let config = root.0.join("config");
    fs::create_dir_all(&config).unwrap();
    let target = config.join("b-common.toml");
    let seen = Arc::new(Mutex::new(Vec::new()));
    struct First(PathBuf);
    #[mod_api::__subscriptions]
    impl Hooks for First {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let target = self.0.clone();
            r.register_config(
                ConfigSpec::new(Common, "a = 1\n"),
                Some(Arc::new(move |_: ConfigEvent<'_>, _: &ConfigSnapshot| {
                    fs::write(&target, "b = 2\n").unwrap();
                    Ok(())
                })),
            )
            .unwrap();
        }
    }
    struct Second(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Second {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let seen = self.0.clone();
            r.register_config(
                ConfigSpec::new(Common, "b = 1\n"),
                Some(Arc::new(
                    move |_: ConfigEvent<'_>, snapshot: &ConfigSnapshot| {
                        seen.lock().unwrap().push(snapshot.text.clone());
                        Ok(())
                    },
                )),
            )
            .unwrap();
        }
    }
    let mods: Mods = vec![
        ModEntry::new("a", Box::new(First(target.clone()))),
        ModEntry::new("b", Box::new(Second(seen.clone()))),
    ]
    .into();
    fs::write(&target, "b = 1\n").unwrap();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        with_client_game(|game| {
            game.config_root = std::sync::Arc::from(config.as_path());
            game.native_configs = NativeConfigs::default();
            game.native_configs_load(&[Common], None);
        });
    });
    assert_eq!(*seen.lock().unwrap(), ["b = 2\n"]);
}
