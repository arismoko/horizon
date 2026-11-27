use mod_api::{EntityRef, HostTransaction, Sidebar, SidebarLine, Text, TextFormError, WorldEpoch};
use std::sync::{Arc, mpsc};
use sunrise_core::nbt::Tag;
use sunrise_protocol::{
    buf::Reader,
    ids::play::clientbound::{SET_DISPLAY_OBJECTIVE, SET_OBJECTIVE, SET_SCORE},
    packets::play::Clientbound as C,
};
use sunrise_sim::lan::{Out, PlayerSender};
use {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_guest_turn_tests::with_guests,
};

fn received_sidebar_packets(rx: &mpsc::Receiver<Out>) -> Vec<C> {
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => panic!("unexpected disconnect"),
        })
        .filter_map(|body| {
            let mut reader = Reader::new(&body);
            let id = reader.varint().unwrap();
            matches!(id, SET_OBJECTIVE | SET_DISPLAY_OBJECTIVE | SET_SCORE)
                .then(|| C::decode(id, reader.rest()).unwrap())
        })
        .collect()
}

#[test]
#[ignore = "daily"]
fn sidebar_routes_by_live_identity_and_publishes_only_valid_committed_changes() {
    if run_fresh_cases(
        "cut_layout::game::native_server::sidebar_tests::sidebar_routes_by_live_identity_and_publishes_only_valid_committed_changes",
        "SUNRISE_TEST_PERSONAL_SIDEBAR",
        &["sidebar"],
    ) {
        return;
    }
    with_guests(|game| {
        let [alpha_rx, beta_rx] = [12, 13].map(|conn| {
            let (tx, rx) = mpsc::channel();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(conn, |guest| {
                    guest.sender = PlayerSender::new(tx, Arc::default());
                })
                .unwrap();
            rx
        });
        let player = game.player(12).unwrap();
        let peer = game.player(13).unwrap();
        let sidebar = |title| {
            Some(Sidebar {
                title: Text::literal(title),
                lines: vec![SidebarLine {
                    text: Text::literal("line"),
                    value: None,
                }],
            })
        };
        let invalid = || {
            Some(Sidebar {
                title: Text::literal("\0".repeat(32_768)),
                lines: Vec::new(),
            })
        };
        let stale_epoch = EntityRef::new(
            WorldEpoch::new(player.entity().epoch().get() + 1),
            player.entity().id(),
        );
        for missing in [
            mod_api::__private::game::player_ref(stale_epoch, player.uuid()),
            mod_api::__private::game::player_ref(peer.entity(), player.uuid()),
            mod_api::__private::game::player_ref(player.entity(), 99),
        ] {
            assert_eq!(missing.set_sidebar(game, invalid()), Ok(false));
        }
        assert!(received_sidebar_packets(&alpha_rx).is_empty());
        assert!(received_sidebar_packets(&beta_rx).is_empty());

        // The acting body can be displaced by another guest's turn; the connection is still live.
        game.as_guest(12, |game| {
            game.as_guest(13, |game| {
                assert_eq!(player.set_sidebar(game, sidebar("Original")), Ok(true));
            })
            .unwrap();
        })
        .unwrap();
        let first = received_sidebar_packets(&alpha_rx);
        let [
            C::SetObjective(original),
            C::SetDisplayObjective(_),
            C::SetScore(_),
        ] = first.as_slice()
        else {
            panic!("only the recipient should get the original private objective");
        };
        assert_eq!(
            original.display,
            Some((Tag::String("Original".into()), 0, None))
        );
        assert!(received_sidebar_packets(&beta_rx).is_empty());
        assert!(game.score.board.objective(&original.name).is_none());

        assert_eq!(
            player.set_sidebar(game, invalid()),
            Err(TextFormError::Malformed)
        );
        assert!(received_sidebar_packets(&alpha_rx).is_empty());
        let aborted: Result<(), mod_api::TransactionError<()>> = game.transaction(|host, _| {
            assert_eq!(player.set_sidebar(host, None), Ok(true));
            assert!(received_sidebar_packets(&alpha_rx).is_empty());
            Err(())
        });
        assert!(aborted.is_err());
        assert!(received_sidebar_packets(&alpha_rx).is_empty());

        game.transaction(|host, _| {
            assert_eq!(player.set_sidebar(host, sidebar("Committed")), Ok(true));
            assert!(received_sidebar_packets(&alpha_rx).is_empty());
            Ok::<(), ()>(())
        })
        .unwrap();
        let committed = received_sidebar_packets(&alpha_rx);
        let [
            C::SetObjective(removed),
            C::SetObjective(added),
            C::SetDisplayObjective(_),
            C::SetScore(_),
        ] = committed.as_slice()
        else {
            panic!(
                "the original objective must survive refusal and abort, then be replaced on commit"
            );
        };
        assert_eq!(removed.name, original.name);
        assert_eq!(removed.method, 1);
        assert_eq!(
            added.display,
            Some((Tag::String("Committed".into()), 0, None))
        );
        assert!(received_sidebar_packets(&beta_rx).is_empty());
    });
}

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
