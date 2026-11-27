// `client_block_changed` through the real client: server updates, batched section updates,
// predictions and their reconciliation each tell a changed block once, after the write.
use mod_api::{ClientBlockChangedEvent, ClientHost};
use std::sync::{Arc, Mutex};
use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::remote::{BlockStateId, C, Chunk, play};
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
};

/// What a listener heard: the dimension, the position, the old and new blocks, whether it was
/// predicted, and the block the level reads there inside the listener.
type Heard = (String, [i32; 3], String, String, bool, String);
static HEARD: Mutex<Vec<Heard>> = Mutex::new(Vec::new());

fn block_name(key: mod_api::BlockKey) -> String {
    mod_api::__private::block_key_id(key).unwrap().to_owned()
}

struct Map;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Map {
    fn client_block_changed(&self, host: &dyn ClientHost, event: &mut ClientBlockChangedEvent<'_>) {
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        let reads = host
            .level(event.dimension)
            .and_then(|level| level.block(host, pos))
            .map(block_name)
            .unwrap_or_default();
        HEARD.lock().unwrap().push((
            event.dimension.to_owned(),
            pos,
            block_name(event.old.block()),
            block_name(event.new.block()),
            event.predicted,
            reads,
        ));
    }
}

fn heard() -> Vec<Heard> {
    std::mem::take(&mut *HEARD.lock().unwrap())
}

fn change(pos: [i32; 3], old: &str, new: &str, predicted: bool) -> Heard {
    let id = |name: &str| format!("minecraft:{name}");
    (id("overworld"), pos, id(old), id(new), predicted, id(new))
}

fn with_client(
    name: &str,
    test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>, BlockStateId),
) {
    let selector = format!("cut_layout::game::remote::tests::client_block_changed_tests::{name}");
    if run_fresh_cases(&selector, "SUNRISE_CLIENT_BLOCK_CHANGED_CHILD", &["map"]) {
        return;
    }
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("map", Box::new(Map))].into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "BlockChangeMap").unwrap();
            let spawn = play::SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".into(),
                seed: 0,
                game_type: 1,
                previous_game_type: None,
                debug: false,
                flat: true,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            };
            game.enter_remote_dimension(
                &spawn,
                RemoteDimensionEntry::Login,
                sunrise_client::level_loading::Reason::Other,
            );
            game.client.remote.as_mut().unwrap().logged_in = true;
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(1, 64, 1, stone).unwrap();
            heard();
            test(game, stone);
        });
    });
}

fn update(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    pos: [i32; 3],
    state: BlockStateId,
) {
    game.handle_packet(C::BlockUpdate(play::BlockUpdate {
        pos,
        state: state.0 as i32,
    }))
    .unwrap();
}

/// A server update is told once, after the write; one that keeps the state is not told; each
/// block of a section update is told, and an unchanged one in the batch is not.
#[test]
#[ignore = "daily"]
fn server_updates_tell_each_changed_block_once() {
    with_client(
        "server_updates_tell_each_changed_block_once",
        |game, stone| {
            let air = game.world.air;
            update(game, [1, 64, 1], air);
            assert_eq!(heard(), [change([1, 64, 1], "stone", "air", false)]);
            update(game, [1, 64, 1], air);
            assert!(
                heard().is_empty(),
                "a write that keeps the state is not told"
            );

            // Section (0, 4, 0): y 64..80. Positions are packed as x << 8 | z << 4 | y.
            let packed = |[x, y, z]: [i32; 3], state: BlockStateId| {
                i64::from(state.0) << 12 | i64::from(x << 8 | z << 4 | (y & 15))
            };
            game.handle_packet(C::SectionBlocksUpdate(play::SectionBlocksUpdate {
                section: [0, 4, 0],
                changes: vec![
                    packed([2, 64, 3], stone),
                    packed([1, 64, 1], air),
                    packed([4, 65, 5], stone),
                ],
            }))
            .unwrap();
            assert_eq!(
                heard(),
                [
                    change([2, 64, 3], "air", "stone", false),
                    change([4, 65, 5], "air", "stone", false),
                ]
            );
        },
    );
}

/// A predicted break is told at once as predicted; the server's update held under it is not
/// told; the acknowledgement that brings the refused block back tells it, not predicted.
#[test]
#[ignore = "daily"]
fn predictions_and_their_reconciliation_are_told() {
    with_client(
        "predictions_and_their_reconciliation_are_told",
        |game, stone| {
            game.predict_break([1, 64, 1], stone, 7);
            assert_eq!(heard(), [change([1, 64, 1], "stone", "air", true)]);
            update(game, [1, 64, 1], stone);
            assert!(heard().is_empty(), "held under the prediction");
            game.handle_packet(C::BlockChangedAck(play::BlockChangedAck { sequence: 7 }))
                .unwrap();
            assert_eq!(heard(), [change([1, 64, 1], "air", "stone", false)]);

            game.predict_blocks(&[(BlockPos::new(2, 64, 2), stone)], 8);
            assert_eq!(heard(), [change([2, 64, 2], "air", "stone", true)]);
        },
    );
}
