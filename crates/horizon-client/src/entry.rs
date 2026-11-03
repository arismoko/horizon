//! Startup shared by the gameplay library and its executable.
use crate::{crash, game, logging, ram_limit};
use horizon_mods::selection::{Argument, LaunchArguments};

/// Start the game. The executable supplies the allocator and retains the mod API marker.
pub fn main() {
    main_with(Default::default());
}

/// Start the game with named trait mods in dispatch order.
pub fn main_with(mods: crate::mod_api::Mods) {
    // First: a trait injection into anything below (logging, the crash handler) calls its mod.
    crate::mod_api::install(mods);
    horizon_threads::configure_allocator();
    horizon_client::telemetry::mark_start();
    horizon_client::spans::startup::init(std::time::Instant::now());
    let launch: crate::Result<_> = std::env::current_dir().map_err(Into::into).and_then(|cwd| {
        Ok(LaunchArguments::parse(
            std::env::args().skip(1),
            &cwd,
            &logging::game_home(),
            std::env::var_os("HORIZON_HOME").as_deref(),
        )?)
    });
    let launch = match launch {
        Ok(launch) => launch,
        Err(error) => {
            eprintln!("Horizon: {error}");
            std::process::exit(game::exit_code::USAGE);
        }
    };
    let help = launch.rest.iter().any(|arg| {
        matches!(arg, Argument::Switch(name) if matches!(name.as_str(), "--help" | "-h" | "--launch-api"))
    });
    logging::set_game_home(launch.context.home.clone());
    // The dedicated server logs in its own folder, as vanilla's does; the game in its home.
    let (home, side, thread) = if launch
        .rest
        .iter()
        .any(|arg| matches!(arg, Argument::Dedicated(_)))
    {
        (launch.context.game_dir.clone(), "server", "Server thread")
    } else {
        // A player's own log stays separate; crash telemetry follows the shared home.
        if let Some(dir) = launch.rest.iter().find_map(|arg| match arg {
            Argument::Value(name, value) if name == "--player-dir" => Some(value),
            _ => None,
        }) {
            logging::set_player_dir(std::path::absolute(dir).unwrap_or_else(|_| dir.into()));
        }
        (launch.context.home.clone(), "client", "Render thread")
    };
    if !help {
        logging::init(
            logging::player_dir_override().unwrap_or(home.as_path()),
            thread,
        );
        // The log's first line names the build, so a bug report that carries the log says which one it was.
        log::info!("Horizon {}", crash::version());
        crash::install(home.clone(), side);
        ram_limit::start(&home, side);
    }
    let renderers = crate::mod_api::mods()
        .iter()
        .filter_map(|m| m.hooks.render())
        .collect();
    if crate::gfx::native_render::install(renderers).is_err() {
        log::warn!("Trait renderers already installed; keeping the first list");
    }
    if let Err(refused) = crate::mod_api::install_parts(crate::mod_api::mods()) {
        panic!("Mod parts refused:\n{}", refused.join("\n"));
    }
    if horizon_world::native_load_hooks::install(
        horizon_world::native_load_hooks::NativeLoadHooks {
            loot_table_load: |id, table| {
                crate::mod_api::dispatch_loot_table_load(id, table);
            },
            modify_default_components: |item, components| {
                crate::mod_api::dispatch_modify_default_components(item, components);
            },
            loot_function: |kind, data, ctx, stack| {
                crate::mod_api::dispatch_loot_function(kind, data, ctx, stack);
            },
            loot_condition: |kind, data, ctx, holds| {
                crate::mod_api::dispatch_loot_condition(kind, data, ctx, holds);
            },
        },
    )
    .is_err()
    {
        log::warn!("Native data-load hooks already installed; keeping the first callbacks");
    }
    if crate::native_client_hooks::install(crate::native_client_hooks::NativeClientHooks {
        menu_slot_rules: |menu, slot, stack, may_place, max_stack| {
            crate::mod_api::dispatch_menu_slot_rules(menu, slot, stack, may_place, max_stack);
        },
        item_stacked_on_other: |event, canceled| {
            crate::mod_api::dispatch_item_stacked_on_other(event, canceled);
        },
        living_heal: |entity, amount, cancel| {
            crate::mod_api::dispatch_living_heal(entity, amount, cancel);
        },
        living_breathe: |entity, kind, can_breathe, consume, refill| {
            crate::mod_api::dispatch_living_breathe(entity, kind, can_breathe, consume, refill);
        },
        farmland_trample: |entity, pos, state, distance, cancel| {
            crate::mod_api::dispatch_farmland_trample(entity, pos, state, distance, cancel);
        },
        has_mods: || !crate::mod_api::mods().is_empty(),
        player_name_format: |username, displayname| {
            crate::mod_api::dispatch_player_name_format(username, displayname);
        },
        player_tab_list_name_format: |player, name, display_name| {
            crate::mod_api::dispatch_player_tab_list_name_format(player, name, display_name);
        },
        living_drown: |entity, kind, is_drowning, damage_amount, bubble_count, cancel| {
            crate::mod_api::dispatch_living_drown(
                entity,
                kind,
                is_drowning,
                damage_amount,
                bubble_count,
                cancel,
            );
        },
        living_fall: |entity, kind, distance, multiplier, cancel| {
            crate::mod_api::dispatch_living_fall(entity, kind, distance, multiplier, cancel);
        },
        living_jump: |entity, kind| {
            crate::mod_api::dispatch_living_jump(entity, kind);
        },
        ender_entity_teleport: |entity, kind, from, target, cancel| {
            crate::mod_api::dispatch_ender_entity_teleport(entity, kind, from, target, cancel);
        },
        ingredient: |kind, data, items| {
            crate::mod_api::dispatch_ingredient(kind, data, items);
        },
        ingredient_test: |kind, data, stack, accept| {
            crate::mod_api::dispatch_ingredient_test(kind, data, stack, accept);
        },
    })
    .is_err()
    {
        log::warn!("Native client hooks already installed; keeping the first callbacks");
    }
    raise_open_file_limit();
    if let Err(error) = game::run_with(launch) {
        // A bad command line is the caller's mistake, not a crash: no report.
        let usage = error.is::<game::UsageError>();
        log::error!("Horizon: {error}");
        logging::flush();
        // The report's path is the last line on stderr (crash::SAVED_LINE).
        if !help && !usage {
            crash::report_error(&home, side, &*error);
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
    logging::flush();
}

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
