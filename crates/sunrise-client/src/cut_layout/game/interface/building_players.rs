#![cfg(test)]
use sunrise_gameplay_common::game::Dim;
#[cfg(test)]
pub(crate) mod tests {

    use super::*;

    use sunrise_core::geometry::Vec3;

    #[test]
    #[ignore = "daily"]
    fn for_player_keeps_the_outer_body_in_its_original_dimension() {
        use crate::settings::Settings;
        use std::sync::{Arc, atomic::AtomicI32, mpsc};
        use sunrise_protocol::packets::configuration::ClientInformation;
        use sunrise_sim::{
            lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
            motion::PlayerMotion,
        };
        use {
            crate::game::load_resources, sunrise_gameplay_common::game::Options,
            sunrise_gameplay_common::game::Progress,
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
            "sunrise-building-turn-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&scratch).unwrap();
        game.server.world_root = scratch.clone();
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(10.5, 64.0, 0.5)).unwrap());
        let original = game.player.as_ref().unwrap().bounds();
        let net = NetConfig {
            announce: false,
            ..NetConfig::lan(0, "building-test".into(), 2)
        };
        game.lan_host(
            sunrise_gameplay::game::server_side::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            net,
            HubConfig {
                default_game_mode: 0,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: Dim::Overworld.id().into(),
                allow_flight: false,
                seed: 0,
                max_players: 2,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn: 2,
                uuid: 2,
                name: "NetherGuest".into(),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            2,
            [20.5, 64.0, 0.5],
            2,
        );
        guest.dimension = Dim::Nether.id().into();
        guest.placed = true;
        guest.loaded = true;
        game.server.lan.as_mut().unwrap().hub.guests.push(guest);
        let seen = game.for_player(
            sunrise_gameplay_common::game::commands_common::Target::Guest(2),
            |g| {
                assert_eq!(g.dim, Dim::Nether);
                // for_player already switched the level before entering the guest.
                // The host's body must not become a phantom blocker in the Nether.
                assert!(g.server.lan_building_boxes.0.boxes(Dim::Nether).is_empty());
                assert_eq!(
                    g.server.lan_building_boxes.0.boxes(Dim::Overworld),
                    vec![(
                        original.min_position().components(),
                        original.max_position().components()
                    )]
                );
                g.player.as_ref().unwrap().position
            },
        );
        assert_eq!(seen, Some(Vec3::new(20.5, 64.0, 0.5)));
        assert_eq!(game.dim, Dim::Overworld);
        assert_eq!(
            game.player.as_ref().unwrap().position,
            Vec3::new(10.5, 64.0, 0.5)
        );
        drop(game);
        assert!(scratch.starts_with(std::env::temp_dir()));
        std::fs::remove_dir_all(scratch).unwrap();
    }
}
