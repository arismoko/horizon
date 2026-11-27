use crate::game::native_entity_refs::tests::with_game;
use mod_api::ConfigKind::Server;
use mod_api::{
    ConfigEvent, ConfigListener, ConfigSnapshot, ConfigSpec, Hooks, HostSide, ModEntry, Mods,
    Registrar, RegistrationContext,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use sunrise_gameplay_common::game::native_configs::NativeConfigsGameExt1 as _;
use sunrise_gameplay_common::game::native_hooks::mod_settings::ModSettingsGameExt as _;
use sunrise_gameplay_common::game::test_fixtures::run_fresh_cases;

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

#[test]
#[ignore = "daily"]
fn panicking_config_hooks_keep_live_writes_and_do_not_stop_later_listeners() {
    use mod_api::{__private::take_hook_panics, SettingSpec, SettingValue};

    if run_fresh_cases(
        "game::native_configs::lifecycle_tests_server::panicking_config_hooks_keep_live_writes_and_do_not_stop_later_listeners",
        "SUNRISE_CONFIG_PANIC_CHILD",
        &["server"],
    ) {
        return;
    }

    struct Broken(Arc<Mutex<Vec<(String, SettingValue)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Broken {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            for key in ["reload", "unload"] {
                let setting = r
                    .register_setting(SettingSpec::toggle(key, format!("fault.{key}"), false))
                    .unwrap();
                r.register_config(
                    ConfigSpec::new(Server, "enabled = true").file(format!("fault-{key}.toml")),
                    Some(Arc::new(
                        move |event: ConfigEvent<'_>, _: &ConfigSnapshot| {
                            let host = match event {
                                ConfigEvent::Reloading(HostSide::Server(host))
                                    if key == "reload" =>
                                {
                                    host
                                }
                                ConfigEvent::Unloading(host) if key == "unload" => host,
                                _ => return Ok(()),
                            };
                            assert_eq!(
                                host.set_setting(setting, SettingValue::Toggle(true)),
                                Ok(true)
                            );
                            panic!("broken {key} listener");
                        },
                    )),
                )
                .unwrap();
            }
        }

        fn setting_changed(
            &self,
            host: &dyn mod_api::Host,
            event: &mut mod_api::SettingChangedEvent<'_>,
        ) {
            let _ = host;
            self.0
                .lock()
                .unwrap()
                .push((event.key.to_owned(), event.value));
        }
    }

    struct Later(Arc<Mutex<Vec<&'static str>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Later {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let calls = self.0.clone();
            r.register_config(
                ConfigSpec::new(Server, "enabled = true"),
                Some(Arc::new(
                    move |event: ConfigEvent<'_>, _: &ConfigSnapshot| {
                        let name = match event {
                            ConfigEvent::Reloading(_) => "reload",
                            ConfigEvent::Unloading(_) => "unload",
                            _ => return Ok(()),
                        };
                        calls.lock().unwrap().push(name);
                        Ok(())
                    },
                )),
            )
            .unwrap();
        }
    }

    let observed = Arc::new(Mutex::new(Vec::new()));
    let changed = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![
        ModEntry::new("fault", Box::new(Broken(changed.clone()))),
        ModEntry::new("later", Box::new(Later(observed.clone()))),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let _ = take_hook_panics();
            let config_dir = game.config_root.clone();
            let reload = mod_api::__private::setting_key_by_name("fault", "reload").unwrap();
            let unload = mod_api::__private::setting_key_by_name("fault", "unload").unwrap();
            for (text, expected) in [("first", true), ("second", false)] {
                assert_eq!(
                    game.set_setting(reload, SettingValue::Toggle(false)),
                    Ok(!expected)
                );
                assert_eq!(
                    game.set_setting(unload, SettingValue::Toggle(false)),
                    Ok(!expected)
                );
                game.native_configs_server_start(Path::new("test-world"), false);
                fs::write(config_dir.join("fault-reload.toml"), text).unwrap();
                fs::write(config_dir.join("later-server.toml"), text).unwrap();
                game.native_configs.reset_poll_deadline_for_test();
                game.native_configs_poll();
                game.native_configs_server_stop();
                for (name, key) in [("reload", reload), ("unload", unload)] {
                    assert_eq!(game.setting(key), Ok(SettingValue::Toggle(expected)));
                    assert_eq!(
                        fs::read_to_string(game.native_settings_root().join("fault").join(name))
                            .unwrap(),
                        expected.to_string()
                    );
                }
            }
            assert_eq!(
                *observed.lock().unwrap(),
                ["reload", "unload", "reload", "unload"]
            );
            assert_eq!(
                changed.lock().unwrap().as_slice(),
                [
                    ("reload".to_owned(), SettingValue::Toggle(true)),
                    ("unload".to_owned(), SettingValue::Toggle(true)),
                    ("reload".to_owned(), SettingValue::Toggle(false)),
                    ("unload".to_owned(), SettingValue::Toggle(false)),
                ]
            );
            let notices = take_hook_panics();
            assert_eq!(notices.len(), 2);
            assert!(notices.iter().all(|notice| notice.mod_id == "fault"));
            assert_eq!(
                notices.iter().map(|notice| notice.hook).collect::<Vec<_>>(),
                ["config_reloading", "config_unloading"]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn owner_bound_config_lifecycle_isolates_owners_refusals_and_panics() {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Moment {
        Loading,
        Reloading,
        Unloading,
    }
    fn moment(event: ConfigEvent<'_>) -> Moment {
        match event {
            ConfigEvent::Loading(_) => Moment::Loading,
            ConfigEvent::Reloading(HostSide::Server(_)) => Moment::Reloading,
            ConfigEvent::Unloading(_) => Moment::Unloading,
            _ => panic!("expected server config callback"),
        }
    }
    type TraceRow = (&'static str, Moment, PathBuf, String);

    let tag = format!("p3-25-lifecycle-{}", std::process::id());
    let alpha_first_rel = PathBuf::from(&tag).join("alpha-first.toml");
    let alpha_second_rel = PathBuf::from(&tag).join("alpha-second.toml");
    let beta_rel = PathBuf::from(&tag).join("beta.toml");

    let world_one = Folder::new("daily-w1");
    let world_two = Folder::new("daily-w2");
    let root = Folder::new("owner-config-root");
    let config_dir = root.0.join("config");

    let override_path = world_one.0.join("serverconfig").join(&alpha_second_rel);
    fs::create_dir_all(override_path.parent().unwrap()).unwrap();
    fs::write(&override_path, "second = override\n").unwrap();

    struct Alpha {
        first: PathBuf,
        second: PathBuf,
        trace: Arc<Mutex<Vec<TraceRow>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Alpha {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let trace = self.trace.clone();
            let first_listener: Arc<ConfigListener> =
                Arc::new(move |event: ConfigEvent<'_>, snapshot: &ConfigSnapshot| {
                    let event = moment(event);
                    assert_eq!(mod_api::content::crash::current_native_mod(), Some("alpha"));
                    assert_eq!(snapshot.mod_id, "alpha");
                    trace.lock().unwrap().push((
                        "alpha-first",
                        event,
                        snapshot.path.clone(),
                        snapshot.text.clone(),
                    ));
                    if event == Moment::Reloading {
                        return Err("refused reload".into());
                    }
                    Ok(())
                });
            r.register_config(
                ConfigSpec::new(Server, "alpha = 1\n").file(self.first.clone()),
                Some(first_listener),
            )
            .unwrap();

            let trace = self.trace.clone();
            let second_listener: Arc<ConfigListener> =
                Arc::new(move |event: ConfigEvent<'_>, snapshot: &ConfigSnapshot| {
                    let event = moment(event);
                    assert_eq!(mod_api::content::crash::current_native_mod(), Some("alpha"));
                    assert_eq!(snapshot.mod_id, "alpha");
                    trace.lock().unwrap().push((
                        "alpha-second",
                        event,
                        snapshot.path.clone(),
                        snapshot.text.clone(),
                    ));
                    Ok(())
                });
            r.register_config(
                ConfigSpec::new(Server, "second = 1\n").file(self.second.clone()),
                Some(second_listener),
            )
            .unwrap();
        }
    }

    struct Beta {
        file: PathBuf,
        trace: Arc<Mutex<Vec<TraceRow>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Beta {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let trace = self.trace.clone();
            let listener: Arc<ConfigListener> =
                Arc::new(move |event: ConfigEvent<'_>, snapshot: &ConfigSnapshot| {
                    let event = moment(event);
                    assert_eq!(mod_api::content::crash::current_native_mod(), Some("beta"));
                    assert_eq!(snapshot.mod_id, "beta");
                    trace.lock().unwrap().push((
                        "beta",
                        event,
                        snapshot.path.clone(),
                        snapshot.text.clone(),
                    ));
                    if event == Moment::Reloading {
                        panic!("beta reload boom");
                    }
                    Ok(())
                });
            r.register_config(
                ConfigSpec::new(Server, "beta = 1\n").file(self.file.clone()),
                Some(listener),
            )
            .unwrap();
        }
    }

    let trace = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![
        ModEntry::new(
            "alpha",
            Box::new(Alpha {
                first: alpha_first_rel.clone(),
                second: alpha_second_rel.clone(),
                trace: trace.clone(),
            }),
        ),
        ModEntry::new(
            "beta",
            Box::new(Beta {
                file: beta_rel.clone(),
                trace: trace.clone(),
            }),
        ),
    ]
    .into();

    let alpha_first_path = config_dir.join(&alpha_first_rel);
    let alpha_second_base = config_dir.join(&alpha_second_rel);
    let beta_path = config_dir.join(&beta_rel);

    mod_platform::testing::with_mods(mods, || {
        let _ = mod_api::__private::take_hook_panics();
        with_game(|game| {
            game.config_root = config_dir.clone().into();

            // World 1: loads in declaration order; world override wins without writing a base file.
            game.native_configs_server_start(&world_one.0, false);
            assert!(!alpha_second_base.exists());
            assert_eq!(
                std::mem::take(&mut *trace.lock().unwrap()),
                [
                    (
                        "alpha-first",
                        Moment::Loading,
                        alpha_first_path.clone(),
                        "alpha = 1\n".into(),
                    ),
                    (
                        "alpha-second",
                        Moment::Loading,
                        override_path.clone(),
                        "second = override\n".into(),
                    ),
                    (
                        "beta",
                        Moment::Loading,
                        beta_path.clone(),
                        "beta = 1\n".into(),
                    ),
                ]
            );

            // Consecutive edits whose listener returns Err must keep delivering only to that owner.
            for text in ["alpha = 2\n", "alpha = 3\n"] {
                fs::write(&alpha_first_path, text).unwrap();
                game.native_configs.reset_poll_deadline_for_test();
                game.native_configs_poll();
                assert_eq!(
                    std::mem::take(&mut *trace.lock().unwrap()),
                    [(
                        "alpha-first",
                        Moment::Reloading,
                        alpha_first_path.clone(),
                        text.into(),
                    )]
                );
            }
            assert!(mod_api::__private::take_hook_panics().is_empty());

            // Editing the world override notifies only its owning registration with the override path.
            fs::write(&override_path, "second = override2\n").unwrap();
            game.native_configs.reset_poll_deadline_for_test();
            game.native_configs_poll();
            assert_eq!(
                std::mem::take(&mut *trace.lock().unwrap()),
                [(
                    "alpha-second",
                    Moment::Reloading,
                    override_path.clone(),
                    "second = override2\n".into(),
                )]
            );

            // A real panic records a notice naming that mod and disables only that registration.
            fs::write(&beta_path, "beta = 2\n").unwrap();
            game.native_configs.reset_poll_deadline_for_test();
            game.native_configs_poll();
            assert_eq!(
                std::mem::take(&mut *trace.lock().unwrap()),
                [(
                    "beta",
                    Moment::Reloading,
                    beta_path.clone(),
                    "beta = 2\n".into(),
                )]
            );
            let notices = mod_api::__private::take_hook_panics();
            assert!(
                notices
                    .iter()
                    .any(|notice| { notice.mod_id == "beta" && notice.hook == "config_reloading" }),
                "missing panic notice for beta: {notices:?}"
            );

            // Further edits still reach alpha-first while poisoned beta stays silent.
            fs::write(&beta_path, "beta = 3\n").unwrap();
            fs::write(&alpha_first_path, "alpha = 4\n").unwrap();
            game.native_configs.reset_poll_deadline_for_test();
            game.native_configs_poll();
            assert_eq!(
                std::mem::take(&mut *trace.lock().unwrap()),
                [(
                    "alpha-first",
                    Moment::Reloading,
                    alpha_first_path.clone(),
                    "alpha = 4\n".into(),
                )]
            );
            assert!(mod_api::__private::take_hook_panics().is_empty());

            // Unload reaches only live registrations in order.
            game.native_configs_server_stop();
            assert_eq!(
                std::mem::take(&mut *trace.lock().unwrap()),
                [
                    (
                        "alpha-first",
                        Moment::Unloading,
                        alpha_first_path.clone(),
                        "alpha = 4\n".into(),
                    ),
                    (
                        "alpha-second",
                        Moment::Unloading,
                        override_path.clone(),
                        "second = override2\n".into(),
                    ),
                ]
            );

            // World 2 has no override: alpha-second creates its base default, while poisoned beta
            // remains disabled across worlds.
            game.native_configs_server_start(&world_two.0, false);
            assert_eq!(
                std::mem::take(&mut *trace.lock().unwrap()),
                [
                    (
                        "alpha-first",
                        Moment::Loading,
                        alpha_first_path.clone(),
                        "alpha = 4\n".into(),
                    ),
                    (
                        "alpha-second",
                        Moment::Loading,
                        alpha_second_base.clone(),
                        "second = 1\n".into(),
                    ),
                ]
            );
            game.native_configs_server_stop();
            assert_eq!(
                std::mem::take(&mut *trace.lock().unwrap()),
                [
                    (
                        "alpha-first",
                        Moment::Unloading,
                        alpha_first_path.clone(),
                        "alpha = 4\n".into(),
                    ),
                    (
                        "alpha-second",
                        Moment::Unloading,
                        alpha_second_base.clone(),
                        "second = 1\n".into(),
                    ),
                ]
            );
        });
    });
}
