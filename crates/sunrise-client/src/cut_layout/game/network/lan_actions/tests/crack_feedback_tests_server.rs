use crate::game::lan_actions::crack_feedback_tests::cracks;
use crate::game::lan_actions::crack_feedback_tests::observer;
use std::sync::Arc;
use sunrise_gameplay_common::game::Hit;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::packets::play;
use sunrise_protocol::packets::play::Serverbound as S;
use sunrise_sim::lan::{NetConfig, hub::HubConfig};
use sunrise_world::registry::BlockStateId;
use {
    crate::game::load_resources, crate::settings::Settings, sunrise_gameplay_common::game::Options,
    sunrise_gameplay_common::game::Progress,
};

pub(crate) fn run_crack_packets_follow_start_clock_and_strict_observer_radius() {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role =
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = sunrise_gameplay::game::ServerGame::new(
        &loaded.resources,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    let scratch =
        std::env::temp_dir().join(format!("sunrise-crack-feedback-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    game.server.world_root = scratch.clone();
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let pos = [0, 100, 0];
    let stone = BlockStateId(
        game.res
            .facts()
            .states
            .iter()
            .position(|f| &*f.name == "minecraft:stone")
            .unwrap() as u32,
    );
    game.world.set(pos[0], pos[1], pos[2], stone).unwrap();
    game.lan_host(
        sunrise_gameplay::game::server_side::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            listen: false,
            announce: false,
            ..NetConfig::loopback(0, "crack-test".into(), 6)
        },
        HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: "minecraft:overworld".into(),
            allow_flight: false,
            seed: 0,
            max_players: 6,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        },
    )
    .unwrap();
    let mut receivers = Vec::new();
    for (index, (position, dimension)) in [
        ([0.5, 100.0, 0.5], "minecraft:overworld"),
        ([31.999, 100.0, 0.0], "minecraft:overworld"),
        ([32.0, 100.0, 0.0], "minecraft:overworld"),
        ([40.0, 100.0, 0.0], "minecraft:overworld"),
        ([0.0, 100.0, 0.0], "minecraft:the_nether"),
    ]
    .into_iter()
    .enumerate()
    {
        let (guest, rx) = observer(index as u32 + 1, position, dimension);
        game.server.lan.as_mut().unwrap().hub.guests.push(guest);
        receivers.push(rx);
    }
    let action = |action| {
        S::PlayerAction(play::PlayerAction {
            action,
            pos,
            direction: 1,
            sequence: 1,
        })
    };
    game.server.lan.as_mut().unwrap().hub.ticks = 100;
    game.lan_guest_packet(1, action(0));
    let mut trace = cracks(&receivers[1]);
    assert_eq!(trace, [(1, pos, 0)], "START sends stage zero immediately");
    for tick in 101..=103 {
        game.server.lan.as_mut().unwrap().hub.ticks = tick;
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.inventory[0] = (tick != 102).then(|| ItemStack::new("minecraft:diamond_pickaxe", 1))
        });
        game.as_guest(1, |g| {
            g.server.forced_hit = Some(Hit {
                inside: false,
                pos,
                face: [0, 1, 0],
                point: [0.5, 100.5, 0.5],
            });
            g.tick_mining().unwrap();
            g.server.forced_hit = None;
        });
        game.guest_dig_feedback(1);
        game.guest_dig_feedback(1);
    }
    // Stone: hand 1/1.5/100; diamond pickaxe 8/1.5/30. Each tick
    // recomputes current speed against elapsed START time, rather than summing it.
    let ticks = cracks(&receivers[1]);
    assert_eq!(ticks, [(1, pos, 3), (1, pos, 0), (1, pos, 7)]);
    trace.extend(ticks);
    game.lan_guest_packet(1, action(3));
    let stop = cracks(&receivers[1]);
    assert_eq!(stop, [(1, pos, 255)], "accepted STOP clears the crack");
    trace.extend(stop);
    game.world.set(pos[0], pos[1], pos[2], stone).unwrap();
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(0, |g| g.inventory[0] = None);
    game.lan_guest_packet(1, action(0));
    game.lan_guest_packet(1, action(2));
    let abort = cracks(&receivers[1]);
    assert_eq!(
        abort,
        [(1, pos, 0), (1, pos, 255)],
        "ABORT clears the crack"
    );
    trace.extend(abort);
    // A restricted START must not publish the previous dig's mining state for
    // a different target, nor replace its START clock.
    game.lan_guest_packet(1, action(0));
    assert_eq!(cracks(&receivers[1]), [(1, pos, 0)]);
    let start = game.server.lan.as_ref().unwrap().play[&1].dig_start;
    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.game_mode = 2;
        g.abilities.update_for_mode(2);
    });
    let other = [1, 100, 0];
    game.world.set(other[0], other[1], other[2], stone).unwrap();
    game.lan_guest_packet(
        1,
        S::PlayerAction(play::PlayerAction {
            action: 0,
            pos: other,
            direction: 1,
            sequence: 2,
        }),
    );
    assert!(cracks(&receivers[1]).is_empty());
    assert_eq!(
        game.server.lan.as_ref().unwrap().play[&1]
            .digging
            .map(|(pos, _)| pos),
        Some(pos)
    );
    assert_eq!(game.server.lan.as_ref().unwrap().play[&1].dig_start, start);
    game.lan_guest_packet(1, action(2));
    assert_eq!(cracks(&receivers[1]), [(1, pos, 255)]);
    game.lan_guest_packet(1, action(0));
    assert!(cracks(&receivers[1]).is_empty(), "fresh restricted START");
    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.game_mode = 1;
        g.abilities.update_for_mode(1);
    });
    game.lan_guest_packet(1, action(0));
    assert!(cracks(&receivers[1]).is_empty(), "creative START");
    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.game_mode = 0;
        g.abilities.update_for_mode(0);
    });
    let torch = BlockStateId(
        game.res
            .facts()
            .states
            .iter()
            .position(|f| &*f.name == "minecraft:torch")
            .unwrap() as u32,
    );
    game.world.set(pos[0], pos[1], pos[2], torch).unwrap();
    game.lan_guest_packet(1, action(0));
    assert!(cracks(&receivers[1]).is_empty(), "instant START");
    for (index, rx) in receivers.iter().enumerate() {
        assert!(
            cracks(rx).is_empty(),
            "unexpected crack for observer {index}"
        );
    }
    if let Some(path) = std::env::var_os("SUNRISE_CRACK_TRACE") {
        let stages: Vec<_> = trace.iter().map(|(_, _, stage)| *stage).collect();
        std::fs::write(path, serde_json::to_vec(&stages).unwrap()).unwrap();
    }
    drop(game);
    std::fs::remove_dir_all(scratch).unwrap();
}
