use crate::game::native_entity_refs::tests::with_client_game;
use mod_api::{
    Hooks, ModEntry, Registrar, RegistrationContext, SettingAccessError, SettingKey, SettingSpec,
    SettingValue,
};
use std::collections::BTreeMap;
use std::fs;
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use sunrise_gameplay_common::game::native_hooks::mod_settings::{
    ModSettingsGameExt as _, registered_setting_keys,
};
use sunrise_gameplay_common::game::test_fixtures::run_fresh_cases;
use sunrise_mod_package::mod_list::{Installed, NativePage, NativeSetting, NativeSettingValue};
use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};

struct RegisteredMod {
    toggle: OnceLock<SettingKey>,
    range: OnceLock<SettingKey>,
    choice: OnceLock<SettingKey>,
    color: OnceLock<SettingKey>,
    seen: Arc<Mutex<Vec<(String, SettingValue)>>>,
    panic_on_change: Arc<AtomicBool>,
}

impl RegisteredMod {
    fn new() -> Self {
        Self {
            toggle: OnceLock::new(),
            range: OnceLock::new(),
            choice: OnceLock::new(),
            color: OnceLock::new(),
            seen: Arc::new(Mutex::new(Vec::new())),
            panic_on_change: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[mod_api::__subscriptions]
impl Hooks for RegisteredMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let _ = self.toggle.set(
            r.register_setting(SettingSpec::toggle("on", "test.on", false))
                .unwrap(),
        );
        let _ = self.range.set(
            r.register_setting(SettingSpec::range(
                "scale",
                "test.scale",
                -2.0,
                2.0,
                0.5,
                0.0,
            ))
            .unwrap(),
        );
        let _ = self.choice.set(
            r.register_setting(SettingSpec::choice("mode", "test.mode", ["a", "b", "c"], 0))
                .unwrap(),
        );
        let _ = self.color.set(
            r.register_setting(SettingSpec::color("tint", "test.tint", 0x11_2233))
                .unwrap(),
        );
    }

    fn setting_changed(
        &self,
        host: &dyn mod_api::Host,
        event: &mut mod_api::SettingChangedEvent<'_>,
    ) {
        if self.panic_on_change.load(Ordering::Relaxed) {
            panic!("simulated setting_changed failure");
        }
        let _ = host;
        self.seen
            .lock()
            .unwrap()
            .push((event.key.to_owned(), event.value));
    }
}

struct OtherMod {
    toggle: OnceLock<SettingKey>,
    calls: Arc<AtomicUsize>,
}

#[mod_api::__subscriptions]
impl Hooks for OtherMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let _ = self.toggle.set(
            r.register_setting(SettingSpec::toggle("on", "other.on", true))
                .unwrap(),
        );
    }

    fn setting_changed(&self, _: &dyn mod_api::Host, _: &mut mod_api::SettingChangedEvent<'_>) {
        self.calls.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
#[ignore = "daily"]
fn single_owner_reads_writes_reloads_exact_f64_and_contains_callback_panics() {
    if run_fresh_cases(
        "game::native_hooks::mod_settings_tests::single_owner_reads_writes_reloads_exact_f64_and_contains_callback_panics",
        "SUNRISE_NATIVE_SETTINGS_CHILD",
        &["client"],
    ) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("settings");
    fs::create_dir_all(root.join("alpha")).unwrap();
    fs::write(root.join("alpha/on"), "true").unwrap();

    let alpha = RegisteredMod::new();
    let alpha_seen = alpha.seen.clone();
    let alpha_panic = alpha.panic_on_change.clone();
    let beta_calls = Arc::new(AtomicUsize::new(0));
    let beta = OtherMod {
        toggle: OnceLock::new(),
        calls: beta_calls.clone(),
    };

    let mods: mod_api::Mods = vec![
        ModEntry::new("alpha", Box::new(alpha)),
        ModEntry::new("beta", Box::new(beta)),
    ]
    .into();

    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        let alpha_on = mod_api::__private::setting_key_by_name("alpha", "on").unwrap();
        let alpha_scale = mod_api::__private::setting_key_by_name("alpha", "scale").unwrap();
        let alpha_mode = mod_api::__private::setting_key_by_name("alpha", "mode").unwrap();
        let alpha_tint = mod_api::__private::setting_key_by_name("alpha", "tint").unwrap();
        let beta_on = mod_api::__private::setting_key_by_name("beta", "on").unwrap();
        let foreign = mod_api::__private::setting_key("ghost", 999);

        with_client_game(|game| {
            game.client.settings.mods.installed = vec![Installed {
                id: "alpha".into(),
                title: "Alpha".into(),
                description: String::new(),
                problem: None,
                dev: false,
                native: Some(NativePage {
                    settings: vec![NativeSetting {
                        key: "on".into(),
                        label: "test.on".into(),
                        tooltip: None,
                        value: NativeSettingValue::Toggle(false),
                        category: None,
                    }],
                    ..Default::default()
                }),
            }];
            fn ui_row<'a>(game: &'a mut crate::game::ClientGame<'_>) -> &'a mut NativeSettingValue {
                &mut game.client.settings.mods.installed[0]
                    .native
                    .as_mut()
                    .unwrap()
                    .settings[0]
                    .value
            }
            // First capture establishes the persisted baseline without notifying.
            let load = LoadContext::new_with_registered(
                DataLoadOrigin::ClientPreload,
                Arc::new(LoadInputs::new(BTreeMap::new(), root.clone())),
                [],
                registered_setting_keys(),
            );
            game.common.loop_context = Arc::new(load.captured_loop_context(LoopSide::Client));
            game.sync_native_settings_from_loop_context();
            assert_eq!(*ui_row(game), NativeSettingValue::Toggle(true));
            assert_eq!(game.setting_bool(alpha_on), Ok(true));
            assert_eq!(game.setting_f64(alpha_scale), Ok(0.0));
            assert_eq!(game.setting_choice(alpha_mode), Ok(0));
            assert_eq!(game.setting_color(alpha_tint), Ok(0x11_2233));
            assert_eq!(game.setting_bool(beta_on), Ok(true));
            assert!(alpha_seen.lock().unwrap().is_empty());

            // Unknown key and wrong-kind reads return typed errors.
            assert_eq!(game.setting(foreign), Err(SettingAccessError::UnknownKey));
            assert_eq!(
                game.setting_f64(alpha_on),
                Err(SettingAccessError::WrongKind)
            );

            // Options > Mods sends its clicked row through the same persist-and-notify path.
            ui_row(game).click();
            game.native_setting_event(0, 0);
            assert_eq!(game.setting_bool(alpha_on), Ok(false));
            let on_file = root.join("alpha/on");
            assert_eq!(fs::read_to_string(&on_file).unwrap(), "false");
            assert_eq!(
                *alpha_seen.lock().unwrap(),
                vec![("on".to_owned(), SettingValue::Toggle(false))]
            );
            assert_eq!(beta_calls.load(Ordering::Relaxed), 0);

            // A failed UI write restores the widget and does not notify. A typed no-op
            // also leaves the value and notification count alone.
            fs::remove_file(&on_file).unwrap();
            fs::create_dir(&on_file).unwrap();
            ui_row(game).click();
            game.native_setting_event(0, 0);
            assert_eq!(*ui_row(game), NativeSettingValue::Toggle(false));
            assert_eq!(game.setting_bool(alpha_on), Ok(false));
            fs::remove_dir(&on_file).unwrap();
            fs::write(&on_file, "false").unwrap();
            assert_eq!(
                game.set_setting(alpha_on, SettingValue::Toggle(false)),
                Ok(false)
            );
            assert_eq!(alpha_seen.lock().unwrap().len(), 1);

            // Exact comparison distinguishes the two zeroes, as OptionInstance.set does.
            assert_eq!(
                game.set_setting(alpha_scale, SettingValue::Range(-0.0)),
                Ok(true)
            );
            assert_eq!(
                game.setting_f64(alpha_scale).unwrap().to_bits(),
                (-0.0f64).to_bits()
            );
            assert_eq!(alpha_seen.lock().unwrap().len(), 2);

            // Invalid kind or out-of-range write refuses without changing live state.
            assert_eq!(
                game.set_setting(alpha_scale, SettingValue::Range(99.0)),
                Err(SettingAccessError::InvalidValue)
            );
            assert_eq!(
                game.set_setting(alpha_scale, SettingValue::Toggle(true)),
                Err(SettingAccessError::WrongKind)
            );
            assert_eq!(
                game.setting_f64(alpha_scale).unwrap().to_bits(),
                (-0.0f64).to_bits()
            );

            // External disk edit is ignored until reload or Options > Mods refresh,
            // then picked up from a single LoopContext snapshot and each changed key is
            // notified once, in registration order.
            alpha_seen.lock().unwrap().clear();
            fs::write(root.join("alpha/on"), "true").unwrap();
            fs::write(root.join("alpha/mode"), "1").unwrap();
            assert_eq!(game.setting_bool(alpha_on), Ok(false));
            let reload = LoadContext::new_with_registered(
                DataLoadOrigin::ClientPreload,
                Arc::new(LoadInputs::new(BTreeMap::new(), root.clone())),
                [],
                registered_setting_keys(),
            );
            game.common.loop_context = Arc::new(reload.captured_loop_context(LoopSide::Client));
            game.sync_native_settings_from_loop_context();
            assert_eq!(game.setting_bool(alpha_on), Ok(true));
            assert_eq!(game.setting_choice(alpha_mode), Ok(1));
            assert_eq!(
                *alpha_seen.lock().unwrap(),
                vec![
                    ("on".to_owned(), SettingValue::Toggle(true)),
                    ("mode".to_owned(), SettingValue::Choice(1)),
                ]
            );

            // A single-owner page refresh notifies the same way.
            alpha_seen.lock().unwrap().clear();
            fs::write(root.join("alpha/on"), "false").unwrap();
            fs::write(root.join("alpha/mode"), "0").unwrap();
            game.refresh_native_settings_owner(&root, "alpha");
            assert_eq!(game.setting_bool(alpha_on), Ok(false));
            assert_eq!(game.setting_choice(alpha_mode), Ok(0));
            assert_eq!(
                *alpha_seen.lock().unwrap(),
                vec![
                    ("on".to_owned(), SettingValue::Toggle(false)),
                    ("mode".to_owned(), SettingValue::Choice(0)),
                ]
            );

            // A panic disables only alpha's setting_changed callback. Its live write stands,
            // and beta continues to receive its own changes through the shared dispatch.
            assert_eq!(
                game.set_setting(alpha_on, SettingValue::Toggle(true)),
                Ok(true)
            );
            let seen_before_panic = alpha_seen.lock().unwrap().len();
            let _ = mod_api::__private::take_hook_panics();
            alpha_panic.store(true, Ordering::Relaxed);
            assert_eq!(
                game.set_setting(alpha_on, SettingValue::Toggle(false)),
                Ok(true)
            );
            assert_eq!(game.setting_bool(alpha_on), Ok(false));
            assert_eq!(fs::read_to_string(&on_file).unwrap(), "false");
            let notices = mod_api::__private::take_hook_panics();
            assert!(
                notices
                    .iter()
                    .any(|n| n.mod_id == "alpha" && n.hook == "setting_changed")
            );

            alpha_panic.store(false, Ordering::Relaxed);
            assert_eq!(
                game.set_setting(alpha_on, SettingValue::Toggle(true)),
                Ok(true)
            );
            assert_eq!(*ui_row(game), NativeSettingValue::Toggle(true));
            assert_eq!(alpha_seen.lock().unwrap().len(), seen_before_panic);
            assert_eq!(
                game.set_setting(beta_on, SettingValue::Toggle(false)),
                Ok(true)
            );
            assert_eq!(beta_calls.load(Ordering::Relaxed), 1);
        });
    });
}
