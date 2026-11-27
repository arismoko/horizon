//! Bootstrap capture, mod registration and the real Game hosts.
#[cfg(feature = "client")]
use mod_api::{Dist, ModEntry, PackageInfo};
#[cfg(feature = "client")]
use mod_api::{Hooks, Host, RegistrationContext, RuntimeEnvironment};
#[cfg(feature = "client")]
use std::sync::{Arc, Mutex};
#[cfg(test)]
use sunrise_gameplay_common::game::Options;
use sunrise_mod_package::selection::LaunchArguments;
#[cfg(feature = "client")]
use {
    sunrise_gameplay_common::game::execution_role::ExecutionRole,
    sunrise_gameplay_common::game::mods,
};

#[cfg(feature = "client")]
#[derive(Default)]
struct Observed {
    pub registration: Vec<RuntimeEnvironment>,
    later: Vec<RuntimeEnvironment>,
    tests: usize,
}
#[cfg(feature = "client")]
struct Probe(Arc<Mutex<Observed>>);
#[cfg(feature = "client")]
fn from_host(host: &dyn Host, tests: bool) -> RuntimeEnvironment {
    RuntimeEnvironment::new(
        host.dist(),
        host.game_version().to_owned(),
        host.sunrise_version().to_owned(),
        tests,
    )
}
#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, _: &mut dyn mod_api::Registrar, cx: &RegistrationContext) {
        self.0
            .lock()
            .unwrap()
            .registration
            .push(RuntimeEnvironment::new(
                cx.dist(),
                cx.game_version().to_owned(),
                cx.sunrise_version().to_owned(),
                cx.game_tests_enabled(),
            ));
    }
    fn client_tick_pre(
        &self,
        host: &mut dyn mod_api::ClientHost,
        _: &mut mod_api::ClientTickPreEvent,
    ) {
        // The flag has only a server Host method; registration already pinned it.
        let mut observed = self.0.lock().unwrap();
        let tests = observed.registration[0].game_tests_enabled();
        observed.later.push(from_host(host, tests));
    }
    fn server_started(
        &self,
        host: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::ServerStartedEvent,
    ) {
        self.0
            .lock()
            .unwrap()
            .later
            .push(from_host(host, host.game_tests_enabled()));
    }
    fn register_game_tests(&self, _: &dyn Host, _: &mut mod_api::RegisterGameTestsEvent<'_>) {
        self.0.lock().unwrap().tests += 1;
    }
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn native_process_environment_in_registration_and_game_hosts() {
    if std::env::var_os("SUNRISE_RUNTIME_ENV_CASE").is_none() {
        let resources = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resource root required"),
        )
        .canonicalize()
        .unwrap();
        let owner = tempfile::tempdir().unwrap();
        let cases = [
            "client_off",
            "client_flag",
            "client_env_one",
            "client_env_true",
            "client_env_false",
            "dedicated_off",
            "dedicated_before",
            "dedicated_after",
            "dedicated_env_one",
            "dedicated_operand",
            "integrated_off",
            "integrated_flag",
            "integrated_env_true",
        ];
        let run = |case: &str| {
            let scratch = owner.path().join(case);
            std::fs::create_dir(&scratch).unwrap();
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--ignored",
                    "--exact",
                    "cut_layout::game::native_hooks::runtime_environment::tests::native_process_environment_in_registration_and_game_hosts",
                    "--nocapture",
                ])
                .current_dir(&scratch)
                .env("SUNRISE_RUNTIME_ENV_CASE", case)
                .env("SUNRISE_TEST_RESOURCES", &resources)
                .env("SUNRISE_NO_AUDIO", "1")
                .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
                .env_remove("SUNRISE_GAME_TESTS");
            for key in [
                "SUNRISE_HOME",
                "SUNRISE_LAUNCHER_HOME",
                "XDG_DATA_HOME",
                "XDG_CONFIG_HOME",
            ] {
                command.env(key, &scratch);
            }
            if case.ends_with("env_one") {
                command.env("SUNRISE_GAME_TESTS", "1");
            } else if case.ends_with("env_true") {
                command.env("SUNRISE_GAME_TESTS", "true");
            } else if case.ends_with("env_false") {
                command.env("SUNRISE_GAME_TESTS", "false");
            }
            command.output().unwrap()
        };
        for case in cases {
            let output = run(case);
            assert!(
                output.status.success(),
                "{case}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "child must execute the exact acceptance test, not zero tests"
            );
        }
        return;
    }
    let case = std::env::var("SUNRISE_RUNTIME_ENV_CASE").unwrap();
    let dedicated = case.starts_with("dedicated");
    let enabled = case.ends_with("flag")
        || case.ends_with("before")
        || case.ends_with("after")
        || case.ends_with("env_one")
        || case.ends_with("env_true");
    let mut args = Vec::new();
    if case.ends_with("flag") || case.ends_with("before") {
        args.push("--game-tests".to_owned());
    }
    if dedicated {
        args.push("--dedicated".to_owned());
    }
    if case.ends_with("after") {
        args.push("--game-tests".to_owned());
    }
    if case.ends_with("operand") {
        args.extend(["--world".to_owned(), "--game-tests".to_owned()]);
    }
    let cwd = std::env::current_dir().unwrap();
    let launch = LaunchArguments::parse(args.into_iter(), &cwd, &cwd, None).unwrap();
    let options = Options::from_launch(launch.clone()).unwrap().unwrap();
    if case.ends_with("operand") {
        assert_eq!(
            options.dedicated.as_deref().unwrap(),
            ["--world", "--game-tests"]
        );
    }
    // The same entry capture used before the executable loads any code.
    sunrise_level::runtime_env::runtime_environment::capture(&launch);
    let expected = RuntimeEnvironment::new(
        if dedicated {
            Dist::DedicatedServer
        } else {
            Dist::Client
        },
        sunrise_core::VANILLA_VERSION.to_owned(),
        sunrise_level::crash::version(),
        enabled,
    );
    let observed = Arc::new(Mutex::new(Observed::default()));
    let entries = vec![ModEntry::new(
        "runtime_probe",
        Box::new(Probe(observed.clone())),
    )];
    let inventory = vec![PackageInfo::new(
        "runtime_probe".into(),
        "1.0.0".into(),
        mod_api::PackageKind::Code,
        mod_api::Side::Both,
        vec!["runtime_probe".into()],
        vec![],
    )];
    crate::game::api_values::install_test_platform();
    mod_platform::__private::install_packages(&inventory, &[]);
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    // Freeze the real bootstrap snapshot before with_mods runs registration hooks.
    mods::install_registration_data(&root, cwd, Default::default(), Default::default()).unwrap();
    mod_platform::testing::with_mods(entries.into(), || {
        let role = if dedicated {
            ExecutionRole::DedicatedServer
        } else if case.starts_with("integrated") {
            ExecutionRole::IntegratedServer
        } else {
            ExecutionRole::Client
        };
        if role.is_client() {
            sunrise_client_gameplay::game::client_side::native_entity_refs::tests::with_game_role_client(
                sunrise_gameplay::game::native_entity_refs::tests::fixture_root(),
                ExecutionRole::Client,
                |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
                    game.native_client_tick_pre();
                },
            );
        }
        // A client process's integrated logical server runs on its own server fixture.
        let server_role = if dedicated {
            ExecutionRole::DedicatedServer
        } else {
            ExecutionRole::IntegratedServer
        };
        sunrise_gameplay::game::native_entity_refs::tests::with_game_role(
            sunrise_gameplay::game::native_entity_refs::tests::fixture_root(),
            server_role,
            |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                game.native_server_started(dedicated);
                game.native_register_game_tests(&mut mod_api::GameTestFunctions::default());
            },
        );
    });
    let observed = observed.lock().unwrap();
    assert!(
        !observed.registration.is_empty(),
        "Hooks::register must execute"
    );
    assert!(observed.registration.iter().all(|value| value == &expected));
    assert_eq!(
        observed.later.len(),
        if case.starts_with("client") { 2 } else { 1 },
        "client Host and logical server Host hooks must each execute"
    );
    assert!(observed.later.iter().all(|value| value == &expected));
    assert_eq!(
        observed.tests, 1,
        "game-test registration runs with the flag off and on"
    );
}

#[test]
fn normal_options_preserve_dedicated_flag_valued_operands() {
    let cwd = std::env::current_dir().unwrap();
    for option in ["world", "universe", "port", "serverId", "pidFile"] {
        let tail = vec![format!("--{option}"), "--game-tests".to_owned()];
        let launch = LaunchArguments::parse(
            std::iter::once("--dedicated".to_owned()).chain(tail.clone()),
            &cwd,
            &cwd,
            None,
        )
        .unwrap();
        let options = Options::from_launch(launch).unwrap().unwrap();
        assert_eq!(options.dedicated.as_ref(), Some(&tail));
        assert_eq!(
            sunrise_level::runtime_env::dedicated_arguments::parse(
                options.dedicated.as_ref().unwrap()
            )
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
            [(option, Some("--game-tests"))],
        );
    }
    let options = Options::parse(["--world", "--game-tests"].map(str::to_owned).into_iter())
        .unwrap()
        .unwrap();
    assert_eq!(options.world.as_deref(), Some("--game-tests"));
}
