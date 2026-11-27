use sunrise_core::geometry::Vec3;
use sunrise_game_data::text::Text;
use sunrise_sim::motion::PlayerMotion;
use {
    crate::game::load_resources, crate::settings::Settings, sunrise_gameplay_common::game::Options,
    sunrise_gameplay_common::game::Progress,
};

/// What `command` told the player.
pub(crate) fn say(game: &mut sunrise_gameplay::game::ServerGame<'_>, command: &str) -> Vec<Text> {
    game.server.chat_capture = Some(Vec::new());
    assert!(game.run_command(command), "{command}");
    game.server.chat_capture.take().unwrap_or_default()
}

/// `TagCommand.listTags` sends through `sendFeedback(source, false, …)`,
/// while `addTag`/`removeTag` pass `true` (`CommandSourceStack.sendSuccess`):
/// only a change is repeated to the operators.
#[test]
#[ignore = "daily"]
fn tag_list_is_not_repeated_to_the_operators() {
    with_command_game("tag-list-broadcast", |game| {
        assert!(game.run_command(
            "summon pig 4.5 200 4.5 {UUID:[I;329,0,0,4],NoAI:1b,Silent:1b,NoGravity:1b}"
        ));
        // The summon's own line (a player informs the operators), sent on a tick.
        game.server.admin_feedback.clear();
        // A source that informs the operators (`shouldInformAdmins`).
        game.server.console_broadcast = Some(true);
        assert!(game.run_command("tag 00000149-0000-0000-0000-000000000004 add audit"));
        assert_eq!(game.server.admin_feedback.len(), 1);
        game.server.admin_feedback.clear();
        let said = say(game, "tag @e[type=minecraft:pig] list");
        assert_eq!(said.len(), 1);
        assert_eq!(game.server.admin_feedback.len(), 0);
    });
}

/// A server-side game that runs commands, its world in a scratch folder.
pub(crate) fn with_command_game(
    name: &str,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
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
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role =
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = sunrise_gameplay::game::ServerGame::new(
        &loaded.resources,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    let scratch = std::env::temp_dir().join(format!(
        "sunrise-saved-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&scratch).unwrap();
    game.server.world_root = scratch.clone();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(10.5, 200.0, 0.5)).unwrap());
    game.reset_entity_residency();
    game.native_lifecycle
        .begin_open(Some(mod_api::Authority::IntegratedServer));
    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(&mut game, 1, 1);
    game.build_registries();
    game.set_allow_commands(true);
    test(&mut game);
    drop(game);
    std::fs::remove_dir_all(scratch).unwrap();
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
