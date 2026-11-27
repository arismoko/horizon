use std::sync::Arc;
use sunrise_gameplay::game::lan_actions::tick::dig_tick_order_tests::DELAYED;
use sunrise_gameplay::game::lan_actions::tick::dig_tick_order_tests::Event;
use sunrise_gameplay::game::lan_actions::tick::dig_tick_order_tests::Observe;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::Dim;
use sunrise_level::chunk::Chunk;
use sunrise_sim::lan::guest::SPECTATOR;
use {
    sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
    sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
};

#[test]
#[ignore = "daily"]
fn advancement_criteria_straddle_the_player_body_even_with_entity_cancel() {
    use sunrise_level::effects::EffectInstance;
    use sunrise_sim::advancements::Tree;
    use sunrise_sim::lan::{ServerAdvancements, data::SyncData};
    use sunrise_world::loot::PackOverlay;

    let _publication = sunrise_sim::datapack_net::isolate_server_publication();
    let heard = Arc::default();
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "observe",
            Box::new(Observe {
                events: Arc::clone(&heard),
                cancel: 23,
                check_delayed_destroy: false,
            }),
        )]
        .into(),
        || {
            with_game(|game| {
                game.server.world_root = game.options.game_dir.clone().unwrap();
                // Load the world's tables before installing the test trees. Otherwise
                // the first tick's scheduled functions replace the game's tree.
                game.open_world_datapacks(&[], None).unwrap();
                open_to_lan(game);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));

                let pack = tempfile::tempdir().unwrap();
                let data = pack.path().join("data");
                let advancements = data.join("test/advancement");
                std::fs::create_dir_all(&advancements).unwrap();
                std::fs::write(
                    advancements.join("phase.json"),
                    br#"{
                "criteria": {
                    "tick": {"trigger": "minecraft:tick"},
                    "levitation": {"trigger": "minecraft:levitation"},
                    "location": {"trigger": "minecraft:location"}
                },
                "requirements": [["tick"], ["levitation"], ["location"]]
            }"#,
                )
                .unwrap();
                game.adv.tree = Arc::new(Tree::load(&data));
                // This unconditional criterion is only in the hub's tree, so it
                // must go through the staged native first-tick grant path.
                std::fs::write(
                    advancements.join("staged.json"),
                    br#"{
                "criteria": {"staged": {"trigger": "minecraft:tick"}},
                "requirements": [["staged"]]
            }"#,
                )
                .unwrap();
                let mut sync = SyncData::empty();
                sync.advancements =
                    ServerAdvancements::load(pack.path(), &PackOverlay::NONE, Default::default());
                game.with_advancement_hub(|hub, owners| {
                    hub.reload_resources(Arc::new(sync), owners)
                });

                let levitation = sunrise_level::brewing::effect("levitation").unwrap().id;
                for (conn, ticks) in [(1, 19), (4, 20)] {
                    let mut player = guest(conn);
                    player.dimension = game.dim.id().into();
                    player.loaded = true;
                    player.position = [5.5 + f64::from(conn), 64.0, 8.5];
                    game.server.lan.as_mut().unwrap().hub.guests.push(player);
                    game.as_guest(conn, |g| {
                        g.player.as_mut().unwrap().health.tick_count = ticks;
                        g.potions
                            .effects
                            .add(EffectInstance::new(levitation, 100, 0));
                    })
                    .unwrap();
                }
                game.tick_server(false).unwrap();

                let host = game.server.lan.as_ref().unwrap();
                assert!(host.hub.guest(1).unwrap().advancements_sent);
                assert!(!host.hub.guest(4).unwrap().advancements_sent);
                assert!(host.hub.native_advancement_grants.is_empty());
                use Event::*;
                let events = heard.lock().unwrap();
                let position = |event: &Event| {
                    events
                        .iter()
                        .position(|e| e == event)
                        .unwrap_or_else(|| panic!("missing {event:?}; saw {events:?}"))
                };
                let post_entity = position(&EntityPost(20));
                for criterion in ["tick", "levitation", "staged"] {
                    assert!(position(&Criterion(123, criterion.into())) < post_entity);
                    assert!(!events.contains(&Criterion(126, criterion.into())));
                }
                for player in [123, 126] {
                    let pre = events
                        .iter()
                        .position(|e| matches!(e, PlayerPre(uuid, ..) if *uuid == player))
                        .unwrap();
                    let post = position(&PlayerPost(player));
                    assert!(post_entity < pre && pre < post);
                    assert!(post < position(&Criterion(player, "location".into())));
                }
            })
        },
    );
}

