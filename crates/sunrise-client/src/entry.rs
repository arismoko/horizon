//! Startup shared by the gameplay library and its executable.
#[cfg(not(feature = "client"))]
#[path = "crash_retention.rs"]
mod crash_retention;
#[cfg(feature = "client")]
#[path = "sunrise_look.rs"]
mod sunrise_look;
use crate::{crash, game, logging};
use mod_api::__private::game::TicketActivation;
use std::sync::OnceLock;
use sunrise_level::runtime_env::runtime_environment;
use sunrise_mod_package::selection::{Argument, LaunchArguments};

static TICKET_ACTIVATION: OnceLock<TicketActivation> = OnceLock::new();

pub(crate) fn ticket_activation() -> &'static TicketActivation {
    TICKET_ACTIVATION
        .get()
        .expect("startup prepared ticket activation")
}

/// Validated startup state. No native hooks or workers may run before prepare returns.
pub struct Startup {
    launch: LaunchArguments,
    options: game::Options,
    _capture: Option<sunrise_mod_package::output::Capture>,
}

impl Startup {
    /// Validated launch context used by the platform's code loader.
    pub fn launch(&self) -> &LaunchArguments {
        &self.launch
    }
}

/// Holder mode runs first. The game's sole Options validation reads arguments/environment
/// and resource marker metadata only: no logging, account loading, mods or worker threads.
/// Account IO, cwd changes and command registration remain in Options::read, after leases.
pub fn prepare() -> Startup {
    sunrise_mod_package::holder::enter();
    let result: sunrise_game_data::Result<_> = (|| {
        let mut command = std::process::Command::new(std::env::current_exe()?);
        command.args(std::env::args_os().skip(1));
        let launch = LaunchArguments::for_command(&command)?;
        let options = game::Options::validate(launch.clone())?;
        Ok((launch, options))
    })();
    let (launch, options) = result.unwrap_or_else(|error| {
        eprintln!("Sunrise: {error}");
        std::process::exit(game::exit_code::USAGE);
    });
    let Some(options) = options else {
        std::process::exit(0);
    };
    let capture = sunrise_mod_package::holder::acquire(&launch);
    // Native libraries (including their initializers and hook factories) have not run yet.
    TICKET_ACTIVATION.get_or_init(|| {
        TicketActivation::take_for_startup().unwrap_or_else(|| {
            eprintln!("Sunrise: refusing startup because ticket activation is already owned");
            std::process::exit(game::exit_code::ERROR);
        })
    });
    Startup {
        launch,
        options,
        _capture: capture,
    }
}

/// Refuse startup if any merged copy could not be installed, before loading any mod.
/// Cells cannot roll back: each copy merges all requesting rows, so stopping the whole
/// start is how we ensure none of a refused mod's rows run.
pub fn installed(patched: &[(&str, &[&str], bool)]) {
    let refusals = installation_refusals(patched);
    for line in &refusals {
        eprintln!("{line}");
    }
    if !refusals.is_empty() {
        std::process::exit(game::exit_code::ERROR);
    }
}

fn installation_refusals(patched: &[(&str, &[&str], bool)]) -> Vec<String> {
    patched.iter().filter(|(_, _, installed)| !installed).map(|(target, mods, _)| {
        format!("Sunrise: {}: {target}: a patched copy was already installed; the game was not started", mods.join(", "))
    }).collect()
}

