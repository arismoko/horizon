use super::*;
use crate::game::server_side::native_entity_refs::tests::with_client_game;

#[test]
#[ignore = "daily"]
fn menu_title_snapshot_tracks_open_replacement_and_close() {
    with_client_game(|game| {
        let (client, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(client, "menu-title-test").unwrap();
        let remote = game.client.remote.as_mut().unwrap();
        for value in [Some("first"), Some("replacement"), None] {
            remote.set_menu_title(value.map(sunrise_game_data::text::Text::literal));
            assert_eq!(
                remote.menu_title_snapshot(),
                remote.menu_title.try_into_api().unwrap().as_ref()
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn menu_title_snapshot_follows_packet_and_close_paths() {
    use std::sync::Arc;
    use sunrise_core::nbt::Tag;
    use sunrise_protocol::packets::play::{self, Clientbound as C};

    with_client_game(|game| {
        let (client, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(client, "menu-title-paths").unwrap();
        let menu_id = |game: &sunrise_client_gameplay::game::ClientGame<'_>, name: &str| {
            game.client
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:menu")
                .unwrap()
                .id(name)
                .unwrap()
        };
        let chest = menu_id(game, "minecraft:generic_9x3");
        let lectern = menu_id(game, "minecraft:lectern");
        let open = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>, menu, title: &str| {
            game.handle_packet(C::OpenScreen(play::OpenScreen {
                container: 7,
                menu,
                title: Tag::String(title.into()),
            }))
            .unwrap();
        };
        let snapshot = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
            let shared = game
                .client
                .remote
                .as_ref()
                .unwrap()
                .shared_menu_title_snapshot()
                .unwrap();
            let observed = game.test_observed_title().unwrap();
            assert!(Arc::ptr_eq(&observed, &shared));
            shared
        };
        open(game, chest, "first");
        let first = snapshot(game);
        open(game, chest, "replacement");
        let replacement = snapshot(game);
        assert!(!Arc::ptr_eq(&first, &replacement));
        assert_ne!(first, replacement);
        game.handle_packet(C::ContainerClose(play::ContainerClose { container: 7 }))
            .unwrap();
        assert!(
            game.client
                .remote
                .as_ref()
                .unwrap()
                .shared_menu_title_snapshot()
                .is_none()
        );
        assert!(game.test_observed_title().is_none());
        open(game, chest, "again");
        snapshot(game);
        game.remote_close_container();
        assert!(
            game.client
                .remote
                .as_ref()
                .unwrap()
                .shared_menu_title_snapshot()
                .is_none()
        );
        open(game, chest, "before reject");
        game.remote_reject_container(7);
        assert!(
            game.client
                .remote
                .as_ref()
                .unwrap()
                .shared_menu_title_snapshot()
                .is_none()
        );
        open(game, chest, "before dimension change");
        spawn_menu_test(game);
        assert!(
            game.client
                .remote
                .as_ref()
                .unwrap()
                .shared_menu_title_snapshot()
                .is_none()
        );
        open(game, chest, "before lectern");
        snapshot(game);
        open(game, lectern, "lectern");
        assert!(
            game.client
                .remote
                .as_ref()
                .unwrap()
                .shared_menu_title_snapshot()
                .is_none()
        );
        game.remote_lectern_page(1);
        assert!(
            game.client
                .remote
                .as_ref()
                .unwrap()
                .shared_menu_title_snapshot()
                .is_none()
        );
        game.handle_packet(C::ContainerClose(play::ContainerClose { container: 7 }))
            .unwrap();
        assert!(
            game.client
                .remote
                .as_ref()
                .unwrap()
                .shared_menu_title_snapshot()
                .is_none()
        );
        assert!(game.test_observed_title().is_none());
    });
}
