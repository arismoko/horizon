#![cfg(test)]
#[cfg(all(test, feature = "client"))]
pub(crate) mod tests {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use mod_api::{
        ConfigEvent,
        ConfigKind::{self, Client, Common, Server},
        ConfigListener, ConfigSnapshot, ConfigSpec, Hooks, ModEntry, Mods, Registrar,
        RegistrationContext, SettingSpec, SettingValue,
    };
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::{
        native_configs::NativeConfigsGameExt1 as _, option_paths::game_dir,
        test_fixtures::run_fresh_cases,
    };

    #[test]
    #[ignore = "daily"]
    fn config_permissions_and_once_per_side_survive_two_worlds() {
        if run_fresh_cases(
            concat!(
                module_path!(),
                "::config_permissions_and_once_per_side_survive_two_worlds"
            )
            .split_once("::")
            .unwrap()
            .1,
            "SUNRISE_CONFIG_LIFECYCLE_CHILD",
            &["lifecycle"],
        ) {
            return;
        }

        struct Probe(Arc<Mutex<Vec<(bool, ConfigKind)>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Probe {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let key = r
                    .register_setting(SettingSpec::toggle(
                        "unloaded",
                        "phase_probe.unloaded",
                        false,
                    ))
                    .unwrap();
                for kind in [Client, Common, Server] {
                    let calls = self.0.clone();
                    let listener: Arc<ConfigListener> =
                        Arc::new(move |event: ConfigEvent<'_>, config: &ConfigSnapshot| {
                            match event {
                                ConfigEvent::Loading(context) => {
                                    assert_eq!(
                                        mod_api::content::crash::current_native_mod(),
                                        Some("phase_probe")
                                    );
                                    assert_eq!(context.config_dir(), game_dir().join("config"));
                                    assert!(matches!(
                                        context.setting(key),
                                        Ok(SettingValue::Toggle(_))
                                    ));
                                    calls.lock().unwrap().push((true, config.kind));
                                }
                                ConfigEvent::Unloading(host) => {
                                    let SettingValue::Toggle(current) = host.setting(key).unwrap()
                                    else {
                                        panic!("expected a toggle")
                                    };
                                    assert_eq!(
                                        host.set_setting(key, SettingValue::Toggle(!current)),
                                        Ok(true)
                                    );
                                    calls.lock().unwrap().push((false, config.kind));
                                }
                                _ => {}
                            }
                            Ok(())
                        });
                    r.register_config(ConfigSpec::new(kind, "enabled=true"), Some(listener))
                        .unwrap();
                }
            }
        }
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mods: Mods = vec![ModEntry::new("phase_probe", Box::new(Probe(calls.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                game.native_configs_client_start();
                game.native_configs_client_start();
            });
            with_game(|game| {
                let key =
                    mod_api::__private::setting_key_by_name("phase_probe", "unloaded").unwrap();
                assert_eq!(game.setting(key), Ok(SettingValue::Toggle(false)));
                for (world, expected) in [("phase-world-one", true), ("phase-world-two", false)] {
                    game.native_configs_server_start(Path::new(world), false);
                    game.native_configs_server_stop();
                    assert_eq!(game.setting(key), Ok(SettingValue::Toggle(expected)));
                }
            });
        });
        assert_eq!(
            *calls.lock().unwrap(),
            [
                (true, Client),
                (true, Common),
                (true, Server),
                (false, Server),
                (true, Server),
                (false, Server)
            ]
        );
    }
}
