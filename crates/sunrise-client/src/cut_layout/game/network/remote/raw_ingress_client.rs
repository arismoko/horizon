#![cfg(all(test, feature = "client"))]
use sunrise_client_gameplay::remote::chunks::{Next, Received};
use sunrise_client_lib::net::Event;
use sunrise_protocol::packets::play;
use sunrise_protocol::packets::play::Clientbound as C;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "bundle_parity_tests_client.rs"]
pub(crate) mod bundle_parity_tests;

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "post_effects_parity_tests_client.rs"]
pub(crate) mod post_effects_parity_tests;

#[cfg(feature = "client")]
#[cfg(test)]
pub(crate) mod tests {
    use super::super::tests::{connect_play_test, spawn_menu_test};
    use super::*;
    use crate::game::server_side::native_entity_refs::tests::with_client_game;

    #[test]
    #[ignore = "daily"]
    fn executor_waits_for_entity_removal_behind_a_decoding_chunk() {
        with_client_game(|game| {
            let (_reader, _writer) = connect_play_test(game, |_, _| {});
            spawn_menu_test(game);
            let remote = game.client.remote.as_mut().unwrap();
            let format = remote.chunk_format.clone();
            let publish = remote.session.received_event_probe();
            publish(Event::Status(String::new()));
            let (_, credit) = remote
                .session
                .poll()
                .into_iter()
                .next()
                .unwrap()
                .into_parts();
            // Insert the leased decoder slot without supplying its result yet.
            assert!(matches!(remote.decoder.next(), Next::Decoding));
            remote.backlog.push_back(Received::Chunk(format, credit));
            publish(Event::Packet(Box::new(C::RemoveEntities(
                play::RemoveEntities { ids: vec![20] },
            ))));
            game.spawn_entity(20, "minecraft:pig", [2.0, 64.0, 0.0], [0.0; 3]);
            let entity = game.entity(20).unwrap();
            assert_eq!(entity.position(game), Some([2.0, 64.0, 0.0]));

            let (record, calls) = std::sync::mpsc::channel();
            let executor = game.client.client_tasks.0.executor("producer").unwrap();
            executor.execute(Box::new(move |host| {
                record.send(entity.position(host)).unwrap()
            }));

            game.poll_remote().unwrap();
            assert!(matches!(
                calls.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ));
            assert_eq!(entity.position(game), Some([2.0, 64.0, 0.0]));

            // Retire the synthetic chunk slot without relying on worker timing.
            assert!(matches!(
                game.client.remote.as_mut().unwrap().backlog.pop_front(),
                Some(Received::Chunk(..))
            ));
            game.poll_remote_before_tick().unwrap();
            assert_eq!(calls.try_recv(), Ok(None));
        });
    }
}
