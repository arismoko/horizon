//! Dedicated server startup (`start_dedicated_game`) and server-side agent control socket setup.
use super::*;
use sunrise_gameplay_common::game::{
    integrated_link::ProfileResolverFactory, packs_common::cache_dir,
    resource_cpu_common::ServerCpuResources,
};
use sunrise_level::sim_settings::SimSettings;
use sunrise_sim::control;

/// The post-load setup for a dedicated server run: builds `ServerGame` with no window,
/// renderer, client reload, or audio device, runs `run_dedicated`, and shuts down.
#[allow(clippy::too_many_arguments)]
pub fn start_dedicated_game(
    res: &Arc<Resources>,
    pack: PackResources,
    options: Options,
    mut settings: SimSettings,
    started: Instant,
    prepared: Option<ServerCpuResources>,
    services: dedicated::Services,
    profile_resolver: ProfileResolverFactory,
    keep_benchmark: fn(&std::path::Path, &sunrise_bench::Report),
    ticket_activation: &'static mod_api::__private::game::TicketActivation,
) -> Result<()> {
    if let Some(d) = options.distance_override {
        settings.render_distance = d;
    }
    let prepared = match prepared {
        Some(prepared) => Some(prepared),
        None => Some(ServerCpuResources::load(
            &sunrise_assets::resource_source::ResourceRoot::new(
                options.resources.clone(),
                cache_dir(),
                pack.source.clone(),
            ),
        )?),
    };
    let mut game =
        crate::game::ServerGame::new(res, pack, options, settings, prepared, profile_resolver)?;
    game.server.ticket_activation = Some(ticket_activation);
    game.server.dedicated_services = Some(services);
    if game.options.benchmark.is_some() {
        game.options.no_pause = true;
    }
    if game.options.benchmark.is_none() {
        game.start_mods();
    }
    game.refresh_loop_context()?;
    game.start_server_agent_control()?;
    if let Some(scenario) = game.options.benchmark.clone() {
        let control = game
            .control
            .get_or_insert_with(sunrise_sim::control::Server::local);
        sunrise_sim::benchmark::start(
            control.sender(),
            sunrise_sim::benchmark::Setup {
                scenario,
                started,
                app: sunrise_bench::App {
                    name: "sunrise-client".into(),
                    version: sunrise_level::crash::version(),
                    commit: option_env!("SUNRISE_COMMIT").map(str::to_owned),
                    mods: Vec::new(),
                },
                simulation_distance: game.server.settings.simulation_distance as u32,
                vsync: false,
                fps_cap: None,
                offscreen: game.options.offscreen,
                game_dir: std::env::current_dir()?,
                set: game.options.benchmark_set.clone(),
                profile: game.options.benchmark_profile,
            },
            keep_benchmark,
        );
    }
    let result = game.run_dedicated();
    game.native_game_shutting_down();
    result
}

impl crate::game::ServerGame<'_> {
    /// Open the server control socket according to the startup options and simulation settings.
    pub(in crate::game::server_side) fn start_server_agent_control(&mut self) -> Result<()> {
        if self.common.options.benchmark.is_some() && self.common.options.control.is_none() {
            return Ok(());
        }
        if self.server.settings.no_ai {
            if self.common.options.control.is_some() {
                log::warn!(
                    "Sunrise: control socket not opened: {}",
                    sunrise_core::no_ai::REFUSAL
                );
            }
            return Ok(());
        }
        if let Some(path) = &self.common.options.control {
            let server = control::Server::start(path)
                .map_err(|e| format!("control socket {}: {e}", path.display()))?;
            log::info!("Sunrise: control socket {}", server.path().display());
            self.common.control = Some(server);
        } else if self.server.settings.agent_control {
            let path = control::default_path();
            match control::Server::start(&path) {
                Ok(server) => {
                    log::info!("Sunrise: control socket {}", server.path().display());
                    self.common.control = Some(server);
                }
                Err(e) => log::warn!("Sunrise: control socket {}: {e}", path.display()),
            }
        }
        Ok(())
    }
}

pub fn configure_launch_spans(on: bool) {
    tick_profile::want_spans(tick_profile::SPANS_SPIKES, on);
}

use sunrise_gameplay_common::game::mods::ModGameExt1 as _;
use sunrise_gameplay_common::game::native_hooks::game_shutting_down::GameHooks as _;
