use crate::audio::Audio;
use crate::game::load_resources;
use crate::settings::Settings;
use sunrise_gameplay_common::game::execution_role::ExecutionRole;
use {sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress};

#[test]
#[ignore = "daily"]
fn a_client_start_joins_the_world_rest_worker_before_it_resolves_the_renderers() {
    with_loading_client(|game| {
        game.publish_initial_client_models(vec![
            sunrise_client_gameplay::game::client_side::client_reload::PreparedClientReload::Vanilla(()),
        ])
        .unwrap();
        assert!(game.rest_loader.is_none(), "the worker was joined");
        assert!(game.res.rest.get().is_some(), "the rest tables are there");
        assert!(
            game.res.mob_types().contains_key("minecraft:zombie"),
            "the renderer table resolved against the loaded mob types"
        );
    });
}

pub(crate) fn with_loading_client(
    run: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
) {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let mut gui = None;
    // As the loading screen loads: the rest tables on their own worker.
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        true,
        ExecutionRole::Client,
        &Progress::new(),
        &mut |ui, _| gui = Some(ui),
    )
    .unwrap();
    assert!(
        loaded.rest_loader.is_some(),
        "a client start loads the rest on a worker"
    );
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = ExecutionRole::Client;
    let saves = tempfile::tempdir().unwrap();
    options.game_dir = Some(saves.path().to_path_buf());
    let mut game = sunrise_client_gameplay::game::ClientGame::new(
        &loaded.resources,
        loaded.client_res.unwrap(),
        loaded.pack,
        loaded.client_pack.unwrap(),
        gui.or(loaded.gui).unwrap(),
        options,
        settings,
        Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            root.to_path_buf(),
        )),
        None,
        sunrise_gameplay::game::server_side::integrated::serve,
        sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
        crate::game::session_verifier(),
        crate::game::profile_resolver,
    )
    .unwrap();
    game.rest_loader = loaded.rest_loader;
    assert!(
        sunrise_client_gameplay::game::client_side::launch_options_client::is_client_start(
            &game.options
        )
    );
    run(&mut game);
}
