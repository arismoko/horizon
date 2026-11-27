use crate::settings::Settings;
use sunrise_sim::mobs::MobEvent;
use {
    crate::game::load_resources, sunrise_gameplay_common::game::Options,
    sunrise_gameplay_common::game::Progress,
};

#[test]
#[ignore = "daily"]
fn local_mob_events_play_without_entering_the_shared_capture() {
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
    options.game_dir =
        Some(std::env::temp_dir().join(format!("sunrise-h069-unit-{}", std::process::id())));
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
    game.start_audio_capture();
    game.handle_mob_events(vec![
        MobEvent::Sound("entity.cow.hurt", [1.0, 100.0, 2.0], 0.5, 1.0),
        MobEvent::LocalSound("entity.breeze.whirl", [3.0, 100.0, 4.0], 0.8, 0.7),
        MobEvent::Sound("entity.zombie.hurt", [5.0, 100.0, 6.0], 1.0, 0.9),
    ])
    .unwrap();
    // Server role queues shared sounds and skips client-only LocalSound.
    let shared = std::mem::take(&mut game.server.pending_sounds);
    game.stop_audio_capture();
    assert_eq!(shared.len(), 2, "LocalSound must not become a shared sound");
    assert_eq!(shared[0].event, "minecraft:entity.cow.hurt");
    assert_eq!(shared[1].event, "minecraft:entity.zombie.hurt");
    assert_eq!(
        (
            shared[0].position,
            shared[0].volume,
            shared[0].pitch,
            shared[0].source
        ),
        ([1.0, 100.0, 2.0], 0.5, 1.0, 6)
    );
    assert_eq!(
        (
            shared[1].position,
            shared[1].volume,
            shared[1].pitch,
            shared[1].source
        ),
        ([5.0, 100.0, 6.0], 1.0, 0.9, 5)
    );
    // Client role plays LocalSound locally without entering shared capture.
    sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
        game.client.audio.0.capture = Some(Vec::new());
        let mut events = vec![MobEvent::LocalSound(
            "entity.breeze.whirl",
            [3.0, 100.0, 4.0],
            0.8,
            0.7,
        )];
        game.handle_client_mob_events(&mut events);
        assert_eq!(
            game.client
                .audio
                .0
                .heard
                .get("minecraft:entity.breeze.whirl"),
            Some(&1)
        );
        assert!(game.client.audio.0.capture.as_ref().unwrap().is_empty());
    });
}

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;
