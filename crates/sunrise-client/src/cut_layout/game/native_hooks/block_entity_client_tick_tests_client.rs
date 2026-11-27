use crate::game::native_hooks::block_entity_client_tick_tests::CHUNK;
use crate::game::native_hooks::block_entity_client_tick_tests::CRATE;
use crate::game::native_hooks::block_entity_client_tick_tests::CRATE_AT;
use crate::game::native_hooks::block_entity_client_tick_tests::EVENTS;
use crate::game::native_hooks::block_entity_client_tick_tests::HUM;
use crate::game::native_hooks::block_entity_client_tick_tests::HUMS;
use crate::game::native_hooks::block_entity_client_tick_tests::OTHER;
use crate::game::native_hooks::block_entity_client_tick_tests::QUIET;
use crate::game::native_hooks::block_entity_client_tick_tests::QUIET_AT;
use crate::game::native_hooks::block_entity_client_tick_tests::SPAWNER_AT;
use crate::game::native_hooks::block_entity_client_tick_tests::TICK_PARTICLES;
use crate::game::native_hooks::block_entity_client_tick_tests::frame;
use crate::game::native_hooks::block_entity_client_tick_tests::install;
use crate::game::native_hooks::block_entity_client_tick_tests::prepared_root;
use crate::game::native_hooks::block_entity_client_tick_tests::rpm;
use crate::game::native_hooks::block_entity_client_tick_tests::take_events;
use crate::game::native_hooks::block_entity_client_tick_tests::ticked;
use std::sync::Arc;
use std::time::{Duration, Instant};
use sunrise_core::nbt::{Compound, Tag};
use sunrise_level::containers::BlockEntity;
use {
    crate::game::server_side::native_entity_refs::tests::with_game_role_client,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
};

