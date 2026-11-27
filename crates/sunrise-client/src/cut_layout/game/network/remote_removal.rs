#![cfg(test)]
#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "daily"]
    fn remove_entities_detaches_only_removed_vehicles() {
        use sunrise_protocol::packets::play::{self, Clientbound as C};
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_client_lib::audio::Audio, sunrise_gameplay_common::game::Options,
            sunrise_gameplay_common::game::Progress,
            sunrise_gameplay_common::game::execution_role::ExecutionRole,
        };
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
        game.connect_local(client, "PassengerRemovalTest").unwrap();
        for (id, name) in [
            (40, "minecart"),
            (41, "minecart"),
            (42, "armor_stand"),
            (43, "zombie"),
            (44, "falling_block"),
            (45, "falling_block"),
            (46, "falling_block"),
        ] {
            let kind = game
                .client
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:entity_type")
                .unwrap()
                .id(&format!("minecraft:{name}"))
                .unwrap();
            game.handle_packet(C::AddEntity(play::AddEntity {
                id,
                uuid: id as u128,
                kind,
                position: [0.0, 64.0, 0.0],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            }))
            .unwrap();
        }
        game.remote_set_passengers(40, vec![42]);
        game.remote_set_passengers(41, vec![43]);
        assert_eq!(game.armor_stands[0].vehicle, Some(40));
        assert_eq!(game.mobs[0].extra.mount, Some(41));
        game.handle_packet(C::RemoveEntities(play::RemoveEntities {
            ids: vec![44, 40, 46],
        }))
        .unwrap();
        let r = game.client.remote.as_ref().unwrap();
        assert_eq!(r.entities[&42].vehicle, None);
        assert_eq!(game.armor_stands[0].vehicle, None);
        assert_eq!(r.entities[&43].vehicle, Some(41));
        assert_eq!(game.mobs[0].extra.mount, Some(41));
        assert!(game.mobs[0].extra.passenger);
        assert_eq!(r.passenger_vehicle(42), None);
        assert_eq!(r.passenger_vehicle(43), Some(41));
        assert_eq!(game.falling.iter().map(|e| e.id).collect::<Vec<_>>(), [45]);
    }

    #[test]
    #[ignore = "daily"]
    fn add_entity_replaces_falling_block_without_removing_other_ids() {
        use sunrise_protocol::packets::play::{self, Clientbound as C};
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_client_lib::audio::Audio, sunrise_gameplay_common::game::Options,
            sunrise_gameplay_common::game::Progress,
            sunrise_gameplay_common::game::execution_role::ExecutionRole,
        };

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
            ExecutionRole::Client,
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
        game.connect_local(client, "SpawnTest").unwrap();
        let kind = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:entity_type")
            .unwrap()
            .id("minecraft:falling_block")
            .unwrap();
        for (id, position) in [
            (40, [1.0, 64.0, 2.0]),
            (41, [3.0, 65.0, 4.0]),
            (40, [5.0, 66.0, 6.0]),
            (42, [7.0, 67.0, 8.0]),
        ] {
            game.handle_packet(C::AddEntity(play::AddEntity {
                id,
                uuid: id as u128,
                kind,
                position,
                velocity: [0.0, -0.1, 0.0],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            }))
            .unwrap();
            assert_eq!(game.falling.iter().filter(|f| f.id == id as u32).count(), 1);
            let block = game.falling.iter().find(|f| f.id == id as u32).unwrap();
            assert_eq!(
                [block.position.x, block.position.y, block.position.z],
                position
            );
            assert_eq!(block.velocity.y, -0.1);
        }
        let blocks: Vec<_> = game
            .falling
            .iter()
            .map(|f| (f.id, [f.position.x, f.position.y, f.position.z]))
            .collect();
        assert_eq!(
            blocks,
            [
                (41, [3.0, 65.0, 4.0]),
                (40, [5.0, 66.0, 6.0]),
                (42, [7.0, 67.0, 8.0]),
            ]
        );
    }

    #[test]
    #[ignore = "daily"]
    fn remove_entities_filters_mixed_entity_lists() {
        use sunrise_protocol::packets::play::{self, Clientbound as C};
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_client_lib::audio::Audio, sunrise_gameplay_common::game::Options,
            sunrise_gameplay_common::game::Progress,
            sunrise_gameplay_common::game::execution_role::ExecutionRole,
        };

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
        game.connect_local(client, "RemovalTest").unwrap();
        for (id, kind) in [
            (40, "minecraft:falling_block"),
            (41, "minecraft:falling_block"),
            (42, "minecraft:falling_block"),
            (43, "minecraft:tnt"),
            (44, "minecraft:tnt"),
            (45, "minecraft:experience_orb"),
            (46, "minecraft:experience_orb"),
            (47, "minecraft:zombie"),
            (48, "minecraft:zombie"),
        ] {
            if matches!(kind, "minecraft:falling_block" | "minecraft:tnt") {
                assert!(game.spawn_remote_projectile(
                    kind,
                    &sunrise_protocol::packets::play::AddEntity {
                        id,
                        uuid: id as u128,
                        kind: 0,
                        position: [0.0, 64.0, 0.0],
                        velocity: [0.0; 3],
                        pitch: 0,
                        yaw: 0,
                        head_yaw: 0,
                        data: 0,
                    },
                    (0.0, 0.0),
                ));
            } else {
                game.spawn_entity(id, kind, [0.0, 64.0, 0.0], [0.0; 3]);
            }
        }
        for id in [49, 50] {
            game.dropped.push(sunrise_sim::entities::ItemEntity::new(
                id,
                sunrise_items::items::ItemStack::new("minecraft:sand", 1),
                sunrise_core::geometry::Vec3::new(0.0, 64.0, 0.0),
                sunrise_core::geometry::Vec3::ZERO,
                0.0,
            ));
        }
        game.handle_packet(C::RemoveEntities(play::RemoveEntities {
            ids: vec![40, 42, 43, 45, 47, 49],
        }))
        .unwrap();
        assert_eq!(game.falling.iter().map(|e| e.id).collect::<Vec<_>>(), [41]);
        assert_eq!(game.tnts.iter().map(|e| e.id).collect::<Vec<_>>(), [44]);
        assert_eq!(game.orbs.iter().map(|e| e.id).collect::<Vec<_>>(), [46]);
        assert_eq!(game.mobs.iter().map(|e| e.id).collect::<Vec<_>>(), [48]);
        assert_eq!(game.dropped.iter().map(|e| e.id).collect::<Vec<_>>(), [50]);
    }
}
