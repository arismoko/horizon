#![cfg(test)]
#[cfg(feature = "client")]
#[cfg(test)]
pub(crate) mod tests {
    use sunrise_items::items::ItemStack;
    use sunrise_sim::first_person_sim::Hand;
    use {
        crate::game::load_resources, crate::settings::Settings, sunrise_client_lib::audio::Audio,
        sunrise_gameplay_common::game::Hit, sunrise_gameplay_common::game::Options,
        sunrise_gameplay_common::game::Progress,
        sunrise_gameplay_common::game::execution_role::ExecutionRole,
    };

    #[test]
    #[ignore = "daily"]
    fn remote_item_use_tnt_predicts_success_without_consumption_or_sound() {
        let root = std::env::var_os("SUNRISE_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                let candidate = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/sunrise-client")
                    .join(sunrise_core::VANILLA_VERSION);
                if candidate.exists() {
                    Some(candidate)
                } else {
                    let main_candidate = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../../sunrise/.local/sunrise-client")
                        .join(sunrise_core::VANILLA_VERSION);
                    main_candidate.exists().then_some(main_candidate)
                }
            })
            .expect("needs prepared resources in SUNRISE_TEST_RESOURCES");
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
        game.connect_local(client, "RemoteItemUseTest").unwrap();
        let tnt_state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:tnt")
            .unwrap();
        let pos = [0, 64, 0];
        let mut chunk = sunrise_level::chunk::Chunk::empty(0, 0);
        chunk.set(0, 64, 0, tnt_state.0);
        game.world.chunks.insert((0, 0), std::sync::Arc::new(chunk));
        game.creative = false;

        let hit = Hit {
            pos,
            face: [0, 1, 0],
            point: [0.5, 65.0, 0.5],
            inside: false,
        };

        // Fire charge on TNT: returns true, leaves consumption and sound to server.
        let fire_charge = ItemStack::new("minecraft:fire_charge", 1);
        game.inventory[0] = Some(fire_charge.clone());
        game.selected = 0;
        game.client.audio.0.heard.clear();
        assert!(game.predict_fire_starter(&fire_charge, &hit, Hand::Main, 1));
        assert_eq!(
            game.inventory[0].as_ref().map(|s| s.count),
            Some(1),
            "survival fire charge must not be consumed on client for TNT"
        );
        assert!(
            game.client.audio.0.heard.is_empty(),
            "TNT ignition sound must not play on client"
        );

        // Flint and steel on TNT: returns true without playing sound.
        let flint_steel = ItemStack::new("minecraft:flint_and_steel", 1);
        game.inventory[0] = Some(flint_steel.clone());
        game.client.audio.0.heard.clear();
        assert!(game.predict_fire_starter(&flint_steel, &hit, Hand::Main, 2));
        assert_eq!(
            game.inventory[0].as_ref().map(|s| s.count),
            Some(1),
            "flint and steel must remain in inventory"
        );
        assert!(
            game.client.audio.0.heard.is_empty(),
            "TNT ignition sound must not play on client"
        );
    }
}