#[test]
#[ignore = "daily"]
fn player_tick_hooks_observe_advanced_dig_progress_and_skipped_body_still_ticks_game_mode() {
    let heard = Arc::default();
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "observe",
            Box::new(Observe {
                events: Arc::clone(&heard),
                cancel: 23,
                check_delayed_destroy: true,
            }),
        )]
        .into(),
        || {
            with_game(|game| {
                game.server.world_root = game.options.game_dir.clone().unwrap();
                open_to_lan(game);
                game.build_registries();
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let start = 20_000;
                game.server.lan.as_mut().unwrap().hub.ticks = start;
                game.native_level_opened(Dim::Overworld);
                for (dim, conns) in [(Dim::Overworld, [1, 3]), (Dim::Nether, [2, 4])] {
                    game.enter_level(dim).unwrap();
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    if dim == Dim::Overworld {
                        let [x, y, z] = DELAYED;
                        game.world.set(x, y, z, stone).unwrap();
                    }
                    for conn in conns {
                        let mut player = guest(conn);
                        player.dimension = dim.id().into();
                        player.loaded = true;
                        let spec = conn == 3;
                        let x = if spec { 15 } else { 5 + conn as i32 };
                        player.position = [f64::from(x) + 0.5, 64.0, 8.5];
                        if spec {
                            player.game_mode = SPECTATOR;
                            assert!(!game.guest_body_ticks(&player));
                        }
                        let pos = [x, 63, 8];
                        game.world.set(pos[0], pos[1], pos[2], stone).unwrap();
                        let host = game.server.lan.as_mut().unwrap();
                        host.hub.guests.push(player);
                        let play = host.play.entry(conn).or_default();
                        if spec {
                            play.destroy_delay = 5;
                        } else {
                            play.arm_mining_for_test(
                                pos,
                                start,
                                (conn == 1).then_some((DELAYED, 0)),
                            );
                        }
                        game.as_guest(conn, |game| {
                            let health = &mut game.player.as_mut().unwrap().health;
                            health.tick_count = 10;
                            health.invulnerable_time = 8;
                            health.hurt.cooldown = 5;
                        })
                        .unwrap();
                    }
                }
                game.enter_level(Dim::Overworld).unwrap();
                game.tick_server(false).unwrap();

                let host = game.server.lan.as_ref().unwrap();
                for (conn, count) in [(1, 11), (2, 11), (3, 11), (4, 10)] {
                    assert_eq!(
                        host.hub
                            .guest(conn)
                            .unwrap()
                            .motion
                            .as_ref()
                            .unwrap()
                            .health
                            .tick_count,
                        count
                    );
                }
                for conn in [1, 2] {
                    let mining = host.play[&conn].mining.as_ref().unwrap();
                    assert_eq!(mining.ticks, 2);
                    assert!(mining.progress > 0.0);
                }
                assert_eq!(host.play[&4].mining.as_ref().unwrap().ticks, 1);
                assert_eq!(host.play[&3].destroy_delay, 4);
                use Event::*;
                assert_eq!(
                    *heard.lock().unwrap(),
                    [
                        EntityPre(20, Some(8)),
                        Dig(123, 0),
                        EntityPost(20),
                        EntityPre(22, Some(8)),
                        EntityPost(22),
                        LevelPost("minecraft:overworld"),
                        EntityPre(21, Some(8)),
                        Dig(124, 0),
                        EntityPost(21),
                        EntityPre(23, Some(8)),
                        LevelPost("minecraft:the_nether"),
                        PlayerPre(123, "minecraft:overworld", Some(7), Some(4)),
                        PlayerPost(123),
                        PlayerPre(124, "minecraft:the_nether", Some(7), Some(4)),
                        PlayerPost(124),
                        PlayerPre(126, "minecraft:the_nether", Some(8), Some(5)),
                        PlayerPost(126),
                    ],
                );
            })
        },
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
