use crate::game::client_side::native_hotbar_slot::hotbar_slot_tests::logged_in;
use crate::game::remote::tests::client_admission_tests::add_entity;
use crate::game::remote::tests::client_admission_tests::deliver;
use crate::game::remote::tests::client_admission_tests::with_isolated_client;
use sunrise_client::sound_engine::{SoundEntity, SoundInstance};
use sunrise_core::random::LegacyRandom;
use {sunrise_protocol::packets::play, sunrise_protocol::packets::play::Clientbound as C};

#[test]
#[ignore = "daily"]
fn add_player_requires_info_before_replacing_a_tracked_entity() {
    with_isolated_client(
        "cut_layout::game::client_side::remote_client::tests::client_admission_tests::add_player_requires_info_before_replacing_a_tracked_entity",
        |game| {
            let _reader = logged_in(game);
            let id = 42;
            let uuid = 123;
            let player_at = [8.0, 65.0, 8.0];
            let cart_at = [2.0, 64.0, 2.0];
            let player = add_entity(game, id, uuid, "minecraft:player", player_at);
            deliver(game, player.clone());
            assert_eq!(game.client.remote.as_ref().unwrap().tracked(id), None);
            assert!(
                !game
                    .client
                    .remote
                    .as_ref()
                    .unwrap()
                    .level_players
                    .contains_key(&id)
            );

            deliver(
                game,
                add_entity(game, id, 124, "minecraft:minecart", cart_at),
            );
            deliver(game, C::SetCamera(play::SetCamera { camera_id: id }));
            assert_eq!(game.live_remote_camera_id(), Some(id as u32));
            let cart = game
                .carts
                .iter()
                .find(|cart| cart.id == id as u32)
                .unwrap()
                .join;
            let sound = SoundEntity::new(id as u32, cart_at, false);
            game.client.audio.0.engine.add(
                SoundInstance::entity_bound(
                    "minecraft:fixture.bound",
                    6,
                    1.0,
                    1.0,
                    LegacyRandom::new(42),
                    sound.clone(),
                ),
                true,
            );
            deliver(game, player.clone());
            assert_eq!(game.live_remote_camera_id(), Some(id as u32));
            assert!(!sound.state().removed);
            assert_eq!(
                game.client.remote.as_ref().unwrap().tracked(id),
                Some(("minecraft:minecart", cart_at))
            );
            assert_eq!(
                game.carts
                    .iter()
                    .find(|cart| cart.id == id as u32)
                    .unwrap()
                    .join,
                cart
            );

            deliver(
                game,
                C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                    actions: 1 | 8,
                    entries: vec![play::PlayerInfo {
                        uuid,
                        name: Some("Unlisted".into()),
                        listed: Some(false),
                        ..Default::default()
                    }],
                }),
            );
            let remote = game.client.remote.as_ref().unwrap();
            assert!(!remote.listed.contains(&uuid));
            assert_eq!(remote.tracked(id), Some(("minecraft:minecart", cart_at)));
            assert!(!remote.level_players.contains_key(&id));

            deliver(game, player.clone());
            let remote = game.client.remote.as_ref().unwrap();
            assert_eq!(remote.tracked(id), Some(("minecraft:player", player_at)));
            assert!(sound.state().removed);
            assert_eq!(remote.level_player(id), Some((uuid, Some("Unlisted"))));
            assert_eq!(game.live_remote_camera_id(), None);
            assert_eq!(game.remote_camera_id(), Some(id as u32));
            assert!(game.carts.iter().all(|cart| cart.id != id as u32));

            deliver(
                game,
                C::RemoveEntities(play::RemoveEntities { ids: vec![id] }),
            );
            deliver(
                game,
                C::PlayerInfoRemove(play::PlayerInfoRemove { uuids: vec![uuid] }),
            );
            deliver(game, player);
            assert_eq!(game.client.remote.as_ref().unwrap().tracked(id), None);
        },
    );
}