/// Start the engine with dependency-ordered code supplied by the host.
pub fn start(
    native: sunrise_mod_package::runtime::Runtime,
    load: impl FnOnce(&sunrise_mod_package::runtime::Runtime, &LaunchArguments),
    startup: Startup,
) {
    let Startup {
        launch,
        options,
        _capture,
    } = startup;
    runtime_environment::capture(&launch);
    sunrise_threads::configure_allocator();
    #[cfg(feature = "client")]
    sunrise_look::give();
    // OptiFine's and Polytone's pack formats (platform, docs/layers.md), before any pack loads.
    #[cfg(feature = "client")]
    sunrise_client::pack_formats::install();
    // sunrise-extras (docs/forking.md): without the feature, no Sunrise Settings or Mods page.
    #[cfg(feature = "client")]
    if !cfg!(feature = "sunrise-extras") {
        sunrise_client::options_ui::SUNRISE_EXTRAS
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
    #[cfg(feature = "client")]
    sunrise_client::telemetry::mark_start();
    // Opt-in telemetry keeps each crash report (docs/telemetry.md), whatever the mode.
    #[cfg(feature = "client")]
    crash::on_report(sunrise_client::telemetry::keep_crash);
    #[cfg(not(feature = "client"))]
    crash::on_report(|home, report| {
        crash_retention::keep_crash(
            home,
            report,
            || {
                let cwd = std::env::current_dir().ok();
                sunrise_telemetry::payload::System::detect_in(cwd.as_deref())
            },
            game::benchmark_retention::version,
        );
    });
    // Crates above the engine registered their hooks before this (sunrise-official's `main`);
    // none registers after this point.
    sunrise_hooks::engine::seal();
    // The text crates ship (`sunrise_hooks::lang`), loaded with the game's own languages.
    sunrise_game_data::lang::install_crate_files(sunrise_hooks::lang::engine::all());
    sunrise_level::spans::startup::init(std::time::Instant::now());
    logging::set_game_home(
        launch.context.home.clone(),
        launch.cache_home().to_path_buf(),
    );
    // The dedicated server logs in its own folder, as vanilla's does; the game in its home.
    let (home, side, thread) = if launch
        .rest
        .iter()
        .any(|arg| matches!(arg, Argument::Dedicated(_)))
    {
        (launch.context.game_dir.clone(), "server", "Server thread")
    } else {
        // A player's own log stays separate; crash reports follow the shared home.
        if let Some(dir) = launch.rest.iter().find_map(|arg| match arg {
            Argument::Value(name, value) if name == "--player-dir" => Some(value),
            _ => None,
        }) {
            logging::set_player_dir(std::path::absolute(dir).unwrap_or_else(|_| dir.into()));
        }
        (launch.context.home.clone(), "client", "Render thread")
    };
    logging::init(
        logging::player_dir_override().unwrap_or(home.as_path()),
        thread,
    );
    // The log's first line names the build, so a bug report that carries the log says which one it was.
    log::info!("Sunrise {}", crash::version());
    // Mods' startup bars and lines (`RegistrationContext::progress`), before they register.
    game::startup_progress::install(side == "server");
    // Mod loading emits refusals; both client and dedicated logs must exist first.
    load(&native, &launch);
    let save_failure_diagnostics = options.save_failure_diagnostics.clone();
    let panic_diagnostics = save_failure_diagnostics.clone();
    crash::install(home.clone(), side, move || {
        panic_diagnostics.suppressed_summary()
    });
    let _ = sunrise_world::loot::types::install(crate::game::loot_types::capture);
    if sunrise_world::native_load_hooks::install(
        sunrise_world::native_load_hooks::NativeLoadHooks {
            item_ability_default: sunrise_items::item_abilities::vanilla_can_perform,
            loot_ability:
                sunrise_gameplay_common::game::api_values::loot_values::dispatch_loot_ability,
            loot_table_load: |context, id, table| {
                if !mod_api::__private::listened(mod_api::HookId::LootTableLoad) {
                    return;
                }
                let reader = sunrise_level::api_context::LoadReader(context);
                let context = mod_api::OwningLoad::new(
                    sunrise_level::api_context::load_origin(context.origin()),
                    &reader,
                );
                mod_api::dispatch_loot_table_load(&context, id, table);
            },
            modify_default_components: |context, item, components| {
                if !mod_api::__private::listened(mod_api::HookId::ModifyDefaultComponents) {
                    return;
                }
                let reader = sunrise_level::api_context::LoadReader(context);
                let context = reader.context();
                mod_api::dispatch_modify_default_components(&context, item, components);
            },
        },
    )
    .is_err()
    {
        log::warn!("Native data-load hooks already installed; keeping the first callbacks");
    }
    if sunrise_world::native_worldgen_hooks::install(
        sunrise_world::native_worldgen_hooks::NativeWorldgenHooks {
            alter_ground: sunrise_gameplay_common::game::native_ground::alter_ground,
            on_tree_grow: crate::game::native_tree_grow::on_tree_grow,
            mirror: sunrise_gameplay_common::game::native_rotate::mirror,
            rotate: sunrise_gameplay_common::game::native_rotate::rotate,
            feature: crate::game::native_worldgen_parts::feature,
        },
    )
    .is_err()
    {
        log::warn!("Native worldgen hooks already installed; keeping the first callbacks");
    }
    if sunrise_items::native_enchantment_hooks::install(
        sunrise_sim::enchantment_levels::get_enchantment_level,
    )
    .is_err()
    {
        log::warn!("The enchantment level hook is already installed; keeping the first");
    }
    install_client_hooks();
    raise_open_file_limit();
    // `Minecraft.run` catches the throwable and then calls `crash`: the panic hook only records,
    // and the report is built here, after the unwind has released the panicking code's locks.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        game::run_with(options, native)
    }))
    .unwrap_or_else(|payload| {
        crash::report_main_panic(&*payload);
        logging::flush();
        std::process::exit(101)
    });
    if let Err(error) = outcome {
        // A bad command line is the caller's mistake, not a crash: no report.
        let missing_mods = game::dedicated::is_missing_mods_refusal(error.as_ref());
        let usage = error.is::<game::UsageError>() || missing_mods;
        if let Some(refusals) =
            error.downcast_ref::<sunrise_gameplay_common::game::mods::DeclarationFailures>()
        {
            for line in refusals.lines() {
                log::error!("Sunrise: {line}");
            }
        } else if !missing_mods {
            log::error!("Sunrise: {error}");
        }
        logging::flush();
        // The report's path is the last line on stderr (crash::SAVED_LINE).
        if !usage {
            crash::report_error(&home, side, &*error, || {
                save_failure_diagnostics.suppressed_summary()
            });
        }
        // Started from Explorer, the console window closes with the game:
        // keep the message on screen until it is read.
        #[cfg(windows)]
        if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            eprintln!("Press Enter to close.");
            let _ = std::io::stdin().read_line(&mut String::new());
        }
        std::process::exit(if usage {
            game::exit_code::USAGE
        } else {
            game::exit_code::ERROR
        });
    }
    sunrise_hooks::engine::shutdown();
    logging::flush();
}

