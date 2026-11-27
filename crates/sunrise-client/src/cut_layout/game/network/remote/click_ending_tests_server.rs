use super::*;

/// EndCrystalItem.useOn ends the click either way: SUCCESS (the arm swings) on obsidian with
/// air and nothing above, FAIL (no swing) on grass or with the player in the space above. Each
/// sends one use_item_on and tries neither UseItem nor the off hand, whose shield stays down.
/// On cooldown its useOn doesn't run, and the click goes on to the item.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn an_end_crystal_click_on_a_block_ends_there() {
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        use std::sync::Arc;
        use sunrise_core::geometry::Vec3;
        use sunrise_gameplay_common::game::Hit;
        use sunrise_protocol::packets::play;
        use {
            sunrise_level::chunk::Chunk, sunrise_sim::first_person_sim::Hand,
            sunrise_sim::motion::PlayerMotion,
        };
        let (mut peer, _writer) = connect_menu_test(game);
        peer.set_read_timeout(Some(std::time::Duration::from_millis(50)))
            .unwrap();
        game.set_local_mode(sunrise_command::args::GameMode::Survival);
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let registry = &game.pack.assets.registry;
        let obsidian = registry.default_state("minecraft:obsidian").unwrap();
        let grass = registry.default_state("minecraft:grass_block").unwrap();
        // Under the player (its own box fills the space above), clear, and grass.
        game.world.set(0, 63, 0, obsidian).unwrap();
        game.world.set(0, 63, 2, obsidian).unwrap();
        game.world.set(0, 63, 4, grass).unwrap();
        let crystal = ItemStack::new("minecraft:end_crystal", 4);
        game.inventory[0] = Some(crystal.clone());
        game.offhand = Some(ItemStack::new("minecraft:shield", 1));
        game.selected = 0;
        while peer.recv().is_ok() {}
        let aim = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>, z: i32| {
            game.set_test_forced_hit(Some(Hit {
                pos: [0, 63, z],
                face: [0, 1, 0],
                point: [0.5, 64.0, f64::from(z) + 0.5],
                inside: false,
            }));
        };
        // The whole click (Minecraft.startUseItem): it ends at the block, so neither the main
        // hand's air use nor the offhand's shield is tried, and nothing is held in use.
        let mut click = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>, z: i32| {
            aim(game, z);
            let swings = game.swings_started;
            game.remote_use(mod_api::InteractionAttempt::Press);
            let mut sent = Vec::new();
            while let Ok((id, bytes)) = peer.recv() {
                match play::Serverbound::decode(id, &bytes) {
                    Ok(play::Serverbound::UseItemOn(_)) => sent.push("use_item_on"),
                    Ok(play::Serverbound::UseItem(_)) => sent.push("use_item"),
                    _ => {}
                }
            }
            assert!(game.using.is_none(), "z={z}");
            (game.swings_started != swings, sent)
        };
        assert_eq!(click(game, 2), (true, vec!["use_item_on"]));
        assert_eq!(click(game, 4), (false, vec!["use_item_on"]));
        assert_eq!(click(game, 0), (false, vec!["use_item_on"]));
        // On cooldown the block use itself passes.
        game.cooldowns
            .add(sunrise_items::cooldowns::group(&crystal, None), 20);
        aim(game, 2);
        assert!(!game.remote_use_block(Hand::Main, mod_api::InteractionAttempt::Press, None));
    });
}

/// #14773: Flint and steel and fire charge useOn on a client level ends the
/// click (returning true), so when held with an unusable off-hand item, the
/// click does not fall through to UseItem or try the off hand.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn flint_and_steel_use_on_ends_the_click() {
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::Hit;
    use {
        crate::game::load_resources, crate::settings::Settings, sunrise_client_lib::audio::Audio,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        sunrise_gameplay_common::game::execution_role::ExecutionRole,
    };
    use {sunrise_sim::first_person_sim::Hand, sunrise_sim::motion::PlayerMotion};

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
        sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = ExecutionRole::Client;
    let mut game = sunrise_client_gameplay::game::ClientGame::new(
        &loaded.resources,
        loaded.client_res.unwrap(),
        loaded.pack,
        loaded.client_pack.unwrap(),
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
        sunrise_gameplay::game::server_side::integrated::serve,
        sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
        crate::game::session_verifier(),
        crate::game::profile_resolver,
    )
    .unwrap();
    let (client, _server) = sunrise_protocol::connection::Pipe::pair();
    game.connect_local(client, "FlintTest").unwrap();
    game.client.remote.as_mut().unwrap().entity_id = 7;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.inventory[0] = Some(ItemStack::new("minecraft:flint_and_steel", 1));
    game.offhand = Some(ItemStack::new("minecraft:stick", 1));
    game.selected = 0;
    game.set_test_forced_hit(Some(Hit {
        pos: [0, 63, 0],
        face: [0, 1, 0],
        point: [0.5, 64.0, 0.5],
        inside: false,
    }));

    let done = game.remote_use_block(Hand::Main, mod_api::InteractionAttempt::Press, None);
    assert!(done, "flint and steel useOn must end the click");
}

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
