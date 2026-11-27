use crate::game::client_side::viewed_worlds::tests::column;
use crate::game::remote::tests::play_bundle_tests::connect_probe;
use crate::game::remote::tests::play_bundle_tests::wait_for_bundle;
use crate::game::remote::tests::play_bundle_tests::with_probe;
use sunrise_protocol::{
    connection_groups::{self, Cell, Entry, Section},
    packets::{
        Empty,
        custom_payload::CustomPayload,
        play::{self, Clientbound as C},
    },
};

#[test]
#[ignore = "daily"]
fn a_split_frame_group_bundle_refuses_a_mismatched_companion() {
    with_probe(|game, _| {
        let (local, _reader, mut writer) = connect_probe(game);
        game.handle_packet(column(0, 0)).unwrap();
        let pos = [3, 70, 5];
        let original = game.world.block(pos[0], pos[1], pos[2]);
        assert_ne!(original.0, 0);
        let delimiter = C::BundleDelimiter(Empty).encode();
        writer.send(&delimiter).unwrap();
        writer
            .send(&C::BlockUpdate(play::BlockUpdate { pos, state: 0 }).encode())
            .unwrap();
        let companion = C::CustomPayload(CustomPayload {
            channel: connection_groups::CHANNEL.into(),
            data: Section {
                pos: [0, 4, 0],
                entries: vec![Entry {
                    cell: Cell::of([4, 70, 5]),
                    group: Some(7),
                }],
            }
            .encode()
            .unwrap(),
        });
        let body = companion
            .try_encode_with_payload_codecs(&local.codecs)
            .unwrap();
        let mut frame = Vec::new();
        writer.frame_into(&writer.contents(&body).unwrap(), &mut frame);
        let (prefix, suffix) = frame.split_at(frame.len() - 1);
        writer.write_raw(prefix).unwrap();
        game.poll_remote_events(false).unwrap();
        assert_eq!(game.world.block(pos[0], pos[1], pos[2]), original);
        assert!(game.client.remote.is_some());
        writer.write_raw(suffix).unwrap();
        writer.send(&delimiter).unwrap();
        wait_for_bundle(game);
        game.poll_remote_events(true).unwrap();
        assert!(game.client.remote.is_none());
        let Some(sunrise_client::title::Mode::Multiplayer(
            sunrise_client::multiplayer::Screen::Disconnected { reason, .. },
        )) = game.client.title.0.as_ref().map(|title| &title.mode)
        else {
            panic!("expected the multiplayer refusal screen");
        };
        assert!(
            reason
                .plain()
                .contains("connection groups: group cell not in block update")
        );
    });
}