pub(crate) use sunrise_gameplay_common::game::native_hooks::install_client_hooks;

/// The JVM raises its open-file limit to the hard limit as it starts
/// (`-XX:+MaxFDLimit`, on by default), so a vanilla server may hold as many
/// sockets and region files as the host allows, not the soft default (often
/// 1024 in a container). The same here; macOS caps it at `OPEN_MAX`, as the
/// JVM does there.
fn raise_open_file_limit() {
    #[cfg(unix)]
    {
        use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
        let limit = getrlimit(Resource::Nofile);
        let hard = if cfg!(target_os = "macos") {
            Some(limit.maximum.map_or(10_240, |h| h.min(10_240)))
        } else {
            limit.maximum
        };
        let raised = match (limit.current, hard) {
            (Some(soft), Some(hard)) => hard > soft,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if raised {
            let to = Rlimit {
                current: hard,
                maximum: limit.maximum,
            };
            match setrlimit(Resource::Nofile, to) {
                Ok(()) => log::info!(
                    "Open file limit raised from {:?} to {hard:?}",
                    limit.current
                ),
                Err(e) => log::warn!("Open file limit stays at {:?}: {e}", limit.current),
            }
        }
    }
}

#[cfg(test)]
mod installation_tests {
    #[test]
    fn refused_install_names_all_mods_and_only_failed_targets() {
        assert_eq!(
            super::installation_refusals(&[
                ("game::tick", &["first", "second"], false),
                ("game::other", &["third"], true)
            ]),
            vec![
                "Sunrise: first, second: game::tick: a patched copy was already installed; the game was not started"
            ]
        );
        assert!(super::installation_refusals(&[]).is_empty());
    }
}

#[cfg(test)]
mod name_conversion_tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_game_data::text::Text;
    use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};

    // Engine -> API rejects unrepresentable opaque metadata (NestedNull);
    // API -> Engine rejects a component tree deeper than 512 nodes at the serialization boundary.
    fn unrepresentable_engine_text() -> Text {
        Text::empty().with_component(serde_json::json!({"text": "leaf", "test:meta": [null]}))
    }

    fn deep_api_text() -> mod_api::Text {
        let mut value = mod_api::Text::literal("leaf");
        for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
            value = mod_api::Text::literal("parent").append(value);
        }
        value
    }

    struct InvalidNames(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for InvalidNames {
        fn player_name_format(
            &self,
            _: &dyn mod_api::LoopContext,
            event: &mut mod_api::PlayerNameFormatEvent<'_>,
        ) {
            self.0.fetch_add(1, Ordering::Relaxed);
            *event.displayname = deep_api_text();
        }
        fn player_tab_list_name_format(
            &self,
            _: &dyn mod_api::LoopContext,
            event: &mut mod_api::PlayerTabListNameFormatEvent<'_>,
        ) {
            self.0.fetch_add(1, Ordering::Relaxed);
            *event.display_name = Some(deep_api_text());
        }
    }

    fn with_names(
        test: impl FnOnce(&sunrise_world::native_load_context::LoopContext, &AtomicUsize),
    ) {
        let calls = Arc::new(AtomicUsize::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "invalid-names",
            Box::new(InvalidNames(calls.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::install_client_hooks();
            let inputs = Arc::new(LoadInputs::new(Default::default(), Default::default()));
            let context = LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs, [])
                .loop_context(LoopSide::Server, []);
            test(&context, &calls);
        });
    }

    #[test]
    fn player_name_conversion_failure_keeps_the_engine_name() {
        with_names(|context, calls| {
            let username = Text::literal("Steve");
            let original = Text::literal("Original");
            let unrepresentable = unrepresentable_engine_text();
            let mut display = original.clone();
            sunrise_client_lib::lan::hub::native_player_name_format(
                context,
                &unrepresentable,
                &mut display,
            );
            assert_eq!(display.component(), original.component());
            assert_eq!(calls.load(Ordering::Relaxed), 0);

            display = unrepresentable.clone();
            sunrise_client_lib::lan::hub::native_player_name_format(
                context,
                &username,
                &mut display,
            );
            assert_eq!(display.component_view(), unrepresentable.component_view());
            assert_eq!(calls.load(Ordering::Relaxed), 0);

            display = original.clone();
            sunrise_client_lib::lan::hub::native_player_name_format(
                context,
                &username,
                &mut display,
            );
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            assert_eq!(display.component(), original.component());
        });
    }

    #[test]
    fn tab_list_name_conversion_failure_keeps_the_engine_name() {
        with_names(|context, calls| {
            let unrepresentable = unrepresentable_engine_text();
            let mut display = Some(unrepresentable.clone());
            sunrise_client_lib::lan::hub::native_player_tab_list_name_format(
                context,
                1,
                "Steve",
                &mut display,
            );
            assert_eq!(
                display.as_ref().unwrap().component_view(),
                unrepresentable.component_view()
            );
            assert_eq!(calls.load(Ordering::Relaxed), 0);

            for original in [None, Some(Text::literal("Original"))] {
                display = original.clone();
                sunrise_client_lib::lan::hub::native_player_tab_list_name_format(
                    context,
                    1,
                    "Steve",
                    &mut display,
                );
                assert_eq!(
                    display.as_ref().map(Text::component),
                    original.as_ref().map(Text::component)
                );
            }
            assert_eq!(calls.load(Ordering::Relaxed), 2);
        });
    }
}
