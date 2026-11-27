//! `/time` addresses the clock the End runs on: the one `World.end_clock`
//! ticks and `world_clocks.dat` saves (`ServerClockManager.getInstance`).
use crate::game::load_resources;
use crate::settings::Settings;
use {
    sunrise_gameplay_common::game::Dim, sunrise_gameplay_common::game::Options,
    sunrise_gameplay_common::game::Progress, sunrise_gameplay_common::game::execution_role,
};

pub(crate) fn game<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> sunrise_gameplay::game::ServerGame<'a> {
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res = &*resources.insert(loaded.resources);
    let mut game = sunrise_gameplay::game::ServerGame::new(
        res,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    // The player stands in the End, so `/time` without `of` names its clock.
    game.dim = Dim::End;
    game
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