#[test]
#[ignore = "daily"]
fn a_clients_frames_tick_its_declared_copies_until_they_go() {
    use sunrise_gameplay_common::game::execution_role::ExecutionRole;
    use sunrise_level::{
        chunk::Chunk,
        containers::{ModBlockEntity, ModBlockEntityFields},
    };
    use sunrise_protocol::packets::play;
    if run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::block_entity_client_tick_tests::a_clients_frames_tick_its_declared_copies_until_they_go",
        "SUNRISE_BLOCK_ENTITY_CLIENT_TICK_CHILD",
        &["game"],
    ) {
        return;
    }
    install();
    let types = [
        "client_tick_probe:hum",
        "client_tick_probe:quiet",
        "client_tick_probe:crate_shulker_box",
    ];
    with_game_role_client(prepared_root(), ExecutionRole::Client, |game| {
        game.options.dedicated = None;

        // Not connected yet: no client level, nothing told.
        game.tick_client_block_entities();
        assert!(take_events().is_empty(), "told without a client level");

        let (_sent, mut server) = crate::game::remote::tests::connect_menu_test(game);
        let remote = game.client.remote.as_mut().unwrap();
        remote.logged_in = true;
        Arc::make_mut(&mut remote.builtin).registries.insert(
            "minecraft:block_entity_type".into(),
            sunrise_protocol::registry::Ids::from_names(types.map(str::to_owned).to_vec()),
        );
        for (x, z) in [CHUNK, (101, 100)] {
            let (common, client) =
                sunrise_gameplay_common::game::SideGame::common_and_state_mut(game);
            client
                .streaming
                .insert_remote(&mut common.world, Chunk::empty(x, z));
        }
        let registry = game.pack.assets.registry;
        let hum = registry.default_state(HUM).unwrap();
        let quiet = registry.default_state(QUIET).unwrap();
        let boxed = registry.default_state(CRATE).unwrap();
        // Arrival order: the second hum, the quiet one, the first hum, the other column's.
        let placed = [
            (HUMS[1], hum, HUM, 10),
            (QUIET_AT, quiet, QUIET, 20),
            (HUMS[0], hum, HUM, 30),
            (OTHER, hum, HUM, 40),
            (CRATE_AT, boxed, CRATE, 50),
        ];
        for (n, ([x, y, z], state, id, speed)) in placed.into_iter().enumerate() {
            game.world.set(x, y, z, state).unwrap();
            let entity = ModBlockEntity::new(
                id.into(),
                ModBlockEntityFields {
                    nbt: rpm(speed),
                    ..Default::default()
                },
            );
            game.world
                .block_entities
                .insert((x, y, z), BlockEntity::Mod(Box::new(entity)));
            if n == 0 {
                // A spawner that sees a player at any range: two particles each client tick.
                let mut pig = Compound::new();
                pig.insert("id".into(), Tag::String("minecraft:pig".into()));
                let spawner = sunrise_level::chunk::block_entity::Spawner {
                    required_player_range: -1,
                    next: Some(sunrise_level::chunk::block_entity::Next::Own(
                        sunrise_level::chunk::block_entity::SpawnData {
                            entity: pig,
                            ..Default::default()
                        },
                    )),
                    ..Default::default()
                };
                let [x, y, z] = SPAWNER_AT;
                let state = registry.default_state("minecraft:spawner").unwrap();
                game.world.set(x, y, z, state).unwrap();
                game.world
                    .block_entities
                    .insert((x, y, z), BlockEntity::Spawner(Box::new(spawner)));
            }
        }
        // The player the spawner looks for (`hasNearbyAlivePlayer` needs one, whatever the range).
        let [x, y, z] = SPAWNER_AT.map(f64::from);
        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::new(x, y + 2.0, z),
            )
            .unwrap(),
        );
        take_events();

        // Each declared copy once per tick, in arrival order, inside the level's brackets; the
        // quiet type never. The spawner ticks between the second hum and the first, as it arrived:
        // its two particles come between their ticks.
        let all = |first| ticked(&[(HUMS[1], 10), (HUMS[0], first), (OTHER, 40), (CRATE_AT, 50)]);
        game.client.particles.0.requested = Some(Vec::new());
        TICK_PARTICLES.lock().unwrap().clear();
        assert_eq!(frame(game, 1), all(30));
        let seen = std::mem::take(&mut *TICK_PARTICLES.lock().unwrap());
        assert_eq!([seen[1] - seen[0], seen[2] - seen[1]], [2, 0], "{seen:?}");
        game.client.particles.0.requested = None;
        assert_eq!(frame(game, 2), [all(30), all(30)].concat());

        // The server's update reaches the next frame's drain: the notice comes first, and both
        // of that frame's ticks see the merged data directly from the resident block entity.
        server
            .send(
                &play::Clientbound::BlockEntityData(play::BlockEntityData {
                    pos: HUMS[0],
                    kind: 0,
                    tag: Tag::Compound(rpm(31)),
                })
                .encode(),
            )
            .unwrap();
        // The frame whose drain (`update_world`) handles it, once the connection has it.
        let deadline = Instant::now() + Duration::from_secs(10);
        while EVENTS.lock().unwrap().is_empty() {
            assert!(Instant::now() < deadline, "the update did not arrive");
            std::thread::sleep(Duration::from_millis(1));
            game.client_update_world();
        }
        game.client_tick_steps(2, None).unwrap();
        let data = format!("data {:?} 31", HUMS[0]);
        assert_eq!(take_events(), [vec![data], all(31), all(31)].concat());

        // Paused, the client tick runs but not the level; frozen, the level runs but not its
        // block entities.
        game.pause = true;
        assert_eq!(
            frame(game, 1),
            ["client_pre", "client_post"].map(str::to_owned)
        );
        game.pause = false;
        game.clock.set_frozen(true);
        assert_eq!(frame(game, 1), ticked(&[]));
        game.clock.set_frozen(false);

        // A copy removed is not told again; its column's other hum still is.
        let [x, y, z] = HUMS[1];
        game.world.block_entities.remove(&(x, y, z));
        assert_eq!(
            frame(game, 1),
            ticked(&[(HUMS[0], 31), (OTHER, 40), (CRATE_AT, 50)])
        );

        // The server forgets the column: its hums are not told again, the other column's is.
        game.forget_remote_columns(&mut [CHUNK].into_iter().collect(), true);
        for _ in 0..2 {
            assert_eq!(frame(game, 1), ticked(&[(OTHER, 40), (CRATE_AT, 50)]));
        }

        // The level going leaves nothing to tell.
        game.forget_all_remote_columns();
        assert_eq!(frame(game, 1), ticked(&[]));
    });
}
