use mod_api::ClientHost;
use sunrise_client::level_loading::Reason;
use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
use sunrise_core::geometry::Vec3;
use sunrise_protocol::{
    connection::Pipe,
    packets::play::{self, Clientbound as C, SpawnInfo},
};
use sunrise_sim::motion::PlayerMotion;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycle,
};

#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn a_named_mob_the_server_camera_is_attached_to_shows_no_name_tag() {
    run_a_named_mob_the_server_camera_is_attached_to_shows_no_name_tag();
}

#[test]
#[ignore = "daily"]
fn client_camera_entity_returns_to_player_after_removal() {
    with_client_game(|game| {
        let camera = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
            (game as &dyn ClientHost).camera_entity()
        };
        assert!(camera(game).is_none(), "the fixture has no player");
        game.native_lifecycle = NativeLifecycle::default();
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "CameraReadTest").unwrap();
        let mut spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 3,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
        let remote = game.client.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;
        assert!(camera(game).is_none(), "a level alone has no player");
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        let local = camera(game).unwrap();
        assert_eq!(
            local.id(),
            7,
            "use the remote identity, not PlayerMotion's default"
        );
        assert_eq!(local.position(game), Some([0.0; 3]));

        let creeper = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:entity_type")
            .unwrap()
            .id("minecraft:creeper")
            .unwrap();
        game.handle_packet(C::AddEntity(play::AddEntity {
            id: 31,
            uuid: 9,
            kind: creeper,
            position: [10.0, 64.0, 12.0],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        }))
        .unwrap();
        game.remote_set_camera(31);
        assert!(!game.controls_camera());
        assert!(game.hides_as_camera(31));
        assert!(game.client.spectated_post_effect.is_some());
        let attached = camera(game).unwrap();
        game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![99] }))
            .unwrap();
        assert_eq!(camera(game), Some(attached));
        assert_eq!(attached.id(), 31);
        assert_eq!(attached.position(game), Some([10.0, 64.0, 12.0]));
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 99 }))
            .unwrap();
        assert_eq!(
            camera(game),
            Some(attached),
            "unknown packet targets are ignored"
        );
        game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![31] }))
            .unwrap();
        // Vanilla 26.3 retains the removed camera entity and post effect until SetCamera replaces it.
        assert_eq!(camera(game), Some(attached));
        assert_eq!(
            attached.position(game),
            None,
            "removed camera entity is no longer in the level"
        );
        assert!(
            !game.controls_camera(),
            "movement stays paused while camera remains detached"
        );
        assert!(!game.hides_as_camera(31));
        assert!(game.client.spectated_post_effect.is_some());
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 7 }))
            .unwrap();
        assert_eq!(camera(game), Some(local));
        assert!(
            game.controls_camera(),
            "movement input and position packets are enabled after SetCamera restores player"
        );
        assert!(game.client.spectated_post_effect.is_none());

        game.client.title.0 =
            Some(game.new_title(std::env::current_dir().unwrap_or_default().join("saves")));
        assert!(
            camera(game).is_none(),
            "the title screen has no camera entity"
        );
        game.client.title.0 = None;
        assert_eq!(camera(game), Some(local));

        spawn.dimension = "test:other_overworld".into();
        game.handle_packet(C::Respawn(play::Respawn { spawn, keep: 0 }))
            .unwrap();
        assert!(
            camera(game).is_none(),
            "replacement waits for its new player"
        );
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        let fresh = camera(game).unwrap();
        assert_eq!(fresh.id(), local.id());
        assert_ne!(fresh.epoch(), local.epoch());
        assert_eq!(local.position(game), None, "old level handles stay stale");
        assert_eq!(fresh.position(game), Some([0.0; 3]));
        game.client_logging_out();
        assert!(camera(game).is_none());
        assert_eq!(fresh.position(game), None);
    });
}

pub(crate) fn run_a_named_mob_the_server_camera_is_attached_to_shows_no_name_tag() {
    let path = std::env::var("SUNRISE_NAME_TAG_CAMERA_WITNESS")
        .expect("run through tools/test_camera_parity.py, which records the witness");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "failed to read SUNRISE_NAME_TAG_CAMERA_WITNESS={path} (run through tools/test_camera_parity.py): {err}"
        )
    });
    let witness: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let shown = |camera_is_entity: bool| {
        witness["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["camera_is_entity"] == camera_is_entity)
            .unwrap()["name"]
            .as_bool()
            .unwrap()
    };
    with_client_game(|game| {
        game.native_lifecycle = NativeLifecycle::default();
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "NameCameraTest").unwrap();
        let spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 3,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
        let remote = game.client.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        let creeper = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:entity_type")
            .unwrap()
            .id("minecraft:creeper")
            .unwrap();
        for (id, x, name) in [(31, 10.0, "Attached"), (32, 12.0, "Other")] {
            game.handle_packet(C::AddEntity(play::AddEntity {
                id,
                uuid: id as u128,
                kind: creeper,
                position: [x, 64.0, 12.0],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            }))
            .unwrap();
            let at = game.mobs.iter().position(|m| m.id == id as u32).unwrap();
            game.mobs[at].custom_name = Some(name.into());
            game.mobs[at].name_visible = true;
        }
        let eye = glam::DVec3::new(11.0, 65.0, 10.0);
        let mut expected = Vec::new();
        if shown(true) {
            expected.push("Attached".to_owned());
        }
        if shown(false) {
            expected.push("Other".to_owned());
        }
        expected.sort();
        let tags = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
            let mut names = game.shown_name_tags(eye);
            names.sort();
            names
        };
        assert_eq!(tags(game), vec!["Attached".to_owned(), "Other".to_owned()]);
        game.remote_set_camera(31);
        assert!(game.hides_as_camera(31), "first person from the creeper");
        assert_eq!(tags(game), expected);
    });
}
