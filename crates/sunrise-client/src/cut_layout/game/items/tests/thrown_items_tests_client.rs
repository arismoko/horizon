use {
    crate::game::load_resources, crate::settings::Settings, sunrise_client_lib::audio::Audio,
    sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
};

/// The one sound `event` played since the last call, with its volume.
pub(crate) fn sound(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    event: &str,
) -> (f32, f32) {
    let heard = game
        .client
        .audio
        .0
        .capture
        .replace(Vec::new())
        .unwrap_or_default();
    let sound = heard
        .iter()
        .find(|s| s.event == event)
        .unwrap_or_else(|| panic!("no {event} sound among {heard:?}"));
    (sound.volume, sound.pitch)
}

/// A window joined to a server through memory, with the sounds it plays
/// recorded: the client half of a pickup (`ClientPacketListener`).
pub(crate) fn with_remote_game<R>(
    f: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>) -> R,
) -> R {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    let mut game = sunrise_client_gameplay::game::ClientGame::new(
        &loaded.resources,
        loaded.client_res.unwrap(),
        loaded.pack,
        loaded.client_pack.unwrap(),
        loaded.gui.unwrap(),
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
    let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
    game.connect_local(pipe, "Steve").unwrap();
    game.client.audio.0.capture = Some(Vec::new());
    f(&mut game)
}
