use crate::game::native_hooks::build_creative_mode_tab_contents::tests::definition;
use std::sync::Arc;
use sunrise_gameplay::game::lan::LanHost;
use sunrise_sim::lan::{
    Lan, NetConfig,
    chunks::ChunkEncoder,
    data::SyncData,
    hub::{Hub, HubConfig},
};
use sunrise_world::loot::PackOverlay;

pub(crate) fn install_dedicated_holders(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    max_level: i32,
) {
    let files = vec![(
        "enchantment/test:levels".into(),
        vec![definition(max_level).to_string().into_bytes()],
    )];
    let sync = Arc::new(
        SyncData::load(
            &game.options.resources,
            &PackOverlay::new(&files),
            game.res.item_defaults(),
            &sunrise_sim::crafting::ResolvedSources::default(),
        )
        .unwrap(),
    );
    let net = NetConfig {
        listen: false,
        ..NetConfig::loopback(0, String::new(), 2)
    };
    let lan = Lan::start_with(sync.clone(), net).unwrap();
    let encoder = ChunkEncoder::new(
        [] as [(Arc<str>, bool); 0],
        &sync.builtin,
        &[],
        &[],
        Default::default(),
    );
    let config = HubConfig {
        default_game_mode: 0,
        guest_commands: false,
        force_game_mode: false,
        hardcore: false,
        view_distance: 2,
        simulation_distance: 2,
        dimension: game.dim.id().to_owned(),
        allow_flight: false,
        seed: 0,
        max_players: 2,
        integrated: false,
        levels: game.server.level_shapes.clone(),
        level_seeds: game.server.levels.0.runtime.seeds(),
    };
    game.server.lan = Some(LanHost::for_test(Hub::new(
        lan,
        sync,
        encoder,
        config,
        game.loop_context.clone(),
    )));
    game.native_lifecycle_open_started();
    game.native_lifecycle.world_open_ready();
}

use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1 as _;
