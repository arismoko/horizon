//! `client_take_item_entity` through a logged-in client's packet handler (#5595).
use crate::game::ClientGame;
use mod_api::{ClientHost, ClientTakeItemEntityEvent};
use std::sync::{Arc, Mutex};
use sunrise_protocol::packets::play::{self, Clientbound as C};

/// What a listener heard: the collected id, whether the client still resolved it then, the
/// collector's id and the amount.
type Heard = (u32, bool, Option<u32>, i32);

struct Notifier(Arc<Mutex<Vec<Heard>>>);

#[mod_api::__subscriptions]
impl mod_api::Hooks for Notifier {
    fn client_take_item_entity(
        &self,
        host: &dyn ClientHost,
        event: &mut ClientTakeItemEntityEvent,
    ) {
        let collected = event.collected.id();
        self.0.lock().unwrap().push((
            collected,
            host.entity(collected).is_some(),
            event.collector.map(|c| c.id()),
            event.amount,
        ));
    }
}

fn add(game: &mut ClientGame<'_>, id: i32, kind: &str) {
    let kind = game
        .client
        .remote
        .as_ref()
        .unwrap()
        .builtin
        .get("minecraft:entity_type")
        .unwrap()
        .id(kind)
        .unwrap();
    game.handle_packet(C::AddEntity(play::AddEntity {
        id,
        uuid: 0x5595_0000 + id as u128,
        kind,
        position: [1.5, 64.0, 1.5],
        velocity: [0.0; 3],
        pitch: 0,
        yaw: 0,
        head_yaw: 0,
        data: 0,
    }))
    .unwrap();
}

fn take(game: &mut ClientGame<'_>, item: i32, player: i32, amount: i32) {
    game.handle_packet(C::TakeItemEntity(play::TakeItemEntity {
        item,
        player,
        amount,
    }))
    .unwrap();
}

/// The local player's pickup of an orb and a zombie's of an arrow are each told once, while the
/// client still tracks what was taken; an untracked collector is None, and an untracked
/// collected entity is not told.
#[test]
#[ignore = "daily"]
fn a_client_hears_pickups_of_entities_it_tracks() {
    use crate::game::client_side::native_client_session::tests::logged_client;
    use crate::game::native_entity_refs::tests::with_client_game;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "notifier",
        Box::new(Notifier(heard.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let _server = logged_client(game);
            // The local player's body, which a logged-in client has by its first tick.
            game.common.player = Some(
                sunrise_sim::motion::PlayerMotion::new(
                    game.common.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            let local = game.client.remote.as_ref().unwrap().entity_id;
            add(game, 50, "minecraft:experience_orb");
            add(game, 51, "minecraft:arrow");
            add(game, 52, "minecraft:zombie");
            take(game, 50, local, 1);
            take(game, 51, 52, 1);
            take(game, 50, 99, 1);
            take(game, 98, local, 3);
            assert_eq!(
                *heard.lock().unwrap(),
                [
                    (50, true, Some(local as u32), 1),
                    (51, true, Some(52), 1),
                    (50, true, None, 1),
                ]
            );
            assert!(game.entity(51).is_none(), "the arrow goes after it is told");
        });
    });
}
