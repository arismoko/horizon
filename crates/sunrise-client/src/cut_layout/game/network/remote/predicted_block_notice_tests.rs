// Applied client writes, held server updates and reconciliation through the real Game.
use mod_api::{BlockPart, LevelDimension, LevelSide, OnBlockStateChangeEvent, Registrar};
use std::sync::{Arc, Mutex};
use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::remote::dry_level::DryLevel;
use sunrise_gameplay_common::game::remote::{BlockStateId, C, Chunk, play};
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
};

#[derive(Debug, PartialEq, Eq)]
struct Notice {
    dimension: LevelDimension,
    pos: [i32; 3],
    old: String,
    new: String,
    predicted: bool,
}
#[derive(Clone, Default)]
struct Observer(Arc<Mutex<Vec<Notice>>>);
#[mod_api::sunrise_part]
impl BlockPart for Observer {
    fn on_block_state_change(&self, level: LevelSide<'_>, event: &mut OnBlockStateChangeEvent) {
        let LevelSide::Client(level) = level else {
            panic!("prediction notice must be client-side")
        };
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        assert_eq!(level.block_state(pos), Some(event.new));
        self.0.lock().unwrap().push(Notice {
            dimension: event.dimension,
            pos,
            old: level.state_block(event.old).unwrap().into(),
            new: level.state_block(event.new).unwrap().into(),
            predicted: event.predicted,
        });
    }
}
struct Mod(Observer);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let stone = r.block_key("minecraft:stone");
        r.block_part(stone, self.0.clone());
    }
}
fn with_client(
    name: &str,
    test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>, &Observer, BlockStateId),
) {
    let selector = format!("cut_layout::game::remote::tests::predicted_block_notice_tests::{name}");
    if run_fresh_cases(
        &selector,
        "SUNRISE_PREDICTED_BLOCK_NOTICE_CHILD",
        &["notice"],
    ) {
        return;
    }
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "notice",
        Box::new(Mod(observer.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        with_client_game(|game| {
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "PredictionNotice").unwrap();
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
            game.world.set(2, 64, 2, stone).unwrap();
            test(game, &observer, stone);
        });
    });
}
fn expected(pos: [i32; 3], old: &str, new: &str, predicted: bool) -> Notice {
    Notice {
        dimension: LevelDimension::Overworld,
        pos,
        old: old.into(),
        new: new.into(),
        predicted,
    }
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
fn ack(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
    game.handle_packet(C::BlockChangedAck(play::BlockChangedAck { sequence: 7 }))
        .unwrap();
}

#[test]
#[ignore = "daily"]
fn local_break_notifies_at_application() {
    with_client(
        "local_break_notifies_at_application",
        |game, seen, stone| {
            game.predict_break([1, 64, 1], stone, 7);
            assert_eq!(game.world.block(1, 64, 1), game.world.air);
            assert_eq!(
                *seen.0.lock().unwrap(),
                [expected(
                    [1, 64, 1],
                    "minecraft:stone",
                    "minecraft:air",
                    true
                )]
            );
        },
    );
}
#[test]
#[ignore = "daily"]
fn confirmed_prediction_has_no_duplicate() {
    with_client(
        "confirmed_prediction_has_no_duplicate",
        |game, seen, stone| {
            game.predict_break([1, 64, 1], stone, 7);
            seen.0.lock().unwrap().clear();
            let air = game.world.air;
            update(game, [1, 64, 1], air);
            ack(game);
            assert_eq!(game.world.block(1, 64, 1), game.world.air);
            assert!(seen.0.lock().unwrap().is_empty());
        },
    );
}
#[test]
#[ignore = "daily"]
fn denied_prediction_notifies_predicted_to_server_once() {
    with_client(
        "denied_prediction_notifies_predicted_to_server_once",
        |game, seen, stone| {
            game.predict_break([1, 64, 1], stone, 7);
            seen.0.lock().unwrap().clear();
            update(game, [1, 64, 1], stone);
            ack(game);
            ack(game);
            assert_eq!(game.world.block(1, 64, 1), stone);
            assert_eq!(
                *seen.0.lock().unwrap(),
                [expected(
                    [1, 64, 1],
                    "minecraft:air",
                    "minecraft:stone",
                    false
                )]
            );
        },
    );
}
#[test]
#[ignore = "daily"]
fn pre_ack_server_update_stays_unheard_and_unapplied() {
    with_client(
        "pre_ack_server_update_stays_unheard_and_unapplied",
        |game, seen, stone| {
            game.predict_break([1, 64, 1], stone, 7);
            seen.0.lock().unwrap().clear();
            update(game, [1, 64, 1], stone);
            assert_eq!(game.world.block(1, 64, 1), game.world.air);
            assert!(seen.0.lock().unwrap().is_empty());
            ack(game);
            assert_eq!(game.world.block(1, 64, 1), stone);
            assert_eq!(
                *seen.0.lock().unwrap(),
                [expected(
                    [1, 64, 1],
                    "minecraft:air",
                    "minecraft:stone",
                    false
                )]
            );
        },
    );
}
#[test]
#[ignore = "daily"]
fn ordinary_server_write_is_not_predicted() {
    with_client("ordinary_server_write_is_not_predicted", |game, seen, _| {
        let air = game.world.air;
        update(game, [2, 64, 2], air);
        assert_eq!(
            *seen.0.lock().unwrap(),
            [expected(
                [2, 64, 2],
                "minecraft:stone",
                "minecraft:air",
                false
            )]
        );
    });
}
#[test]
#[ignore = "daily"]
fn equal_state_writes_are_not_notices() {
    with_client("equal_state_writes_are_not_notices", |game, seen, stone| {
        update(game, [2, 64, 2], stone);
        game.predict_blocks(&[(BlockPos::new(1, 64, 1), stone)], 7);
        ack(game);
        assert!(seen.0.lock().unwrap().is_empty());
    });
}
#[test]
#[ignore = "daily"]
fn applied_placement_carries_predicted_true() {
    with_client(
        "applied_placement_carries_predicted_true",
        |game, seen, stone| {
            game.common
                .world
                .set(1, 64, 1, game.common.world.air)
                .unwrap();
            game.predict_blocks(&[(BlockPos::new(1, 64, 1), stone)], 7);
            assert_eq!(
                *seen.0.lock().unwrap(),
                [expected(
                    [1, 64, 1],
                    "minecraft:air",
                    "minecraft:stone",
                    true
                )]
            );
        },
    );
}
#[test]
#[ignore = "daily"]
fn unapplied_guesses_are_unheard() {
    with_client("unapplied_guesses_are_unheard", |game, seen, stone| {
        let air = game.world.air;
        game.with_level(|b, level, _| {
            let mut preview = DryLevel::new(level);
            b.set_block(
                &mut preview,
                &mut sunrise_world::level::Ticks::default(),
                BlockPos::new(1, 64, 1),
                air,
                3,
            );
        });
        assert_eq!(game.world.block(1, 64, 1), stone);
        assert!(seen.0.lock().unwrap().is_empty());
    });
}
